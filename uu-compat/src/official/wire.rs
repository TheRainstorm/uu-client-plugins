//! Host messages assembled from the existing observed CONTROL schema. No OS
//! operation is selected by an arbitrary RPC number or caller-supplied path.
use crate::control_wire::{decode_wire,bytes,int,envelope,WireValue};
use anyhow::{ensure,Result};
use uu_core::{native_quality::Update,plugin_backend::CaptureOptions};
use uu_proto::media::VideoCodec;

fn field<'a>(data:&'a [u8],id:u32)->Result<Option<WireValue<'a>>> {
    let fields=decode_wire(data).map_err(|_|anyhow::anyhow!("invalid control protobuf"))?;
    let mut found=fields.into_iter().filter(|(n,_)|*n==id).map(|(_,v)|v);
    let value=found.next();ensure!(found.next().is_none(),"duplicate control field");Ok(value)
}
pub fn child(data:&[u8],id:u32)->Result<Option<&[u8]>> {
    match field(data,id)? {Some(WireValue::Bytes(v))=>Ok(Some(v)),None=>Ok(None),_=>anyhow::bail!("invalid message field type")}
}
fn number(data:&[u8],id:u32)->Result<Option<u64>> {
    match field(data,id)? {Some(WireValue::Varint(v))=>Ok(Some(v)),None=>Ok(None),_=>anyhow::bail!("invalid number field type")}
}
#[derive(Clone,Debug)]
pub struct ConnectOptions {pub fps:u32,pub bounds:Option<[u32;2]>}
impl ConnectOptions {
    pub fn parse(attachment:&[u8])->Result<Self> {
        ensure!(attachment.len()<=16*1024,"connection options exceed limit");
        let data=if attachment.first()==Some(&4){&attachment[1..]}else{attachment};
        ensure!(number(data,1)?==Some(1),"only desktop capture is supported");
        let mut supported=None;
        for (_,value) in decode_wire(data).map_err(|_|anyhow::anyhow!("invalid connection options"))?.into_iter().filter(|(n,_)|*n==4).take(32) {
            let WireValue::Bytes(cap)=value else{anyhow::bail!("invalid decoder capability");};
            if number(cap,2)?!=Some(1){continue;}
            let fps=u32::try_from(number(cap,1)?.unwrap_or(60))?.clamp(5,144);
            let width=u32::try_from(number(cap,3)?.unwrap_or(3840))?;
            let height=u32::try_from(number(cap,4)?.unwrap_or(2160))?;
            ensure!((2..=8192).contains(&width)&&(2..=8192).contains(&height)&&width%2==0&&height%2==0,"invalid decoder dimensions");
            ensure!(number(cap,5)?.unwrap_or(1)==1,"only 4:2:0 is supported");
            supported=Some(Self {fps:fps.min(60),bounds:Some([width,height])});break;
        }
        supported.ok_or_else(||anyhow::anyhow!("controller did not advertise H.264 SDR 4:2:0"))
    }
    pub fn capture(&self,config:&uu_core::native::SharingConfig)->CaptureOptions {
        CaptureOptions {codec:VideoCodec::H264,fps:self.fps,bitrate_kbps:config.bitrate_kbps,bounds:self.bounds}
    }
}
#[derive(Clone,Copy,Debug)]
pub struct CaptureRequest {pub id:u64,pub update:Update,pub bounds:Option<[u32;2]>}
pub fn capture_request_id(data:&[u8])->Option<u64>{number(child(child(data,21).ok()??,1).ok()??,1).ok()?}
pub fn capture_request(data:&[u8],current:uu_media::encoder::Settings)->Result<Option<CaptureRequest>> {
    let Some(rpc)=child(data,21)? else{return Ok(None);};
    let Some(setting)=child(rpc,2)? else{return Ok(None);};
    let header=child(rpc,1)?.ok_or_else(||anyhow::anyhow!("missing request header"))?;
    let id=number(header,1)?.ok_or_else(||anyhow::anyhow!("missing request id"))?;
    ensure!(number(setting,13)?.unwrap_or(1)==1,"only H.264 is implemented by the official host");
    ensure!(number(setting,3)?.unwrap_or(0)==0,"host cursor compositing is not implemented");
    for field in [5,6,16,17] {ensure!(number(setting,field)?.is_none_or(|v|v==0||v==u64::MAX),"desktop resolution changes are not implemented");}
    let fps=match number(setting,18)?.filter(|n|*n!=0) {
        Some(value)=>u32::try_from(value)?,
        None=>match number(setting,1)?.unwrap_or(0) {1=>30,2=>60,3=>90,4=>144,0=>current.fps,_=>anyhow::bail!("unsupported frame rate option")},
    };
    ensure!((5..=144).contains(&fps),"frame rate outside compatible protocol range");
    let quality=number(setting,2)?.unwrap_or(5);
    let bitrate_kbps=if quality==6 {
        let bps=number(setting,8)?.ok_or_else(||anyhow::anyhow!("custom bitrate missing"))?;
        ensure!((64_000..=1_000_000_000).contains(&bps),"custom bitrate outside encoder range");
        Some(u32::try_from(bps.div_ceil(1000))?)
    }else{ensure!((1..=5).contains(&quality),"invalid quality preset");None};
    let bounds=match (number(setting,14)?,number(setting,15)?) {
        (Some(w),Some(h))=>{let(w,h)=(u32::try_from(w)?,u32::try_from(h)?);ensure!((2..=8192).contains(&w)&&(2..=8192).contains(&h)&&w%2==0&&h%2==0,"invalid capture bounds");Some([w,h])},
        (None,None)=>None,_=>anyhow::bail!("incomplete capture bounds"),
    };
    let screen=number(setting,4)?.unwrap_or(0) as i64;
    ensure!(matches!(screen,-2|-1|0),"capture source was not advertised");
    Ok(Some(CaptureRequest {id,update:Update {fps,bitrate_kbps,rate_control:None},bounds}))
}
pub fn capture_reply(sequence:u64,request:u64,accepted:bool)->Vec<u8> {
    let mut header=Vec::new();int(1,request,&mut header);
    let mut response=Vec::new();
    if !accepted {let mut error=Vec::new();int(1,1,&mut error);bytes(1,&error,&mut response);}
    let mut rpc=Vec::new();bytes(1,&header,&mut rpc);bytes(2,&response,&mut rpc);envelope(sequence,22,&rpc)
}
pub fn screen_inventory(sequence:u64,width:u32,height:u32,refresh_hz:u32)->Vec<u8> {
    let mut rectangle=Vec::new();int(3,u64::from(width),&mut rectangle);int(4,u64::from(height),&mut rectangle);int(5,u64::from(width),&mut rectangle);int(6,u64::from(height),&mut rectangle);
    let mut screen=Vec::new();int(1,0,&mut screen);int(2,u64::from(refresh_hz),&mut screen);bytes(4,&rectangle,&mut screen);int(5,0,&mut screen);int(7,1,&mut screen);bytes(10,b"Shared desktop",&mut screen);int(12,0,&mut screen);
    let mut inventory=Vec::new();bytes(1,&screen,&mut inventory);int(2,0,&mut inventory);envelope(sequence,7,&inventory)
}
pub fn capture_started(sequence:u64)->Vec<u8> {envelope(sequence,8,&[])}
/// Only fields actually provided by the first host implementation are advertised.
pub fn host_echo(sequence:u64,response_to:Option<u64>,clipboard:bool)->Vec<u8> {
    let mut flags=Vec::new();int(1,1,&mut flags);if clipboard{int(8,3,&mut flags);}
    let mut action=Vec::new();int(1,u64::from(response_to.is_some()),&mut action);
    if let Some(id)=response_to{bytes(2,serde_json::json!({"seq":id}).to_string().as_bytes(),&mut action);}
    bytes(4,&flags,&mut action);envelope(sequence,3,&action)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewer_capability_and_host_responses_roundtrip_through_existing_client_decoders() {
        let options=crate::socketio::encode_app_data("generated-controller");
        let parsed=ConnectOptions::parse(&options).unwrap();assert_eq!(parsed.fps,60);
        assert!(ConnectOptions::parse(&[4,8,2]).is_err());
        assert_eq!(crate::control_wire::capture_setting_response(&capture_reply(1,100,true)),Some((100,true)));
        assert_eq!(crate::control_wire::capture_setting_response(&capture_reply(2,101,false)),Some((101,false)));
        assert_eq!(crate::control_wire::echo_request_seq(&host_echo(20,None,true)),Some(20));
        assert!(crate::control_wire::echo_matches(&host_echo(21,Some(20),true),20));
        assert!(crate::control_wire::echo_clipboard_supported(&host_echo(21,None,true)));
        let screens=crate::screen_control::screen_sources(&screen_inventory(1,1920,1080,60)).unwrap();
        assert_eq!(screens.screens[0].assigned_track(),Some(0));assert_eq!(screens.screens[0].width,1920);
    }
}
