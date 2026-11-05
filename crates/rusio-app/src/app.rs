use crossbeam_channel::{unbounded, Receiver, TryRecvError};
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke, Vec2};
use rusio_core::config::{ConfigStore, ConnectionProfile};
use rusio_core::settings::Settings;
use rusio_plugin_api::icons;
use rusio_plugin_api::serde_json::{json, Value};
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{Category, ProtocolIcon};
use rusio_plugin_api::{ConnectRequest, Host, Params, QuickLaunch, SettingField, SettingKind, TextFile};
use std::any::Any;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use uuid::Uuid;

use crate::editor::EditorSession;
use crate::registry::Registry;
use crate::session::SessionTab;
use crate::tunnels;
use crate::ui::{
    render_conn_modal, render_new_folder_modal, render_sidebar, render_tabs, render_type_picker, titlebar,
    ConnEditorState, ModalResult, SidebarAction, SidebarState, TabAction, TypePick,
};

enum PendingDelete {
    Connection(uuid::Uuid, String),
    Folder(uuid::Uuid, String),
}

/// Collects what sessions ask of the app while they draw; applied after the frame's panels.
#[derive(Default)]
struct AppHost {
    editors: Vec<Arc<dyn TextFile>>,
    status: Option<String>,
}

impl Host for AppHost {
    fn open_text_editor(&mut self, file: Arc<dyn TextFile>) {
        self.editors.push(file);
    }

    fn set_status(&mut self, msg: String) {
        self.status = Some(msg);
    }
}

/// A connection waiting for its tunnels to come up (they open on a worker thread).
struct PendingConnect {
    rx: Receiver<Result<tunnels::Resolved, String>>,
    plugin_id: String,
    action: Option<String>,
}

/// A card on the welcome screen.
struct WelcomeItem {
    category: Category,
    plugin_id: &'static str,
    icon: ProtocolIcon,
    color: Color32,
    title: String,
    subtitle: String,
    button: &'static str,
    action: WelcomeAction,
}

