//! Generated media over real DTLS/SRTP; never opens a desktop or OS input.
use super::{session::Session,wire::ConnectOptions,Settings};
use std::{sync::{Arc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant}};
use tokio::{sync::{mpsc,watch},time::timeout};
use uu_core::{internet::{Connection,Transport},plugins::{HostContext,ProtocolSession},shared_host::SessionGate};

struct Signaling {receive:mpsc::Receiver<serde_json::Value>,send:mpsc::Sender<serde_json::Value>,closed:Arc<AtomicBool>}
impl Drop for Signaling {fn drop(&mut self){self.closed.store(true,Ordering::Release);}}
#[async_trait::async_trait]
impl Transport for Signaling {
    async fn receive(&mut self)->anyhow::Result<serde_json::Value>{self.receive.recv().await.ok_or_else(||anyhow::anyhow!("fixture signaling closed"))}
    async fn send(&mut self,value:serde_json::Value)->anyhow::Result<()>{self.send.send(value).await?;Ok(())}
    async fn close(&mut self){self.closed.store(true,Ordering::Release);self.receive.close();}
}
fn context(gate:Arc<SessionGate>,stop:watch::Receiver<bool>)->HostContext {
    HostContext {sharing:uu_core::native::SharingConfig {encoder_tuning:Default::default(),session:None,video_policy:None,
        codec:uu_proto::media::VideoCodec::H264,rate_control:uu_media::encoder::RateControl::Cbr,
        source:uu_capture::CaptureSource::Synthetic{width:320,height:180,fps:60},fps:60,bitrate_kbps:2000,
        encoder:uu_media::encoder::Preference::Software,session_limit:None,allow_input:false,audio:None,clipboard:None},
        sessions:gate,stop,progress:watch::channel(uu_core::native::HostStatus::Starting).0,clipboard:None}
}
fn connection()->(Connection,mpsc::Sender<serde_json::Value>,mpsc::Receiver<serde_json::Value>,Arc<AtomicBool>){
    let (tx,receive)=mpsc::channel(8);let(send,rx)=mpsc::channel(8);let closed=Arc::new(AtomicBool::new(false));
    (Connection{session:"generated-session".into(),ice:uu_core::direct_ice::InternetIce::from_servers(vec![],false,false).unwrap(),
        transport:Box::new(Signaling{receive,send,closed:closed.clone()})},tx,rx,closed)
}
fn session(connection:Connection,enabled:bool,created:Instant)->Box<Session>{
    Box::new(Session::new(connection,ConnectOptions{fps:60,bounds:Some([320,180])},Settings{enabled,..Default::default()},"generated-controller".into(),created))
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn official_generated_media_crosses_turn_with_no_direct_candidates() {
    crate::install_crypto_provider();
    let turn=crate::relay_tests::Turn::new().await;
    let stats=Arc::new(uu_core::media::MediaStats::default());
    let _=stats.decoder_preference.set(uu_core::media::DecoderKind::Software);
    stats.audio.output_enabled.store(false,Ordering::Release);
    let(frames,_)=uu_core::media::frame_channel();
    let media=crate::media::ControllerMedia::new_with_ice_policy(turn.ice(),Some(frames),Some(stats.clone())).await.unwrap();
    let offer=media.offer().await.unwrap();crate::relay_tests::relay_only(&offer);
    let(mut connection,tx,mut rx,closed)=connection();connection.ice=turn.ice();
    tx.send(serde_json::json!({"type":"offer","sdp":offer})).await.unwrap();
    let(stop,receiver)=watch::channel(false);let gate=Arc::new(SessionGate::default());
    let host=tokio::spawn(session(connection,true,Instant::now()).serve(context(gate.clone(),receiver)));
    let answer=timeout(Duration::from_secs(12),rx.recv()).await.unwrap().unwrap();
    crate::relay_tests::relay_only(answer["sdp"].as_str().unwrap());
    media.set_answer(answer["sdp"].as_str().unwrap()).await.unwrap();
    timeout(Duration::from_secs(12),async {while stats.decoded_total.load(Ordering::Acquire)<10 {assert!(!host.is_finished());tokio::time::sleep(Duration::from_millis(20)).await;}}).await.unwrap();
    let pair=media.pc.sctp().transport().ice_transport().get_selected_candidate_pair().await.unwrap();
    use webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType;
    assert_eq!(pair.local().typ,RTCIceCandidateType::Relay);assert_eq!(pair.remote().typ,RTCIceCandidateType::Relay);
    stop.send_replace(true);let report=timeout(Duration::from_secs(6),host).await.unwrap().unwrap().unwrap();
    assert!(report.sent>=10);assert!(closed.load(Ordering::Acquire));assert!(gate.sessions().is_empty());
    media.pc.close().await.unwrap();turn.close().await;
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn official_generated_h264_echo_quality_and_stop_release_the_shared_slot(){
    crate::install_crypto_provider();
    let gate=Arc::new(SessionGate::default());
    for cancel in [false,true] {
        let stats=Arc::new(uu_core::media::MediaStats::default());
        stats.audio.output_enabled.store(false,Ordering::Release);
        let (frames,_)=uu_core::media::frame_channel();
        let media=crate::media::ControllerMedia::new_with_frames(vec![],Some(frames),Some(stats.clone())).await.unwrap();
        let offer=media.offer().await.unwrap();
        let (connection,tx,mut rx,closed)=connection();
        tx.send(serde_json::json!({"type":"offer","sdp":offer})).await.unwrap();
        let(stop,receiver)=watch::channel(false);
        let mut host_context=context(gate.clone(),receiver);
        host_context.sharing.audio=Some(uu_media::audio_capture::Source::Synthetic);
        let host=tokio::spawn(session(connection,true,Instant::now()).serve(host_context));
        let answer=timeout(Duration::from_secs(10),rx.recv()).await.unwrap().unwrap();
        assert!(!answer["sdp"].as_str().unwrap().contains("x-eclipse-"));
        media.set_answer(answer["sdp"].as_str().unwrap()).await.unwrap();
        timeout(Duration::from_secs(10),async {while stats.decoded_total.load(Ordering::Relaxed)<10 {tokio::time::sleep(Duration::from_millis(20)).await;}}).await.unwrap();
        assert_eq!(gate.sessions().len(),1);
        assert_eq!(gate.active_peer().as_deref(),Some("uu:generated-controller"));
        assert!(gate.administration.lock().unwrap().is_none());
        timeout(Duration::from_secs(3),async {while stats.audio.decoded_samples.load(Ordering::Relaxed)==0 {tokio::time::sleep(Duration::from_millis(20)).await;}}).await.unwrap();
        let before=stats.decoded_total.load(Ordering::Relaxed);
        let request=crate::control_wire::capture_setting_request_with_resolution(901,0,1,6,Some(1_500_000),uu_core::video_options::CaptureResolution::Exact(160,90));
        media.send_control(crate::control_wire::capture_codec(&request,false).unwrap()).await.unwrap();
        timeout(Duration::from_secs(3),async {while stats.settings_response.load(Ordering::Acquire)!=(901<<1) {tokio::time::sleep(Duration::from_millis(20)).await;}}).await.unwrap();
        let request=crate::control_wire::capture_setting_request_with_resolution(902,0,1,6,Some(1_500_000),uu_core::video_options::CaptureResolution::Exact(320,180));
        media.send_control(crate::control_wire::capture_codec(&request,false).unwrap()).await.unwrap();
        timeout(Duration::from_secs(5),async {while stats.decoded_total.load(Ordering::Relaxed)<before+10||stats.settings_response.load(Ordering::Acquire)!=(902<<1|1) {tokio::time::sleep(Duration::from_millis(20)).await;}}).await.unwrap();
        if cancel {host.abort();assert!(host.await.unwrap_err().is_cancelled());}
        else {stop.send_replace(true);let report=timeout(Duration::from_secs(6),host).await.unwrap().unwrap().unwrap();assert!(report.sent>=10);assert_eq!(report.input_events,0);}
        timeout(Duration::from_secs(2),async {while !gate.sessions().is_empty() {tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
        assert!(closed.load(Ordering::Acquire));
        media.pc.close().await.unwrap();drop(tx);
    }
}
#[tokio::test]
async fn disabled_expired_or_full_host_rejects_before_reading_an_offer(){
    let gate=Arc::new(SessionGate::default());let(_stop,receiver)=watch::channel(false);
    for (enabled,created) in [(false,Instant::now()),(true,Instant::now()-Duration::from_secs(31))] {
        let(c,_tx,_rx,closed)=connection();
        assert!(timeout(Duration::from_millis(200),session(c,enabled,created).serve(context(gate.clone(),receiver.clone()))).await.unwrap().is_err());
        assert!(closed.load(Ordering::Acquire));assert!(gate.sessions().is_empty());
    }
    let leases:Vec<_>=(0..uu_core::shared_host::MAX_SESSIONS).map(|_|gate.claim("native-fixture").unwrap()).collect();
    let(c,_tx,_rx,closed)=connection();
    assert!(timeout(Duration::from_millis(200),session(c,true,Instant::now()).serve(context(gate.clone(),receiver))).await.unwrap().is_err());
    assert!(closed.load(Ordering::Acquire));assert_eq!(gate.sessions().len(),leases.len());drop(leases);
}
