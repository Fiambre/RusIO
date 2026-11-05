//! VNC remote desktop (RFB).
mod session;

use rusio_plugin_api::egui::{self, Color32, Pos2, Rect, RichText, Sense};
use rusio_plugin_api::icons::{self, Icon};
use rusio_plugin_api::serde_json;
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{
    Category, ConnectRequest, Connected, Endpoint, Host, Params, Plugin, ProtocolDef, ProtocolIcon,
    QuickConnect, SecretField, Session, WelcomeCard,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

use session::VncSession;

pub const PROTOCOL_ID: &str = "Vnc";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VncParams {
    pub host: String,
    pub port: u16,
    pub password: Option<String>,
}

impl VncParams {
    fn from_value(v: &Params) -> Result<Self, String> {
        let mut p: VncParams =
            serde_json::from_value(v.clone()).map_err(|e| format!("parámetros inválidos: {e}"))?;
        // An empty password (blanked by the secret store) means "none".
        if p.password.as_deref() == Some("") {
            p.password = None;
        }
        Ok(p)
    }

    fn to_value(&self) -> Params {
        serde_json::to_value(self).unwrap_or(Params::Null)
    }

    fn defaults() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 5900,
            password: None,
        }
    }
}

fn endpoint_get(params: &Params) -> Option<(String, u16)> {
    VncParams::from_value(params).ok().map(|p| (p.host, p.port))
}

fn endpoint_set(params: &Params, host: &str, port: u16) -> Params {
    match VncParams::from_value(params) {
        Ok(mut p) => {
            p.host = host.to_string();
            p.port = port;
            p.to_value()
        }
        Err(_) => params.clone(),
    }
}

/// `host:puerto` (port defaults to 5900).
fn parse_quick_connect(raw: &str) -> Option<Params> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let (host, port) = match raw.split_once(':') {
        Some((h, p)) => (h.to_string(), p.parse::<u16>().unwrap_or(5900)),
        None => (raw.to_string(), 5900),
    };
    if host.is_empty() {
        return None;
    }
    Some(VncParams { host, port, password: None }.to_value())
}

pub struct VncPlugin;

impl Plugin for VncPlugin {
    fn protocol(&self) -> ProtocolDef {
        ProtocolDef {
            id: PROTOCOL_ID,
            name: "VNC (Escritorio Remoto)",
            badge: "VNC",
            color: Theme::default().accent_vnc,
            secrets: vec![SecretField { tag: "vnc-password", pointer: "/password" }],
            quick_connect: Some(QuickConnect {
                label: "VNC",
                hint: "host:puerto",
                parse: parse_quick_connect,
            }),
            actions: vec![],
            welcome: Some(WelcomeCard {
                title: "VNC Remoto",
                subtitle: "Escritorio gráfico RFB",
            }),
            category: Category::Remote,
            icon: ProtocolIcon::Glyph(Icon::Monitor),
            gateway: false,
            endpoint: Some(Endpoint {
                get: endpoint_get,
                set: endpoint_set,
            }),
        }
    }

    fn default_params(&self) -> Params {
        VncParams::defaults().to_value()
    }

    fn summary(&self, params: &Params) -> String {
        VncParams::from_value(params)
            .map(|p| format!("{}:{}", p.host, p.port))
            .unwrap_or_default()
    }