#[derive(Clone)]
enum WelcomeAction {
    Quick(&'static str, Params),
    NewConnection(&'static str),
}

pub struct RusioApp {
    registry: Arc<Registry>,
    pending: Vec<PendingConnect>,
    /// Cached one-click launchers of all plugins.
    quick: Vec<(&'static str, QuickLaunch)>,
    config: ConfigStore,
    sidebar_state: SidebarState,
    tabs: Vec<SessionTab>,
    active_tab_idx: Option<usize>,
    modal_editor: Option<ConnEditorState>,
    /// "Pick a type" step before a new connection; holds the target folder.
    modal_type_picker: Option<Option<Uuid>>,
    modal_new_folder: Option<String>,
    pending_delete: Option<PendingDelete>,
    pending_discard: Option<usize>,
    settings: Settings,
    show_settings: bool,
    quick_connect_host: String,
    /// Index into the registry's quick-connect protocols.
    quick_connect_proto: usize,
    status_msg: Option<(String, Instant)>,
    /// Result of the background check for a newer release.
    update_rx: Option<crossbeam_channel::Receiver<Result<Option<crate::updates::UpdateInfo>, String>>>,
    /// A newer release, once found.
    update: Option<crate::updates::UpdateInfo>,
    /// Outcome of a manual "Buscar ahora", shown in Settings.
    update_note: Option<String>,
    show_update: bool,
    /// The installer being downloaded and verified.
    update_job: Option<UpdateJob>,
    update_error: Option<String>,
    /// A verified installer to start once the app has closed.
    install_on_exit: Option<std::path::PathBuf>,
}

struct UpdateJob {
    rx: crossbeam_channel::Receiver<Result<std::path::PathBuf, String>>,
    progress: crate::updates::Progress,
}

impl RusioApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let registry = Arc::new(Registry::builtin());
        let settings = Settings::load();
        let config = ConfigStore::load(settings.connections_path_or_default(), registry.secret_schema());
        let startup_msg = if config.secrets_unavailable {
            "Aviso: el almacén de credenciales no está disponible; las contraseñas se guardan en el archivo de conexiones."
        } else {
            "RusIO iniciado en Full Rust. Listo para conectar."
        };
        let quick = registry.quick_launches();
        let mut app = Self {
            registry,
            pending: Vec::new(),
            quick,
            config,
            sidebar_state: SidebarState::default(),
            tabs: Vec::new(),
            active_tab_idx: None,
            modal_editor: None,
            modal_type_picker: None,
            modal_new_folder: None,
            pending_delete: None,
            pending_discard: None,
            settings,
            show_settings: false,
            quick_connect_host: String::new(),
            quick_connect_proto: 0,
            status_msg: Some((startup_msg.to_string(), Instant::now())),
            update_rx: None,
            update: None,
            update_note: None,
            show_update: false,
            update_job: None,
            update_error: None,
            install_on_exit: None,
        };
        if app.settings.check_updates {
            app.start_update_check();
        }
        app
    }

    fn start_update_check(&mut self) {
        if self.update_rx.is_some() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.update_rx = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(crate::updates::check(env!("CARGO_PKG_VERSION")));
        });
    }

    /// Window that offers the new version and, on request, downloads, verifies and runs its installer.
    fn render_update(&mut self, ctx: &egui::Context) {
        use crossbeam_channel::TryRecvError;
        if let Some(job) = &self.update_job {
            match job.rx.try_recv() {
                Ok(Ok(path)) => {
                    self.update_job = None;
                    // The installer is started from `on_exit`, once the window and sessions are closing.
                    self.install_on_exit = Some(path);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                Ok(Err(e)) => {
                    self.update_job = None;
                    self.update_error = Some(e);
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
                Err(TryRecvError::Disconnected) => self.update_job = None,
            }
        }
        if !self.show_update {
            return;
        }
        let Some(info) = self.update.clone() else { return };
        let theme = Theme::default();
        let mut open = true;
        let mut start = false;
        egui::Window::new("Actualizar RusIO")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!("Versión {} disponible (tienes la {})", info.version, env!("CARGO_PKG_VERSION")))
                        .strong(),
                );
                ui.add_space(6.0);
                if info.installer.is_some() {
                    ui.label(
                        RichText::new(
                            "Se descarga el instalador, se verifica su firma y RusIO se cierra para instalarlo y \
                             volver a abrirse. Las sesiones abiertas se cerrarán; tus conexiones y contraseñas no cambian.",
                        )
                        .size(12.0)
                        .color(theme.text_secondary),
                    );
                } else {
                    ui.label(
                        RichText::new("Esta versión no trae un instalador firmado: descárgala desde GitHub.")
                            .size(12.0)
                            .color(theme.text_secondary),
                    );
                }
                if let Some(job) = &self.update_job {
                    ui.add_space(8.0);
                    ui.add(egui::ProgressBar::new(job.progress.fraction()).show_percentage());
                }
                if let Some(e) = &self.update_error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(e).size(12.0).color(theme.status_offline));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if info.installer.is_some()
                        && ui
                            .add_enabled(self.update_job.is_none(), egui::Button::new("Actualizar ahora"))
                            .clicked()
                    {
                        start = true;
                    }
                    if ui.button("Ver en GitHub").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&info.url));
                    }
                });
            });
        if start {
            if let Some(installer) = info.installer {
                self.update_error = None;
                let progress = crate::updates::Progress::default();
                let (tx, rx) = crossbeam_channel::bounded(1);
                let p = progress.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(crate::updates::download_installer(&installer, &p));
                });
                self.update_job = Some(UpdateJob { rx, progress });
            }
        }
        if !open {
            self.show_update = false;
        }
    }

    fn poll_update(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.update_rx else { return };
        match rx.try_recv() {
            Ok(result) => {
                self.update_rx = None;
                match result {
                    Ok(Some(info)) => {
                        self.update_note = Some(format!("Hay una versión nueva: {}.", info.version));
                        self.update = Some(info);
                    }
                    Ok(None) => self.update_note = Some("Tienes la última versión.".to_string()),
                    Err(e) => self.update_note = Some(format!("Error al buscar actualizaciones: {e}")),
                }
            }
            Err(crossbeam_channel::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(500))
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => self.update_rx = None,
        }
    }

    pub fn set_status(&mut self, msg: String) {
        self.status_msg = Some((msg, Instant::now()));
    }

    /// Opens a session of `plugin_id` with the given params (and optional plugin action).
    fn connect_params(&mut self, plugin_id: &str, params: &Params, action: Option<&str>, ctx: &egui::Context) {
        self.connect_params_with(plugin_id, params, action, ctx, Vec::new());
    }

    /// Same, keeping `guards` (open tunnels) alive for as long as the new tab exists.
    fn connect_params_with(
        &mut self,
        plugin_id: &str,
        params: &Params,
        action: Option<&str>,
        ctx: &egui::Context,
        guards: Vec<Box<dyn Any + Send>>,
    ) {
        let settings = self.settings.plugin_values(plugin_id);
        let req = ConnectRequest {
            params,
            action,
            settings: &settings,
            terminal_font_size: self.settings.terminal_font_size,
            ctx,
        };
        let outcome = match self.registry.get(plugin_id) {
            Some(entry) => entry.plugin.connect(&req),
            None => Err(format!("no hay ningún plugin para el protocolo \"{plugin_id}\"")),
        };

        match outcome {
            Ok(connected) => {
                let msg = format!("Conectando: {}", connected.title);
                self.tabs
                    .push(SessionTab::new(connected.title, connected.session).with_guards(guards));
                self.active_tab_idx = Some(self.tabs.len() - 1);
                self.set_status(msg);
            }
            Err(e) => self.set_status(format!("Error: {e}")),
        }
    }

    fn connect_profile(&mut self, profile: &ConnectionProfile, action: Option<&str>, ctx: &egui::Context) {
        if profile.via.is_none() {
            self.connect_params(&profile.protocol.kind, &profile.protocol.params, action, ctx);
            return;
        }

        // Through gateways: the tunnels open on a worker thread, then `poll_pending` connects.
        let hops = {
            let settings_for = |id: &str| self.settings.plugin_values(id);
            tunnels::build_chain(profile, &self.config.connections, &self.registry, &settings_for)
        };
        let hops = match hops {
            Ok(h) => h,
            Err(e) => {
                self.set_status(format!("Error: {e}"));
                return;
            }
        };

        let names: Vec<&str> = hops.iter().map(|h| h.name.as_str()).collect();
        self.set_status(format!("Abriendo túnel vía {}...", names.join(" → ")));

        let registry = self.registry.clone();
        let kind = profile.protocol.kind.clone();
        let params = profile.protocol.params.clone();
        let repaint = ctx.clone();
        let (tx, rx) = unbounded();
        let thread_kind = kind.clone();
        thread::spawn(move || {
            let resolved = tunnels::resolve(&registry, &hops, &thread_kind, &params);
            let _ = tx.send(resolved);
            repaint.request_repaint();
        });
        self.pending.push(PendingConnect {
            rx,
            plugin_id: kind,
            action: action.map(String::from),
        });
    }

    /// Finishes the connections whose tunnels have come up (or failed).
    fn poll_pending(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        for (i, p) in self.pending.iter().enumerate() {
            match p.rx.try_recv() {
                Ok(r) => finished.push((i, Some(r))),
                Err(TryRecvError::Disconnected) => finished.push((i, None)),
                Err(TryRecvError::Empty) => {}
            }
        }
        for (i, result) in finished.into_iter().rev() {
            let p = self.pending.remove(i);
            match result {
                Some(Ok(r)) => self.connect_params_with(&p.plugin_id, &r.params, p.action.as_deref(), ctx, r.guards),
                Some(Err(e)) => self.set_status(format!("Error: {e}")),
                None => self.set_status("Error: se interrumpió la apertura del túnel.".to_string()),
            }
        }
    }

    fn apply_connections_path(&mut self, new_path: std::path::PathBuf) {
        match self.config.switch_to(new_path.clone()) {
            Ok(loaded) => {
                self.settings.connections_path = if new_path == ConfigStore::default_path() {
                    None
                } else {
                    Some(new_path.clone())
                };
                self.settings.save();
                self.set_status(if loaded {
                    format!("Conexiones cargadas desde {}", new_path.display())
                } else {
                    format!("Conexiones guardadas en {}", new_path.display())
                });
            }
            Err(e) => {
                self.set_status(format!("Error: no se pudo cambiar la ubicación: {}", e));
            }
        }
    }

    fn render_settings(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let theme = Theme::default();
        let mut open = true;
        let mut chosen_dir: Option<std::path::PathBuf> = None;
        let mut reset = false;
        let mut changed = false;

        egui::Window::new("Configuración")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .default_width(480.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(520.0).show(ui, |ui| {
                    ui.label(RichText::new("CONEXIONES").size(10.0).strong().color(theme.text_muted));
                    ui.add_space(2.0);
                    ui.label("Archivo de conexiones");
                    ui.add(
                        egui::Label::new(
                            RichText::new(self.config.path.display().to_string())
                                .monospace()
                                .size(11.0)
                                .color(theme.text_secondary),
                        )
                        .wrap(),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cambiar carpeta...").clicked() {
                            chosen_dir = rfd::FileDialog::new()
                                .set_title("Carpeta para connections.json")
                                .pick_folder();
                        }
                        if ui
                            .button("Restablecer")
                            .on_hover_text("Volver a la ubicación predeterminada")
                            .clicked()
                        {
                            reset = true;
                        }
                        if ui.button("Mostrar en el Explorador").clicked() {
                            let _ = std::process::Command::new("explorer")
                                .arg(format!("/select,{}", self.config.path.display()))
                                .spawn();
                        }
                    });
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(
                            "Si la carpeta elegida ya tiene un connections.json se carga; si no, \
                             se copian allí tus conexiones actuales. El archivo anterior no se borra.",
                        )
                        .size(11.0)
                        .color(theme.text_muted),
                    );
                    ui.add_space(2.0);
                    if self.config.secrets_unavailable {
                        ui.label(
                            RichText::new("Contraseñas: el almacén de credenciales no está disponible; se guardan en el archivo de conexiones.")
                                .size(11.0)
                                .color(theme.status_offline),
                        );
                    } else {
                        ui.label(
                            RichText::new("Contraseñas: se guardan en el almacén de credenciales del sistema, no en el archivo. En otro equipo habrá que reingresarlas.")
                                .size(11.0)
                                .color(theme.text_muted),
                        );
                    }

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(RichText::new("FUENTES").size(10.0).strong().color(theme.text_muted));
                    ui.add_space(2.0);
                    egui::Grid::new("settings_fonts").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                        ui.label("Terminal");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.terminal_font_size)
                                    .range(9.0..=28.0)
                                    .speed(0.2)
                                    .suffix(" px"),
                            )
                            .changed();
                        ui.end_row();
                        ui.label("Editor de texto");
                        changed |= ui
                            .add(
                                egui::DragValue::new(&mut self.settings.editor_font_size)
                                    .range(9.0..=28.0)
                                    .speed(0.2)
                                    .suffix(" px"),
                            )
                            .changed();
                        ui.end_row();
                    });
                    ui.label(
                        RichText::new("Se aplican a las sesiones nuevas.")
                            .size(11.0)
                            .color(theme.text_muted),
                    );

                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);
                    ui.label(RichText::new("ACTUALIZACIONES").size(10.0).strong().color(theme.text_muted));
                    ui.add_space(2.0);
                    changed |= ui
                        .checkbox(&mut self.settings.check_updates, "Buscar actualizaciones al iniciar")
                        .changed();
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.update_rx.is_none(), egui::Button::new("Buscar ahora"))
                            .clicked()
                        {
                            self.update_note = None;
                            self.start_update_check();
                        }
                        if self.update_rx.is_some() {
                            ui.spinner();
                        } else if let Some(note) = &self.update_note {
                            ui.label(RichText::new(note).size(11.0).color(theme.text_secondary));
                        }
                    });
                    ui.label(
                        RichText::new("Solo consulta la última versión publicada en GitHub; no envía datos tuyos ni instala nada.")
                            .size(11.0)
                            .color(theme.text_muted),
                    );

                    // One section per plugin that declares settings.
                    for entry in self.registry.entries() {
                        let schema = entry.plugin.settings_schema();
                        if schema.is_empty() {
                            continue;
                        }
                        let plugin_id = entry.def.id;
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(entry.def.name.to_uppercase())
                                .size(10.0)
                                .strong()
                                .color(theme.text_muted),
                        );
                        ui.add_space(2.0);

                        let values = self.settings.plugin_values(plugin_id);
                        let mut updates: Vec<(&'static str, Value)> = Vec::new();
                        egui::Grid::new(("plugin_settings", plugin_id))
                            .num_columns(2)
                            .spacing([16.0, 6.0])
                            .show(ui, |ui| {
                                for field in &schema {
                                    ui.label(field.label).on_hover_text(field.help);
                                    let current = values
                                        .get(field.key)
                                        .cloned()
                                        .unwrap_or_else(|| field.kind.default_value());
                                    if let Some(v) = setting_widget(ui, plugin_id, field, &current) {
                                        updates.push((field.key, v));
                                    }
                                    ui.end_row();
                                }
                            });
                        for (key, v) in updates {
                            self.settings.set_plugin_value(plugin_id, key, v);
                            changed = true;
                        }
                    }
                });
            });

        if !open {
            self.show_settings = false;
        }
        if let Some(dir) = chosen_dir {
            self.apply_connections_path(dir.join("connections.json"));
        }
        if reset {
            self.apply_connections_path(ConfigStore::default_path());
        }
        if changed {
            self.settings.save();
        }
    }

    pub fn close_tab(&mut self, idx: usize) {
        if self.tabs.get(idx).is_some_and(|t| t.session.is_dirty()) {
            self.pending_discard = Some(idx);
            return;
        }
        self.remove_tab(idx);
    }

    fn remove_tab(&mut self, idx: usize) {
        if idx < self.tabs.len() {
            let mut tab = self.tabs.remove(idx);
            tab.session.close();
            if self.tabs.is_empty() {
                self.active_tab_idx = None;
            } else if let Some(active) = self.active_tab_idx {
                if active >= self.tabs.len() {
                    self.active_tab_idx = Some(self.tabs.len() - 1);
                }
            }
        }
    }

    fn welcome_items(&self) -> Vec<WelcomeItem> {
        let mut items = Vec::new();
        for (plugin_id, q) in self.quick.iter().filter(|(_, q)| q.featured) {
            let Some(e) = self.registry.get(plugin_id) else {
                continue;
            };
            items.push(WelcomeItem {
                category: e.def.category,
                plugin_id,
                icon: e.def.icon,
                color: q.color,
                title: q.label.clone(),
                subtitle: q.subtitle.clone(),
                button: "Abrir",
                action: WelcomeAction::Quick(plugin_id, q.params.clone()),
            });
        }
        for e in self.registry.entries() {
            if let Some(w) = &e.def.welcome {
                items.push(WelcomeItem {
                    category: e.def.category,
                    plugin_id: e.def.id,
                    icon: e.def.icon,
                    color: e.def.color,
                    title: w.title.to_string(),
                    subtitle: w.subtitle.to_string(),
                    button: "Conectar",
                    action: WelcomeAction::NewConnection(e.def.id),
                });
            }
        }
        items
    }
}

