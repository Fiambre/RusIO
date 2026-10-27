use eframe::egui::{self, Color32, Pos2, Rect, RichText, Rounding, Sense, Stroke, Vec2};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;

use crate::config::{
    ConfigStore, ConnectionProfile, LocalShellType, ProtocolConfig, SshAuth,
};
use crate::session::{SessionKind, SessionTab};
use crate::terminal::{LocalSession, SharedTerminal, SshSession, TerminalEmulator, TerminalSession};
use crate::theme::Theme;
use crate::ui::{
    render_conn_modal, render_new_folder_modal, render_sidebar, render_tabs, ConnEditorState,
    ModalResult, SidebarAction, SidebarState, TabAction,
};
use crate::vnc::VncSession;

pub struct PulentoApp {
    config: ConfigStore,
    sidebar_state: SidebarState,
    tabs: Vec<SessionTab>,
    active_tab_idx: Option<usize>,
    modal_editor: Option<ConnEditorState>,
    modal_new_folder: Option<String>,
    quick_connect_host: String,
    quick_connect_proto: usize, // 0 = SSH, 1 = VNC
    status_msg: Option<(String, Instant)>,
}

impl PulentoApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let config = ConfigStore::load();
        Self {
            config,
            sidebar_state: SidebarState::default(),
            tabs: Vec::new(),
            active_tab_idx: None,
            modal_editor: None,
            modal_new_folder: None,
            quick_connect_host: String::new(),
            quick_connect_proto: 0,
            status_msg: Some((
                "Pulento iniciado en Full Rust. Listo para conectar.".to_string(),
                Instant::now(),
            )),
        }
    }

    pub fn set_status(&mut self, msg: String) {
        self.status_msg = Some((msg, Instant::now()));
    }

    pub fn launch_local(
        &mut self,
        shell: LocalShellType,
        working_dir: Option<String>,
        ctx: &egui::Context,
    ) {
        let cols = 100;
        let rows = 30;
        let emulator: SharedTerminal = Arc::new(Mutex::new(TerminalEmulator::new(cols, rows)));

        let ctx_clone = ctx.clone();
        let repaint_signal = move || {
            ctx_clone.request_repaint();
        };

        match LocalSession::spawn(shell.clone(), working_dir, cols, rows, emulator.clone(), repaint_signal) {
            Ok(session) => {
                let term = TerminalSession::new_local(session, emulator);
                let tab = SessionTab::new_terminal(shell.display_name().to_string(), term);
                self.tabs.push(tab);
                self.active_tab_idx = Some(self.tabs.len() - 1);
                self.set_status(format!("Iniciada terminal local {}", shell.display_name()));
            }
            Err(e) => {
                self.set_status(format!("Error iniciando terminal local: {}", e));
            }
        }
    }

    pub fn connect_ssh(
        &mut self,
        host: String,
        port: u16,
        username: String,
        auth: SshAuth,
        ctx: &egui::Context,
    ) {
        let cols = 100;
        let rows = 30;
        let emulator: SharedTerminal = Arc::new(Mutex::new(TerminalEmulator::new(cols, rows)));

        let ctx_clone = ctx.clone();
        let repaint_signal = move || {
            ctx_clone.request_repaint();
        };

        let title = format!("{}@{}:{}", username, host, port);

        match SshSession::connect(
            host.clone(),
            port,
            username,
            auth,
            cols,
            rows,
            emulator.clone(),
            repaint_signal,
        ) {
            Ok(session) => {
                let term = TerminalSession::new_ssh(session, emulator);
                let tab = SessionTab::new_terminal(title, term);
                self.tabs.push(tab);
                self.active_tab_idx = Some(self.tabs.len() - 1);
                self.set_status(format!("Conectando SSH a {}:{}...", host, port));
            }
            Err(e) => {
                self.set_status(format!("Error SSH: {}", e));
            }
        }
    }

    pub fn connect_vnc(
        &mut self,
        host: String,
        port: u16,
        password: Option<String>,
        ctx: &egui::Context,
    ) {
        let ctx_clone = ctx.clone();
        let repaint_signal = move || {
            ctx_clone.request_repaint();
        };

        let title = format!("VNC {}:{}", host, port);

        match VncSession::connect(host.clone(), port, password, repaint_signal) {
            Ok(session) => {
                let tab = SessionTab::new_vnc(title, session);
                self.tabs.push(tab);
                self.active_tab_idx = Some(self.tabs.len() - 1);
                self.set_status(format!("Conectando VNC a {}:{}...", host, port));
            }
            Err(e) => {
                self.set_status(format!("Error VNC: {}", e));
            }
        }
    }

    pub fn connect_profile(&mut self, profile: ConnectionProfile, ctx: &egui::Context) {
        match profile.protocol {
            ProtocolConfig::Ssh {
                host,
                port,
                username,
                auth,
            } => {
                self.connect_ssh(host, port, username, auth, ctx);
            }
            ProtocolConfig::Local { shell, working_dir } => {
                self.launch_local(shell, working_dir, ctx);
            }
            ProtocolConfig::Vnc {
                host,
                port,
                password,
            } => {
                self.connect_vnc(host, port, password, ctx);
            }
        }
    }

    pub fn close_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            let tab = self.tabs.remove(idx);
            tab.close();
            if self.tabs.is_empty() {
                self.active_tab_idx = None;
            } else if let Some(active) = self.active_tab_idx {
                if active >= self.tabs.len() {
                    self.active_tab_idx = Some(self.tabs.len() - 1);
                }
            }
        }
    }
}

