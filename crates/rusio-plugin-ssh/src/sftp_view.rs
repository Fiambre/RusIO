use rusio_plugin_api::egui::{self, Color32, Rect, RichText, CornerRadius, Sense, Vec2};

use crate::sftp::{human_size, SftpCmd, SftpEntry, SftpSession};
use rusio_plugin_api::icons::{self, Icon};
use rusio_plugin_api::theme::Theme;

const CTRL_H: f32 = 26.0;

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{}{}", dir, name)
    } else {
        format!("{}/{}", dir, name)
    }
}

fn parent_of(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
    }
}

/// "YYYY-MM-DD HH:MM" in UTC from a unix timestamp.
fn format_time(ts: u64) -> String {
    let days = (ts / 86_400) as i64;
    let secs = ts % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, m, d, secs / 3600, (secs % 3600) / 60)
}

fn format_perm(perm: Option<u32>, is_dir: bool) -> String {
    let Some(p) = perm else { return String::new() };
    let mut s = String::with_capacity(10);
    s.push(if is_dir { 'd' } else { '-' });
    for shift in [6, 3, 0] {
        s.push(if p >> (shift + 2) & 1 == 1 { 'r' } else { '-' });
        s.push(if p >> (shift + 1) & 1 == 1 { 'w' } else { '-' });
        s.push(if p >> shift & 1 == 1 { 'x' } else { '-' });
    }
    s
}

fn paint_entry_icon_at(ui: &egui::Ui, min: egui::Pos2, is_dir: bool) {
    let theme = Theme::default();
    let p = ui.painter();
    if is_dir {
        let col = theme.accent_vnc;
        p.rect_filled(
            Rect::from_min_size(min + Vec2::new(0.0, 1.0), Vec2::new(7.0, 4.0)),
            CornerRadius::same(2),
            col,
        );
        p.rect_filled(
            Rect::from_min_size(min + Vec2::new(0.0, 3.0), Vec2::new(15.0, 10.0)),
            CornerRadius::same(2),
            col,
        );
    } else {
        p.rect_filled(
            Rect::from_min_size(min + Vec2::new(2.0, 0.0), Vec2::new(11.0, 14.0)),
            CornerRadius::same(2),
            theme.text_muted,
        );
    }
}

fn pick_and_download(session: &SftpSession, cwd: &str, entry: &SftpEntry) {
    let remote = join(cwd, &entry.name);
    if entry.is_dir {
        if let Some(dir) = rfd::FileDialog::new().set_title("Descargar en...").pick_folder() {
            session.send(SftpCmd::Download { remote, local: dir.join(&entry.name) });
        }
    } else if let Some(path) = rfd::FileDialog::new()
        .set_title("Guardar como")
        .set_file_name(&entry.name)
        .save_file()
    {
        session.send(SftpCmd::Download { remote, local: path });
    }
}