/// Draws one plugin setting; returns the new value if the user changed it.
fn setting_widget(ui: &mut egui::Ui, plugin_id: &str, field: &SettingField, current: &Value) -> Option<Value> {
    match &field.kind {
        SettingKind::Bool { default } => {
            let mut v = current.as_bool().unwrap_or(*default);
            ui.checkbox(&mut v, "").changed().then(|| json!(v))
        }
        SettingKind::Int {
            default,
            min,
            max,
            suffix,
        } => {
            let mut v = current.as_i64().unwrap_or(*default);
            ui.add(egui::DragValue::new(&mut v).range(*min..=*max).suffix(*suffix))
                .changed()
                .then(|| json!(v))
        }
        SettingKind::Text { default } => {
            let mut v = current.as_str().unwrap_or(default).to_string();
            ui.text_edit_singleline(&mut v).changed().then(|| json!(v))
        }
        SettingKind::Choice { default, options } => {
            let cur = current.as_str().unwrap_or(default);
            let mut sel = cur.to_string();
            let cur_label = options.iter().find(|(v, _)| *v == cur).map_or(cur, |(_, l)| *l);
            egui::ComboBox::from_id_salt((plugin_id, field.key))
                .selected_text(cur_label)
                .show_ui(ui, |ui| {
                    for (value, label) in options.iter() {
                        ui.selectable_value(&mut sel, value.to_string(), *label);
                    }
                });
            (sel != cur).then(|| json!(sel))
        }
    }
}