impl eframe::App for PulentoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let theme = Theme::default();

        // Top bar: Branding & Quick Connect
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("🚀 Pulento")
                        .strong()
                        .size(15.0)
                        .color(theme.accent_primary),
                );
                ui.label(
                    RichText::new("v0.1.0 (Full Rust)")
                        .size(11.0)
                        .color(theme.text_muted),
                );

                ui.separator();

                // Quick Connect input
                ui.label(RichText::new("Conexión rápida:").size(12.0));
                egui::ComboBox::from_id_salt("quick_proto")
                    .selected_text(if self.quick_connect_proto == 0 {
                        "SSH"
                    } else {
                        "VNC"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.quick_connect_proto, 0, "SSH");
                        ui.selectable_value(&mut self.quick_connect_proto, 1, "VNC");
                    });

                let hint = if self.quick_connect_proto == 0 {
                    "usuario@host:puerto (ej: root@192.168.1.10)"
                } else {
                    "host:puerto (ej: 192.168.1.10:5900)"
                };

                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.quick_connect_host)
                        .hint_text(hint)
                        .desired_width(260.0),
                );

                let enter_pressed = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                if (ui.button(RichText::new("Conectar").strong().color(Color32::WHITE)).clicked() || enter_pressed)
                    && !self.quick_connect_host.trim().is_empty()
                {
                    let raw = self.quick_connect_host.trim().to_string();
                    if self.quick_connect_proto == 0 {
                        // SSH quick parse
                        let (user, rest) = if let Some((u, r)) = raw.split_once('@') {
                            (u.to_string(), r.to_string())
                        } else {
                            ("root".to_string(), raw)
                        };

                        let (host, port) = if let Some((h, p)) = rest.split_once(':') {
                            (h.to_string(), p.parse::<u16>().unwrap_or(22))
                        } else {
                            (rest, 22)
                        };

                        self.connect_ssh(host, port, user, SshAuth::Password(String::new()), ctx);
                    } else {
                        // VNC quick parse
                        let (host, port) = if let Some((h, p)) = raw.split_once(':') {
                            (h.to_string(), p.parse::<u16>().unwrap_or(5900))
                        } else {
                            (raw, 5900)
                        };

                        self.connect_vnc(host, port, None, ctx);
                    }
                }

                // Status message on right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((ref msg, _)) = self.status_msg {
                        ui.label(RichText::new(msg).size(11.0).color(theme.text_secondary));
                    }
                });
            });
            ui.add_space(4.0);
        });

        // Left Panel: Connection Manager with Folders
        egui::SidePanel::left("sidebar_panel")
            .resizable(true)
            .default_width(280.0)
            .width_range(220.0..=450.0)
            .show(ctx, |ui| {
                if let Some(action) =
                    render_sidebar(ui, &mut self.config, &mut self.sidebar_state)
                {
                    match action {
                        SidebarAction::ConnectProfile(profile) => {
                            self.connect_profile(profile, ctx);
                        }
                        SidebarAction::LaunchLocal(shell) => {
                            self.launch_local(shell, None, ctx);
                        }
                        SidebarAction::CreateNewConnection(folder_id) => {
                            let mut state = ConnEditorState::default();
                            state.folder_id = folder_id;
                            self.modal_editor = Some(state);
                        }
                        SidebarAction::CreateNewFolder => {
                            self.modal_new_folder = Some("Nueva Carpeta".to_string());
                        }
                        SidebarAction::EditConnection(profile) => {
                            self.modal_editor = Some(ConnEditorState::from_profile(&profile));
                        }
                        SidebarAction::DeleteConnection(id) => {
                            self.config.remove_connection(id);
                            self.set_status("Conexión eliminada.".to_string());
                        }
                        SidebarAction::DeleteFolder(id) => {
                            self.config.remove_folder(id);
                            self.set_status("Carpeta eliminada.".to_string());
                        }
                    }
                }
            });

        // Central Panel: Tabs + Session Viewport
        egui::CentralPanel::default().show(ctx, |ui| {
            // Tab bar
            if let Some(tab_action) =
                render_tabs(ui, &self.tabs, self.active_tab_idx)
            {
                match tab_action {
                    TabAction::SelectTab(idx) => {
                        self.active_tab_idx = Some(idx);
                    }
                    TabAction::CloseTab(idx) => {
                        self.close_tab(idx);
                    }
                    TabAction::NewTabLocal(shell) => {
                        self.launch_local(shell, None, ctx);
                    }
                    TabAction::OpenQuickConnect => {
                        self.modal_editor = Some(ConnEditorState::default());
                    }
                }
            }

            ui.separator();

            // Main Viewport
            if let Some(active_idx) = self.active_tab_idx {
                if active_idx < self.tabs.len() {
                    let tab = &mut self.tabs[active_idx];

                    match &mut tab.kind {
                        SessionKind::Terminal(terminal) => {
                            // Top session toolbar
                            ui.horizontal(|ui| {
                                let is_alive = terminal.is_alive();
                                let status_badge = if is_alive {
                                    RichText::new("● CONECTADO").color(theme.status_online).size(10.0)
                                } else {
                                    RichText::new("● DESCONECTADO").color(theme.status_offline).size(10.0)
                                };
                                ui.label(status_badge);

                                let (cols, rows) = {
                                    let emu = terminal.emulator.lock();
                                    (emu.cols(), emu.rows())
                                };
                                ui.label(
                                    RichText::new(format!("{}x{}", cols, rows))
                                        .size(10.0)
                                        .color(theme.text_muted),
                                );

                                ui.separator();

                                ui.label(RichText::new("Fuente:").size(11.0));
                                if ui.small_button("-").clicked() && terminal.font_size > 9.0 {
                                    terminal.font_size -= 1.0;
                                }
                                ui.label(format!("{:.0}", terminal.font_size));
                                if ui.small_button("+").clicked() && terminal.font_size < 28.0 {
                                    terminal.font_size += 1.0;
                                }

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui
                                        .button(RichText::new("Cerrar").size(11.0))
                                        .clicked()
                                    {
                                        terminal.close();
                                    }
                                    if ui
                                        .button(RichText::new("Limpiar").size(11.0))
                                        .clicked()
                                    {
                                        terminal.write(b"\x0c");
                                    }
                                });
                            });

                            ui.separator();

                            // Terminal Grid
                            let (input_bytes, resized_to) = {
                                let mut emu = terminal.emulator.lock();
                                emu.render(ui, terminal.font_size, true)
                            };

                            if let Some((new_cols, new_rows)) = resized_to {
                                terminal.resize(new_cols, new_rows);
                            }

                            if let Some(bytes) = input_bytes {
                                terminal.write(&bytes);
                            }
                        }
                        SessionKind::Vnc { session, texture } => {
                            // VNC Toolbar
                            ui.horizontal(|ui| {
                                let is_alive = session.is_alive.load(std::sync::atomic::Ordering::Relaxed);
                                let status_badge = if is_alive {
                                    RichText::new("● VNC CONECTADO").color(theme.status_online).size(10.0)
                                } else {
                                    RichText::new("● VNC DESCONECTADO").color(theme.status_offline).size(10.0)
                                };
                                ui.label(status_badge);

                                let status = session.status_text.lock().clone();
                                ui.label(RichText::new(status).size(10.0).color(theme.text_secondary));

                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("Desconectar").clicked() {
                                        session.close();
                                    }
                                    if ui
                                        .button("Enviar Ctrl+Alt+Del")
                                        .on_hover_text("Enviar combinación especial al host remoto")
                                        .clicked()
                                    {
                                        // Standard RFB keysyms: Ctrl (0xffe3), Alt (0xffe9), Del (0xffff)
                                        session.send_key(true, 0xffe3);
                                        session.send_key(true, 0xffe9);
                                        session.send_key(true, 0xffff);
                                        session.send_key(false, 0xffff);
                                        session.send_key(false, 0xffe9);
                                        session.send_key(false, 0xffe3);
                                    }
                                });
                            });

                            ui.separator();

                            // Framebuffer Texture Update
                            {
                                let mut fb = session.framebuffer.lock();
                                if fb.dirty || texture.is_none() {
                                    let img = egui::ColorImage::from_rgba_unmultiplied(
                                        [fb.width, fb.height],
                                        &fb.pixels,
                                    );
                                    *texture = Some(ctx.load_texture(
                                        "vnc_frame",
                                        img,
                                        egui::TextureOptions::LINEAR,
                                    ));
                                    fb.dirty = false;
                                }
                            }

                            // Render VNC Canvas
                            if let Some(tex) = texture {
                                let avail = ui.available_size();
                                let (rect, response) = ui.allocate_exact_size(
                                    avail,
                                    Sense::click_and_drag(),
                                );

                                ui.painter().image(
                                    tex.id(),
                                    rect,
                                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0)),
                                    Color32::WHITE,
                                );

                                // Mouse Pointer Events
                                if let Some(hover_pos) = response.hover_pos() {
                                    let fb = session.framebuffer.lock();
                                    let fw = fb.width as f32;
                                    let fh = fb.height as f32;

                                    let rel_x = ((hover_pos.x - rect.min.x) / rect.width() * fw)
                                        .clamp(0.0, fw - 1.0) as u16;
                                    let rel_y = ((hover_pos.y - rect.min.y) / rect.height() * fh)
                                        .clamp(0.0, fh - 1.0) as u16;

                                    let mut buttons = 0u8;
                                    ui.input(|i| {
                                        if i.pointer.primary_down() {
                                            buttons |= 1; // Left button
                                        }
                                        if i.pointer.middle_down() {
                                            buttons |= 2; // Middle button
                                        }
                                        if i.pointer.secondary_down() {
                                            buttons |= 4; // Right button
                                        }
                                    });

                                    session.send_pointer(buttons, rel_x, rel_y);
                                }
                            }
                        }
                    }
                }
            } else {
                // Empty state: Modern Welcome Dashboard
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(
                        RichText::new("⚡ PULENTO")
                            .size(32.0)
                            .strong()
                            .color(theme.accent_primary),
                    );
                    ui.label(
                        RichText::new("Cliente SSH y Gestor Remoto Nativo en Rust")
                            .size(15.0)
                            .color(theme.text_secondary),
                    );

                    ui.add_space(30.0);

                    ui.horizontal(|ui| {
                        let card_width = 180.0;
                        let card_height = 110.0;
                        ui.spacing_mut().item_spacing = Vec2::new(14.0, 14.0);

                        // Card: PowerShell
                        let frame = egui::Frame::none()
                            .fill(theme.bg_panel_alt)
                            .stroke(Stroke::new(1.0_f32, theme.border))
                            .rounding(Rounding::same(8.0))
                            .inner_margin(egui::Margin::same(14.0));

                        frame.show(ui, |ui| {
                            ui.set_width(card_width);
                            ui.set_height(card_height);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("⚡").size(24.0));
                                ui.label(RichText::new("PowerShell").strong().size(14.0));
                                ui.label(RichText::new("Terminal local ConPTY").size(11.0).color(theme.text_muted));
                                ui.add_space(4.0);
                                if ui.button(RichText::new("Abrir").color(Color32::WHITE)).clicked() {
                                    self.launch_local(LocalShellType::PowerShell, None, ctx);
                                }
                            });
                        });

                        // Card: WSL Linux
                        frame.show(ui, |ui| {
                            ui.set_width(card_width);
                            ui.set_height(card_height);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("🐧").size(24.0));
                                ui.label(RichText::new("WSL Linux").strong().size(14.0));
                                ui.label(RichText::new("Ubuntu / Debian Bash").size(11.0).color(theme.text_muted));
                                ui.add_space(4.0);
                                if ui.button(RichText::new("Abrir").color(Color32::WHITE)).clicked() {
                                    self.launch_local(LocalShellType::Wsl, None, ctx);
                                }
                            });
                        });

                        // Card: Nueva Conexión SSH
                        frame.show(ui, |ui| {
                            ui.set_width(card_width);
                            ui.set_height(card_height);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("🌐").size(24.0));
                                ui.label(RichText::new("Servidor SSH").strong().size(14.0));
                                ui.label(RichText::new("Conexión remota segura").size(11.0).color(theme.text_muted));
                                ui.add_space(4.0);
                                if ui.button(RichText::new("Conectar").color(Color32::WHITE)).clicked() {
                                    self.modal_editor = Some(ConnEditorState::default());
                                }
                            });
                        });

                        // Card: VNC
                        frame.show(ui, |ui| {
                            ui.set_width(card_width);
                            ui.set_height(card_height);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new("🖥").size(24.0));
                                ui.label(RichText::new("VNC Remoto").strong().size(14.0));
                                ui.label(RichText::new("Escritorio gráfico RFB").size(11.0).color(theme.text_muted));
                                ui.add_space(4.0);
                                if ui.button(RichText::new("Conectar").color(Color32::WHITE)).clicked() {
                                    let mut state = ConnEditorState::default();
                                    state.protocol_kind = 2;
                                    self.modal_editor = Some(state);
                                }
                            });
                        });
                    });

                    ui.add_space(30.0);
                    ui.label(
                        RichText::new("💡 Selecciona una conexión en el gestor de carpetas de la izquierda o crea una nueva.")
                            .size(12.0)
                            .color(theme.text_muted),
                    );
                });
            }
        });

        // Modals
        if let Some(mut state) = self.modal_editor.take() {
            if let Some(res) = render_conn_modal(ctx, &mut state, &self.config.folders) {
                match res {
                    ModalResult::Save(profile) => {
                        if state.id.is_some() {
                            self.config.update_connection(profile);
                            self.set_status("Conexión actualizada.".to_string());
                        } else {
                            self.config.add_connection(profile);
                            self.set_status("Nueva conexión guardada.".to_string());
                        }
                    }
                    ModalResult::SaveAndConnect(profile) => {
                        let p = profile.clone();
                        if state.id.is_some() {
                            self.config.update_connection(profile);
                        } else {
                            self.config.add_connection(profile);
                        }
                        self.connect_profile(p, ctx);
                    }
                    ModalResult::Cancel => {}
                }
            } else {
                self.modal_editor = Some(state);
            }
        }

        if let Some(mut name) = self.modal_new_folder.take() {
            if let Some(created) = render_new_folder_modal(ctx, &mut name) {
                if created && !name.trim().is_empty() {
                    self.config.add_folder(name.trim().to_string());
                    self.set_status("Carpeta creada.".to_string());
                }
            } else {
                self.modal_new_folder = Some(name);
            }
        }
    }
}
