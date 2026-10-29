//! One controller actor owns wire input and all accepted held state.
use anyhow::Result;
use uu_core::native_input::{InputCommand as I,MouseButton};
use serde_json::json;
#[derive(Default)]
pub(crate) struct Input {held:Vec<I>,seq:u64}
impl Input {
    fn frame(&mut self,command:I,screen:i32)->Result<Vec<u8>> {
        self.seq=self.seq.wrapping_add(1);
        let mut value=match command {
            I::Key{usage,pressed}=>{
                let vk=(0..=255u64).find(|vk|crate::official::input::usage(*vk)==Some(usage)).ok_or_else(||anyhow::anyhow!("unsupported UU keyboard usage"))?;
                json!({"action":if pressed{"kbd_press"}else{"kbd_release"},"interrept":"i","key":vk,"kseq":self.seq})
            },
            I::Button{button,pressed}=>json!({"action":if pressed{"mouse_press"}else{"mouse_release"},"button":match button {MouseButton::Left=>1,MouseButton::Right=>2,MouseButton::Middle=>16,MouseButton::Back=>32,MouseButton::Forward=>64}}),
            I::Absolute{x,y}=>json!({"action":"mouse_move_absolute","abs_x":f64::from(x)/65535.,"abs_y":f64::from(y)/65535.}),
            I::Relative{dx,dy}=>json!({"action":"mouse_move_relative","delta_x":dx,"delta_y":dy}),
            I::Wheel{dx,dy}=>json!({"action":"mouse_scroll","delta_x":dx,"delta_y":dy}),
            I::ReleaseAll{}=>anyhow::bail!("release-all must use the actor reset path"),
        };value["screen_id"]=screen.into();Ok(serde_json::to_vec(&value)?)
    }
    pub async fn send(&mut self,media:&crate::media::ControllerMedia,command:I,screen:i32)->Result<()> {
        if matches!(command,I::ReleaseAll{}){return self.reset(media,screen).await;}
        let frame=self.frame(command,screen)?;
        // Record a possible press before the cancellable send. Cleanup releases
        // it even when cancellation occurs after SCTP accepted the bytes.
        if matches!(command,I::Key{pressed:true,..}|I::Button{pressed:true,..})&&!self.held.contains(&command){self.held.push(command);}
        media.send_input(frame).await?;
        match command {I::Key{usage,pressed:false}=>self.held.retain(|v|!matches!(v,I::Key{usage:u,..}if *u==usage)),I::Button{button,pressed:false}=>self.held.retain(|v|!matches!(v,I::Button{button:b,..}if *b==button)),_=>{}}Ok(())
    }
    pub async fn reset(&mut self,media:&crate::media::ControllerMedia,screen:i32)->Result<()> {
        while let Some(pressed)=self.held.last().copied() {
            let release=match pressed{I::Key{usage,..}=>I::Key{usage,pressed:false},I::Button{button,..}=>I::Button{button,pressed:false},_=>unreachable!()};
            let frame=self.frame(release,screen)?;media.send_input(frame).await?;self.held.pop();
        }Ok(())
    }
}
pub(crate) fn screen(stats:&uu_core::media::MediaStats)->i32 {
    let track=stats.selected_video_track.load(std::sync::atomic::Ordering::Acquire);
    stats.remote_screens.lock().unwrap().for_track(track).map(|s|s.id).unwrap_or(-1)
}
