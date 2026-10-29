//! Official host protocol over the shared capture/input SDK. No native grants.
use super::{input,wire,Settings};
use anyhow::{ensure,Context,Result};
use std::{collections::HashMap,sync::{Arc,atomic::{AtomicBool,AtomicU64,Ordering}},time::{Duration,Instant}};
use tokio::{sync::{mpsc,watch},time::timeout};
use uu_core::{internet::Connection,plugin_backend::{EncodedCapture,InputSession,VideoSender,OpusSender,AudioCapture},plugins::{HostContext,ProtocolSession}};
use webrtc::{api::{APIBuilder,media_engine::MediaEngine,interceptor_registry::register_default_interceptors},
    data_channel::{RTCDataChannel,data_channel_state::RTCDataChannelState},interceptor::registry::Registry,
    peer_connection::{RTCPeerConnection,peer_connection_state::RTCPeerConnectionState,sdp::session_description::RTCSessionDescription},
    rtp_transceiver::{rtp_codec::{RTCRtpCodecCapability,RTCRtpCodecParameters,RTPCodecType},RTCPFeedback,rtp_transceiver_direction::RTCRtpTransceiverDirection},
    track::track_local::{TrackLocal,track_local_static_rtp::TrackLocalStaticRTP}};

pub(crate) struct Session {connection:Connection,options:wire::ConnectOptions,settings:Settings,client:String,created:Instant}
impl Session {
    pub fn new(connection:Connection,options:wire::ConnectOptions,settings:Settings,client:String,created:Instant)->Self {Self{connection,options,settings,client,created}}
}
enum Event {Open(Arc<RTCDataChannel>),Data(String,bytes::Bytes),Closed(String)}
struct Peer(Arc<RTCPeerConnection>);
impl Drop for Peer {
    fn drop(&mut self) {
        let pc=self.0.clone();
        if let Ok(runtime)=tokio::runtime::Handle::try_current(){runtime.spawn(async move{let _=timeout(Duration::from_secs(1),pc.close()).await;});}
    }
}
struct Clipboard(Option<Arc<uu_core::clipboard::Clipboard>>);
impl Drop for Clipboard {
    fn drop(&mut self){if let Some(c)=&self.0 {uu_core::plugin_backend::stop_clipboard(c);}}
}
struct AudioTask {stop:watch::Sender<bool>,task:Option<tokio::task::JoinHandle<Result<()>>>}
impl AudioTask {
    fn start(kind:uu_media::audio_capture::Source,mut sender:OpusSender)->Result<Self>{
        let mut worker=AudioCapture::start(kind)?;let(stop,mut receiver)=watch::channel(false);
        let task=tokio::spawn(async move {
            let result=async {loop {tokio::select! {
                _=stopped(&mut receiver)=>break,
                frame=worker.next()=>{
                    let frame=frame.context("official audio capture ended")?;
                    timeout(Duration::from_secs(2),sender.send(frame)).await.context("official audio send timed out")??;
                }
            }}Ok(())}.await;
            worker.stop().await;result
        });
        Ok(Self{stop,task:Some(task)})
    }
    async fn stop(&mut self)->Result<()>{
        self.stop.send_replace(true);
        if let Some(mut task)=self.task.take(){
            match timeout(Duration::from_secs(3),&mut task).await {
                Ok(result)=>result.context("official audio worker failed")??,
                Err(_)=>{task.abort();let _=task.await;anyhow::bail!("official audio cleanup timed out");}
            }
        }Ok(())
    }
}
impl Drop for AudioTask {fn drop(&mut self){self.stop.send_replace(true);}}
async fn stopped(receiver:&mut watch::Receiver<bool>) {
    loop {if *receiver.borrow_and_update(){return;}if receiver.changed().await.is_err(){return;}}
}
async fn send(channel:&Arc<RTCDataChannel>,data:Vec<u8>)->Result<()> {
    timeout(Duration::from_millis(200),channel.send(&data.into())).await.context("official control send timed out")??;Ok(())
}
async fn attach(pc:&Arc<RTCPeerConnection>,track:Arc<TrackLocalStaticRTP>)->Result<Arc<webrtc::rtp_transceiver::rtp_sender::RTCRtpSender>> {
    for transceiver in pc.get_transceivers().await {
        if transceiver.kind()==track.kind() && transceiver.mid().is_some() {
            let sender=transceiver.sender().await;
            if sender.track().await.is_none() {
                sender.replace_track(Some(track as Arc<dyn TrackLocal+Send+Sync>)).await?;
                transceiver.set_direction(RTCRtpTransceiverDirection::Sendonly).await;
                return Ok(sender);
            }
        }
    }
    anyhow::bail!("controller has no compatible receiving transceiver")
}
fn h264()->RTCRtpCodecCapability {
    RTCRtpCodecCapability {mime_type:"video/H264".into(),clock_rate:90_000,
        sdp_fmtp_line:"level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=420034".into(),
        rtcp_feedback:vec![RTCPFeedback{typ:"nack".into(),parameter:"pli".into()},RTCPFeedback{typ:"ccm".into(),parameter:"fir".into()}],..Default::default()}
}
#[async_trait::async_trait]
impl ProtocolSession for Session {
    async fn serve(mut self:Box<Self>,mut context:HostContext)->Result<uu_core::native::HostReport> {
        let result=self.run(&mut context).await;
        self.connection.transport.close().await;
        result
    }
}
impl Session {
    async fn run(&mut self,context:&mut HostContext)->Result<uu_core::native::HostReport> {
        ensure!(self.settings.enabled&&self.created.elapsed()<Duration::from_secs(30),"official connection authorization expired or disabled");
        ensure!(!*context.stop.borrow(),"sharing has stopped");
        let lease=context.sessions.claim(&format!("uu:{}",self.client))?;
        let mut revoked=lease.stopped();
        let offer=timeout(Duration::from_secs(3),self.connection.transport.receive()).await??;
        ensure!(offer["type"]=="offer","official offer missing");
        let sdp=offer["sdp"].as_str().filter(|v|v.len()<=256*1024).context("invalid official SDP")?;
        let filtered=crate::ice::remote_sdp(sdp,self.connection.ice.development_relay());
        let sdp=filtered.as_str();
        ensure!(!sdp.lines().any(|l|l.starts_with("a=x-eclipse-")),"native extension cannot enter the official protocol");
        let mut engine=MediaEngine::default();
        engine.register_codec(RTCRtpCodecParameters {capability:h264(),payload_type:96,..Default::default()},RTPCodecType::Video)?;
        let opus=RTCRtpCodecCapability {mime_type:"audio/opus".into(),clock_rate:48_000,channels:2,sdp_fmtp_line:"minptime=10;useinbandfec=1".into(),..Default::default()};
        let audio_source=context.sharing.audio.filter(|_|self.settings.audio&&sdp.lines().any(|l|l.starts_with("m=audio ")&&!l.starts_with("m=audio 0 ")));
        if audio_source.is_some(){engine.register_codec(RTCRtpCodecParameters{capability:opus.clone(),payload_type:111,..Default::default()},RTPCodecType::Audio)?;}
        let registry=register_default_interceptors(Registry::new(),&mut engine)?;
        let mut settings=webrtc::api::setting_engine::SettingEngine::default();
        uu_core::plugin_backend::configure_srtp(&mut settings);
        settings.set_network_types(vec![webrtc::ice::network_type::NetworkType::Udp4,webrtc::ice::network_type::NetworkType::Udp6]);
        let peer=Peer(Arc::new(APIBuilder::new().with_media_engine(engine).with_interceptor_registry(registry).with_setting_engine(settings).build().new_peer_connection(self.connection.ice.configuration()).await?));
        let pc=&peer.0;
        let (events,mut receive)=mpsc::channel(64);
        let fault=Arc::new(AtomicBool::new(false));
        let incoming=events.clone();let failed=fault.clone();
        pc.on_data_channel(Box::new(move |channel| {
            let sender=incoming.clone();let fault=failed.clone();
            Box::pin(async move {
                let label=channel.label().to_owned();
                if !matches!(label.as_str(),"CONTROL_DATA_CHANNEL"|"TEXT_DATA_CHANNEL"){return;}
                let opened=channel.clone();let tx=sender.clone();let bad=fault.clone();
                channel.on_open(Box::new(move||Box::pin(async move {if tx.try_send(Event::Open(opened)).is_err(){bad.store(true,Ordering::Release);}})));
                let tx=sender.clone();let bad=fault.clone();let name=label.clone();
                channel.on_message(Box::new(move |message| {
                    if message.data.len()>64*1024||tx.try_send(Event::Data(name.clone(),message.data)).is_err(){bad.store(true,Ordering::Release);}
                    Box::pin(async {})
                }));
                channel.on_close(Box::new(move||{let _=sender.try_send(Event::Closed(label.clone()));Box::pin(async {})}));
            })
        }));
        let mut capture=None;let mut input=None;
        let mut audio:Option<AudioTask>=None;
        let clipboard=Clipboard(None);
        let mut clipboard=clipboard;
        let mut tasks=tokio::task::JoinSet::<Result<()>>::new();
        let sent=Arc::new(AtomicU64::new(0));let bytes=Arc::new(AtomicU64::new(0));
        let result=async {
            pc.set_remote_description(RTCSessionDescription::offer(sdp.to_owned())?).await?;
            context.sharing.session=None;
            context.sharing.allow_input&=self.settings.control;
            let options=self.options.capture(&context.sharing);
            context.progress.send_replace(uu_core::native::HostStatus::Preparing);
            let source=tokio::select! {
                value=EncodedCapture::open_sdr(context.sharing.clone(),options)=>value?,
                _=stopped(&mut context.stop)=>return Ok(()),
                _=stopped(&mut revoked)=>return Ok(()),
            };
            capture=Some(source);
            let source=capture.as_mut().unwrap();
            input=InputSession::for_capture(source,context.sessions.input.clone(),context.sharing.allow_input,context.sharing.allow_input).await?;
            let shape=source.settings();let refresh=source.keyframe_flag();
            let track=Arc::new(TrackLocalStaticRTP::new(h264(),"video_0".into(),"uu-desktop".into()));
            let video=attach(pc,track.clone()).await?;
            tasks.spawn(async move {
                use webrtc::rtcp::payload_feedbacks::{picture_loss_indication::PictureLossIndication,full_intra_request::FullIntraRequest};
                while let Ok((packets,_))=video.read_rtcp().await {if packets.iter().any(|p|p.as_any().is::<PictureLossIndication>()||p.as_any().is::<FullIntraRequest>()){refresh.store(true,Ordering::Release);}}
                Ok(())
            });
            let mut video=VideoSender::new(track,uu_proto::media::VideoCodec::H264);
            let (video_tx,mut video_rx)=mpsc::channel::<uu_core::plugin_backend::CapturedSample>(2);let count=sent.clone();let size=bytes.clone();
            tasks.spawn(async move {
                while let Some(frame)=video_rx.recv().await {
                    ensure!(frame.current(),"capture generation retired");
                    timeout(Duration::from_secs(2),video.send(&frame)).await.context("official video send timed out")??;
                    count.fetch_add(1,Ordering::Relaxed);size.fetch_add(frame.data.len() as u64,Ordering::Relaxed);
                }Ok(())
            });
            let mut audio_sender=None;
            if audio_source.is_some() {
                let track=Arc::new(TrackLocalStaticRTP::new(opus,"audio_0".into(),"uu-desktop".into()));
                let sender=attach(pc,track.clone()).await?;
                tasks.spawn(async move {while sender.read_rtcp().await.is_ok(){}Ok(())});
                audio_sender=Some(OpusSender::new(track));
            }
            if self.settings.clipboard {if let Some(factory)=&context.clipboard {clipboard.0=Some(factory(&self.connection.session)?);}}
            let answer=pc.create_answer(None).await?;let mut gathering=pc.gathering_complete_promise().await;
            pc.set_local_description(answer).await?;
            timeout(Duration::from_secs(10),gathering.recv()).await.context("official ICE gathering timed out")?;
            let answer=pc.local_description().await.context("official answer missing")?;
            self.connection.transport.send(serde_json::json!({"type":"answer","sdp":answer.sdp})).await?;
            let mut channels=HashMap::<String,Arc<RTCDataChannel>>::new();
            let sequence=AtomicU64::new(2);let mut ready=false;let mut initialized=false;let mut streaming=false;
            let deadline=Instant::now()+Duration::from_secs(30);
            let mut tick=tokio::time::interval(Duration::from_millis(50));tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut protocol=crate::clipboard::Protocol::default();let mut generation=0;let mut last_echo=Instant::now()-Duration::from_secs(2);
            let mut pending_update:Option<(u64,String,Instant,tokio::sync::oneshot::Receiver<std::result::Result<u32,String>>)>=None;
            let next=||sequence.fetch_add(1,Ordering::Relaxed);
            loop {
                ensure!(!fault.load(Ordering::Acquire),"official control queue overflow");
                tokio::select! {
                    _=stopped(&mut context.stop)=>break,
                    _=stopped(&mut revoked)=>break,
                    response=async {match &mut pending_update {Some((_,_,_,receiver))=>receiver.await,None=>std::future::pending().await}}=>{
                        let (id,label,_,_)=pending_update.take().unwrap();
                        if let Some(channel)=channels.get(&label){send(channel,wire::capture_reply(next(),id,response.is_ok_and(|r|r.is_ok()))).await?;}
                    },
                    task=tasks.join_next(),if !tasks.is_empty()=>{task.context("official worker missing")??.context("official media worker ended")?;anyhow::bail!("official media worker stopped");},
                    message=self.connection.transport.receive()=>{
                        let message=message?;
                        if message["type"]=="candidate" {
                            let value=message.get("candidate").context("ICE candidate missing")?;
                            let candidate=if value.is_string(){serde_json::json!({"candidate":value,"sdpMid":message["sdpMid"],"sdpMLineIndex":message["sdpMLineIndex"]})}else{value.clone()};
                            let candidate:webrtc::ice_transport::ice_candidate::RTCIceCandidateInit=serde_json::from_value(candidate)?;
                            if crate::ice::candidate_allowed(&candidate.candidate,self.connection.ice.development_relay()) {
                                pc.add_ice_candidate(candidate).await?;
                            }
                        }
                    },
                    frame=source.next(),if streaming=>{
                        let frame=frame.context("official capture ended")??;
                        ensure!(frame.current(),"capture generation retired");
                        video_tx.try_send(frame).map_err(|_|anyhow::anyhow!("official video queue is full or closed"))?;
                    },
                    event=receive.recv()=>{match event.context("official control event queue closed")? {
                        Event::Open(channel)=>{let label=channel.label().to_owned();ensure!(!channels.contains_key(&label),"duplicate official control channel");channels.insert(label,channel);},
                        Event::Closed(label)=>{channels.remove(&label);if label=="CONTROL_DATA_CHANNEL"{break;}},
                        Event::Data(label,data)=>{
                            let Some(channel)=channels.get(&label) else{continue;};
                            if let Some(request)=crate::control_wire::echo_request_seq(&data) {
                                send(channel,wire::host_echo(next(),Some(request),clipboard.0.is_some())).await?;ready=true;
                                if let Some(c)=&clipboard.0{c.supported.store(crate::control_wire::echo_clipboard_supported(&data),Ordering::Release);}
                            }else if crate::control_wire::echo_matches(&data,1){ready=true;}
                            else if ready&&data.first()==Some(&b'{') {
                                if let Ok(message)=input::parse(&data) {if let Some(input)=&input {
                                    input.request(true);
                                    match message {input::Message::Heartbeat=>input.touch(),input::Message::Commands(commands)=>{for command in commands {input.submit(command);}},input::Message::Text(value)=>{input.text(&value);}}
                                }}
                            }else if ready&&crate::clipboard::is_clipboard(&data) {
                                if let Some(c)=&clipboard.0 {if c.ready()&&c.available.load(Ordering::Acquire) {
                                    let received=protocol.receive(&data,&sequence)?;
                                    if let Some(text)=received.text{*c.remote.lock().unwrap()=Some(text.into());c.received.fetch_add(1,Ordering::Relaxed);}
                                    for reply in received.replies {send(channel,reply).await?;}
                                }}
                            }else if ready {
                                if let Some(request)=match wire::capture_request(&data,source.settings()) {
                                    Ok(request)=>request,
                                    Err(_)=>{if let Some(id)=wire::capture_request_id(&data){send(channel,wire::capture_reply(next(),id,false)).await?;}continue;},
                                } {
                                    let requested=request.bounds.map(|bounds|uu_capture::scale::dimensions((shape.width,shape.height),Some(bounds)));
                                    let accepted=pending_update.is_none()&&requested.is_none_or(|size|size==(shape.width,shape.height));
                                    if accepted {pending_update=Some((request.id,label,Instant::now(),source.update(request.update)?));}
                                    else {send(channel,wire::capture_reply(next(),request.id,false)).await?;}
                                }
                            }
                        }
                    }},
                    _=tick.tick()=>{
                        let state=pc.connection_state();
                        ensure!(audio.as_ref().is_none_or(|a|!a.task.as_ref().is_some_and(|t|t.is_finished())),"official audio worker stopped");
                        ensure!(!matches!(state,RTCPeerConnectionState::Failed|RTCPeerConnectionState::Closed|RTCPeerConnectionState::Disconnected),"official peer disconnected");
                        ensure!(streaming||Instant::now()<deadline,"official media initialization timed out");
                        ensure!(pending_update.as_ref().is_none_or(|(_,_,at,_)|at.elapsed()<Duration::from_secs(3)),"official capture update timed out");
                        if let Some(channel)=channels.get("CONTROL_DATA_CHANNEL") {
                            if channel.ready_state()==RTCDataChannelState::Open&&!ready&&last_echo.elapsed()>=Duration::from_secs(1){send(channel,wire::host_echo(1,None,clipboard.0.is_some())).await?;last_echo=Instant::now();}
                        }
                        if ready&&!initialized {if let Some(channel)=channels.get("TEXT_DATA_CHANNEL") {
                            send(channel,wire::screen_inventory(next(),shape.width,shape.height,shape.fps)).await?;
                            send(channel,wire::capture_started(next())).await?;initialized=true;
                        }}
                        if !streaming&&ready&&state==RTCPeerConnectionState::Connected {
                            streaming=true;source.request_keyframe();
                            if let (Some(kind),Some(mut sender))=(audio_source,audio_sender.take()) {
                                audio=Some(AudioTask::start(kind,sender)?);
                            }
                            let status=uu_core::native::HostStatus::Streaming {width:shape.width,height:shape.height,fps:shape.fps,audio:if audio_source.is_some(){"Opus"}else{"disabled"}.into(),encoder:source.backend()};
                            lease.progress(status.clone());context.progress.send_replace(status);
                        }
                        if let Some(c)=&clipboard.0 {
                            let current=c.generation.load(Ordering::Acquire);
                            if current!=generation {generation=current;protocol.cancel_pending();}
                            c.active.store(ready,Ordering::Release);
                            if ready&&c.ready()&&c.available.load(Ordering::Acquire)&&c.supported.load(Ordering::Acquire) {
                                if let Some(channel)=channels.get("TEXT_DATA_CHANNEL") {
                                    let value=c.local.lock().unwrap().take();
                                    if let Some(value)=value {if let Some(text)=value.as_text(){send(channel,crate::clipboard::text_change(next(),text)?).await?;c.sent.fetch_add(1,Ordering::Relaxed);}}
                                }
                            }
                        }
                    },
                }
            }
            Ok(())
        }.await;
        if let Some(input)=&input {input.request(false);input.stop().await;}
        drop(clipboard);
        let audio_result=if let Some(audio)=&mut audio {audio.stop().await}else{Ok(())};
        let _=timeout(Duration::from_secs(1),pc.close()).await;
        tasks.abort_all();while tasks.join_next().await.is_some(){}
        let mut report=capture.as_ref().map_or_else(Default::default,EncodedCapture::report);
        if let Some(capture)=&mut capture {timeout(Duration::from_secs(4),capture.stop()).await.context("official capture cleanup timed out")??;}
        report.sent=sent.load(Ordering::Relaxed);report.bytes=bytes.load(Ordering::Relaxed);
        if let Some(input)=&input {(report.input_events,report.input_errors)=input.counters();}
        result?;audio_result?;Ok(report)
    }
}
