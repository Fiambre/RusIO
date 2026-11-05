//! A SQL console shared by the database plugins: schema browser, editor and results grid.
//! A plugin only supplies a [`SqlBackend`] (its worker thread and dialect).
use egui::{self, Color32, Modifiers, RichText};
use egui_code_editor::{CodeEditor, ColorTheme, Completer, Syntax};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::data::{data_grid, DataTable, GridState};
use crate::icons::{self, Icon};
use crate::sql_text::{destructive_sql, format_sql};
use crate::theme::Theme;
use crate::{Host, Session};

const CTRL_H: f32 = 26.0;

/// Editor colors, matching the colors of the rest of the app.
const EDITOR_THEME: ColorTheme = ColorTheme {
    name: "RusIO",
    dark: true,
    bg: "0d1017",
    cursor: "e5e7eb",
    selection: "2b3a67",
    comments: "6b7280",
    functions: "6ebeff",
    keywords: "c586ff",
    literals: "e5e7eb",
    numerics: "f4a460",
    punctuation: "9ca3af",
    strs: "98d379",
    types: "78dcd2",
    special: "6ebeff",
};

#[derive(Debug, Clone)]
pub struct SchemaTable {
    pub name: String,
    pub is_view: bool,
    /// (column name, declared type)
    pub columns: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct QueryResult {
    /// `None` when the last statement returned no rows (INSERT, CREATE...).
    pub table: Option<DataTable>,
    pub truncated: bool,
    pub elapsed_ms: u128,
    pub message: String,
}

/// What the console shows; the backend's worker thread keeps it up to date.
#[derive(Default)]
pub struct ConsoleState {
    pub tables: Vec<SchemaTable>,
    pub result: Option<QueryResult>,
    pub error: Option<String>,
    pub busy: bool,
}

pub trait SqlBackend: Send {
    fn state(&self) -> Arc<Mutex<ConsoleState>>;
    fn is_alive(&self) -> bool;
    /// Runs the statements asynchronously; the outcome lands in [`ConsoleState`].
    fn run(&self, sql: String);
    fn refresh_schema(&self);
    fn cancel(&self);
    fn close(&self);
    /// Query that previews the first rows of `table` (a [`SchemaTable::name`]).
    fn select_all_sql(&self, table: &str) -> String;

    /// False for query languages that are not SQL (no SQL colors, no SQL formatter).
    fn is_sql(&self) -> bool {
        true
    }
    /// What could be destroyed by running `text` (empty = safe); the console asks to confirm.
    fn destructive(&self, text: &str) -> Vec<String> {
        destructive_sql(text)
    }
    /// Words offered by the autocompleter and colored as keywords in a language that is not
    /// SQL (SQL has its own list built in).
    fn completion_words(&self) -> &'static [&'static str] {
        &[]
    }
    /// Placeholder shown in the empty editor.
    fn editor_hint(&self) -> &'static str {
        "SELECT * FROM tabla;   (Ctrl+Enter para ejecutar)"
    }
    /// Databases the session can switch to; empty hides the selector.
    fn databases(&self) -> Vec<String> {
        Vec::new()
    }
    fn current_database(&self) -> String {
        String::new()
    }
    /// Reconnects to another database of the same server (asynchronously).
    fn switch_database(&self, _name: &str) {}
}

pub struct SqlConsole {
    backend: Box<dyn SqlBackend>,
    badge: (&'static str, Color32),
    /// Keeps egui panel/widget ids apart when several databases are open.
    uid: u64,
    sql: String,
    syntax: Syntax,
    completer: Completer,
    /// Fingerprint of the schema the completer was last fed with.
    schema_sig: u64,
    grid: GridState,
    /// A destructive statement waiting for the user's confirmation, with the reasons.
    confirm: Option<(String, Vec<String>)>,
}

impl SqlConsole {
    pub fn new(backend: Box<dyn SqlBackend>, badge: (&'static str, Color32)) -> Self {
        static NEXT_UID: AtomicU64 = AtomicU64::new(1);
        let syntax = if backend.is_sql() {
            Syntax::sql()
        } else {
            Syntax {
                language: "query",
                comment: "//",
                keywords: backend.completion_words().iter().copied().collect(),
                ..Syntax::default()
            }
        };
        let completer = Completer::new_with_syntax(&syntax).with_auto_indent();
        Self {
            syntax,
            completer,
            schema_sig: 0,
            backend,
            badge,
            uid: NEXT_UID.fetch_add(1, Ordering::Relaxed),
            sql: String::new(),
            grid: GridState::default(),
            confirm: None,
        }
    }
}

impl Session for SqlConsole {
    fn badge(&self) -> (&'static str, Color32) {
        self.badge
    }