pub fn render_sftp(ui: &mut egui::Ui, s: &mut SftpSession) {
    let theme = Theme::default();

    let (connected, cwd, entries, status, is_error, busy) = {
        let st = s.state.lock();
        (
            st.connected,
            st.cwd.clone(),
            st.entries.clone(),
            st.status.clone(),
            st.is_error,
            st.busy.clone(),
        )
    };

    if s.ui.shown_cwd != cwd {
        s.ui.shown_cwd = cwd.clone();
        s.ui.path_edit = cwd.clone();
        s.ui.selected = None;
    }

    // Files dropped from the OS onto the window are uploaded to the current directory.
    if connected {
        let dropped: Vec<_> = ui.ctx().input(|i| i.raw.dropped_files.clone());
        for f in dropped {
            let path = f.path();
            if !path.as_os_str().is_empty() {
                s.send(SftpCmd::Upload { local: path.to_path_buf(), remote_dir: cwd.clone() });
            }
        }
    }

    // Toolbar
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = CTRL_H;
        ui.spacing_mut().button_padding = Vec2::new(10.0, 4.0);

        icons::status_dot(
            ui,
            if connected { theme.status_online } else if is_error { theme.status_offline } else { theme.text_muted },
        );
        ui.label(
            RichText::new(if connected { "SFTP CONECTADO" } else if is_error { "SFTP DESCONECTADO" } else { "CONECTANDO..." })
                .size(10.0)
                .color(if connected { theme.status_online } else if is_error { theme.status_offline } else { theme.text_secondary }),
        );
        ui.separator();

        ui.add_enabled_ui(connected, |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if icons::icon_button_sized(ui, Icon::ArrowUp, None, "Subir un nivel", CTRL_H) {
                s.send(SftpCmd::List(parent_of(&cwd)));
            }
            if icons::icon_button_sized(ui, Icon::Refresh, None, "Actualizar", CTRL_H) {
                s.send(SftpCmd::List(cwd.clone()));
            }

            let resp = ui.add_sized(
                [(ui.available_width() - 4.0 * (CTRL_H + 4.0) - 4.0).max(120.0), CTRL_H],
                egui::TextEdit::singleline(&mut s.ui.path_edit)
                    .vertical_align(egui::Align::Center),
            );
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                s.send(SftpCmd::List(s.ui.path_edit.trim().to_string()));
            }

            if icons::icon_button_sized(ui, Icon::Upload, None, "Subir archivos...", CTRL_H) {
                if let Some(files) = rfd::FileDialog::new().set_title("Subir archivos").pick_files() {
                    for f in files {
                        s.send(SftpCmd::Upload { local: f, remote_dir: cwd.clone() });
                    }
                }
            }
            if icons::icon_button_sized(ui, Icon::UploadFolder, None, "Subir carpeta...", CTRL_H) {
                if let Some(dir) = rfd::FileDialog::new().set_title("Subir carpeta").pick_folder() {
                    s.send(SftpCmd::Upload { local: dir, remote_dir: cwd.clone() });
                }
            }
            if icons::icon_button_sized(ui, Icon::FolderPlus, None, "Nueva carpeta...", CTRL_H) {
                s.ui.mkdir = Some(String::new());
            }
            if icons::icon_button_sized(ui, Icon::FilePlus, None, "Nuevo archivo...", CTRL_H) {
                s.ui.newfile = Some(String::new());
            }
        });
    });

    // Status line
    ui.horizontal(|ui| {
        if let Some(b) = &busy {
            ui.spinner();
            ui.label(RichText::new(b).size(11.0).color(theme.text_primary));
        } else if !status.is_empty() {
            ui.label(
                RichText::new(&status)
                    .size(11.0)
                    .color(if is_error { theme.status_offline } else { theme.text_secondary }),
            );
        } else {
            ui.label(
                RichText::new("Clic para seleccionar · doble clic para abrir · Supr elimina · F2 renombra · clic derecho para más opciones · arrastra archivos aquí para subirlos")
                    .size(11.0)
                    .color(theme.text_muted),
            );
        }
    });
    ui.separator();

    // File table: every row reserves its full-width rect as the click target first,
    // then paints its columns on top.
    const ROW_H: f32 = 26.0;
    const SIZE_W: f32 = 90.0;
    const DATE_W: f32 = 140.0;
    const PERM_W: f32 = 90.0;

    egui::ScrollArea::vertical()
        .id_salt("sftp_files")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let width = ui.available_width();
            let name_w = (width - SIZE_W - DATE_W - PERM_W - 24.0).max(120.0);

            let text_at = |ui: &egui::Ui, rect: Rect, col_x: f32, col_w: f32, right: bool, text: &str, size: f32, color: Color32, mono: bool| {
                let col = Rect::from_min_size(
                    egui::pos2(rect.left() + col_x, rect.top()),
                    Vec2::new(col_w, rect.height()),
                )
                .intersect(ui.clip_rect());
                let font = if mono {
                    egui::FontId::monospace(size)
                } else {
                    egui::FontId::proportional(size)
                };
                let (anchor, x) = if right {
                    (egui::Align2::RIGHT_CENTER, rect.left() + col_x + col_w)
                } else {
                    (egui::Align2::LEFT_CENTER, rect.left() + col_x)
                };
                ui.painter()
                    .with_clip_rect(col)
                    .text(egui::pos2(x, rect.center().y), anchor, text, font, color);
            };

            // Header
            let (hrect, _) = ui.allocate_exact_size(Vec2::new(width, 22.0), Sense::hover());
            let name_x = 28.0;
            let size_x = name_x + name_w;
            let date_x = size_x + SIZE_W + 12.0;
            let perm_x = date_x + DATE_W + 12.0;
            text_at(ui, hrect, name_x, name_w, false, "Nombre", 10.0, theme.text_muted, false);
            text_at(ui, hrect, size_x, SIZE_W, true, "Tamaño", 10.0, theme.text_muted, false);
            text_at(ui, hrect, date_x, DATE_W, false, "Modificado", 10.0, theme.text_muted, false);
            text_at(ui, hrect, perm_x, PERM_W, false, "Permisos", 10.0, theme.text_muted, false);

            // ".." row
            if connected && cwd != "/" {
                let up_selected = s.ui.selected.as_deref() == Some("..");
                let (rect, row) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());
                if up_selected {
                    ui.painter().rect_filled(rect, CornerRadius::same(4), theme.bg_active);
                } else if row.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(4), theme.bg_hover);
                }
                paint_entry_icon_at(ui, rect.min + Vec2::new(8.0, 6.0), true);
                text_at(ui, rect, name_x, name_w, false, "..", 12.5, theme.text_primary, false);
                // Selectable like any row; Supr/F2 ignore it because it is not a real entry.
                if row.clicked() || row.secondary_clicked() {
                    s.ui.selected = Some("..".to_string());
                }
                if row.double_clicked() {
                    s.send(SftpCmd::List(parent_of(&cwd)));
                }
            }

            for entry in &entries {
                let is_selected = s.ui.selected.as_deref() == Some(entry.name.as_str());
                let (rect, row) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::click());

                if is_selected {
                    ui.painter().rect_filled(rect, CornerRadius::same(4), theme.bg_active);
                } else if row.hovered() {
                    ui.painter().rect_filled(rect, CornerRadius::same(4), theme.bg_hover);
                }

                paint_entry_icon_at(ui, rect.min + Vec2::new(8.0, 6.0), entry.is_dir);
                text_at(ui, rect, name_x, name_w, false, &entry.name, 12.5, theme.text_primary, false);
                if !entry.is_dir {
                    text_at(ui, rect, size_x, SIZE_W, true, &human_size(entry.size), 11.0, theme.text_secondary, false);
                }
                text_at(
                    ui,
                    rect,
                    date_x,
                    DATE_W,
                    false,
                    &entry.mtime.map(format_time).unwrap_or_default(),
                    11.0,
                    theme.text_secondary,
                    false,
                );
                text_at(
                    ui,
                    rect,
                    perm_x,
                    PERM_W,
                    false,
                    &format_perm(entry.perm, entry.is_dir),
                    11.0,
                    theme.text_muted,
                    true,
                );

                if row.clicked() || row.secondary_clicked() {
                    s.ui.selected = Some(entry.name.clone());
                }
                if row.double_clicked() {
                    if entry.is_dir {
                        s.send(SftpCmd::List(join(&cwd, &entry.name)));
                    } else {
                        pick_and_download(s, &cwd, entry);
                    }
                }

                row.context_menu(|ui| {
                    if entry.is_dir && ui.button("Abrir").clicked() {
                        s.send(SftpCmd::List(join(&cwd, &entry.name)));
                        ui.close();
                    }
                    if !entry.is_dir && ui.button("Editar").clicked() {
                        s.ui.pending_edit = Some(join(&cwd, &entry.name));
                        ui.close();
                    }
                    if ui.button("Descargar...").clicked() {
                        pick_and_download(s, &cwd, entry);
                        ui.close();
                    }
                    if ui.button("Renombrar...").clicked() {
                        s.ui.rename = Some((join(&cwd, &entry.name), entry.name.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .button(RichText::new("Eliminar...").color(theme.status_offline))
                        .clicked()
                    {
                        s.ui.delete =
                            Some((join(&cwd, &entry.name), entry.is_dir, entry.name.clone()));
                        ui.close();
                    }
                });
            }

            if connected && entries.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("(Carpeta vacía)").size(11.0).color(theme.text_muted));
            }
        });

    // Keyboard shortcuts act on the selected row (never while typing or in a dialog).
    let dialog_open = s.ui.delete.is_some()
        || s.ui.rename.is_some()
        || s.ui.mkdir.is_some()
        || s.ui.newfile.is_some();
    if connected && !dialog_open && !ui.ctx().egui_wants_keyboard_input() {
        if let Some(sel) = s.ui.selected.clone() {
            if let Some(entry) = entries.iter().find(|e| e.name == sel) {
                if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                    s.ui.delete = Some((join(&cwd, &entry.name), entry.is_dir, entry.name.clone()));
                } else if ui.input(|i| i.key_pressed(egui::Key::F2)) {
                    s.ui.rename = Some((join(&cwd, &entry.name), entry.name.clone()));
                }
            }
        }
    }

    dialogs(ui.ctx(), s, &cwd, &theme);
}

