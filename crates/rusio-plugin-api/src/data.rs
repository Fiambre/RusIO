//! A tabular result (query output) and a virtualized grid widget to show it. Shared by every
//! plugin that returns rows, so they all look and behave the same.
use egui::{Align2, Color32, FontId, Rect, CornerRadius, Sense, Vec2};

use crate::theme::Theme;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataTable {
    pub columns: Vec<String>,
    /// Every cell already rendered as text (NULL is shown by the producer, e.g. "NULL").
    pub rows: Vec<Vec<String>>,
}

impl DataTable {
    /// RFC 4180 CSV with a header row.
    pub fn to_csv(&self) -> String {
        fn field(s: &str, out: &mut String) {
            if s.contains([',', '"', '\n', '\r']) {
                out.push('"');
                out.push_str(&s.replace('"', "\"\""));
                out.push('"');
            } else {
                out.push_str(s);
            }
        }
        let mut out = String::new();
        let line = |cells: &[String], out: &mut String| {
            for (i, c) in cells.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                field(c, out);
            }
            out.push_str("\r\n");
        };
        line(&self.columns, &mut out);
        for r in &self.rows {
            line(r, &mut out);
        }
        out
    }
}

#[derive(Default)]
pub struct GridState {
    pub selected_row: Option<usize>,
    /// Cell under the last right-click, used by the context menu.
    menu_cell: Option<(usize, usize)>,
}

const ROW_H: f32 = 22.0;
const HEADER_H: f32 = 26.0;
const CELL_PAD: f32 = 8.0;
const MIN_COL: f32 = 60.0;
const MAX_COL: f32 = 360.0;
const CHAR_W: f32 = 7.0;

fn column_widths(table: &DataTable) -> Vec<f32> {
    (0..table.columns.len())
        .map(|c| {
            let header = table.columns[c].chars().count();
            let body = table
                .rows
                .iter()
                .take(200)
                .map(|r| r.get(c).map_or(0, |s| s.chars().count().min(60)))
                .max()
                .unwrap_or(0);
            (header.max(body) as f32 * CHAR_W + CELL_PAD * 2.0).clamp(MIN_COL, MAX_COL)
        })
        .collect()
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Draws `table` with a sticky header and only the visible rows. Click selects a row;
/// right-click offers to copy the cell or the whole row.
pub fn data_grid(ui: &mut egui::Ui, table: &DataTable, state: &mut GridState) {
    let theme = Theme::default();
    let widths = column_widths(table);
    let total_w: f32 = widths.iter().sum();

    egui::ScrollArea::horizontal()
        .id_salt("data_grid_h")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_min_width(total_w);
            ui.spacing_mut().item_spacing.y = 0.0;

            let (hrect, _) = ui.allocate_exact_size(Vec2::new(total_w, HEADER_H), Sense::hover());
            ui.painter().rect_filled(hrect, CornerRadius::ZERO, theme.bg_panel_alt);
            let mut x = hrect.left();
            for (i, name) in table.columns.iter().enumerate() {
                let col = Rect::from_min_size(
                    egui::pos2(x, hrect.top()),
                    Vec2::new(widths[i], HEADER_H),
                );
                ui.painter().with_clip_rect(col.intersect(ui.clip_rect())).text(
                    egui::pos2(x + CELL_PAD, hrect.center().y),
                    Align2::LEFT_CENTER,
                    name,
                    FontId::proportional(11.5),
                    theme.text_primary,
                );
                x += widths[i];
            }

            let body_h = ui.available_height();
            egui::ScrollArea::vertical()
                .id_salt("data_grid_v")
                .auto_shrink([false, false])
                .max_height(body_h)
                .show_rows(ui, ROW_H, table.rows.len(), |ui, range| {
                    for ri in range {
                        let row = &table.rows[ri];
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(total_w, ROW_H), Sense::click());

                        if state.selected_row == Some(ri) {
                            ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg_active);
                        } else if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::ZERO, theme.bg_hover);
                        } else if ri % 2 == 1 {
                            ui.painter().rect_filled(
                                rect,
                                CornerRadius::ZERO,
                                Color32::from_white_alpha(4),
                            );
                        }

                        let mut x = rect.left();
                        for (ci, w) in widths.iter().enumerate() {
                            let cell = row.get(ci).map(String::as_str).unwrap_or("");
                            let is_null = cell == "NULL";
                            let col = Rect::from_min_size(
                                egui::pos2(x, rect.top()),
                                Vec2::new(*w, ROW_H),
                            );
                            ui.painter()
                                .with_clip_rect(col.intersect(ui.clip_rect()))
                                .text(
                                    egui::pos2(x + CELL_PAD, rect.center().y),
                                    Align2::LEFT_CENTER,
                                    one_line(cell),
                                    FontId::proportional(12.0),
                                    if is_null { theme.text_muted } else { theme.text_primary },
                                );
                            x += *w;
                        }

                        if resp.clicked() || resp.secondary_clicked() {
                            state.selected_row = Some(ri);
                        }
                        if resp.secondary_clicked() {
                            let px = resp
                                .interact_pointer_pos()
                                .map_or(0.0, |p| p.x - rect.left());
                            let mut acc = 0.0;
                            let mut col = widths.len().saturating_sub(1);
                            for (ci, w) in widths.iter().enumerate() {
                                acc += *w;
                                if px < acc {
                                    col = ci;
                                    break;
                                }
                            }
                            state.menu_cell = Some((ri, col));
                        }

                        resp.context_menu(|ui| {
                            let (mr, mc) = state.menu_cell.unwrap_or((ri, 0));
                            if let Some(r) = table.rows.get(mr) {
                                if ui.button("Copiar celda").clicked() {
                                    ui.ctx().copy_text(r.get(mc).cloned().unwrap_or_default());
                                    ui.close();
                                }
                                if ui.button("Copiar fila").clicked() {
                                    ui.ctx().copy_text(r.join("\t"));
                                    ui.close();
                                }
                            }
                        });
                    }
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_quotes_only_when_needed() {
        let t = DataTable {
            columns: vec!["id".into(), "nota".into()],
            rows: vec![
                vec!["1".into(), "hola".into()],
                vec!["2".into(), "a,b".into()],
                vec!["3".into(), "dice \"hi\"\nluego".into()],
            ],
        };
        assert_eq!(
            t.to_csv(),
            "id,nota\r\n1,hola\r\n2,\"a,b\"\r\n3,\"dice \"\"hi\"\"\nluego\"\r\n"
        );
    }

    #[test]
    fn column_width_is_clamped() {
        let t = DataTable {
            columns: vec!["x".into(), "a".repeat(100)],
            rows: vec![],
        };
        let w = column_widths(&t);
        assert_eq!(w[0], MIN_COL);
        assert_eq!(w[1], MAX_COL);
    }
}
