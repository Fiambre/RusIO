use egui::{Color32, RichText};

use super::emulator::SharedTerminal;
use crate::icons;
use crate::theme::Theme;
use crate::{Host, Session};

/// The byte-stream side of a terminal session (a PTY, an SSH channel...).
pub trait TerminalBackend: Send {
    fn write(&self, data: &[u8]);
    fn resize(&self, cols: u16, rows: u16);
    fn is_alive(&self) -> bool;
    fn close(&self);
}

/// A tab showing a terminal: status toolbar plus the emulator grid.
pub struct TerminalSession {
    backend: Box<dyn TerminalBackend>,
    emulator: SharedTerminal,
    font_size: f32,
    badge: (&'static str, Color32),
}

impl TerminalSession {
    pub fn new(
        backend: Box<dyn TerminalBackend>,
        emulator: SharedTerminal,
        font_size: f32,
        badge: (&'static str, Color32),
    ) -> Self {
        Self {
            backend,
            emulator,
            font_size,
            badge,
        }
    }
}

impl Session for TerminalSession {
    fn badge(&self) -> (&'static str, Color32) {
        self.badge
    }

    fn is_alive(&self) -> bool {
        self.backend.is_alive()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _host: &mut dyn Host) {
        let theme = Theme::default();

        ui.horizontal(|ui| {
            let alive = self.backend.is_alive();
            let (text, color) = if alive {
                ("CONECTADO", theme.status_online)
            } else {
                ("DESCONECTADO", theme.status_offline)
            };
            icons::status_dot(ui, color);
            ui.label(RichText::new(text).color(color).size(10.0));

            let (cols, rows) = {
                let emu = self.emulator.lock();
                (emu.cols(), emu.rows())
            };
            ui.label(
                RichText::new(format!("{}x{}", cols, rows))
                    .size(10.0)
                    .color(theme.text_muted),
            );

            ui.separator();

            ui.label(RichText::new("Fuente:").size(11.0));
            if ui.small_button("-").clicked() && self.font_size > 9.0 {
                self.font_size -= 1.0;
            }
            ui.label(format!("{:.0}", self.font_size));
            if ui.small_button("+").clicked() && self.font_size < 28.0 {
                self.font_size += 1.0;
            }
        });

        ui.separator();

        let (input_bytes, resized_to) = {
            let mut emu = self.emulator.lock();
            emu.render(ui, self.font_size, true)
        };
        if let Some((cols, rows)) = resized_to {
            self.backend.resize(cols, rows);
        }
        if let Some(bytes) = input_bytes {
            self.backend.write(&bytes);
        }
    }

    fn close(&mut self) {
        self.backend.close();
    }
}
