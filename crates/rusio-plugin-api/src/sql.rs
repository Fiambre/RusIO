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

/// Schema of a table name: the part before the first dot, `public` when there is none.
fn table_schema(name: &str) -> &str {
    name.split_once('.').map_or("public", |(s, _)| s)
}

#[derive(Debug, Clone)]
pub struct SchemaTable {
    pub name: String,
    pub is_view: bool,
    /// (column name, declared type)
    pub columns: Vec<(String, String)>,
    /// Primary-key columns, in key order (empty = none known, so rows cannot be edited safely).
    pub pk: Vec<String>,
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

    /// Quotes a column name for this dialect.
    fn quote_ident(&self, name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
    /// Quotes a text value as a SQL literal for this dialect.
    fn quote_str(&self, value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }
    /// Fully qualified, quoted reference to a [`SchemaTable::name`].
    fn table_ref(&self, table: &str) -> String {
        self.quote_ident(table)
    }
}

/// Preview of one page of `table`, ordered by its primary key so pages do not shuffle.
fn page_query(b: &dyn SqlBackend, t: &SchemaTable, limit: usize, offset: usize) -> String {
    let order = if t.pk.is_empty() {
        String::new()
    } else {
        let cols: Vec<String> = t.pk.iter().map(|c| b.quote_ident(c)).collect();
        format!(" ORDER BY {}", cols.join(", "))
    };
    format!(
        "SELECT * FROM {}{order} LIMIT {limit} OFFSET {offset};",
        b.table_ref(&t.name)
    )
}

/// One cell of a row being edited.
#[derive(Debug, Clone)]
struct EditCell {
    column: String,
    /// What the grid showed (`NULL` included).
    original: String,
    value: String,
    null: bool,
    original_null: bool,
    is_key: bool,
    /// Binary data shown as a placeholder: left alone.
    locked: bool,
}

impl EditCell {
    fn changed(&self) -> bool {
        !self.locked && (self.null != self.original_null || (!self.null && self.value != self.original))
    }
}

/// A row being edited (`insert == false`) or added (`insert == true`) in a table preview.
struct RowEdit {
    table: String,
    insert: bool,
    cells: Vec<EditCell>,
}

impl RowEdit {
    fn begin(table: &SchemaTable, grid: &DataTable, row: Option<usize>) -> Result<Self, String> {
        if table.pk.is_empty() {
            return Err(format!(
                "«{}» no tiene clave primaria: no se puede identificar una fila sin riesgo.",
                table.name
            ));
        }
        for k in &table.pk {
            if !grid.columns.contains(k) {
                return Err(format!("El resultado no incluye la clave primaria «{k}»."));
            }
        }
        let cells = grid
            .columns
            .iter()
            .enumerate()
            .map(|(i, column)| {
                let original = row
                    .and_then(|r| grid.rows.get(r))
                    .and_then(|r| r.get(i))
                    .cloned()
                    .unwrap_or_default();
                let original_null = row.is_some() && original == "NULL";
                EditCell {
                    column: column.clone(),
                    value: if original_null { String::new() } else { original.clone() },
                    original,
                    null: original_null,
                    original_null,
                    is_key: table.pk.contains(column),
                    locked: row.is_some() && original_is_blob(grid, row, i),
                }
            })
            .collect();
        Ok(Self {
            table: table.name.clone(),
            insert: row.is_none(),
            cells,
        })
    }

