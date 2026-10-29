//! Three independently persisted capabilities of the same installed library.
use anyhow::{ensure,Result};
use uu_core::plugins::PluginPreferences;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct Modules {pub client:bool,pub server:bool,pub account:bool}
impl Default for Modules {
    fn default()->Self {Self{client:true,server:false,account:true}}
}
impl Modules {
    pub fn from_preferences(preferences:&PluginPreferences)->Result<Self> {
        let read=|key:&str,default|->Result<bool> {
            let Some(value)=preferences.options.get(key) else{return Ok(default);};
            ensure!(value.is_boolean(),"invalid UU module setting: {key}");Ok(value.as_bool().unwrap())
        };
        Ok(Self{client:read("compatibility_client",true)?,server:read("compatibility_server",false)?,account:read("account_service",true)?})
    }
    pub fn store(self,preferences:&mut PluginPreferences) {
        for (key,value) in [("compatibility_client",self.client),("compatibility_server",self.server),("account_service",self.account)] {
            preferences.options.insert(key.into(),value.into());
        }
    }
    pub const fn bits(self)->u8 {self.client as u8|((self.server as u8)<<1)|((self.account as u8)<<2)}
    pub fn from_bits(bits:u8)->Self {Self{client:bits&1!=0,server:bits&2!=0,account:bits&4!=0}}
    pub fn connection(self)->bool {self.account||self.server}
    pub fn admits(self,native:bool)->bool {if native{self.account}else{self.server}}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_every_module_combination_remain_independent() {
        assert_eq!(Modules::from_preferences(&PluginPreferences::default()).unwrap(),Modules::default());
        for bits in 0..8 {
            let modules=Modules::from_bits(bits);let mut preferences=PluginPreferences::default();
            preferences.options.insert("other_option".into(),"preserved".into());modules.store(&mut preferences);
            assert_eq!(Modules::from_preferences(&preferences).unwrap(),modules);
            assert_eq!(modules.bits(),bits);assert_eq!(modules.admits(true),bits&4!=0);assert_eq!(modules.admits(false),bits&2!=0);
            assert_eq!(modules.connection(),bits&6!=0);assert_eq!(preferences.options["other_option"],"preserved");
        }
    }
    #[test]
    fn damaged_explicit_flags_do_not_fall_back_to_enabled_defaults() {
        let mut preferences=PluginPreferences::default();preferences.options.insert("compatibility_client".into(),"invalid".into());
        assert!(Modules::from_preferences(&preferences).is_err());
    }
}
