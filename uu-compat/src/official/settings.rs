use anyhow::{ensure,Context,Result};
use serde::{Deserialize,Serialize};

/// This permission is separate from native public-key device pairing.
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
#[serde(default,deny_unknown_fields)]
pub struct Settings {
    pub enabled:bool,
    pub control:bool,
    pub audio:bool,
    pub clipboard:bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_stay_disabled_and_permissions_survive_reload_without_overwriting_bad_files(){
        let directory=std::env::temp_dir().join(format!("eclipse-official-settings-{}",uuid::Uuid::new_v4()));
        let config=uu_core::Config {data_dir:directory.clone(),state_dir:directory.join("state"),signaling_host:String::new()};
        assert!(!Settings::load(&config).unwrap().enabled);
        let wanted=Settings {enabled:true,control:false,audio:true,clipboard:false};wanted.save(&config).unwrap();
        assert_eq!(Settings::load(&config).unwrap(),wanted);
        let path=config.at("host.json");std::fs::write(&path,b"{broken}").unwrap();
        assert!(Settings::load(&config).is_err());assert_eq!(std::fs::read(path).unwrap(),b"{broken}");
        assert!(!Settings::for_module(&config,false).unwrap().enabled,"disabled server configuration must not block the account transport");
        assert!(Settings::for_module(&config,true).is_err(),"enabled server must retain configuration validation");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
impl Default for Settings {
    fn default()->Self {Self {enabled:false,control:true,audio:true,clipboard:true}}
}
impl Settings {
    pub(crate) fn for_module(config:&uu_core::Config,enabled:bool)->Result<Self> {
        if !enabled{return Ok(Self::default());}
        let mut value=Self::load(config)?;value.enabled=true;Ok(value)
    }
    pub fn load(config:&uu_core::Config)->Result<Self> {
        let path=config.at("host.json");
        if !path.exists(){return Ok(Self::default());}
        let bytes=crate::runtime_plugin::read_bounded(&path,16*1024).context("cannot read compatibility host settings")?;
        ensure!(bytes.len()<=16*1024,"compatibility host settings exceed size limit");
        serde_json::from_slice(&bytes).context("invalid compatibility host settings; original file preserved")
    }
    pub fn save(&self,config:&uu_core::Config)->Result<()> {
        let path=config.at("host.json");
        std::fs::create_dir_all(path.parent().unwrap())?;
        uu_core::plugin_backend::write_private_file(&path,&serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}
