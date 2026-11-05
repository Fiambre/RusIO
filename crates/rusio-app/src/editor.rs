use crossbeam_channel::{unbounded, Receiver};

use rusio_plugin_api::egui::{self, Color32};
use rusio_plugin_api::theme::Theme;
use rusio_plugin_api::{Host, Session, TextFile};
use std::sync::Arc;

pub struct EditorSession {
    pub path: String,
    pub text: String,
    pub modified: bool,
    pub font_size: f32,
    pub status: String,
    pub is_error: bool,
    crlf: bool,
    loaded: bool,
    file: Arc<dyn TextFile>,
    load_rx: Option<Receiver<Result<Vec<u8>, String>>>,
    save_rx: Option<Receiver<Result<(), String>>>,
}

impl EditorSession {
    pub fn open(file: Arc<dyn TextFile>, font_size: f32) -> Self {
        let path = file.path();
        let (tx, rx) = unbounded();
        file.read(tx);
        Self {
            path,
            text: String::new(),
            modified: false,
            font_size,
            status: "Abriendo archivo...".to_string(),
            is_error: false,
            crlf: false,
            loaded: false,
            file,
            load_rx: Some(rx),
            save_rx: None,
        }
    }

    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    pub fn tab_title(&self) -> String {
        if self.modified {
            format!("* {}", self.file_name())
        } else {
            self.file_name().to_string()
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn is_saving(&self) -> bool {
        self.save_rx.is_some()
    }

    pub fn is_connected(&self) -> bool {
        self.file.is_alive()
    }

    pub fn reload(&mut self) {
        let (tx, rx) = unbounded();
        self.file.read(tx);
        self.load_rx = Some(rx);
        self.loaded = false;
        self.status = "Recargando...".to_string();
        self.is_error = false;
    }

    pub fn save(&mut self) {
        if !self.loaded || self.save_rx.is_some() {
            return;
        }
        if !self.file.is_alive() {
            self.status = "La conexión SFTP está cerrada; no se puede guardar.".to_string();
            self.is_error = true;
            return;
        }
        let data = if self.crlf {
            self.text.replace('\n', "\r\n").into_bytes()
        } else {
            self.text.clone().into_bytes()
        };
        let (tx, rx) = unbounded();
        self.file.write(data, tx);
        self.save_rx = Some(rx);
        self.status = "Guardando...".to_string();
        self.is_error = false;
    }

    /// Picks up results from the SFTP worker. Call once per frame.
    pub fn poll(&mut self) {
        if let Some(rx) = &self.load_rx {
            if let Ok(res) = rx.try_recv() {
                self.load_rx = None;
                match res {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(text) => {
                            self.crlf = text.contains("\r\n");
                            self.text = if self.crlf { text.replace("\r\n", "\n") } else { text };
                            self.loaded = true;
                            self.modified = false;
                            self.status = String::new();
                        }
                        Err(_) => {
                            self.status = "El archivo no es texto UTF-8; no se puede editar.".to_string();
                            self.is_error = true;
                        }
                    },
                    Err(e) => {
                        self.status = format!("Error: {}", e);
                        self.is_error = true;
                    }
                }
            }
        }

        if let Some(rx) = &self.save_rx {
            if let Ok(res) = rx.try_recv() {
                self.save_rx = None;
                match res {
                    Ok(()) => {
                        self.modified = false;
                        self.status = "Guardado".to_string();
                        self.is_error = false;
                    }
                    Err(e) => {
                        self.status = format!("Error al guardar: {}", e);
                        self.is_error = true;
                    }
                }
            }
        }
    }
}

impl Session for EditorSession {
    fn badge(&self) -> (&'static str, Color32) {
        ("EDIT", Theme::default().accent_vnc)
    }

    fn is_alive(&self) -> bool {
        self.is_connected()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _host: &mut dyn Host) {
        crate::ui::editor_view::render_editor(ui, self);
    }

    fn title(&self) -> Option<String> {
        Some(self.tab_title())
    }

    fn is_dirty(&self) -> bool {
        self.modified
    }
}
