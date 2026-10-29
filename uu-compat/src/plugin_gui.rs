//! Per-window account and navigation state. The host supplies no UU layout.
use anyhow::Result;
use uu_plugin_sdk::{egui,Context,Action,PageState};
use crate::{login::{Worker,LoginEvent},device_library::{Library,Filter}};
mod devices;
mod account_ui;
mod assistance_ui;
#[derive(Clone,Copy,PartialEq,Eq,Default)]
enum View {#[default] Devices,Assistance}
pub struct Ui {
    context:Context,token:Option<String>,login:Option<Worker>,qr:Option<egui::TextureHandle>,
    name:String,notice:String,devices:Vec<crate::Device>,library:Library,filter:Filter,query:String,
    account:crate::uu_devices::Manager,loading:Option<ListWorker>,
    assistance_text:zeroize::Zeroizing<String>,assistance_id:String,assistance_code:zeroize::Zeroizing<String>,assistance_error:String,requested:bool,
    selected:Option<usize>,view:View,view_only:bool,actions:Vec<Action>,list_error:Option<String>,account_error:Option<String>,device_id:Option<String>,
    wallpapers:std::collections::HashMap<usize,egui::TextureHandle>,wallpaper_worker:Option<crate::wallpapers::Worker>,
    official:std::result::Result<crate::official::Settings,String>,
}
struct ListWorker {rx:std::sync::mpsc::Receiver<(Option<String>,Result<Vec<crate::Device>>)>,cancel:Option<uu_plugin_sdk::tokio::sync::oneshot::Sender<()>>,thread:Option<std::thread::JoinHandle<()>>}
impl Drop for ListWorker {fn drop(&mut self){if let Some(cancel)=self.cancel.take(){let _=cancel.send(());}if let Some(thread)=self.thread.take(){let _=thread.join();}}}
impl Ui {
    pub fn new(context:Context)->Result<Self> {
        let state=crate::ClientState::read(&context.data.at("state.json"))?;
        let token=state.as_ref().and_then(|s|s.token.clone()).filter(|s|!s.is_empty());
        let library=Library::for_account(Some(&context.data),token.as_deref());
        let name=state.as_ref().and_then(|s|s.device_name.clone()).unwrap_or_else(crate::config::uu_device_name);
        let device_id=state.and_then(|s|s.device_id);
        let official=crate::official::Settings::load(&context.data).map_err(|e|e.to_string());
        Ok(Self{context,token,login:None,qr:None,name,device_id,official,notice:String::new(),devices:Vec::new(),library,filter:Filter::All,query:String::new(),account:Default::default(),loading:None,assistance_text:zeroize::Zeroizing::new(String::new()),assistance_id:String::new(),assistance_code:zeroize::Zeroizing::new(String::new()),assistance_error:String::new(),requested:false,selected:None,view:View::Devices,view_only:false,actions:Vec::new(),list_error:None,account_error:None,wallpapers:Default::default(),wallpaper_worker:None})
    }
    fn refresh(&mut self) {
        if self.token.is_none(){return;}
        self.requested=true;self.loading=None;self.list_error=None;
        let config=self.context.data.clone();let token=self.token.clone();let (tx,rx)=std::sync::mpsc::channel();
        let (cancel,cancelled)=tokio::sync::oneshot::channel();
        let thread=std::thread::spawn(move|| {
            let result=tokio::runtime::Runtime::new().map_err(anyhow::Error::from).and_then(|runtime| {
                let result=runtime.block_on(async {tokio::select! {biased;_=cancelled=>Err(anyhow::anyhow!("cancelled")),result=tokio::time::timeout(std::time::Duration::from_secs(15),async {
                    let api=crate::UuApi::new(crate::SignalClient::new(&config)?,config.at("state.json"))?;
                    anyhow::ensure!(api.state.token==token,"UU account changed");crate::parse_devices(api.device_list().await?)
                })=>result.map_err(|_|anyhow::anyhow!("device list timed out")).and_then(|r|r)}});
                runtime.shutdown_background();result
            });let _=tx.send((token,result));
        });self.loading=Some(ListWorker{rx,cancel:Some(cancel),thread:Some(thread)});
    }
    fn start_login(&mut self) {self.login=Some(crate::login::spawn(self.context.data.clone(),self.name.clone()));self.qr=None;self.notice.clear();}
    fn poll(&mut self,ctx:&egui::Context) {
        let mut account_changed=false;
        let mut login_done=false;
        if let Some(worker)=&self.login {
            while let Ok(event)=worker.try_recv() {match event {
                LoginEvent::Qr(value)=>match crate::login::qr_rgba(&value,5) {
                    Ok((w,h,bytes))=>self.qr=Some(ctx.load_texture("uu-login",egui::ColorImage::from_rgba_unmultiplied([w,h],&bytes),egui::TextureOptions::NEAREST)),Err(error)=>self.notice=error.to_string(),
                },
                LoginEvent::Status(message)=>self.notice=message,
                LoginEvent::Done{token,user_id,device_name,name_error}=> {
                    login_done=true;
                    match crate::account::store_login(&self.context.data.at("state.json"),token.clone(),user_id,device_name) {
                        Ok(())=>{self.token=Some(token);self.qr=None;self.notice=name_error.unwrap_or_default();account_changed=true;self.actions.push(Action::ProviderReset);},Err(error)=>self.notice=error.to_string(),
                    }
                },
                LoginEvent::NameSaved{name,token}=>{login_done=true;match crate::account::store_device_name(&self.context.data.at("state.json"),&token,name.clone()){Ok(())=>self.name=name,Err(error)=>self.notice=error.to_string()}},
                LoginEvent::Error(error)=>{login_done=true;self.notice=error;},
            }}
        }
        if login_done {self.login=None;}
        if account_changed {if let Ok(Some(state))=crate::ClientState::read(&self.context.data.at("state.json")){self.device_id=state.device_id;if let Some(name)=state.device_name{self.name=name;}}}
        if account_changed {self.login=None;self.library=Library::for_account(Some(&self.context.data),self.token.as_deref());self.refresh();}
        if let Some((token,result))=self.loading.as_ref().and_then(|w|w.rx.try_recv().ok()) {
            self.loading=None;
            match crate::account::read_token(&self.context.data.at("state.json")) {
                Ok(current) if current==token=>match result {
                    Ok(devices)=>{
                        let selected=self.selected.and_then(|i|self.devices.get(i)).map(|d|d.device_id.clone());
                        self.devices=devices;self.selected=selected.and_then(|id|self.devices.iter().position(|d|d.device_id==id));self.ensure_visible_device_selection();self.list_error=None;
                        self.wallpapers.clear();
                        self.wallpaper_worker=crate::wallpapers::Worker::start(self.devices.iter().filter(|d|!d.wallpaper_url.is_empty()).map(|d|crate::wallpapers::Request{device_id:d.device_id.clone(),url:d.wallpaper_url.clone()}).collect(),ctx.clone()).ok();
                    },Err(error)=>self.list_error=Some(error.to_string()),
                },
                Ok(current)=>{self.token=current;self.devices.clear();self.wallpaper_worker=None;self.wallpapers.clear();self.library=Library::for_account(Some(&self.context.data),self.token.as_deref());self.actions.push(Action::ProviderReset);self.requested=false;},
                Err(error)=>self.account_error=Some(error.to_string()),
            }
        }
        if let Some(worker)=&mut self.wallpaper_worker {
            while let Some(image)=worker.try_recv() {
                if let Some(index)=self.devices.iter().position(|d|d.device_id==image.device_id&&d.wallpaper_url==image.url) {
                    self.wallpapers.insert(index,ctx.load_texture(format!("uu-preview-{index}"),egui::ColorImage::from_rgba_unmultiplied([image.width as usize,image.height as usize],&image.rgba),egui::TextureOptions::LINEAR));
                }
            }
            if worker.finished(){self.wallpaper_worker=None;}
        }
        if self.account.show(ctx,Some(&self.context.data),self.token.as_deref())==Some(true){self.sign_out();}
        if self.login.is_some()||self.loading.is_some(){ctx.request_repaint_after(std::time::Duration::from_millis(200));}
    }
    fn sign_out(&mut self) {
        self.login=None;self.loading=None;
        match crate::account::sign_out_matching(&self.context.data.at("state.json"),self.token.as_deref()) {
            Ok(true)=>{self.token=None;self.devices.clear();self.qr=None;self.library=Library::default();self.wallpaper_worker=None;self.wallpapers.clear();self.requested=false;self.selected=None;self.actions.push(Action::ProviderReset);},
            Ok(false)=>self.notice=uu_i18n::text("账号已变化，请刷新后重试").into(),Err(error)=>self.notice=error.to_string(),
        }
    }
    fn ensure_visible_device_selection(&mut self) {let visible=self.library.visible_indices(&self.devices,self.filter,&self.query);self.selected=Library::retain_selection(self.selected,&visible);}
    fn connect_device(&mut self) {
        let Some(device)=self.selected.and_then(|i|self.devices.get(i)) else{return;};
        if !device.controllable||!matches!(device.status.as_str(),"CONNECTED"|"ready"|"ONLINE"){return;}
        let target=super::runtime_plugin::Target::Device{device:device.device_id.clone()};
        if let Ok(target)=serde_json::to_string(&target){self.library.record_opened(&device.device_id);self.actions.push(Action::Connect{identity:device.device_id.clone(),target:zeroize::Zeroizing::new(target),name:device.alias.clone(),view_only:self.view_only});}
    }
}
impl PageState for Ui {
    fn show(&mut self,ui:&mut egui::Ui,page:&str)->Vec<Action> {
        self.poll(ui.ctx());
        match page {
            "account"=>self.account_ui(ui),
            "server"=>self.official_ui(ui),
            "settings"=>self.settings_ui(ui),
            _=>{},
        }
        std::mem::take(&mut self.actions)
    }
    fn show_navigation(&mut self,ctx:&egui::Context,_page:&str)->Vec<Action> {
        self.poll(ctx);
        if self.token.is_some()&&!self.requested{self.refresh();}
        self.navigation(ctx);
        std::mem::take(&mut self.actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Owned {ui:Ui,path:std::path::PathBuf}
    impl Drop for Owned {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.path);}}
    fn owned()->Owned {
        let path=std::env::temp_dir().join(format!("extension-ui-{}",uuid::Uuid::new_v4()));
        let config=crate::Config{data_dir:path.clone(),state_dir:path.join("state"),signaling_host:String::new()};
        let mut ui=Ui::new(Context::new(&config,"uu")).unwrap();ui.token=Some("generated-ui-account".into());ui.requested=true;
        ui.devices=vec![crate::Device{alias:"Generated desktop".into(),device_id:"generated-device".into(),status:"ONLINE".into(),controllable:true,wallpaper_url:String::new(),platform:0}];
        ui.ensure_visible_device_selection();Owned{ui,path}
    }
    fn texts(output:&egui::FullOutput)->Vec<(String,egui::Pos2)> {
        fn visit(shape:&egui::Shape,out:&mut Vec<(String,egui::Pos2)>) {match shape {
            egui::Shape::Text(text)=>out.push((text.galley.job.text.clone(),text.pos+text.galley.rect.center().to_vec2())),
            egui::Shape::Vec(shapes)=>for shape in shapes{visit(shape,out)},_=>{},
        }}
        let mut values=Vec::new();for shape in &output.shapes{visit(&shape.shape,&mut values);}values
    }
    #[test]
    fn restored_device_navigation_opens_the_full_assistance_form_in_compact_windows() {
        let mut owned=owned();let ui=&mut owned.ui;let ctx=egui::Context::default();uu_plugin_sdk::theme::apply(&ctx);
        let input=|events|egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(960.,600.))),events,..Default::default()};
        // First pass settles panels before hit testing the original sidebar.
        let _=ctx.run(input(Vec::new()),|ctx|{ui.show_navigation(ctx,"devices");});
        let output=ctx.run(input(Vec::new()),|ctx|{ui.show_navigation(ctx,"devices");});
        let labels=texts(&output);assert!(labels.iter().any(|(s,_)|s=="Generated desktop"));
        let pos=labels.iter().find(|(s,_)|s=="远程协助").expect("compact sidebar must retain assistance").1;
        let pointer=|pressed|egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
        let _=ctx.run(input(vec![egui::Event::PointerMoved(pos),pointer(true)]),|ctx|{ui.show_navigation(ctx,"devices");});
        let _=ctx.run(input(vec![pointer(false)]),|ctx|{ui.show_navigation(ctx,"devices");});
        assert!(ui.view==View::Assistance);
        let output=ctx.run(input(Vec::new()),|ctx|{ui.show_navigation(ctx,"devices");});let labels=texts(&output);
        assert!(labels.iter().any(|(s,_)|s=="识别分享文本"));assert!(labels.iter().any(|(s,_)|s=="发起远程协助"));
        assert!(!labels.iter().any(|(s,_)|s=="连接原生设备"||s=="让别人连接本机"||s=="添加设备"));
    }
    #[test]
    fn account_card_has_original_controls_and_no_assistance_form() {
        let mut owned=owned();let ctx=egui::Context::default();uu_plugin_sdk::theme::apply(&ctx);
        let draw=|ctx:&egui::Context,owned:&mut Owned|{egui::CentralPanel::default().show(ctx,|ui|{owned.ui.show(ui,"account");});};
        let _=ctx.run(Default::default(),|ctx|draw(ctx,&mut owned));let out=ctx.run(Default::default(),|ctx|draw(ctx,&mut owned));let labels=texts(&out);
        for label in ["UU 本机名称","账号状态：已登录","登出 UU 账号","管理 UU 设备"] {assert!(labels.iter().any(|(s,_)|s==label),"{label}");}
        assert!(!labels.iter().any(|(s,_)|s.contains("通过设备 ID 和验证码连接")||s=="设备验证码"||s.contains("官方 UU 客户端连接本机")));
    }
    #[test]
    fn module_settings_and_server_permissions_are_visible_without_disclosure() {
        let mut owned=owned();let ctx=egui::Context::default();uu_plugin_sdk::theme::apply(&ctx);
        let input=||egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1100.,1000.))),..Default::default()};
        for (page,expected) in [("settings",vec!["UU 兼容客户端","UU 兼容服务器（实验性）","UU 账号连接服务","允许官方中转"]),("server",vec!["官方 UU 客户端连接本机","键盘和鼠标","共享系统声音","文本剪贴板","保存兼容设置"])] {
            let draw=|ctx:&egui::Context,owned:&mut Owned|{egui::CentralPanel::default().show(ctx,|ui|{owned.ui.show(ui,page);});};
            let _=ctx.run(input(),|ctx|draw(ctx,&mut owned));let output=ctx.run(input(),|ctx|draw(ctx,&mut owned));let labels=texts(&output);
            for wanted in expected{assert!(labels.iter().any(|(s,_)|s==wanted),"{page}: {wanted}");}
        }
    }
    #[test]
    fn module_checkboxes_persist_only_the_selected_capability() {
        let mut owned=owned();let ctx=egui::Context::default();uu_plugin_sdk::theme::apply(&ctx);
        let input=|events|egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1100.,1000.))),events,..Default::default()};
        let draw=|ctx:&egui::Context,owned:&mut Owned|{egui::CentralPanel::default().show(ctx,|ui|{owned.ui.show(ui,"settings");});};
        for (label,wanted) in [("UU 兼容客户端",crate::modules::Modules{client:false,server:false,account:true}),("UU 兼容服务器（实验性）",crate::modules::Modules{client:false,server:true,account:true}),("UU 账号连接服务",crate::modules::Modules{client:false,server:true,account:false})] {
            let _=ctx.run(input(Vec::new()),|ctx|draw(ctx,&mut owned));let output=ctx.run(input(Vec::new()),|ctx|draw(ctx,&mut owned));
            let labels=texts(&output);let pos=labels.iter().find(|(s,_)|s==label).unwrap().1;
            let pointer=|pressed|egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed,modifiers:Default::default()};
            let _=ctx.run(input(vec![egui::Event::PointerMoved(pos),pointer(true)]),|ctx|draw(ctx,&mut owned));
            let _=ctx.run(input(vec![pointer(false)]),|ctx|draw(ctx,&mut owned));
            let preferences=owned.ui.context.preferences().unwrap();assert_eq!(crate::modules::Modules::from_preferences(&preferences).unwrap(),wanted);assert!(!preferences.enabled);
        }
    }
    #[test]
    fn watch_mode_preserves_the_plugin_target_and_opaque_launch_contract() {
        let mut owned=owned();owned.ui.view_only=true;owned.ui.connect_device();
        let Action::Connect{target,view_only,identity,..}=owned.ui.actions.pop().unwrap() else{panic!("expected session launch")};
        assert!(view_only);assert_eq!(identity,"generated-device");assert_eq!(serde_json::from_str::<serde_json::Value>(&target).unwrap()["device"],identity);
    }
}
