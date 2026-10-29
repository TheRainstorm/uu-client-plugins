//! Original account controls embedded in the corresponding connection card.
use super::*;
use uu_plugin_sdk::theme;
impl Ui {
    pub(super) fn account_ui(&mut self,ui:&mut egui::Ui) {
        ui.add_space(8.);
        ui.horizontal(|ui| {
            ui.label(uu_i18n::text("UU 本机名称"));ui.monospace(&self.name);
            if self.token.is_some()&&ui.add_enabled(self.login.is_none(),egui::Button::new(uu_i18n::text("更新注册信息"))).clicked(){self.login=Some(crate::login::rename(self.context.data.clone(),crate::config::uu_device_name()));}
        });
        ui.label(egui::RichText::new(uu_i18n::text("自动使用电脑名加 -Eclipse，便于与官方客户端区分；原生共享名称独立设置。")).color(theme::MUTED));
        if self.token.is_some() {
            ui.colored_label(theme::SUCCESS,uu_i18n::text("账号状态：已登录"));
            ui.horizontal(|ui| {
                if ui.button(uu_i18n::text("登出 UU 账号")).clicked(){self.sign_out();}
                if ui.button(uu_i18n::text("管理 UU 设备")).clicked(){if let Some(token)=self.token.clone(){self.account.open(self.context.data.clone(),token);}}
            });
            if let Some(id)=&self.device_id {ui.horizontal(|ui|{ui.label(uu_i18n::format!("本机 UU 编号：{id}"));if ui.small_button(uu_i18n::text("复制")).clicked(){ui.ctx().copy_text(id.clone());}});}
        } else {
            ui.label(uu_i18n::text("账号状态：未登录"));ui.label(uu_i18n::text("登录后可通过 UU 寻址原生设备。"));
            if let Some(qr)=&self.qr {ui.image((qr.id(),egui::vec2(180.,180.)));}
            let label=if self.qr.is_some(){uu_i18n::text("刷新登录二维码")}else{uu_i18n::text("登录 UU 账号")};
            if ui.add(egui::Button::new(egui::RichText::new(label).color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked(){self.start_login();}
            if self.login.is_some()&&ui.button(uu_i18n::text("取消登录")).clicked(){self.login=None;self.qr=None;self.notice=uu_i18n::text("已取消登录").into();}
        }
        if self.login.is_some(){ui.spinner();}
        if !self.notice.is_empty(){ui.label(&self.notice);}
        self.official_ui(ui);
        ui.add_space(8.);ui.label(egui::RichText::new(uu_i18n::text("启动服务后发布已启用入口。Eclipse 原生连接仍需设备配对。")).color(theme::MUTED));
    }
    fn official_ui(&mut self,ui:&mut egui::Ui) {
        ui.collapsing(uu_i18n::text("官方 UU 客户端连接本机（实验性）"),|ui| {
            let Ok(settings)=&mut self.official else {if let Err(error)=&self.official{ui.colored_label(theme::DANGER,error);}return;};
            ui.checkbox(&mut settings.enabled,uu_i18n::text("允许官方 UU 客户端连接本机"));
            ui.label(egui::RichText::new(uu_i18n::text("默认关闭。使用 UU 账号允许的连接，不授予 Eclipse 设备管理权限。")).color(theme::MUTED));
            ui.add_enabled_ui(settings.enabled,|ui| {
                ui.checkbox(&mut settings.control,uu_i18n::text("键盘和鼠标"));ui.checkbox(&mut settings.audio,uu_i18n::text("共享系统声音"));ui.checkbox(&mut settings.clipboard,uu_i18n::text("文本剪贴板"));
            });
            ui.label(egui::RichText::new(uu_i18n::text("仍受本机服务的输入、声音和剪贴板许可限制。保存后重启服务生效。")).color(theme::MUTED));
            if ui.button(uu_i18n::text("保存兼容设置")).clicked(){match settings.save(&self.context.data){Ok(())=>self.notice=uu_i18n::text("兼容设置已保存，重启服务后生效").into(),Err(error)=>self.notice=error.to_string()}}
        });
    }
    pub(super) fn settings_ui(&mut self,ui:&mut egui::Ui) {
        match self.context.preferences() {
            Ok(expected)=> {
                let mut relay=super::super::runtime_plugin::relay(&self.context).unwrap_or(false);
                if ui.checkbox(&mut relay,uu_i18n::text("允许官方中转（默认关闭）")).changed() {
                    let mut value=expected.clone();value.options.insert("official_relay".into(),relay.into());
                    if let Err(error)=self.context.save_preferences(&expected,value){self.notice=error.to_string();}
                }
                ui.label(egui::RichText::new(uu_i18n::text("优先使用点对点连接；修改中转设置后重新连接或重启共享入口生效。")).color(theme::MUTED));
                self.official_ui(ui);
            },Err(error)=>{ui.colored_label(theme::DANGER,error.to_string());},
        }
        if !self.notice.is_empty(){ui.label(&self.notice);}
    }
}
