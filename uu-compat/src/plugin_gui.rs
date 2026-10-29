//! Per-window UI state owned by the extension, including cancellation.
use anyhow::Result;
use uu_plugin_sdk::{egui,Context,Action,PageState};
use crate::{login::{Worker,LoginEvent},device_library::{Library,Filter}};
pub struct Ui {
    context:Context,token:Option<String>,login:Option<Worker>,qr:Option<egui::TextureHandle>,
    name:String,notice:String,devices:Vec<crate::Device>,library:Library,filter:Filter,query:String,
    account:crate::uu_devices::Manager,loading:Option<ListWorker>,
    assistance_id:String,assistance_code:zeroize::Zeroizing<String>,requested:bool,
}
struct ListWorker {rx:std::sync::mpsc::Receiver<(Option<String>,Result<Vec<crate::Device>>)>,cancel:Option<uu_plugin_sdk::tokio::sync::oneshot::Sender<()>>,thread:Option<std::thread::JoinHandle<()>>}
impl Drop for ListWorker {fn drop(&mut self){if let Some(cancel)=self.cancel.take(){let _=cancel.send(());}if let Some(thread)=self.thread.take(){let _=thread.join();}}}
impl Ui {
    pub fn new(context:Context)->Result<Self> {
        let token=crate::account::read_token(&context.data.at("state.json"))?;
        let library=Library::for_account(Some(&context.data),token.as_deref());
        Ok(Self{context,token,login:None,qr:None,name:crate::config::uu_device_name(),notice:String::new(),devices:Vec::new(),library,filter:Filter::All,query:String::new(),account:Default::default(),loading:None,assistance_id:String::new(),assistance_code:zeroize::Zeroizing::new(String::new()),requested:false})
    }
    fn refresh(&mut self) {
        self.requested=true;
        self.loading=None;
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
    fn poll(&mut self,ctx:&egui::Context)->Vec<Action> {
        let mut actions=Vec::new();
        if let Some(worker)=&self.login {
            while let Ok(event)=worker.try_recv() {match event {
                LoginEvent::Qr(value)=>match crate::login::qr_rgba(&value,5) {
                    Ok((w,h,bytes))=>self.qr=Some(ctx.load_texture("uu-login",egui::ColorImage::from_rgba_unmultiplied([w,h],&bytes),egui::TextureOptions::NEAREST)),Err(error)=>self.notice=error.to_string(),
                },
                LoginEvent::Status(message)=>self.notice=message,
                LoginEvent::Done{token,user_id,device_name,name_error}=> {
                    match crate::account::store_login(&self.context.data.at("state.json"),token.clone(),user_id,device_name) {
                        Ok(())=>{self.token=Some(token);self.qr=None;self.notice=name_error.unwrap_or_default();actions.push(Action::ProviderReset);},Err(error)=>self.notice=error.to_string(),
                    }
                },
                LoginEvent::NameSaved{name,token}=>if let Err(error)=crate::account::store_device_name(&self.context.data.at("state.json"),&token,name){self.notice=error.to_string();},
                LoginEvent::Error(error)=>self.notice=error,
            }}
        }
        if !actions.is_empty(){self.login=None;self.library=Library::for_account(Some(&self.context.data),self.token.as_deref());self.refresh();}
        if let Some((token,result))=self.loading.as_ref().and_then(|w|w.rx.try_recv().ok()) {
            self.loading=None;
            match crate::account::read_token(&self.context.data.at("state.json")) {
                Ok(current) if current==token=>match result {Ok(devices)=>{self.devices=devices;self.notice.clear();},Err(error)=>self.notice=error.to_string()},
                Ok(current)=>{self.token=current;self.devices.clear();self.library=Library::for_account(Some(&self.context.data),self.token.as_deref());actions.push(Action::ProviderReset);},
                Err(error)=>self.notice=error.to_string(),
            }
        }
        if self.login.is_some()||self.loading.is_some(){ctx.request_repaint_after(std::time::Duration::from_millis(200));}
        actions
    }
    fn sign_out(&mut self,actions:&mut Vec<Action>) {
        self.login=None;self.loading=None;
        match crate::account::sign_out_matching(&self.context.data.at("state.json"),self.token.as_deref()) {
            Ok(true)=>{self.token=None;self.devices.clear();self.qr=None;self.library=Library::default();actions.push(Action::ProviderReset);},
            Ok(false)=>self.notice="UU account changed; refresh before signing out".into(),Err(error)=>self.notice=error.to_string(),
        }
    }
}
impl PageState for Ui {
    fn show(&mut self,ui:&mut egui::Ui,page:&str)->Vec<Action> {
        let mut actions=self.poll(ui.ctx());
        if page=="devices"&&self.token.is_some()&&!self.requested {self.refresh();}
        if page=="settings" {
            match self.context.preferences() {
                Ok(expected)=>{let mut relay=super::runtime_plugin::relay(&self.context).unwrap_or(false);
                    if ui.checkbox(&mut relay,uu_i18n::text("允许官方中转（默认关闭）")).changed(){let mut value=expected.clone();value.options.insert("official_relay".into(),relay.into());if let Err(error)=self.context.save_preferences(&expected,value){self.notice=error.to_string();}}
                    ui.weak("P2P 优先；允许中转时使用服务返回的 TURN。修改后重新连接或重启共享入口生效。");
                    match crate::official::Settings::load(&self.context.data) {
                        Ok(mut host)=>{
                            let old=host.clone();
                            ui.checkbox(&mut host.enabled,"允许 UU 官方客户端连接本机");
                            ui.add_enabled_ui(host.enabled,|ui|{ui.horizontal(|ui| {
                                ui.checkbox(&mut host.control,"键鼠控制");ui.checkbox(&mut host.audio,"声音");ui.checkbox(&mut host.clipboard,"剪贴板");
                            });});
                            if host!=old {if let Err(error)=host.save(&self.context.data){self.notice=error.to_string();}else{self.notice="权限已保存，重启共享入口后生效".into();}}
                        },Err(error)=>{ui.colored_label(egui::Color32::RED,error.to_string());},
                    }
                },Err(error)=>{ui.colored_label(egui::Color32::RED,error.to_string());},
            }
        } else {
            ui.heading(if page=="account" {"UU 账号"}else{"UU 设备"});
            ui.horizontal(|ui| {
                ui.label(uu_i18n::text("设备名称"));ui.text_edit_singleline(&mut self.name);
                if self.token.is_none(){if ui.button(uu_i18n::text("扫码登录")).clicked(){self.login=Some(crate::login::spawn(self.context.data.clone(),self.name.clone()));self.qr=None;}}
                else {
                    if ui.button(uu_i18n::text("登出")).clicked(){self.sign_out(&mut actions);}
                    if ui.button(uu_i18n::text("管理 UU 设备")).clicked(){self.account.open(self.context.data.clone(),self.token.clone().unwrap());}
                    if ui.button(uu_i18n::text("保存设备名称")).clicked(){self.login=Some(crate::login::rename(self.context.data.clone(),self.name.clone()));}
                }
                if self.login.is_some()&&ui.button(uu_i18n::text("取消")).clicked(){self.login=None;self.qr=None;}
            });
            if let Some(qr)=&self.qr {ui.image(qr);}
            if self.account.show(ui.ctx(),Some(&self.context.data),self.token.as_deref())==Some(true){self.sign_out(&mut actions);}
            if self.token.is_some()&&page=="devices" {
                ui.horizontal(|ui| {
                    if ui.button(uu_i18n::text("刷新列表")).clicked(){self.refresh();actions.push(Action::RefreshDevices);}
                    if self.loading.is_some(){ui.spinner();}
                    for filter in [Filter::All,Filter::Recent,Filter::Favorites] {ui.selectable_value(&mut self.filter,filter,filter.label());}
                    ui.text_edit_singleline(&mut self.query);
                });
                for index in self.library.visible_indices(&self.devices,self.filter,&self.query) {
                    let device=self.devices[index].clone();
                    ui.push_id(&device.device_id,|ui|egui::Frame::group(ui.style()).inner_margin(12.).show(ui,|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(&device.alias);ui.weak(&device.status);
                            let favorite=self.library.is_favorite(&device.device_id);
                            if ui.selectable_label(favorite,"★").clicked(){self.library.toggle_favorite(&device.device_id);}
                            if ui.add_enabled(device.controllable,egui::Button::new(uu_i18n::text("连接"))).clicked(){
                                let target=super::runtime_plugin::Target::Device{device:device.device_id.clone()};
                                if let Ok(target)=serde_json::to_string(&target){self.library.record_opened(&device.device_id);actions.push(Action::Connect{identity:device.device_id.clone(),target:zeroize::Zeroizing::new(target),name:device.alias.clone(),view_only:false});}
                            }
                        });ui.monospace(&device.device_id);
                    }));
                }
                if let Some(notice)=self.library.notice(){ui.weak(notice);}
            }
            ui.separator();ui.label("通过设备 ID 和验证码连接");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.assistance_id);
                ui.add(egui::TextEdit::singleline(&mut *self.assistance_code).password(true));
                if ui.button(uu_i18n::text("连接")).clicked(){
                    match crate::assistance::Request::new(&self.assistance_id,&self.assistance_code) {
                        Ok(request)=>{let target=super::runtime_plugin::Target::Assistance{id:request.id.clone(),code:request.code().into()};
                            if let Ok(target)=serde_json::to_string(&target){actions.push(Action::Connect{identity:request.id.clone(),target:zeroize::Zeroizing::new(target),name:request.id,view_only:false});self.assistance_code.clear();}},
                        Err(error)=>self.notice=error.to_string(),
                    }
                }
            });
        }
        if !self.notice.is_empty(){ui.label(&self.notice);}
        actions
    }
}
