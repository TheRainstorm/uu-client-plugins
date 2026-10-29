//! Session routing above one UU room socket. Native authentication remains per peer.
use super::*;
use std::collections::HashMap;
use tokio::sync::mpsc;

pub struct Listener { incoming: mpsc::Receiver<uu_core::plugins::Incoming>, task: tokio::task::JoinHandle<()> }
struct Route { target: String, ctrl: String, incoming: mpsc::Sender<Value>,official:bool }
struct Pending {
    session:Option<String>,client:Option<String>,control:Option<String>,ice:InternetIce,
    created:std::time::Instant,options:Option<crate::official::wire::ConnectOptions>,
}
fn identity(header:&Value,key:&str)->Option<String> {
    let outer=header.get(key).and_then(Value::as_str);
    let inner=header.get("data").and_then(|v|v.get(key)).and_then(Value::as_str);
    if outer.zip(inner).is_some_and(|(a,b)|a!=b) {return None;}
    outer.or(inner).filter(|s|!s.is_empty()&&s.len()<=128).map(str::to_owned)
}
impl Pending {
    fn matches(&self,session:&str,client:&str,control:&str)->bool {
        self.created.elapsed()<Duration::from_secs(30)
            && self.session.as_deref().is_none_or(|s|s==session)
            && self.client.as_deref().is_none_or(|s|s==client)
            && self.control.as_deref().is_none_or(|s|s==control)
    }
    fn official_bound(&self,session:&str,client:&str,control:&str)->bool {
        self.session.as_deref()==Some(session)&&self.client.as_deref()==Some(client)
            &&self.control.as_deref()==Some(control)&&self.matches(session,client,control)
    }
}
enum Outgoing { Signal(String,Value), Close(String) }
impl Listener {
    pub async fn online(config:&Config, development_relay:bool)->Result<Self> {
        let mut host=Host::online(config,development_relay).await?;
        let development_relay=host.development_relay;
        Ok(Self::with_settings(host.socket.take().context("UU room socket unavailable")?,development_relay,crate::official::Settings::load(config)?))
    }
    #[cfg(test)]
    fn from_socket(socket:SocketIo,development_relay:bool)->Self {
        Self::with_settings(socket,development_relay,crate::official::Settings::default())
    }
    fn with_settings(mut socket:SocketIo,development_relay:bool,settings:crate::official::Settings)->Self {
        let (outgoing,mut requests)=mpsc::channel(128);
        let (accepted,incoming)=mpsc::channel(crate::shared_host::MAX_SESSIONS);
        let task=tokio::spawn(async move {
            let mut pending:Vec<Pending>=Vec::new();
            let mut sessions=HashMap::<String,Route>::new();
            loop {
                tokio::select! {
                    request=requests.recv()=>{
                        match request {
                            Some(Outgoing::Signal(session,envelope))=>{
                                let Some(route)=sessions.get(&session) else{continue;};
                                let sdp=if route.official {envelope["sdp"].as_str().map(str::to_owned).context("official SDP missing")}else{pack_sdp(envelope)};
                                let Ok(sdp)=sdp else{sessions.remove(&session);continue;};
                                let Ok(bytes)=gzip_sdp04(&sdp) else{sessions.remove(&session);continue;};
                                if socket.emit("soac",json!({"client_id":route.target,"data":{
                                    "app_control_id":route.ctrl,"ice_id":session,"type":"answer","ice_network_type":0,
                                    "sdp":"","gzip_sdp":{"_placeholder":true,"num":0}}}),&[bytes]).await.is_err(){break;}
                            },
                            Some(Outgoing::Close(session))=>{
                                if let Some(route)=sessions.remove(&session) {
                                    if socket.emit("release",json!({"app_control_id":route.ctrl,"control_id":route.ctrl,"ice_id":session}),&[]).await.is_err(){break;}
                                }
                            },
                            None=>break,
                        }
                    },
                    event=socket.recv_event()=>{
                        let Ok((event,header,attachments))=event else{break;};
                        let body=header.get("data").unwrap_or(&header);
                        pending.retain(|p|p.created.elapsed()<Duration::from_secs(30));
                        if event=="be-controlled" {
                            if pending.len()>=8 {continue;}
                            if let Ok(ice)=crate::ice::from_room(&header,development_relay) {
                                let control=identity(&header,"app_control_id").or_else(||{
                                    let metadata:Value=serde_json::from_str(header["streamer_data"].as_str()?).ok()?;
                                    identity(&metadata,"control_id")
                                });
                                pending.push(Pending {session:identity(&header,"ice_id"),client:identity(&header,"client_id"),control,ice,created:std::time::Instant::now(),
                                    options:attachments.first().and_then(|a|crate::official::wire::ConnectOptions::parse(a).ok())});
                            }
                        }else if event=="soac" {
                            let Some(session)=body["ice_id"].as_str().filter(|s|!s.is_empty()&&s.len()<=128).map(str::to_owned) else{continue;};
                            let Some(target)=identity(&header,"client_id") else{continue;};
                            let Some(ctrl)=identity(&header,"app_control_id") else{continue;};
                            let target=target.as_str();let ctrl=ctrl.as_str();
                            if let Some(route)=sessions.get(&session) {
                                if route.target!=target||route.ctrl!=ctrl {continue;}
                                let message=if route.official {
                                    match body["type"].as_str() {Some("candidate")=>Ok(body.clone()),_=>continue}
                                }else if body["type"]=="offer" {description(body,&attachments).and_then(|sdp|unpack_sdp(&sdp))}else{continue;};
                                if let Ok(message)=message {if route.incoming.try_send(message).is_err(){sessions.remove(&session);}}
                                continue;
                            }
                            if body["type"]!="offer" {continue;}
                            let Some(index)=pending.iter().position(|p|p.session.as_deref()==Some(&session)&&p.matches(&session,target,ctrl))
                                .or_else(||pending.iter().position(|p|p.session.is_none()&&p.matches(&session,target,ctrl))) else{continue;};
                            let request=pending.remove(index);
                            let Ok(sdp)=description(body,&attachments) else{continue;};
                            let private=sdp.lines().any(|l|l.starts_with("a=x-eclipse-"));
                            let (first,official)=if private {
                                let Ok(message)=unpack_sdp(&sdp) else{continue;};(message,false)
                            }else {
                                if !settings.enabled||request.options.is_none()||!request.official_bound(&session,target,ctrl){continue;}
                                (json!({"type":"offer","sdp":sdp}),true)
                            };
                            if sessions.len()>=crate::shared_host::MAX_SESSIONS {
                                let _=socket.emit("release",json!({"app_control_id":ctrl,"control_id":ctrl,"ice_id":session}),&[]).await;
                                continue;
                            }
                            let (send,receive)=mpsc::channel(64);let _=send.try_send(first);
                            sessions.insert(session.clone(),Route{target:target.into(),ctrl:ctrl.into(),incoming:send,official});
                            let connection=Connection{session:session.clone(),ice:request.ice,transport:Box::new(Routed{session,outgoing:outgoing.clone(),incoming:receive,closed:false})};
                            let incoming=if official {
                                uu_core::plugins::Incoming::Protocol(Box::new(crate::official::Session::new(connection,request.options.unwrap(),settings.clone(),target.to_owned(),request.created)))
                            }else{uu_core::plugins::Incoming::Native(connection)};
                            if accepted.try_send(incoming).is_err(){continue;}
                        }else if matches!(event.as_str(),"released"|"left"|"leave"|"fail") {
                            let field=|key|body.get(key).or_else(||header.get(key)).and_then(Value::as_str).filter(|s|!s.is_empty());
                            let session=field("ice_id");
                            let control=field("app_control_id").or_else(||field("control_id"));
                            let client=field("client_id");
                            // A viewer leaving is scoped to its route. A late or
                            // unscoped release never tears down the host room.
                            if session.is_some()||control.is_some()||client.is_some() {
                                sessions.retain(|id,route| {
                                    !(session.map_or(true,|v|v==id)
                                        &&control.map_or(true,|v|v==route.ctrl)
                                        &&client.map_or(true,|v|v==route.target))
                                });
                                if let Some(session)=session {pending.retain(|p|p.session.as_deref()!=Some(session));}
                            }
                        }
                    },
                }
            }
            let _=timeout(Duration::from_secs(1),socket.close()).await;
        });
        Self{incoming,task}
    }
    pub async fn accept_session(&mut self)->Result<uu_core::plugins::Incoming>{self.incoming.recv().await.context("UU signaling listener disconnected")}
    pub async fn accept(&mut self)->Result<Connection>{match self.accept_session().await? {uu_core::plugins::Incoming::Native(c)=>Ok(c),_=>bail!("protocol session requires the shared host dispatcher")}}
    pub async fn close(&mut self){self.task.abort();let _=(&mut self.task).await;self.incoming.close();}
}
impl Drop for Listener {fn drop(&mut self){self.task.abort();}}
struct Routed {session:String,outgoing:mpsc::Sender<Outgoing>,incoming:mpsc::Receiver<Value>,closed:bool}
#[async_trait::async_trait]
impl Transport for Routed {
    async fn send(&mut self,envelope:Value)->Result<()> {
        ensure!(!self.closed,"UU signaling session closed");
        timeout(Duration::from_secs(1),self.outgoing.send(Outgoing::Signal(self.session.clone(),envelope))).await.context("UU signaling queue timed out")?.context("UU signaling listener closed")
    }
    async fn receive(&mut self)->Result<Value>{self.incoming.recv().await.context("UU native peer disconnected")}
    async fn close(&mut self){if !self.closed {self.closed=true;let _=timeout(Duration::from_secs(1),self.outgoing.send(Outgoing::Close(self.session.clone()))).await;self.incoming.close();}}
}
impl Drop for Routed {fn drop(&mut self){if !self.closed {let _=self.outgoing.try_send(Outgoing::Close(self.session.clone()));}}}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt,StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    fn envelope(n:u32)->Value {json!({"v":2,"kind":"device_bootstrap","n":n,"payload":{"sdp":{"type":"offer","sdp":"v=0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n"}}})}
    fn event(name:&str,body:Value)->Message {Message::Text(format!("42{}",json!([name,body])).into())}
    #[test]
    fn official_requests_require_all_bound_identities_and_expire(){
        let mut request=Pending {session:Some("session".into()),client:Some("client".into()),control:Some("control".into()),
            ice:crate::ice::from_room(&json!({"iceServers":[]}),false).unwrap(),created:std::time::Instant::now(),options:None};
        assert!(request.official_bound("session","client","control"));
        for ids in [("other","client","control"),("session","other","control"),("session","client","other")] {assert!(!request.official_bound(ids.0,ids.1,ids.2));}
        request.client=None;assert!(!request.official_bound("session","client","control"));request.client=Some("client".into());
        request.created-=Duration::from_secs(31);assert!(!request.matches("session","client","control"));
        assert_eq!(identity(&json!({"client_id":"outer","data":{"client_id":"inner"}}),"client_id"),None);
    }
    async fn fixture()->(SocketIo,mpsc::Sender<Message>,mpsc::Receiver<Message>,tokio::task::JoinHandle<()>) {
        let tcp=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let address=tcp.local_addr().unwrap();
        let (commands,mut actions)=mpsc::channel::<Message>(16);let (observed,mut responses)=mpsc::channel(16);
        let server=tokio::spawn(async move {
            let (tcp,_)=tcp.accept().await.unwrap();let mut ws=tokio_tungstenite::accept_async(tcp).await.unwrap();
            ws.send(Message::Text("0{}".into())).await.unwrap();ws.next().await.unwrap().unwrap();ws.send(Message::Text("40{}".into())).await.unwrap();
            loop {tokio::select! {
                msg=actions.recv()=>match msg{Some(msg)=>if ws.send(msg).await.is_err(){break;},None=>break},
                msg=ws.next()=>match msg{Some(Ok(msg))=>{if matches!(msg,Message::Close(_)){break;}if observed.send(msg).await.is_err(){break;}},_=>break},
            }}
        });
        let socket=SocketIo::connect(&format!("ws://{address}"),"generated-token").await.unwrap();
        (socket,commands,responses,server)
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn official_routing_is_opt_in_and_invalid_native_extensions_never_fall_back(){
        for enabled in [false,true] {
            let(socket,commands,_responses,server)=fixture().await;
            let mut listener=Listener::with_settings(socket,false,crate::official::Settings{enabled,..Default::default()});
            for (session,client,offer,accepted) in [
                ("mismatch","wrong-client","v=0\r\n",false),
                ("broken-native","client","v=0\r\na=x-eclipse-invalid:broken\r\n",false),
                ("official","client","v=0\r\n",enabled),
            ] {
                let header=json!({"ice_id":session,"client_id":"client","app_control_id":"control","iceServers":[],"app_data":{"_placeholder":true,"num":0}});
                commands.send(Message::Text(format!("451-{}",json!(["be-controlled",header])).into())).await.unwrap();
                commands.send(Message::Binary(crate::socketio::encode_app_data("generated-controller").into())).await.unwrap();
                commands.send(event("soac",json!({"client_id":client,"data":{"ice_id":session,"app_control_id":"control","type":"offer","sdp":offer}}))).await.unwrap();
                let incoming=timeout(Duration::from_millis(100),listener.accept_session()).await;
                if accepted {assert!(matches!(incoming.unwrap().unwrap(),uu_core::plugins::Incoming::Protocol(_)));}
                else {assert!(incoming.is_err());}
            }
            listener.close().await;drop(commands);timeout(Duration::from_secs(2),server).await.unwrap().unwrap();
        }
    }
    #[tokio::test(flavor="multi_thread",worker_threads=2)]
    async fn room_routes_two_sessions_and_releasing_one_keeps_the_other_socket_alive() {
        let(socket,commands,mut responses,server)=fixture().await;
        let mut listener=Listener::from_socket(socket,false);
        for n in 1..=2 {
            commands.send(event("be-controlled",json!({"data":{"ice_id":format!("session-{n}")},"iceServers":[]}))).await.unwrap();
            commands.send(event("soac",json!({"client_id":format!("client-{n}"),"data":{"ice_id":format!("session-{n}"),"app_control_id":format!("control-{n}"),"type":"offer","sdp":pack_sdp(envelope(n)).unwrap()}}))).await.unwrap();
        }
        let mut a=timeout(Duration::from_secs(2),listener.accept()).await.unwrap().unwrap();
        let mut b=timeout(Duration::from_secs(2),listener.accept()).await.unwrap().unwrap();
        assert_eq!(a.transport.receive().await.unwrap()["n"],1);assert_eq!(b.transport.receive().await.unwrap()["n"],2);
        a.transport.close().await;
        let msg=timeout(Duration::from_secs(2),responses.recv()).await.unwrap().unwrap().into_text().unwrap();
        assert!(msg.contains("release") && msg.contains("session-1"));
        commands.send(event("released",json!({"data":{"control_id":"control-1"}}))).await.unwrap();
        commands.send(event("released",json!({"data":{}}))).await.unwrap();
        b.transport.send(envelope(9)).await.unwrap();
        let text=timeout(Duration::from_secs(2),responses.recv()).await.unwrap().unwrap().into_text().unwrap();
        assert!(text.starts_with("451-") && text.contains("session-2") && text.contains("client-2"));
        let bytes=timeout(Duration::from_secs(2),responses.recv()).await.unwrap().unwrap().into_data();
        assert_eq!(unpack_sdp(&gunzip_sdp04(&bytes).unwrap()).unwrap()["n"],9);
        commands.send(event("soac",json!({"client_id":"wrong-client","data":{"ice_id":"session-2","app_control_id":"control-2","type":"offer","sdp":pack_sdp(envelope(90)).unwrap()}}))).await.unwrap();
        assert!(timeout(Duration::from_millis(50),b.transport.receive()).await.is_err(),"another route cannot inject into this session");
        commands.send(event("soac",json!({"client_id":"client-2","data":{"ice_id":"session-2","app_control_id":"control-2","type":"offer","sdp":pack_sdp(envelope(10)).unwrap()}}))).await.unwrap();
        assert_eq!(timeout(Duration::from_secs(2),b.transport.receive()).await.unwrap().unwrap()["n"],10);
        listener.close().await;drop(commands);timeout(Duration::from_secs(2),server).await.unwrap().unwrap();
    }
}