fn dialogs(ctx: &egui::Context, s: &mut SftpSession, cwd: &str, theme: &Theme) {
    // New folder
    if let Some(mut name) = s.ui.mkdir.take() {
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new("Nueva carpeta")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text("Nombre"));
                r.request_focus();
                ok = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Crear").color(Color32::WHITE)).clicked() {
                        ok = true;
                    }
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if ok && !name.trim().is_empty() {
            s.send(SftpCmd::Mkdir(join(cwd, name.trim())));
        } else if !cancel {
            s.ui.mkdir = Some(name);
        }
    }

    // New file
    if let Some(mut name) = s.ui.newfile.take() {
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new("Nuevo archivo")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut name).hint_text("nombre.txt"));
                r.request_focus();
                ok = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Crear").color(Color32::WHITE)).clicked() {
                        ok = true;
                    }
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if ok && !name.trim().is_empty() {
            s.send(SftpCmd::CreateFile(join(cwd, name.trim())));
        } else if !cancel {
            s.ui.newfile = Some(name);
        }
    }

    // Rename
    if let Some((path, mut new_name)) = s.ui.rename.take() {
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new("Renombrar")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut new_name));
                r.request_focus();
                ok = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Renombrar").color(Color32::WHITE)).clicked() {
                        ok = true;
                    }
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if ok && !new_name.trim().is_empty() {
            s.send(SftpCmd::Rename { from: path.clone(), to: join(&parent_of(&path), new_name.trim()) });
        } else if !cancel {
            s.ui.rename = Some((path, new_name));
        }
    }

    // Delete confirmation
    if let Some((path, is_dir, name)) = s.ui.delete.take() {
        let (mut ok, mut cancel) = (false, false);
        egui::Window::new(if is_dir { "Eliminar carpeta" } else { "Eliminar archivo" })
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                ui.label(format!("¿Eliminar \"{}\" del servidor?", name));
                ui.label(
                    RichText::new(if is_dir {
                        "Se eliminará la carpeta y todo su contenido. No se puede deshacer."
                    } else {
                        "Esta acción no se puede deshacer."
                    })
                    .size(11.0)
                    .color(theme.text_muted),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Eliminar").color(Color32::WHITE)).clicked() {
                        ok = true;
                    }
                    if ui.button("Cancelar").clicked() {
                        cancel = true;
                    }
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if ok {
            s.send(SftpCmd::Remove { path, is_dir });
        } else if !cancel {
            s.ui.delete = Some((path, is_dir, name));
        }
    }
}
