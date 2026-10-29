//! Everything identifying this service stays in its separately built library.
use anyhow::{ensure,Result};
use std::sync::{Arc,atomic::{AtomicBool,AtomicU8,Ordering}};
use uu_plugin_sdk::{Context,Plugin,Page,Placement,PageState};
use uu_core::{Config,client_settings::ClientSettings};
// Unit tests run only generated media and owned protocol fixtures. Production
// and integration-test builds always start disabled.
static ENABLED:AtomicBool=AtomicBool::new(cfg!(test));
static CONTEXT:std::sync::OnceLock<Context>=std::sync::OnceLock::new();
static MODULES:AtomicU8=AtomicU8::new(5);
pub fn modules()->super::modules::Modules {super::modules::Modules::from_bits(MODULES.load(Ordering::Acquire))}
pub fn require_client()->Result<()> {super::require_enabled()?;ensure!(modules().client,"UU compatibility client is disabled");Ok(())}
pub fn require_account()->Result<()> {super::require_enabled()?;ensure!(modules().account,"UU account connection service is disabled");Ok(())}
pub fn admits(native:bool)->bool {enabled()&&modules().admits(native)}
pub fn enabled()->bool {ENABLED.load(Ordering::Acquire)}
pub fn allow_relay()->Result<bool> {relay(CONTEXT.get().ok_or_else(||anyhow::anyhow!("UU plugin context unavailable"))?)}
pub fn relay(context:&Context)->Result<bool> {
    Ok(context.preferences()?.options.get("official_relay").and_then(serde_json::Value::as_bool).unwrap_or(false))
}
mod translations {include!(concat!(env!("OUT_DIR"),"/catalogue.rs"));}
pub struct Extension {pub context:Context}
fn create(context:Context)->Result<Box<dyn Plugin>> {
    uu_i18n::register_catalogue(translations::VALUES).map_err(anyhow::Error::msg)?;
    ensure!(context.id=="uu","UU plugin manifest ID must be uu");
    ensure!(CONTEXT.set(context.clone()).is_ok(),"UU plugin already initialized");
    Ok(Box::new(Extension{context}))
}
#[no_mangle]
pub extern "C" fn eclipse_plugin_entry_v1()->*const uu_plugin_sdk::Entry {
    static ENTRY:uu_plugin_sdk::Entry=uu_plugin_sdk::entry(create);&ENTRY
}
impl Plugin for Extension {
    fn configure(&self,preferences:&uu_core::plugins::PluginPreferences)->Result<()> {
        MODULES.store(super::modules::Modules::from_preferences(preferences)?.bits(),Ordering::Release);Ok(())
    }
    fn connection_enabled(&self)->bool {modules().connection()}
    fn client_support(&self)->uu_plugin_sdk::ClientSupport {uu_plugin_sdk::ClientSupport{host_managed_bitrate:true,max_fps:144,codecs:vec![uu_core::native::VideoCodec::H264,uu_core::native::VideoCodec::Hevc]}}
    fn set_enabled(&self,value:bool)->Result<()> {
        if value {migrate(&self.context)?;}
        ENABLED.store(value,Ordering::Release);Ok(())
    }
    fn pages(&self)->Vec<Page> {
        pages(modules())
    }
    fn gui(&self)->Result<Box<dyn PageState>> {Ok(Box::new(super::plugin_gui::Ui::new(self.context.clone())?))}
    fn service(&self)->Option<Arc<dyn uu_core::plugins::ConnectionService>> {Some(Arc::new(super::Service{context:self.context.clone()}))}
    fn client_settings(&self,mut defaults:ClientSettings)->Result<ClientSettings> {
        let path=self.context.data.at("client-settings.json");
        if path.exists(){let bytes=read_bounded(&path,64*1024)?;defaults=serde_json::from_slice(&bytes).map_err(|_|anyhow::anyhow!("UU client preferences are invalid"))?;}
        else {
            defaults.native_clipboard=defaults.clipboard;defaults.rate_control=None;defaults.fps=defaults.fps.min(144);
            if matches!(defaults.codec,uu_core::client_settings::CodecPreference::Av1|uu_core::client_settings::CodecPreference::Pyrowave){defaults.codec=uu_core::client_settings::CodecPreference::Auto;}
        }
        defaults.validate()?;Ok(defaults)
    }
    fn save_client(&self,settings:ClientSettings)->Result<()> {
        settings.validate()?;
        let path=self.context.data.at("client-settings.json");
        if path.exists(){let previous:ClientSettings=serde_json::from_slice(&read_bounded(&path,64*1024)?).map_err(|_|anyhow::anyhow!("UU client preferences are invalid; preserved original"))?;previous.validate()?;}
        super::private_file::write_atomic(&path,&serde_json::to_vec_pretty(&settings)?)?;Ok(())
    }
    fn connect(&self,target:zeroize::Zeroizing<String>,client:ClientSettings,stats:Arc<uu_core::media::MediaStats>)->Result<uu_core::controller::ControllerHandle> {
        require_client()?;client.validate()?;
        ensure!(client.fps<=144,"UU supports at most 144 FPS");
        ensure!(!matches!(client.codec,uu_core::client_settings::CodecPreference::Av1|uu_core::client_settings::CodecPreference::Pyrowave),"UU does not support the selected codec");
        let target:Target=serde_json::from_str(&target).map_err(|_|anyhow::anyhow!("invalid UU connection target"))?;
        let target=match target {Target::Device{device}=>{ensure!(!device.is_empty()&&device.len()<=128&&!device.chars().any(char::is_control),"invalid device ID");super::controller::Target::Device(device)},Target::Assistance{id,code}=>super::controller::Target::Assistance(super::assistance::Request::new(&id,&code)?)};
        stats.refresh_modifiers.store(true,Ordering::Release);
        stats.display_mode.store(u32::from(client.display_mode==uu_core::screen_control::DisplayMode::ExclusiveVirtual),Ordering::Release);
        stats.prefer_hevc.store(client.hevc_preferred(),Ordering::Release);
        let resolution=match client.resolution {1=>uu_core::video_options::CaptureResolution::Exact(1920,1080),2=>uu_core::video_options::CaptureResolution::Exact(2560,1440),3=>uu_core::video_options::CaptureResolution::Exact(3840,2160),_=>uu_core::video_options::CaptureResolution::Unchanged};
        let _=stats.output_resolution.set(resolution);
        super::controller::spawn_typed(self.context.data.clone(),target,super::config::geo_exit_ip(),stats,client)
    }
    fn cli(&self,arguments:Vec<String>)->Result<()> {super::require_enabled()?;super::cli::run(self.context.data.clone(),arguments)}
}
fn pages(modules:super::modules::Modules)->Vec<Page> {
    let mut pages=Vec::new();
    if modules.client {pages.push(Page{id:"devices".into(),title:uu_i18n::text("UU 兼容客户端").into(),placement:Placement::Navigation,
        icon:Some(uu_plugin_sdk::ui::Icon::Image{id:"uu-service-mark",bytes:include_bytes!("../assets/uu-remote.ico")})});}
    if modules.account {pages.push(Page{id:"account".into(),title:uu_i18n::text("UU 账号连接服务").into(),placement:Placement::Connections,icon:None});}
    if modules.server {pages.push(Page{id:"server".into(),title:uu_i18n::text("UU 兼容服务器（实验性）").into(),placement:Placement::Services,icon:None});}
    pages.push(Page{id:"settings".into(),title:uu_i18n::text("UU 插件设置").into(),placement:Placement::Settings,icon:None});pages
}
#[derive(serde::Deserialize,serde::Serialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum Target {Device{device:String},Assistance{id:String,code:String}}
pub async fn devices(config:&Config)->Result<Vec<uu_core::plugins::RemoteDevice>> {
    super::require_enabled()?;
    let api=super::UuApi::new(super::SignalClient::new(config)?,config.at("state.json"))?;
    let token=api.state.token.clone();let body=api.device_list().await?;
    ensure!(super::account::read_token(&config.at("state.json"))?==token,"UU account changed while reading devices");
    Ok(super::parse_devices(body)?.into_iter().map(|device|uu_core::plugins::RemoteDevice{id:device.device_id,name:device.alias,online:Some(matches!(device.status.as_str(),"CONNECTED"|"ready"|"ONLINE"))}).collect())
}
pub fn read_bounded(path:&std::path::Path,limit:u64)->Result<Vec<u8>> {
    use std::io::Read;
    let meta=std::fs::symlink_metadata(path)?;ensure!(meta.is_file()&&!meta.file_type().is_symlink()&&meta.len()<=limit,"invalid plugin configuration file");
    let mut bytes=Vec::new();std::fs::File::open(path)?.take(limit+1).read_to_end(&mut bytes)?;ensure!(bytes.len()as u64<=limit,"plugin configuration too large");Ok(bytes)
}
fn migrate(context:&Context)->Result<()> {
    // Explicit opt-in is the only path that touches old account files. Leave
    // originals intact, never overwrite an already namespaced configuration.
    std::fs::create_dir_all(&context.data.data_dir)?;
    for name in ["state.json","signing-key"] {
        let old=context.application.at(name);let new=context.data.at(name);
        if !new.exists()&&old.exists() {
            let bytes=read_bounded(&old,1024*1024)?;
            if name=="state.json" {super::ClientState::read(&old)?;}
            super::private_file::write_atomic(&new,&bytes)?;
        }
    }
    super::device_library::Library::migrate(context)?;
    let old_host=context.application.at("plugins/uu-compat-host.json");
    let host=context.data.at("host.json");
    if !host.exists()&&old_host.exists() {
        let bytes=read_bounded(&old_host,16*1024)?;
        let _:super::official::Settings=serde_json::from_slice(&bytes).map_err(|_|anyhow::anyhow!("legacy UU host settings are invalid; preserved original"))?;
        super::private_file::write_atomic(&host,&bytes)?;
    }
    let store=uu_core::device_book::Store::open(context.application.at("device-trust"))?;
    let book=store.read()?;
    let has_old=book.devices.iter().flat_map(|d|&d.routes).any(|r|matches!(&r.destination,uu_core::device_book::Destination::Unavailable(v) if v["kind"]=="uu"));
    let legacy_sharing=book.sharing.extensions.get("uu_enabled").and_then(serde_json::Value::as_bool);
    if has_old||legacy_sharing.is_some() {
        // Preserve the exact pre-migration document before the first rewrite.
        let original=context.application.at("device-trust/connections.json");
        let backup=context.data.at("legacy-connections.json");
        if !backup.exists(){super::private_file::write_atomic(&backup,&read_bounded(&original,1024*1024)?)?;}
        store.edit(|book| {
            if book.sharing.extensions.remove("uu_enabled").and_then(|v|v.as_bool()).unwrap_or(false)&&!book.sharing.plugin_services.contains(&context.id) {book.sharing.plugin_services.push(context.id.clone());}
            for route in book.devices.iter_mut().flat_map(|d|&mut d.routes) {
                if let uu_core::device_book::Destination::Unavailable(value)=&route.destination {
                    if value["kind"]=="uu" {
                        let device=value["device"].as_str().ok_or_else(||anyhow::anyhow!("invalid legacy UU route; original preserved"))?;
                        route.destination=uu_core::device_book::Destination::Plugin{plugin:context.id.clone(),device:device.into()};
                    }
                }
            }Ok(())
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_roles_follow_each_module_without_renaming_the_client() {
        for bits in 0..8 {
            let modules=crate::modules::Modules::from_bits(bits);let pages=pages(modules);
            assert_eq!(pages.iter().any(|p|p.id=="devices"),modules.client);
            assert_eq!(pages.iter().any(|p|p.id=="account"),modules.account);
            assert_eq!(pages.iter().any(|p|p.id=="server"),modules.server);
            assert!(pages.iter().any(|p|p.id=="settings"));
            if let Some(client)=pages.iter().find(|p|p.id=="devices") {assert_eq!(client.title,"UU 兼容客户端");assert!(client.icon.is_some());}
            if let Some(server)=pages.iter().find(|p|p.id=="server") {assert_eq!(server.placement,Placement::Services);}
        }
    }
    #[test]
    fn explicit_migration_preserves_accounts_unknown_routes_and_default_opt_out() {
        use uu_core::device_book::{Destination,Profile,Store};
        let root=std::env::temp_dir().join(format!("extension-migration-{}",uuid::Uuid::new_v4()));
        let config=Config{data_dir:root.join("data"),state_dir:root.join("state"),signaling_host:String::new()};
        let mut state=crate::ClientState::default();state.token=Some("generated-migration-token".into());
        state.save(&config.at("state.json")).unwrap();
        let original=std::fs::read(config.at("state.json")).unwrap();
        let mut library=crate::device_library::Library::for_account(Some(&config),Some("generated-migration-token"));
        library.toggle_favorite("generated-favorite");library.record_opened("generated-favorite");
        let store=Store::open(config.at("device-trust")).unwrap();
        store.edit(|book| {
            book.devices.push(Profile::new("Saved UU route",Destination::Unavailable(serde_json::json!({"kind":"uu","device":"generated-device"}))));
            book.devices.push(Profile::new("Other extension",Destination::Unavailable(serde_json::json!({"kind":"future-provider","opaque":"retained"}))));
            book.sharing.extensions.insert("uu_enabled".into(),true.into());Ok(())
        }).unwrap();
        let context=Context::new(&config,"uu");migrate(&context).unwrap();
        assert!(!context.preferences().unwrap().enabled);
        assert_eq!(std::fs::read(config.at("state.json")).unwrap(),original);
        assert_eq!(std::fs::read(context.data.at("state.json")).unwrap(),original);
        assert!(crate::device_library::Library::for_account(Some(&context.data),Some("generated-migration-token")).is_favorite("generated-favorite"));
        let book=store.read().unwrap();
        assert!(matches!(&book.devices[0].routes[0].destination,Destination::Plugin{plugin,device} if plugin=="uu"&&device=="generated-device"));
        assert!(matches!(&book.devices[1].routes[0].destination,Destination::Unavailable(value) if value["opaque"]=="retained"));
        assert!(book.sharing.plugin_services.contains(&"uu".into()));
        assert!(context.data.at("legacy-connections.json").is_file());
        crate::account::sign_out(&context.data.at("state.json")).unwrap();migrate(&context).unwrap();
        assert!(crate::account::read_token(&context.data.at("state.json")).unwrap().is_none(),"migration cannot restore signed-out credentials");
        std::fs::remove_dir_all(root).unwrap();
    }
}