impl eframe::App for RusioApp {
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(path) = self.install_on_exit.take() {
            let _ = crate::updates::launch_installer(&path);
        }
    }

    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &root.ctx().clone();
        let theme = Theme::default();
        titlebar::resize_edges(ctx);
        self.poll_pending(ctx);
        self.poll_update(ctx);

        // Custom title bar: logo, quick connect, window controls
        egui::Panel::top("top_panel")
            .exact_size(titlebar::TITLEBAR_HEIGHT)
            .frame(
                egui::Frame::NONE
                    .fill(theme.bg_dark)
                    .stroke(Stroke::new(1.0_f32, theme.border)),
            )
            .show(root, |ui| {
                let bar_rect = ui.max_rect();
                titlebar::drag_area(ui, bar_rect);

                // (protocol id, label, hint, parser) of every protocol with quick connect
                let quick_protos: Vec<_> = self
                    .registry
                    .quick_connect_entries()
                    .into_iter()
                    .filter_map(|e| {
                        e.def
                            .quick_connect
                            .as_ref()
                            .map(|q| (e.def.id, q.label, q.hint, q.parse))
                    })
                    .collect();
                if self.quick_connect_proto >= quick_protos.len() {
                    self.quick_connect_proto = 0;
                }
                let mut connect_quick = false;

                ui.horizontal_centered(|ui| {
                    const CTRL_H: f32 = 26.0;
                    ui.spacing_mut().interact_size.y = CTRL_H;
                    ui.spacing_mut().button_padding = Vec2::new(10.0, 4.0);
                    ui.add_space(12.0);
                    titlebar::paint_logo(ui, 20.0);
                    ui.label(RichText::new("RusIO").strong().size(14.0).color(theme.text_primary));
                    ui.add_space(14.0);

                    if !quick_protos.is_empty() {
                        egui::ComboBox::from_id_salt("quick_proto")
                            .width(60.0)
                            .selected_text(quick_protos[self.quick_connect_proto].1)
                            .show_ui(ui, |ui| {
                                for (i, (_, label, _, _)) in quick_protos.iter().enumerate() {
                                    ui.selectable_value(&mut self.quick_connect_proto, i, *label);
                                }
                            });

                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.quick_connect_host)
                                .hint_text(quick_protos[self.quick_connect_proto].2)
                                .vertical_align(egui::Align::Center)
                                .min_size(Vec2::new(240.0, CTRL_H))
                                .desired_width(240.0),
                        );
                        let enter_pressed = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                        if ui
                            .add_sized(
                                [84.0, CTRL_H],
                                egui::Button::new(RichText::new("Conectar").color(Color32::WHITE)),
                            )
                            .clicked()
                            || enter_pressed
                        {
                            connect_quick = true;
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        titlebar::window_controls(ui);
                        ui.add_space(8.0);
                        if icons::icon_button_sized(ui, icons::Icon::Gear, None, "Configuración", 28.0) {
                            self.show_settings = !self.show_settings;
                        }
                    });
                });

                if connect_quick && !self.quick_connect_host.trim().is_empty() {
                    let (plugin_id, _, _, parse) = quick_protos[self.quick_connect_proto];
                    match parse(&self.quick_connect_host) {
                        Some(params) => self.connect_params(plugin_id, &params, None, ctx),
                        None => self.set_status("Error: formato de conexión no válido.".to_string()),
                    }
                }
            });

        // Status bar (declared before the side panel so it spans the full window width)
        let live_sessions = self.tabs.iter().filter(|t| t.session.is_alive()).count();
        let mut open_update = false;
        egui::Panel::bottom("status_bar")
            .exact_size(26.0)
            .frame(
                egui::Frame::NONE
                    .fill(theme.bg_dark)
                    .stroke(Stroke::new(1.0_f32, theme.border))
                    .inner_margin(egui::Margin::symmetric(12, 0)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some((ref msg, _)) = self.status_msg {
                        let color = if msg.starts_with("Error") {
                            theme.status_offline
                        } else {
                            theme.text_secondary
                        };
                        ui.add(egui::Label::new(RichText::new(msg).size(11.0).color(color)).truncate());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                .size(11.0)
                                .color(theme.text_muted),
                        );
                        ui.separator();
                        if let Some(up) = &self.update {
                            let link = ui
                                .add(
                                    egui::Label::new(
                                        RichText::new(format!("Nueva versión {} disponible", up.version))
                                            .size(11.0)
                                            .color(theme.accent_primary),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text("Ver y actualizar")
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if link.clicked() {
                                open_update = true;
                            }
                            ui.separator();
                        }
                        ui.label(
                            RichText::new(format!(
                                "{} {}",
                                live_sessions,
                                if live_sessions == 1 {
                                    "sesión activa"
                                } else {
                                    "sesiones activas"
                                }
                            ))
                            .size(11.0)
                            .color(theme.text_muted),
                        );
                    });
                });
            });

        if open_update {
            self.show_update = true;
            self.update_error = None;
        }

        // Left Panel: Connection Manager with Folders
        egui::Panel::left("sidebar_panel")
            .resizable(true)
            .default_size(280.0)
            .size_range(220.0..=450.0)
            .show(root, |ui| {
                let action = render_sidebar(ui, &mut self.config, &mut self.sidebar_state, &self.registry);
                if let Some(action) = action {
                    match action {
                        SidebarAction::ConnectProfile(profile, plugin_action) => {
                            self.connect_profile(&profile, plugin_action, ctx);
                        }
                        SidebarAction::CreateNewConnection(folder_id) => {
                            self.modal_type_picker = Some(folder_id);
                        }
                        SidebarAction::CreateNewFolder => {
                            self.modal_new_folder = Some("Nueva Carpeta".to_string());
                        }
                        SidebarAction::EditConnection(profile) => {
                            self.modal_editor = Some(ConnEditorState::from_profile(&profile));
                        }
                        SidebarAction::DeleteConnection(id) => {
                            let name = self
                                .config
                                .connections
                                .iter()
                                .find(|c| c.id == id)
                                .map(|c| c.name.clone())
                                .unwrap_or_default();
                            self.pending_delete = Some(PendingDelete::Connection(id, name));
                        }
                        SidebarAction::DeleteFolder(id) => {
                            let name = self
                                .config
                                .folders
                                .iter()
                                .find(|f| f.id == id)
                                .map(|f| f.name.clone())
                                .unwrap_or_default();
                            self.pending_delete = Some(PendingDelete::Folder(id, name));
                        }
                    }
                }
            });

        // Central Panel: Tabs + Session Viewport
        let mut host = AppHost::default();
        let welcome = if self.active_tab_idx.is_none() {
            self.welcome_items()
        } else {
            Vec::new()
        };
        egui::CentralPanel::default().show(root, |ui| {
            if let Some(tab_action) = render_tabs(ui, &self.tabs, self.active_tab_idx, &self.quick) {
                match tab_action {
                    TabAction::SelectTab(idx) => {
                        self.active_tab_idx = Some(idx);
                    }
                    TabAction::CloseTab(idx) => {
                        self.close_tab(idx);
                    }
                    TabAction::Quick { plugin_id, params } => {
                        self.connect_params(plugin_id, &params, None, ctx);
                    }
                    TabAction::OpenQuickConnect => {
                        self.modal_type_picker = Some(None);
                    }
                }
            }

            ui.separator();

            if let Some(active_idx) = self.active_tab_idx.filter(|i| *i < self.tabs.len()) {
                self.tabs[active_idx].session.ui(ui, &mut host);
            } else {
                // Empty state: welcome dashboard
                let mut clicked: Option<WelcomeAction> = None;
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        titlebar::paint_logo(ui, 56.0);
                        ui.add_space(8.0);
                        ui.label(RichText::new("RusIO").size(28.0).strong().color(theme.text_primary));
                        ui.label(
                            RichText::new("Cliente SSH y Gestor Remoto Nativo en Rust")
                                .size(15.0)
                                .color(theme.text_secondary),
                        );

                        ui.add_space(30.0);

                        let card_width = 180.0;
                        let card_height = 110.0;
                        let frame = egui::Frame::NONE
                            .fill(theme.bg_panel_alt)
                            .stroke(Stroke::new(1.0_f32, theme.border))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(egui::Margin::same(14));

                        for cat in Category::ALL {
                            let in_cat: Vec<&WelcomeItem> = welcome.iter().filter(|i| i.category == cat).collect();
                            if in_cat.is_empty() {
                                continue;
                            }
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(cat.label().to_uppercase())
                                    .size(11.0)
                                    .strong()
                                    .color(theme.text_muted),
                            );
                            ui.add_space(4.0);
                            // Rows of equal cards, centered as a block.
                            let gap = 14.0;
                            let cell = card_width + 28.0 + gap; // card + inner margins + gap
                            let per_row = (((ui.available_width() + gap) / cell) as usize).max(1);
                            for row in in_cat.chunks(per_row) {
                                ui.horizontal_top(|ui| {
                                    ui.spacing_mut().item_spacing = Vec2::new(gap, gap);
                                    let used = row.len() as f32 * cell - gap;
                                    ui.add_space(((ui.available_width() - used) / 2.0).max(0.0));
                                    for item in row {
                                        frame.show(ui, |ui| {
                                            ui.set_width(card_width);
                                            ui.set_height(card_height);
                                            ui.vertical_centered(|ui| {
                                                icons::protocol_icon(ui, item.plugin_id, item.icon, item.color, 26.0);
                                                ui.label(RichText::new(&item.title).strong().size(14.0));
                                                ui.label(
                                                    RichText::new(&item.subtitle).size(11.0).color(theme.text_muted),
                                                );
                                                ui.add_space(4.0);
                                                if ui.button(RichText::new(item.button).color(Color32::WHITE)).clicked()
                                                {
                                                    clicked = Some(item.action.clone());
                                                }
                                            });
                                        });
                                    }
                                });
                                ui.add_space(gap - 4.0);
                            }
                        }

                        ui.add_space(30.0);
                        ui.label(
                            RichText::new(
                                "Selecciona una conexión en el gestor de carpetas de la izquierda o crea una nueva.",
                            )
                            .size(12.0)
                            .color(theme.text_muted),
                        );
                    });
                });

                match clicked {
                    Some(WelcomeAction::Quick(plugin_id, params)) => {
                        self.connect_params(plugin_id, &params, None, ctx);
                    }
                    Some(WelcomeAction::NewConnection(plugin_id)) => {
                        self.modal_editor = Some(ConnEditorState::new(&self.registry, Some(plugin_id)));
                    }
                    None => {}
                }
            }
        });

        // Requests from the sessions that just drew
        if let Some(msg) = host.status.take() {
            self.set_status(msg);
        }
        for file in host.editors {
            let editor = EditorSession::open(file, self.settings.editor_font_size);
            let title = editor.tab_title();
            self.tabs.push(SessionTab::new(title, Box::new(editor)));
            self.active_tab_idx = Some(self.tabs.len() - 1);
        }

        self.render_settings(ctx);
        self.render_update(ctx);

        // Modals
        if let Some(folder_id) = self.modal_type_picker.take() {
            match render_type_picker(ctx, &self.registry) {
                Some(TypePick::Chosen(id)) => {
                    let mut state = ConnEditorState::new(&self.registry, Some(id));
                    state.folder_id = folder_id;
                    self.modal_editor = Some(state);
                }
                Some(TypePick::Cancel) => {}
                None => self.modal_type_picker = Some(folder_id),
            }
        }

        if let Some(mut state) = self.modal_editor.take() {
            if let Some(res) = render_conn_modal(
                ctx,
                &mut state,
                &self.config.folders,
                &self.config.connections,
                &self.registry,
            ) {
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
                        self.connect_profile(&p, None, ctx);
                    }
                    ModalResult::Cancel => {}
                }
            } else {
                self.modal_editor = Some(state);
            }
        }

        if let Some(idx) = self.pending_discard.take() {
            let name = self.tabs.get(idx).map(|t| t.display_title()).unwrap_or_default();
            let mut discard = false;
            let mut cancelled = false;
            egui::Window::new("Cambios sin guardar")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(format!("\"{}\" tiene cambios sin guardar.", name));
                    ui.label(
                        RichText::new("Si cierras la pestaña se perderán.")
                            .size(11.0)
                            .color(theme.text_muted),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("Cerrar sin guardar").color(Color32::WHITE))
                            .clicked()
                        {
                            discard = true;
                        }
                        if ui.button("Cancelar").clicked() {
                            cancelled = true;
                        }
                    });
                });
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancelled = true;
            }
            if discard {
                self.remove_tab(idx);
            } else if !cancelled {
                self.pending_discard = Some(idx);
            }
        }

        if let Some(pending) = self.pending_delete.take() {
            let (title, detail) = match &pending {
                PendingDelete::Connection(_, n) => ("Eliminar conexión", format!("¿Eliminar la conexión \"{}\"?", n)),
                PendingDelete::Folder(_, n) => ("Eliminar carpeta", format!("¿Eliminar la carpeta \"{}\"?", n)),
            };
            let mut confirmed = false;
            let mut cancelled = false;
            egui::Window::new(title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(detail);
                    ui.label(
                        RichText::new("Esta acción no se puede deshacer.")
                            .size(11.0)
                            .color(theme.text_muted),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Eliminar").color(Color32::WHITE)).clicked() {
                            confirmed = true;
                        }
                        if ui.button("Cancelar").clicked() {
                            cancelled = true;
                        }
                    });
                });
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                cancelled = true;
            }
            if confirmed {
                match pending {
                    PendingDelete::Connection(id, _) => {
                        self.config.remove_connection(id);
                        self.set_status("Conexión eliminada.".to_string());
                    }
                    PendingDelete::Folder(id, _) => {
                        self.config.remove_folder(id);
                        self.set_status("Carpeta eliminada.".to_string());
                    }
                }
            } else if !cancelled {
                self.pending_delete = Some(pending);
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
