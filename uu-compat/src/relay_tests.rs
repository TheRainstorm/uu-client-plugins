//! Owned loopback TURN; relay-only candidates make direct success impossible.
use std::{net::{IpAddr,SocketAddr},sync::{Arc,atomic::{AtomicUsize,Ordering}},time::Duration};
use tokio::{net::UdpSocket,sync::{mpsc,watch},time::timeout};
use uu_core::{direct_ice::InternetIce,internet::Transport,native_signaling::SessionSignaling};
use webrtc::{turn::{auth::{AuthHandler,generate_auth_key},server::{Server,config::{ServerConfig,ConnConfig}},relay::relay_static::RelayAddressGeneratorStatic},util::vnet::net::Net};

pub(crate) struct Turn {server:Server,pub room:serde_json::Value,requests:Arc<AtomicUsize>}
struct Auth(Arc<AtomicUsize>);
impl AuthHandler for Auth {
    fn auth_handle(&self,user:&str,realm:&str,_source:SocketAddr)->Result<Vec<u8>,webrtc::turn::Error> {
        if user!="generated-user" {return Err(webrtc::turn::Error::ErrFakeErr);}
        self.0.fetch_add(1,Ordering::Relaxed);
        Ok(generate_auth_key(user,realm,"generated-password"))
    }
}
impl Turn {
    pub async fn new()->Self {
        let socket=Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let address=socket.local_addr().unwrap();let requests=Arc::new(AtomicUsize::new(0));
        let server=Server::new(ServerConfig {
            conn_configs:vec![ConnConfig{conn:socket,relay_addr_generator:Box::new(RelayAddressGeneratorStatic {
                relay_address:IpAddr::from([127,0,0,1]),address:"127.0.0.1".into(),net:Arc::new(Net::new(None)),
            })}],realm:"generated-relay".into(),auth_handler:Arc::new(Auth(requests.clone())),
            channel_bind_timeout:Duration::ZERO,alloc_close_notify:None,
        }).await.unwrap();
        Self{server,requests,room:serde_json::json!({"force_relay":true,"iceServers":[{
            "urls":[format!("turn:{address}?transport=udp")],"username":"generated-user","credential":"generated-password"
        }]})}
    }
    pub fn ice(&self)->InternetIce {crate::ice::from_room(&self.room,true).unwrap()}
    pub async fn close(self) {assert!(self.requests.load(Ordering::Relaxed)>=2);self.server.close().await.unwrap();}
}
pub(crate) fn relay_only(sdp:&str) {
    let candidates:Vec<_>=sdp.lines().filter(|l|l.starts_with("a=candidate:")).collect();
    assert!(!candidates.is_empty());
    assert!(candidates.iter().all(|l|l.split_ascii_whitespace().nth(7)==Some("relay")),"direct candidate escaped relay-only fixture");
}
struct Signal {send:mpsc::Sender<serde_json::Value>,receive:mpsc::Receiver<serde_json::Value>}
#[async_trait::async_trait]
impl SessionSignaling for Signal {
    async fn send(&mut self,value:serde_json::Value)->anyhow::Result<()> {
        if let Some(sdp)=value["sdp"].as_str(){relay_only(sdp);}
        self.send.send(value).await?;Ok(())
    }
    async fn receive(&mut self)->anyhow::Result<serde_json::Value>{self.receive.recv().await.ok_or_else(||anyhow::anyhow!("fixture signaling ended"))}
}
#[async_trait::async_trait]
impl Transport for Signal {
    async fn send(&mut self,value:serde_json::Value)->anyhow::Result<()> {SessionSignaling::send(self,value).await}
    async fn receive(&mut self)->anyhow::Result<serde_json::Value>{SessionSignaling::receive(self).await}
    async fn close(&mut self){self.receive.close();}
}
#[test]
fn relay_policy_filters_all_url_shapes_and_remote_candidates_without_credentials_in_debug() {
    let room=serde_json::json!({"data":{"iceServers":[
        {"urls":["stun:stun.example.invalid:3478","turn:turn.example.invalid:3478?transport=udp","turns:turn.example.invalid:5349"],"username":"generated-user","credential":"generated-password"},
        {"url":"turn:second.example.invalid:3478","username":"generated-user","credential":"generated-password"}
    ]}});
    let off=crate::ice::from_room(&room,false).unwrap();
    assert_eq!(off.configuration().ice_servers.len(),1);
    let on=crate::ice::from_room(&room,true).unwrap();
    assert_eq!(on.configuration().ice_servers.len(),3);
    assert!(!format!("{on:?}").contains("generated-password"));
    let relay="candidate:1 1 udp 1 127.0.0.1 12345 typ relay";
    let host="candidate:2 1 udp 1 127.0.0.1 12346 typ host";
    let sdp=format!("v=0\r\na={relay}\r\na={host}\r\n");
    assert!(!crate::ice::candidate_allowed(relay,false));
    assert!(crate::ice::candidate_allowed(host,false));
    assert!(!crate::ice::remote_sdp(&sdp,false).contains("typ relay"));
    assert_eq!(crate::ice::remote_sdp(&sdp,true),sdp);
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn native_generated_media_crosses_turn_with_no_direct_candidates() {
    crate::install_crypto_provider();
    let turn=Turn::new().await;
    assert!(crate::ice::from_room(&turn.room,false).is_err());
    let (a,ar)=mpsc::channel(8);let(b,br)=mpsc::channel(8);
    let mut host_signal=Signal{send:a,receive:br};let mut viewer_signal=Signal{send:b,receive:ar};
    let (stop,receiver)=watch::channel(false);let ice=turn.ice();let host_ice=ice.clone();
    let host_stop=receiver.clone();
    let host=tokio::spawn(async move {
        uu_core::native::serve_direct(&mut host_signal,&host_ice,uu_core::native::SharingConfig {
            encoder_tuning:Default::default(),session:None,video_policy:None,
            codec:uu_proto::media::VideoCodec::H264,rate_control:uu_media::encoder::RateControl::Cbr,
            source:uu_capture::CaptureSource::Synthetic{width:320,height:180,fps:60},fps:60,bitrate_kbps:2000,
            encoder:uu_media::encoder::Preference::Software,session_limit:None,allow_input:false,audio:None,clipboard:None,
        },host_stop,None).await
    });
    let stats=Arc::new(uu_core::media::MediaStats::default());let _=stats.decoder_preference.set(uu_core::media::DecoderKind::Software);
    let observed=stats.clone();let(frames,mut received)=uu_core::media::frame_channel();
    let viewer=tokio::spawn(async move{uu_core::native::view_direct(&mut viewer_signal,&ice,frames,observed,receiver,None).await});
    timeout(Duration::from_secs(20),async {
        while stats.decoded_total.load(Ordering::Acquire)<10 {
            assert!(!host.is_finished()&&!viewer.is_finished());
            while received.try_recv().is_ok(){}tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }).await.unwrap();
    assert_eq!(stats.errors.load(Ordering::Relaxed),0);
    stop.send_replace(true);
    timeout(Duration::from_secs(6),viewer).await.unwrap().unwrap().unwrap();
    let report=timeout(Duration::from_secs(6),host).await.unwrap().unwrap().unwrap();
    assert!(report.sent>=10);assert_eq!(report.input_events,0);
    turn.close().await;
}