    fn edit_ui(&self, ui: &mut egui::Ui, params: &mut Params) {
        let mut p = VncParams::from_value(params).unwrap_or_else(|_| VncParams::defaults());

        ui.horizontal(|ui| {
            ui.label("Host / IP:");
            ui.text_edit_singleline(&mut p.host);
            ui.label("Puerto:");
            let mut port_str = p.port.to_string();
            if ui
                .add(egui::TextEdit::singleline(&mut port_str).desired_width(60.0))
                .changed()
            {
                if let Ok(port) = port_str.parse::<u16>() {
                    p.port = port;
                }
            }
        });

        let mut pw = p.password.clone().unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label("Contraseña VNC (opcional):");
            ui.add(egui::TextEdit::singleline(&mut pw).password(true));
        });
        p.password = if pw.is_empty() { None } else { Some(pw) };

        p.host = p.host.trim().to_string();
        *params = p.to_value();
    }

    fn connect(&self, req: &ConnectRequest) -> Result<Connected, String> {
        let p = VncParams::from_value(req.params)?;
        let title = format!("VNC {}:{}", p.host, p.port);
        let ctx = req.ctx.clone();
        let session = VncSession::connect(p.host, p.port, p.password, move || ctx.request_repaint())
            .map_err(|e| format!("Error VNC: {e}"))?;
        Ok(Connected {
            title,
            session: Box::new(VncPane {
                session,
                texture: None,
            }),
        })
    }
}

/// The VNC tab: toolbar plus the remote framebuffer.
struct VncPane {
    session: VncSession,
    texture: Option<egui::TextureHandle>,
}

impl Session for VncPane {
    fn badge(&self) -> (&'static str, Color32) {
        ("VNC", Theme::default().accent_vnc)
    }

    fn is_alive(&self) -> bool {
        self.session.is_alive.load(Ordering::Relaxed)
    }

    fn ui(&mut self, ui: &mut egui::Ui, _host: &mut dyn Host) {
        let theme = Theme::default();

        ui.horizontal(|ui| {
            let (text, color) = if self.is_alive() {
                ("VNC CONECTADO", theme.status_online)
            } else {
                ("VNC DESCONECTADO", theme.status_offline)
            };
            icons::status_dot(ui, color);
            ui.label(RichText::new(text).color(color).size(10.0));

            let status = self.session.status_text.lock().clone();
            ui.label(RichText::new(status).size(10.0).color(theme.text_secondary));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Desconectar").clicked() {
                    self.session.close();
                }
                if ui
                    .button("Enviar Ctrl+Alt+Del")
                    .on_hover_text("Enviar combinación especial al host remoto")
                    .clicked()
                {
                    // Standard RFB keysyms: Ctrl (0xffe3), Alt (0xffe9), Del (0xffff)
                    self.session.send_key(true, 0xffe3);
                    self.session.send_key(true, 0xffe9);
                    self.session.send_key(true, 0xffff);
                    self.session.send_key(false, 0xffff);
                    self.session.send_key(false, 0xffe9);
                    self.session.send_key(false, 0xffe3);
                }
            });
        });

        ui.separator();

        // Framebuffer texture update
        {
            let mut fb = self.session.framebuffer.lock();
            if fb.dirty || self.texture.is_none() {
                let img = egui::ColorImage::from_rgba_unmultiplied([fb.width, fb.height], &fb.pixels);
                self.texture = Some(ui.ctx().load_texture(
                    "vnc_frame",
                    img,
                    egui::TextureOptions::LINEAR,
                ));
                fb.dirty = false;
            }
        }

        // Remote canvas
        if let Some(tex) = &self.texture {
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());

            ui.painter().image(
                tex.id(),
                rect,
                Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );

            if let Some(hover_pos) = response.hover_pos() {
                let (fw, fh) = {
                    let fb = self.session.framebuffer.lock();
                    (fb.width as f32, fb.height as f32)
                };
                let rel_x = ((hover_pos.x - rect.min.x) / rect.width() * fw).clamp(0.0, fw - 1.0) as u16;
                let rel_y = ((hover_pos.y - rect.min.y) / rect.height() * fh).clamp(0.0, fh - 1.0) as u16;

                let mut buttons = 0u8;
                ui.input(|i| {
                    if i.pointer.primary_down() {
                        buttons |= 1; // left
                    }
                    if i.pointer.middle_down() {
                        buttons |= 2; // middle
                    }
                    if i.pointer.secondary_down() {
                        buttons |= 4; // right
                    }
                });
                self.session.send_pointer(buttons, rel_x, rel_y);
            }
        }
    }

    fn close(&mut self) {
        self.session.close();
    }
}
