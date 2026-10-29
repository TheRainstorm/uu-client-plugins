//! Original device sidebar, preview and assistance navigation, owned by the plugin.
use super::*;
use uu_plugin_sdk::ui::{Icon,icon,icon_button,icon_button_selected};
use egui::{Color32 as C,Pos2,Rect,RichText,Sense,Stroke,Vec2};
use uu_plugin_sdk::theme::{ACCENT as BLUE,INK,MUTED,SIDEBAR};
fn text_at(p:&egui::Painter,pos:Pos2,text:impl Into<String>,size:f32,color:C) {p.text(pos,egui::Align2::LEFT_CENTER,text.into(),egui::FontId::proportional(size),color);}
fn online(status:&str)->bool {matches!(status,"CONNECTED"|"ready"|"ONLINE")}
impl Ui {
    pub(super) fn navigation(&mut self, ctx: &egui::Context) {
        if let Some(error) = self.account_error.clone() {
            egui::TopBottomPanel::top("local-account-error").show(ctx, |ui| {
                ui.add(egui::Label::new(RichText::new(error).color(C::from_rgb(177, 48, 48))).wrap());
                if ui.button(uu_i18n::text("检查配置后重试")).clicked() { self.account_error=None; }
            });
        }
        if !self.token.is_some() {
            egui::CentralPanel::default().show(ctx,|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(38.0);
                    ui.heading(uu_i18n::text("连接你的每一台设备"));
                    ui.add_space(12.0);
                    ui.label(RichText::new(uu_i18n::text("登录 UU 账号访问官方设备；点击左上角标志返回原生设备页")).color(MUTED));
                    ui.add_space(24.0);
                    ui.label(uu_i18n::text("UU 本机名称"));
                    ui.monospace(crate::config::uu_device_name());
                    ui.small(uu_i18n::text("自动使用电脑名加 -Eclipse，与官方客户端区分。"));
                    ui.add_space(12.0);
                    if let Some(texture)=&self.qr {
                        ui.image((texture.id(),egui::vec2(224.0,224.0)));
                        ui.add_space(14.0);
                    }else if self.login.is_some() {
                        ui.add(egui::Spinner::new().size(28.0));ui.add_space(14.0);
                    }
                    if !self.notice.is_empty(){ui.label(&self.notice);ui.add_space(10.0);}
                    if ui.add_enabled_ui(self.account_error.is_none(), |ui| ui.add_sized([180.0,36.0],egui::Button::new(RichText::new(if self.qr.is_some(){uu_i18n::text("刷新二维码")}else{uu_i18n::text("扫码登录")}).color(C::WHITE)).fill(BLUE))).inner.clicked(){self.start_login();}
                    if self.login.is_some()&&ui.button(uu_i18n::text("取消登录")).clicked(){self.login=None;self.qr=None;self.notice=uu_i18n::text("已取消登录").into();}
                    ui.add_space(14.0);
                    ui.add_space(8.0);
                    if ui.add_sized([180.0,32.0],egui::Button::new(if false{uu_i18n::text("管理当前共享")}else{uu_i18n::text("共享此设备")})).clicked(){self.actions.push(Action::OpenSharing);}
                    ui.add_space(8.0);
                    ui.small(uu_i18n::text("原生 IP 直连无需账号；旧邀请文件仍可拖入窗口"));
                    if !self.notice.is_empty(){ui.add_space(16.0);ui.label(RichText::new(&self.notice).color(MUTED));}
                });
            });
            return;
        }
        let compact_home = ctx.screen_rect().height() < 640.0;
        let sidebar_width = if ctx.screen_rect().width() < 800.0 {
            205.0
        } else {
            250.0
        };
        egui::SidePanel::left("home-devices")
            .exact_width(sidebar_width)
            .resizable(false)
            .frame(
                egui::Frame::none()
                    .fill(SIDEBAR)
                    .inner_margin(egui::Margin::same(10.0)),
            )
            .show(ctx, |ui| {
                if !compact_home {
                let (card, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 108.0), Sense::hover());
                ui.painter().rect_filled(card, 5.0, BLUE);
                text_at(
                    ui.painter(),
                    card.left_top() + egui::vec2(18.0, 38.0),
                    "远端，如在眼前",
                    17.0,
                    C::WHITE,
                );
                text_at(
                    ui.painter(),
                    card.left_top() + egui::vec2(18.0, 72.0),
                    uu_i18n::text("开源连接，自由协作"),
                    13.0,
                    C::from_white_alpha(220),
                );
                ui.add_space(15.0);
                }
                ui.horizontal(|ui| {
                    icon(
                        ui.painter(),
                        Rect::from_min_size(ui.cursor().min, Vec2::splat(22.0)),
                        Icon::Desktop,
                        INK,
                    );
                    ui.add_space(28.0);
                    if compact_home {
                        egui::ComboBox::from_id_salt("compact-device-filter").selected_text(self.filter.label()).show_ui(ui, |ui| {
                            for filter in [Filter::All, Filter::Recent, Filter::Favorites] {
                                ui.selectable_value(&mut self.filter, filter, filter.label());
                            }
                        });
                    } else { ui.label(RichText::new(uu_i18n::text("我的设备")).size(16.0)); }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icon_button(ui, Icon::Refresh, uu_i18n::text("刷新设备")).clicked() {
                            self.refresh();
                        }
                    });
                });
                ui.add_space(8.0);
                if ui.add(
                    egui::TextEdit::singleline(&mut self.query)
                        .hint_text(uu_i18n::text("搜索设备"))
                        .desired_width(f32::INFINITY)
                        .margin(egui::vec2(10.0, 6.0)),
                ).changed() { self.ensure_visible_device_selection(); }
                self.ensure_visible_device_selection();
                ui.add_space(8.0);
                let mut selected = None;
                let visible = self.library.visible_indices(&self.devices, self.filter, &self.query);
                let footer_height = if compact_home { 42.0 } else { 170.0 } + if self.library.notice().is_some() { 42.0 } else { 0.0 };
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - footer_height).max(50.0))
                    .show(ui, |ui| {
                        for index in visible {
                            let device = &self.devices[index];
                            let (row, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 45.0),
                                Sense::click(),
                            );
                            let active = self.selected == Some(index);
                            if active || response.hovered() {
                                ui.painter().rect_filled(
                                    row,
                                    5.0,
                                    if active {
                                        C::from_rgb(225, 232, 238)
                                    } else {
                                        C::from_rgb(232, 239, 244)
                                    },
                                );
                            }
                            if active {
                                ui.painter().rect_filled(
                                    Rect::from_min_size(
                                        row.left_center() + egui::vec2(0.0, -10.0),
                                        egui::vec2(3.0, 20.0),
                                    ),
                                    2.0,
                                    BLUE,
                                );
                            }
                            ui.painter().circle_filled(
                                row.left_center() + egui::vec2(17.0, 0.0),
                                3.5,
                                if online(&device.status) {
                                    C::from_rgb(46, 202, 83)
                                } else {
                                    C::from_rgb(182, 187, 194)
                                },
                            );
                            let tile = Rect::from_center_size(
                                row.left_center() + egui::vec2(44.0, 0.0),
                                Vec2::splat(25.0),
                            );
                            ui.painter().rect_filled(
                                tile,
                                5.0,
                                if online(&device.status) {
                                    BLUE
                                } else {
                                    C::from_rgb(178, 183, 190)
                                },
                            );
                            icon(ui.painter(), tile.shrink(4.0), Icon::Grid, C::WHITE);
                            let color = if online(&device.status) { INK } else { MUTED };
                            let mut job = egui::text::LayoutJob::simple(device.alias.clone(), egui::FontId::proportional(15.0), color, (row.width()-91.0).max(20.0));
                            job.wrap.max_rows = 1; job.wrap.break_anywhere = true;
                            let galley = ui.painter().layout_job(job);
                            let origin = row.left_center() + egui::vec2(66.0, -galley.size().y/2.0);
                            ui.painter().with_clip_rect(row).galley(origin, galley, color);
                            if self.library.is_favorite(&device.device_id) {
                                icon(ui.painter(), Rect::from_center_size(row.right_center()-egui::vec2(13.0,0.0), Vec2::splat(16.0)), Icon::Star, BLUE);
                            }
                            if response.clicked() {
                                selected = Some(index);
                            }
                            response.on_hover_text(&device.alias);
                        }
                    });
                if let Some(index) = selected {
                    self.selected = Some(index);
                    self.view =
                        View::Devices;
                }
                if !compact_home {
                ui.add_space(10.0);
                ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                let devices_view = self.view
                    == View::Devices;
                for (kind, label, filter) in [
                    (Icon::Grid, uu_i18n::text("全部设备"), Filter::All),
                    (Icon::Clock, uu_i18n::text("最近使用"), Filter::Recent),
                    (Icon::Star, uu_i18n::text("收藏设备"), Filter::Favorites),
                ] {
                    let active = devices_view && filter == self.filter;
                    let (row, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 34.0),
                        Sense::click(),
                    );
                    if active || response.hovered() { ui.painter().rect_filled(row, 5.0, C::from_rgb(225, 235, 247)); }
                    icon(
                        ui.painter(),
                        Rect::from_center_size(
                            row.left_center() + egui::vec2(28.0, 0.0),
                            Vec2::splat(22.0),
                        ),
                        kind,
                        if active { BLUE } else { MUTED },
                    );
                    text_at(
                        ui.painter(),
                        row.left_center() + egui::vec2(55.0, 0.0),
                        label,
                        15.0,
                        INK,
                    );
                    if response.clicked() {
                        self.view =
                            View::Devices;
                        self.filter = filter;
                        self.ensure_visible_device_selection();
                    }
                }
                });
                }
                let assistance_active = self.view
                    == View::Assistance;
                let (row, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 34.0),
                    Sense::click(),
                );
                if assistance_active || response.hovered() {
                    ui.painter().rect_filled(row, 5.0, C::from_rgb(225, 235, 247));
                }
                icon(
                    ui.painter(),
                    Rect::from_center_size(
                        row.left_center() + egui::vec2(28.0, 0.0),
                        Vec2::splat(22.0),
                    ),
                    Icon::Link,
                    if assistance_active { BLUE } else { MUTED },
                );
                text_at(
                    ui.painter(),
                    row.left_center() + egui::vec2(55.0, 0.0),
                    uu_i18n::text("远程协助"),
                    15.0,
                    INK,
                );
                if response.clicked() {
                    self.view =
                        View::Assistance;
                }
                if let Some(notice) = self.library.notice() { ui.add_space(6.0); ui.label(RichText::new(notice).small().color(MUTED)); }

            });
        if self.view
            == View::Assistance
        {
            let mut actions = Vec::new();
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::none()
                        .fill(C::from_rgb(248, 251, 253))
                        .inner_margin(egui::Margin::symmetric(36.0, 26.0)),
                )
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.heading(uu_i18n::text("远程协助"));
                        ui.label(RichText::new(uu_i18n::text("通过临时邀请帮助另一台设备")).color(MUTED));
                        ui.add_space(18.0);
                        actions = self.assistance(ui);
                    });
                });
            self.actions.extend(actions);
            return;
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(C::from_rgb(248, 251, 253))
                    .inner_margin(egui::Margin::symmetric(36.0, 26.0)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let Some(index) = self.selected.filter(|i| *i < self.devices.len()) else {
                        ui.add_space(80.0);
                        if let Some(error) = &self.list_error {
                            ui.heading(uu_i18n::text("设备加载失败"));
                            ui.add_space(12.0);
                            ui.add(egui::Label::new(RichText::new(error).color(C::from_rgb(177, 48, 48))).wrap());
                            ui.add_space(12.0);
                            ui.label(uu_i18n::text("设备列表暂时无法获取；本次加载失败不会清除本地登录状态。"));
                            ui.hyperlink_to(uu_i18n::text("连接凭据配置说明"), "https://github.com/TheRainstorm/uu-client/blob/master/docs/connection-credentials.md");
                            if ui.button(uu_i18n::text("重试加载设备")).clicked() {
                                self.refresh();
                            }
                        } else if self.loading.is_some() {
                            ui.heading(uu_i18n::text("正在加载设备…"));
                            ui.spinner();
                        } else if self.devices.is_empty() {
                            ui.heading(uu_i18n::text("此账号暂无设备"));
                            if ui.button(uu_i18n::text("刷新设备")).clicked() {
                                self.refresh();
                            }
                        } else {
                            ui.heading(if self.query.trim().is_empty() { self.filter.empty_message() } else { uu_i18n::text("没有匹配的设备") });
                            ui.label(match self.filter {
                                Filter::Favorites => uu_i18n::text("点击设备标题旁的星标，即可加入收藏。"),
                                Filter::Recent => uu_i18n::text("打开过串流窗口的设备会显示在这里。"),
                                Filter::All => uu_i18n::text("调整搜索条件，或刷新设备列表。"),
                            });
                            if !self.query.is_empty() && ui.button(uu_i18n::text("清除搜索")).clicked() { self.query.clear(); self.ensure_visible_device_selection(); }
                            if self.filter != Filter::All && ui.button(uu_i18n::text("查看全部设备")).clicked() { self.filter = Filter::All; self.ensure_visible_device_selection(); }
                        }
                        return;
                    };
                    let device = self.devices[index].clone();
                    let is_online = online(&device.status);
                    ui.horizontal(|ui| {
                        let (pill, _) =
                            ui.allocate_exact_size(egui::vec2(61.0, 27.0), Sense::hover());
                        ui.painter().rect_filled(pill, 14.0, INK);
                        ui.painter().circle_filled(
                            pill.left_center() + egui::vec2(13.0, 0.0),
                            4.0,
                            if is_online {
                                C::from_rgb(0, 225, 168)
                            } else {
                                MUTED
                            },
                        );
                        text_at(
                            ui.painter(),
                            pill.left_center() + egui::vec2(24.0, 0.0),
                            if is_online { uu_i18n::text("在线") } else { uu_i18n::text("离线") },
                            13.0,
                            C::WHITE,
                        );
                        ui.add_space(5.0);
                        let title_width = (ui.available_width()-96.0).max(24.0);
                        ui.allocate_ui_with_layout(egui::vec2(title_width, 40.0), egui::Layout::left_to_right(egui::Align::Center), |ui| ui.add(egui::Label::new(RichText::new(&device.alias)
                                .strong()
                                .size(if ui.available_width() < 300.0 {
                                    22.0
                                } else {
                                    29.0
                                })
                                .color(INK)).truncate())).inner.on_hover_text(&device.alias);
                        let favorite = self.library.is_favorite(&device.device_id);
                        if icon_button_selected(ui, Icon::Star, if favorite { uu_i18n::text("取消收藏") } else { uu_i18n::text("收藏设备") }, favorite).clicked() {
                            self.library.toggle_favorite(&device.device_id);
                            self.ensure_visible_device_selection();
                        }
                        ui.menu_button("···", |ui| {
                            ui.label(&device.device_id);
                            if ui.button(uu_i18n::text("复制设备 ID")).clicked() {
                                ui.output_mut(|o| o.copied_text = device.device_id.clone());
                                ui.close_menu();
                            }
                        });
                    });
                    ui.add_space(25.0);
                    let width = ui.available_width();
                    let height = (width * 0.40).clamp(140.0, 360.0);
                    let (hero, response) =
                        ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
                    if let Some(texture) = self.wallpapers.get(&index) {
                        let image_size = texture.size_vec2();
                        let scale = (hero.width() / image_size.x).max(hero.height() / image_size.y);
                        let uv_size = hero.size() / (image_size * scale);
                        let uv = Rect::from_center_size(egui::pos2(0.5, 0.5), uv_size);
                        ui.painter().image(texture.id(), hero, uv, C::WHITE);
                    } else {
                        ui.painter()
                            .rect_filled(hero, 7.0, C::from_rgb(169, 211, 250));
                        for n in (0..7).rev() {
                            let center =
                                hero.center() + egui::vec2(n as f32 * 24.0 - 50.0, n as f32 * 8.0);
                            ui.painter().with_clip_rect(hero).circle_stroke(
                                center,
                                hero.height() * 0.44 + n as f32 * 12.0,
                                Stroke::new(22.0, C::from_rgb(65 + n * 9, 133 + n * 8, 243)),
                            );
                        }
                    }
                    let label = if !is_online {
                        uu_i18n::text("设备当前离线")
                    } else if !device.controllable {
                        uu_i18n::text("设备暂不可连接")
                    } else {
                        uu_i18n::text("进入桌面  →")
                    };
                    let enter = Rect::from_center_size(
                        hero.center_bottom() + egui::vec2(0.0, -46.0),
                        egui::vec2(178.0, 46.0),
                    );
                    ui.painter().rect_filled(
                        enter,
                        24.0,
                        C::from_black_alpha(if response.hovered() { 130 } else { 90 }),
                    );
                    ui.painter().text(
                        enter.center(),
                        egui::Align2::CENTER_CENTER,
                        label,
                        egui::FontId::proportional(20.0),
                        C::WHITE,
                    );
                    if response.clicked() && is_online && device.controllable {
                        self.view_only = false;
                        self.connect_device();
                    }
                    if response.hovered() && is_online && device.controllable {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    ui.add_space(0.0);
                    egui::Frame::none()
                        .stroke(Stroke::new(1.0, C::from_rgb(216, 222, 228)))
                        .rounding(egui::Rounding {
                            nw: 0.0,
                            ne: 0.0,
                            sw: 7.0,
                            se: 7.0,
                        })
                        .inner_margin(egui::Margin::symmetric(4.0, 12.0))
                        .show(ui, |ui| {
                            let cell = (width - 16.0) / 4.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                for (kind, label, enabled) in [
                                    (Icon::Folder, uu_i18n::text("文件传输"), false),
                                    (Icon::Play, uu_i18n::text("观看模式"), is_online && device.controllable),
                                    (Icon::Terminal, uu_i18n::text("终端"), false),
                                    (Icon::Link, uu_i18n::text("端口映射"), false),
                                ] {
                                    let (r, response) = ui.allocate_exact_size(
                                        egui::vec2(cell, 30.0),
                                        if enabled {
                                            Sense::click()
                                        } else {
                                            Sense::hover()
                                        },
                                    );
                                    let color = if enabled {
                                        INK
                                    } else {
                                        C::from_rgb(167, 175, 184)
                                    };
                                    icon(
                                        ui.painter(),
                                        Rect::from_center_size(
                                            r.center()
                                                - egui::vec2(
                                                    if cell < 110.0 { 0.0 } else { 34.0 },
                                                    0.0,
                                                ),
                                            Vec2::splat(20.0),
                                        ),
                                        kind,
                                        color,
                                    );
                                    if cell >= 110.0 {
                                        text_at(
                                            ui.painter(),
                                            r.center() - egui::vec2(17.0, 0.0),
                                            label,
                                            14.0,
                                            color,
                                        );
                                    }
                                    if !enabled {
                                        response.on_hover_text(uu_i18n::text("暂未开放"));
                                    } else if response.on_hover_text(label).clicked() {
                                        self.view_only = true;
                                        self.connect_device();
                                    }
                                }
                            });
                        });
                    ui.add_space(24.0);
                    ui.label(RichText::new(uu_i18n::text("快速启动")).size(17.0).color(MUTED));
                    ui.add_space(11.0);
                    let (quick, response) =
                        ui.allocate_exact_size(egui::vec2(width, 115.0), Sense::click());
                    // Fine dashes match the reference's quiet empty-state card.
                    let stroke = Stroke::new(1.0, C::from_rgb(214, 222, 230));
                    for x in (0..width as usize).step_by(14) {
                        let x = quick.left() + x as f32;
                        ui.painter().line_segment(
                            [
                                egui::pos2(x, quick.top()),
                                egui::pos2((x + 7.0).min(quick.right()), quick.top()),
                            ],
                            stroke,
                        );
                        ui.painter().line_segment(
                            [
                                egui::pos2(x, quick.bottom()),
                                egui::pos2((x + 7.0).min(quick.right()), quick.bottom()),
                            ],
                            stroke,
                        );
                    }
                    for y in (0..115).step_by(14) {
                        let y = quick.top() + y as f32;
                        for x in [quick.left(), quick.right()] {
                            ui.painter().line_segment(
                                [
                                    egui::pos2(x, y),
                                    egui::pos2(x, (y + 7.0).min(quick.bottom())),
                                ],
                                stroke,
                            );
                        }
                    }
                    let center = quick.center() - egui::vec2(0.0, 10.0);
                    ui.painter()
                        .circle_filled(center, 22.0, C::from_rgb(232, 238, 243));
                    ui.painter().text(
                        center,
                        egui::Align2::CENTER_CENTER,
                        "+",
                        egui::FontId::proportional(30.0),
                        MUTED,
                    );
                    ui.painter().text(
                        center + egui::vec2(0.0, 38.0),
                        egui::Align2::CENTER_CENTER,
                        uu_i18n::text("连接此设备"),
                        egui::FontId::proportional(13.0),
                        INK,
                    );
                    if response.clicked() && is_online && device.controllable {
                        self.view_only = false;
                        self.connect_device();
                    }
                    ui.add_space(16.0);
                    ui.label(RichText::new(&self.notice).size(12.0).color(MUTED));
                });
            });
    }

}
