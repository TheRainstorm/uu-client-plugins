//! Optional protocol plugin. The engine does not depend on this crate.
extern crate self as uu_compat;
pub mod api;
mod runtime_plugin;
mod plugin_gui;
mod login;
mod account;
mod uu_devices;
mod device_library;
pub mod cli;
pub mod protocol;
pub mod selfloop;
pub use selfloop::SelfLoopConfig;
pub mod error;
pub mod assistance;
pub mod config;
pub mod connection;
mod ice;
pub mod controller;
mod controller_input;
#[cfg(test)]
mod relay_tests;
pub mod official;
pub mod media;
pub mod sign;
pub mod signal;
pub mod signing_key;
pub mod socketio;
mod compatibility_signing;
mod registration;

pub use api::{ClientState, QrLogin, UuApi};
pub use sign::ClientIdentity;
pub use signal::SignalClient;
pub use uu_core::{Config, capabilities, direct_ice, display_frames, session, connection_info, rtx, shared_host, native, install_crypto_provider};
pub mod control_wire;
pub mod screen_control;
pub mod cursor;
pub mod clipboard;
mod private_file { pub use uu_core::plugin_backend::write_private_file as write_atomic; }
mod congestion { pub use uu_core::plugin_backend::configure_congestion as configure; }
mod srtp_policy { pub use uu_core::plugin_backend::configure_srtp as configure; }
mod network_probe;

pub fn available()->bool {runtime_plugin::enabled()}
pub fn require_enabled()->anyhow::Result<()> {
    anyhow::ensure!(available(), "UU plugin is disabled");Ok(())
}
// Protocol entry points are registered by the generic SDK after explicit opt-in.
struct Service {context:uu_plugin_sdk::Context}
#[async_trait::async_trait]
impl uu_core::plugins::ConnectionService for Service {
    fn descriptor(&self) -> uu_core::plugins::Descriptor {
        uu_core::plugins::Descriptor { id: "uu", name: "UU compatibility", version: env!("CARGO_PKG_VERSION") }
    }
    async fn devices(&self,_config:&Config)->anyhow::Result<Vec<uu_core::plugins::RemoteDevice>> {
        runtime_plugin::devices(&self.context.data).await
    }
    async fn join(&self, config: &Config, device: &str, relay: bool) -> anyhow::Result<uu_core::internet::Connection> {
        connection::join(&self.context.data, device, runtime_plugin::relay(&self.context)?).await
    }
    async fn host(&self, config: &Config, relay: bool) -> anyhow::Result<Box<dyn uu_core::plugins::Listener>> {
        account::Listener::bind(Box::new(connection::Host::online(&self.context.data, runtime_plugin::relay(&self.context)?).await?),&self.context.data)
    }
    async fn listen(&self, config: &Config, relay: bool) -> anyhow::Result<Box<dyn uu_core::plugins::Listener>> {
        account::Listener::bind(Box::new(connection::Listener::online(&self.context.data, runtime_plugin::relay(&self.context)?).await?),&self.context.data)
    }
}
#[async_trait::async_trait]
impl uu_core::plugins::Listener for connection::Host {
    async fn accept(&mut self) -> anyhow::Result<uu_core::internet::Connection> { self.accept().await }
    async fn close(&mut self) { self.close().await; }
}
#[async_trait::async_trait]
impl uu_core::plugins::Listener for connection::Listener {
    async fn accept(&mut self) -> anyhow::Result<uu_core::internet::Connection> { self.accept().await }
    async fn accept_session(&mut self)->anyhow::Result<uu_core::plugins::Incoming>{self.accept_session().await}
    async fn close(&mut self) { self.close().await; }
}

#[derive(Clone, Debug, serde::Deserialize)]
struct Device {
    alias: String,
    device_id: String,
    status: String,
    #[serde(default)]
    controllable: bool,
    #[serde(default)]
    wallpaper_url: String,
    #[serde(default)]
    #[allow(dead_code)]
    platform: i64,
}

#[derive(Debug, serde::Deserialize)]
struct DeviceListData {
    #[serde(default)]
    my_binded_devices: Vec<Device>,
}

#[derive(Debug, serde::Deserialize)]
struct DeviceListResp {
    code: i64,
    #[serde(default)]
    data: Option<DeviceListData>,
}

fn parse_devices(body: serde_json::Value) -> anyhow::Result<Vec<Device>> {
    let resp: DeviceListResp = serde_json::from_value(body)?;
    anyhow::ensure!(resp.code == 0, "{}", uu_i18n::format!("获取设备列表失败，服务返回代码 {}", resp.code));
    Ok(resp.data.map(|d| d.my_binded_devices).unwrap_or_default())
}