    /// The statement this edit stands for; `None` when there is nothing to write.
    fn sql(&self, b: &dyn SqlBackend) -> Option<String> {
        let literal = |c: &EditCell| {
            if c.null {
                "NULL".to_string()
            } else {
                b.quote_str(&c.value)
            }
        };
        let table = b.table_ref(&self.table);
        if self.insert {
            let set: Vec<&EditCell> = self.cells.iter().filter(|c| c.null || !c.value.is_empty()).collect();
            if set.is_empty() {
                return None;
            }
            let cols: Vec<String> = set.iter().map(|c| b.quote_ident(&c.column)).collect();
            let vals: Vec<String> = set.iter().map(|c| literal(c)).collect();
            Some(format!(
                "INSERT INTO {table} ({}) VALUES ({});",
                cols.join(", "),
                vals.join(", ")
            ))
        } else {
            let set: Vec<String> = self
                .cells
                .iter()
                .filter(|c| c.changed() && !c.is_key)
                .map(|c| format!("{} = {}", b.quote_ident(&c.column), literal(c)))
                .collect();
            if set.is_empty() {
                return None;
            }
            let key: Vec<String> = self
                .cells
                .iter()
                .filter(|c| c.is_key)
                .map(|c| {
                    if c.original_null {
                        format!("{} IS NULL", b.quote_ident(&c.column))
                    } else {
                        format!("{} = {}", b.quote_ident(&c.column), b.quote_str(&c.original))
                    }
                })
                .collect();
            Some(format!(
                "UPDATE {table} SET {} WHERE {};",
                set.join(", "),
                key.join(" AND ")
            ))
        }
    }
}

fn original_is_blob(grid: &DataTable, row: Option<usize>, col: usize) -> bool {
    row.and_then(|r| grid.rows.get(r))
        .and_then(|r| r.get(col))
        .is_some_and(|v| v.starts_with("<blob "))
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
    /// Schema the browser is limited to (`None` = all).
    schema_filter: Option<String>,
    /// Paging of the table preview: zero-based page and rows per page.
    page: usize,
    page_size: usize,
    /// Table whose preview the grid is showing (the only case where rows can be edited).
    source: Option<String>,
    edit: Option<RowEdit>,
    /// Why editing is not possible, shown under the buttons.
    edit_note: Option<String>,
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
            schema_filter: None,
            page: 0,
            page_size: 100,
            source: None,
            edit: None,
            edit_note: None,
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
        let mut preview_of: Option<String> = None;
        let mut begin_edit: Option<Option<usize>> = None;
        let mut page_to: Option<(usize, usize)> = None;
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

        let schemas: Vec<String> = {
            let mut v: Vec<String> = st.tables.iter().map(|t| table_schema(&t.name).to_string()).collect();
            v.sort();
            v.dedup();
            v
        };
        if let Some(s) = &self.schema_filter {
            if !schemas.contains(s) {
                self.schema_filter = None;
            }
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
                if is_sql && schemas.len() > 1 {
                    let shown = self.schema_filter.clone().unwrap_or_else(|| "Todos".to_string());
                    egui::ComboBox::from_id_salt(("sql_schema_select", uid))
                        .width(ui.available_width() - 8.0)
                        .selected_text(shown)
                        .show_ui(ui, |ui| {
                            if ui.selectable_label(self.schema_filter.is_none(), "Todos").clicked() {
                                self.schema_filter = None;
                            }
                            for s in &schemas {
                                let on = self.schema_filter.as_deref() == Some(s.as_str());
                                if ui.selectable_label(on, s).clicked() {
                                    self.schema_filter = Some(s.clone());
                                }
                            }
                        });
                }
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
                            if let Some(s) = &self.schema_filter {
                                if table_schema(&t.name) != s {
                                    continue;
                                }
                            }
                            let title = if t.is_view {
                                format!("{}  (vista)", t.name)
                            } else {
                                t.name.clone()
                            };
                            // Only the arrow opens the columns; the name is for double-click.
                            let id = ui.make_persistent_id(("sql_table", uid, &t.name));
                            let state =
                                egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false);
                            let (_, name, _) = state
                                .show_header(ui, |ui| {
                                    ui.add(
                                        egui::Label::new(RichText::new(title).size(12.5)).sense(egui::Sense::click()),
                                    )
                                })
                                .body(|ui| {
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
                            if name.inner.double_clicked() {
                                let sql = if is_sql {
                                    page_query(self.backend.as_ref(), t, self.page_size, 0)
                                } else {
                                    self.backend.select_all_sql(&t.name)
                                };
                                self.sql = sql.clone();
                                run = Some(sql);
                                preview_of = Some(t.name.clone());
                                self.page = 0;
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
                        if is_sql && self.source.is_some() {
                            ui.horizontal(|ui| {
                                if ui
                                    .add_enabled(self.grid.selected_row.is_some(), egui::Button::new("Editar fila"))
                                    .on_hover_text("También: doble clic en la fila")
                                    .clicked()
                                {
                                    begin_edit = Some(self.grid.selected_row);
                                }
                                if ui.button("Nueva fila").clicked() {
                                    begin_edit = Some(None);
                                }
                                ui.separator();
                                if ui.add_enabled(self.page > 0, egui::Button::new("◀")).clicked() {
                                    page_to = Some((self.page - 1, self.page_size));
                                }
                                ui.label(RichText::new(format!("Página {}", self.page + 1)).size(11.5));
                                if ui
                                    .add_enabled(table.rows.len() >= self.page_size, egui::Button::new("▶"))
                                    .clicked()
                                {
                                    page_to = Some((self.page + 1, self.page_size));
                                }
                                let mut size = self.page_size;
                                egui::ComboBox::from_id_salt(("sql_page_size", uid))
                                    .width(70.0)
                                    .selected_text(format!("{size} filas"))
                                    .show_ui(ui, |ui| {
                                        for n in [50usize, 100, 250, 500, 1000] {
                                            ui.selectable_value(&mut size, n, format!("{n} filas"));
                                        }
                                    });
                                if size != self.page_size {
                                    page_to = Some((0, size));
                                }
                                if let Some(note) = &self.edit_note {
                                    ui.label(RichText::new(note).size(11.0).color(theme.status_offline));
                                }
                            });
                        }
                        data_grid(ui, table, &mut self.grid);
                        if let Some(r) = self.grid.activated_row.take() {
                            if self.source.is_some() && is_sql {
                                begin_edit = Some(Some(r));
                            }
                        }
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
            self.source = None;
            self.edit = None;
            self.grid = GridState::default();
            self.backend.switch_database(&db);
        }
        if cancel {
            self.backend.cancel();
        }
        if refresh {
            self.backend.refresh_schema();
        }
        if let (Some((page, size)), Some(table)) = (page_to, self.source.clone()) {
            let state = self.backend.state();
            let sql = state
                .lock()
                .tables
                .iter()
                .find(|t| t.name == table)
                .map(|t| page_query(self.backend.as_ref(), t, size, page * size));
            if let Some(sql) = sql {
                self.page = page;
                self.page_size = size;
                self.sql = sql.clone();
                run = Some(sql);
                preview_of = Some(table);
            }
        }
        if let (Some(row), Some(table)) = (begin_edit, self.source.clone()) {
            let state = self.backend.state();
            let st = state.lock();
            let found = st.tables.iter().find(|t| t.name == table);
            let grid = st.result.as_ref().and_then(|r| r.table.as_ref());
            match (found, grid) {
                (Some(t), Some(g)) => match RowEdit::begin(t, g, row) {
                    Ok(e) => {
                        self.edit = Some(e);
                        self.edit_note = None;
                    }
                    Err(e) => self.edit_note = Some(e),
                },
                _ => self.edit_note = Some("No se encontró la tabla en el esquema; actualízalo.".to_string()),
            }
        }
        if let Some(mut edit) = self.edit.take() {
            let mut save = false;
            let mut keep = true;
            let sql = edit.sql(self.backend.as_ref());
            egui::Window::new(if edit.insert { "Nueva fila" } else { "Editar fila" })
                .id(egui::Id::new(("sql_row_edit", uid)))
                .collapsible(false)
                .resizable(true)
                .default_width(560.0)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ui.ctx(), |ui| {
                    ui.label(RichText::new(&edit.table).strong());
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        let insert = edit.insert;
                        for c in &mut edit.cells {
                            let fixed = c.locked || (c.is_key && !insert);
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(if c.is_key {
                                        format!("{} (clave)", c.column)
                                    } else {
                                        c.column.clone()
                                    })
                                    .size(11.5)
                                    .color(theme.text_secondary),
                                );
                                ui.add_enabled_ui(!fixed, |ui| {
                                    ui.checkbox(&mut c.null, "NULL");
                                });
                            });
                            // Long values wrap inside a field as wide as the window.
                            let rows = if c.value.len() > 80 || c.value.contains('\n') {
                                3
                            } else {
                                1
                            };
                            ui.add_enabled(
                                !fixed && !c.null,
                                egui::TextEdit::multiline(&mut c.value)
                                    .desired_rows(rows)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.add_space(4.0);
                        }
                    });
                    ui.add_space(6.0);
                    match &sql {
                        Some(s) => ui.label(RichText::new(s).monospace().size(11.0).color(theme.text_secondary)),
                        None => ui.label(RichText::new("Sin cambios.").size(11.0).color(theme.text_muted)),
                    };
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(sql.is_some(), egui::Button::new("Guardar")).clicked() {
                            save = true;
                        }
                        if ui.button("Cancelar").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            keep = false;
                        }
                    });
                });
            if save {
                if let Some(sql) = sql {
                    // The write and a fresh preview go together so the grid shows the outcome.
                    let state = self.backend.state();
                    let refresh_sql = state
                        .lock()
                        .tables
                        .iter()
                        .find(|t| t.name == edit.table)
                        .map(|t| page_query(self.backend.as_ref(), t, self.page_size, self.page * self.page_size))
                        .unwrap_or_else(|| self.backend.select_all_sql(&edit.table));
                    let full = format!("{sql}\n{refresh_sql}");
                    self.grid = GridState::default();
                    self.backend.run(full);
                }
            } else if keep {
                self.edit = Some(edit);
            }
        }
        if let Some(sql) = run {
            self.source = preview_of.take();
            self.edit_note = None;
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

#[cfg(test)]
mod edit_tests {
    use super::*;

    struct Dummy;
    impl SqlBackend for Dummy {
        fn state(&self) -> Arc<Mutex<ConsoleState>> {
            Arc::default()
        }
        fn is_alive(&self) -> bool {
            true
        }
        fn run(&self, _sql: String) {}
        fn refresh_schema(&self) {}
        fn cancel(&self) {}
        fn close(&self) {}
        fn select_all_sql(&self, _table: &str) -> String {
            String::new()
        }
    }

    fn table(pk: &[&str]) -> SchemaTable {
        SchemaTable {
            name: "t".into(),
            is_view: false,
            columns: vec![],
            pk: pk.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn grid() -> DataTable {
        DataTable {
            columns: vec!["id".into(), "nombre".into(), "nota".into()],
            rows: vec![vec!["7".into(), "Ana".into(), "NULL".into()]],
        }
    }

    #[test]
    fn update_sets_only_changed_cells_and_keys_on_the_original_pk() {
        let mut e = RowEdit::begin(&table(&["id"]), &grid(), Some(0)).unwrap();
        assert_eq!(e.sql(&Dummy), None);
        e.cells[1].value = "O'Brien".into();
        e.cells[2].null = false;
        e.cells[2].value = "ok".into();
        assert_eq!(
            e.sql(&Dummy).unwrap(),
            "UPDATE \"t\" SET \"nombre\" = 'O''Brien', \"nota\" = 'ok' WHERE \"id\" = '7';"
        );
    }

    #[test]
    fn insert_skips_empty_cells_and_honors_null() {
        let mut e = RowEdit::begin(&table(&["id"]), &grid(), None).unwrap();
        assert_eq!(e.sql(&Dummy), None);
        e.cells[1].value = "Luis".into();
        e.cells[2].null = true;
        assert_eq!(
            e.sql(&Dummy).unwrap(),
            "INSERT INTO \"t\" (\"nombre\", \"nota\") VALUES ('Luis', NULL);"
        );
    }

    #[test]
    fn tables_without_primary_key_are_refused() {
        assert!(RowEdit::begin(&table(&[]), &grid(), Some(0)).is_err());
        assert!(RowEdit::begin(&table(&["otra"]), &grid(), Some(0)).is_err());
    }
}