    fn is_alive(&self) -> bool {
        self.backend.is_alive()
    }

    fn ui(&mut self, ui: &mut egui::Ui, host: &mut dyn Host) {
        let theme = Theme::default();
        let uid = self.uid;

        // Actions are applied after drawing so the state lock is never held while sending.
        let mut run: Option<String> = None;
        let mut refresh = false;
        let mut cancel = false;
        let mut export: Option<String> = None;
        let mut switch_to: Option<String> = None;
        let mut format = false;
        let is_sql = self.backend.is_sql();
        let hint = self.backend.editor_hint();
        let databases = self.backend.databases();
        let current_db = self.backend.current_database();

        let state = self.backend.state();
        let st = state.lock();

        // Offer the tables and columns of the schema in the autocompleter.
        let sig = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for t in &st.tables {
                t.name.hash(&mut h);
                t.columns.iter().for_each(|(c, _)| c.hash(&mut h));
            }
            h.finish()
        };
        if sig != self.schema_sig {
            self.schema_sig = sig;
            let mut completer = Completer::new_with_syntax(&self.syntax).with_auto_indent();
            for t in &st.tables {
                completer.push_word(&t.name);
                for (c, _) in &t.columns {
                    completer.push_word(c);
                }
            }
            self.completer = completer;
        }

