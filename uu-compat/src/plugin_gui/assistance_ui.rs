//! The full original assistance form, separate from account configuration.
use super::*;
use uu_plugin_sdk::theme;
impl Ui {
    pub(super) fn assistance(&mut self,ui:&mut egui::Ui)->Vec<Action> {
        let mut actions=Vec::new();
        egui::Frame::none().fill(theme::SURFACE).stroke(egui::Stroke::new(1.,theme::BORDER)).rounding(12.).inner_margin(18.).show(ui,|ui| {
            ui.set_min_width(ui.available_width());
            ui.strong(uu_i18n::text("UU 官方远程协助"));
            ui.label(uu_i18n::text("让对方在 UU 官方被控端开启远程协助，再发送设备 ID 和验证码。"));
            ui.add_space(10.);ui.label(uu_i18n::text("粘贴对方分享的完整文本"));
            ui.add(egui::TextEdit::multiline(&mut *self.assistance_text).desired_rows(3).desired_width(f32::INFINITY).hint_text(uu_i18n::text("设备ID：…\n设备验证码：…")));
            if ui.button(uu_i18n::text("识别分享文本")).clicked(){match crate::assistance::Request::parse(&self.assistance_text) {
                Ok(request)=>{self.assistance_id=request.id.clone();self.assistance_code=zeroize::Zeroizing::new(request.code().into());self.assistance_text.clear();self.assistance_error.clear();},
                Err(error)=>self.assistance_error=error.to_string(),
            }}
            ui.add_space(10.);ui.label(uu_i18n::text("设备 ID"));
            ui.add(egui::TextEdit::singleline(&mut self.assistance_id).desired_width(f32::INFINITY).hint_text(uu_i18n::text("对方分享的数字 ID")));
            ui.add_space(6.);ui.label(uu_i18n::text("设备验证码"));
            ui.add(egui::TextEdit::singleline(&mut *self.assistance_code).password(true).desired_width(f32::INFINITY));ui.add_space(10.);
            if !self.assistance_error.is_empty(){ui.colored_label(theme::DANGER,&self.assistance_error);}
            if ui.add(egui::Button::new(egui::RichText::new(uu_i18n::text("发起远程协助")).color(egui::Color32::WHITE)).fill(theme::ACCENT)).clicked(){match crate::assistance::Request::new(&self.assistance_id,&self.assistance_code) {
                Ok(request)=>{let target=super::super::runtime_plugin::Target::Assistance{id:request.id.clone(),code:request.code().into()};
                    if let Ok(target)=serde_json::to_string(&target){actions.push(Action::Connect{identity:request.id.clone(),target:zeroize::Zeroizing::new(target),name:request.id,view_only:false});self.assistance_code.clear();self.assistance_text.clear();self.assistance_error.clear();}},
                Err(error)=>self.assistance_error=error.to_string(),
            }}
            ui.label(egui::RichText::new(uu_i18n::text("需登录 UU 账号。验证码只用于本次连接，不保存为设备授权。")).color(theme::MUTED));
        });
        actions
    }
}
