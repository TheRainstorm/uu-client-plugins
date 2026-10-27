//! Finite normal-account contract probe. Creates only an explicitly requested
//! disposable registration; never emits credential values or joins user devices.
use anyhow::{ensure,Context,Result};
use serde_json::{json,Value};
use std::{path::PathBuf,time::Duration};
use uu_compat::{ClientState,Config,SignalClient,UuApi,socketio::SocketIo};
fn shape(value:&Value,depth:usize)->Value {
    if depth>6{return json!("depth-limit");}
    match value {
        Value::Object(o)=>Value::Object(o.iter().take(96).map(|(k,v)|{
            let secret=["token","auth","credential","password","sign"].iter().any(|word|k.to_ascii_lowercase().contains(word));
            (k.clone(),if secret{json!("redacted")}else{shape(v,depth+1)})
        }).collect()),
        Value::Array(a)=>Value::Array(a.iter().take(6).map(|v|shape(v,depth+1)).collect()),
        Value::String(s)=>json!(format!("string({})",s.len())),
        other=>other.clone(),
    }
}
async fn socket(data:&Value)->Result<SocketIo>{
    let endpoint=data["signaling_server"].as_str().context("missing signaling server")?;
    ensure!(endpoint.starts_with("wss://"),"WSS required");
    SocketIo::connect(&format!("{}/socket.io/?EIO=4&transport=websocket",endpoint.trim_end_matches('/')),data["token"].as_str().context("missing room credential")?).await
}
#[tokio::main]
async fn main()->Result<()> {
    uu_core::install_crypto_provider();
    let mut args=std::env::args().skip(1);
    ensure!(args.next().as_deref()==Some("--allow-online-fixture"),"requires --allow-online-fixture DIRECTORY");
    let root=PathBuf::from(args.next().context("private fixture directory required")?);
    ensure!(!root.join("state.json").exists(),"fixture directory already contains a registration");
    let source=Config::from_env()?;
    let existing=ClientState::load(&source.at("state.json"))?;
    let token=existing.token.as_deref().context("existing account login required")?;
    let config=Config{data_dir:root.clone(),state_dir:root.join("diagnostics"),signaling_host:source.signaling_host.clone()};
    config.ensure_dirs().await?;
    let mut api=UuApi::new(SignalClient::new(&config)?,config.at("state.json"))?;
    api.register("Eclipse disposable protocol fixture").await?;
    api.store_token(token,existing.user_id.clone())?;
    let device=api.state.device_id.clone().context("registration missing device")?;
    let result=tokio::time::timeout(Duration::from_secs(35),async {
        let room=api.room_create().await?;
        ensure!(room["code"].as_i64()==Some(0),"room create rejected, code {:?}",room["code"].as_i64());
        let mut host=socket(&room["data"]).await?;
        host.request_ack("room_info",1).await?;
        let join=uu_compat::connection::join(&source,&device,false);
        tokio::pin!(join);
        let mut connected=None;
        let mut records=Vec::new();
        let mut request_seen=false;
        loop {tokio::select! {
            result=&mut join,if connected.is_none()=>{connected=Some(result?);if request_seen{break;}},
            event=host.recv_event()=>{
                let (name,body,attachments)=event?;
                records.push(json!({"event":name,"shape":shape(&body,0),"attachments":attachments.iter().map(Vec::len).collect::<Vec<_>>()}));
                if name=="be-controlled" {request_seen=true;if connected.is_some(){break;}}
            },
        }}
        if let Some(mut connection)=connected{connection.transport.close().await;}
        host.close().await;
        Ok::<_,anyhow::Error>(json!({"request_observed":request_seen,"records":records}))
    }).await;
    let cleanup=api.unbind_account_device(&device).await.is_ok();
    let report=match result {Ok(Ok(mut report))=>{report["fixture_unbound"]=cleanup.into();report},other=>json!({"error":format!("{other:?}"),"fixture_unbound":cleanup})};
    std::fs::write(root.join("contract.json"),serde_json::to_vec_pretty(&report)?)?;
    println!("{}",serde_json::to_string(&report)?);
    Ok(())
}
