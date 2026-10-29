//! Official-client host adapter, opt-in and distinct from native device auth.
mod settings;
pub(crate) mod wire;
mod session;
pub(crate) mod input;
#[cfg(test)]
mod session_tests;
pub use settings::Settings;
pub(crate) use session::Session;