        // ---- schema browser
        egui::Panel::left(egui::Id::new(("sql_schema", uid)))
            .resizable(true)
            .default_size(210.0)
            .size_range(150.0..=400.0)
            .show(ui, |ui| {
                if !databases.is_empty() {
                    ui.label(
                        RichText::new("BASE DE DATOS")
                            .size(10.0)
                            .strong()
                            .color(theme.text_muted),
                    );
                    ui.add_enabled_ui(!st.busy, |ui| {
                        egui::ComboBox::from_id_salt(("sql_db_select", uid))
                            .width(ui.available_width() - 8.0)
                            .selected_text(&current_db)
                            .show_ui(ui, |ui| {
                                for db in &databases {
                                    if ui.selectable_label(*db == current_db, db).clicked() && *db != current_db {
                                        switch_to = Some(db.clone());
                                    }
                                }
                            });
                    });
                    ui.add_space(6.0);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("ESQUEMA").size(10.0).strong().color(theme.text_muted));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::icon_button(ui, Icon::Refresh, None, "Actualizar esquema") {
                            refresh = true;
                        }
                    });
                });
                ui.label(
                    RichText::new("Doble clic en una tabla para ver sus filas")
                        .size(10.0)
                        .color(theme.text_muted),
                );
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt(("sql_schema_scroll", uid))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if st.tables.is_empty() {
                            ui.label(RichText::new("(sin tablas)").size(11.0).color(theme.text_muted));
                        }
                        for t in &st.tables {
                            let title = if t.is_view {
                                format!("{}  (vista)", t.name)
                            } else {
                                t.name.clone()
                            };
                            let resp = egui::CollapsingHeader::new(RichText::new(title).size(12.5))
                                .id_salt(("sql_table", uid, &t.name))
                                .show(ui, |ui| {
                                    for (col, ty) in &t.columns {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(col).size(11.5));
                                            ui.label(
                                                RichText::new(if ty.is_empty() { "-" } else { ty })
                                                    .size(10.5)
                                                    .color(theme.text_muted),
                                            );
                                        });
                                    }
                                });
                            if resp.header_response.double_clicked() {
                                let sql = self.backend.select_all_sql(&t.name);
                                self.sql = sql.clone();
                                run = Some(sql);
                            }
                        }
                    });
            });

        // ---- SQL editor
        egui::Panel::top(egui::Id::new(("sql_editor", uid)))
            .resizable(true)
            .default_size(160.0)
            .size_range(90.0..=520.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.add_enabled_ui(!st.busy, |ui| {
                        if icons::icon_button_sized(
                            ui,
                            Icon::Play,
                            Some(theme.status_online),
                            "Ejecutar (Ctrl+Enter)",
                            CTRL_H,
                        ) {
                            run = Some(self.sql.clone());
                        }
                    });
                    ui.add_enabled_ui(st.busy, |ui| {
                        if icons::icon_button_sized(
                            ui,
                            Icon::Close,
                            Some(theme.status_offline),
                            "Cancelar la consulta",
                            CTRL_H,
                        ) {
                            cancel = true;
                        }
                    });
                    if is_sql
                        && icons::icon_button_sized(ui, Icon::Format, None, "Formatear SQL (Ctrl+Mayús+F)", CTRL_H)
                    {
                        format = true;
                    }
                    if st.busy {
                        ui.spinner();
                        ui.label(RichText::new("Ejecutando...").size(11.0).color(theme.text_secondary));
                    }
                });

                if is_sql && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, egui::Key::F)) {
                    format = true;
                }
                if !st.busy && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, egui::Key::Enter)) {
                    run = Some(self.sql.clone());
                }

                let rows = ((ui.available_height() / 17.0) as usize).max(3);
                CodeEditor::default()
                    .id_source(format!("sql_editor_{uid}"))
                    .with_theme(EDITOR_THEME)
                    .with_fontsize(13.0)
                    .with_rows(rows)
                    .with_numlines(true)
                    .with_clickable_links(false)
                    .hint_text(hint)
                    .show_with_completer(ui, &mut self.sql, &self.syntax, &mut self.completer);
            });

        // ---- results
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(err) = &st.error {
                ui.add(egui::Label::new(RichText::new(err).size(12.0).color(theme.status_offline)).wrap());
            }
            match &st.result {
                Some(res) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&res.message).size(11.0).color(if res.truncated {
                            theme.status_connecting
                        } else {
                            theme.text_secondary
                        }));
                        ui.label(
                            RichText::new(format!("· {} ms", res.elapsed_ms))
                                .size(11.0)
                                .color(theme.text_muted),
                        );
                        if let Some(table) = &res.table {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if icons::icon_button(ui, Icon::Save, None, "Exportar a CSV...") {
                                    export = Some(table.to_csv());
                                }
                            });
                        }
                    });
                    ui.separator();
                    if let Some(table) = &res.table {
                        data_grid(ui, table, &mut self.grid);
                    }
                }
                None if st.error.is_none() => {
                    ui.label(
                        RichText::new("Escribe una consulta y pulsa Ctrl+Enter.")
                            .size(12.0)
                            .color(theme.text_muted),
                    );
                }
                None => {}
            }
        });

        drop(st);

        if let Some(csv) = export {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Exportar resultado")
                .set_file_name("resultado.csv")
                .add_filter("CSV", &["csv"])
                .save_file()
            {
                match std::fs::write(&path, csv) {
                    Ok(()) => host.set_status(format!("Resultado exportado a {}", path.display())),
                    Err(e) => host.set_status(format!("Error: no se pudo exportar: {e}")),
                }
            }
        }
        if format && !self.sql.trim().is_empty() {
            self.sql = format_sql(&self.sql);
        }
        if let Some(db) = switch_to {
            self.sql.clear();
            self.grid = GridState::default();
            self.backend.switch_database(&db);
        }
        if cancel {
            self.backend.cancel();
        }
        if refresh {
            self.backend.refresh_schema();
        }
        if let Some(sql) = run {
            let reasons = self.backend.destructive(&sql);
            if reasons.is_empty() {
                self.grid = GridState::default();
                self.backend.run(sql);
            } else {
                self.confirm = Some((sql, reasons));
            }
        }

        if let Some((sql, reasons)) = self.confirm.take() {
            let mut go = false;
            let mut keep_open = true;
            egui::Window::new("Confirmar ejecución")
                .id(egui::Id::new(("sql_confirm", uid)))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    ui.set_max_width(420.0);
                    ui.label(RichText::new("Esta consulta puede destruir datos:").strong());
                    for r in &reasons {
                        ui.label(RichText::new(format!("• {r}")).color(theme.status_offline));
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(if sql.len() > 300 {
                            format!("{}…", &sql[..sql.floor_char_boundary(300)])
                        } else {
                            sql.clone()
                        })
                        .monospace()
                        .size(11.5)
                        .color(theme.text_secondary),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui
                            .button(RichText::new("Ejecutar de todos modos").color(theme.status_offline))
                            .clicked()
                        {
                            go = true;
                        }
                        if ui.button("Cancelar").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            keep_open = false;
                        }
                    });
                });
            if go {
                self.grid = GridState::default();
                self.backend.run(sql);
            } else if keep_open {
                self.confirm = Some((sql, reasons));
            }
        }
    }

    fn close(&mut self) {
        self.backend.close();
    }
}
