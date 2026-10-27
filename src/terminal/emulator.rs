use eframe::egui::{self, Color32, FontId, Pos2, Rect, Vec2};
use parking_lot::Mutex;
use std::num::NonZeroU16;
use std::sync::Arc;

pub struct TerminalEmulator {
    parser: vt100::Parser,
    cols: u16,
    rows: u16,
    cursor_blink: f64,
}

impl TerminalEmulator {
    pub fn new(cols: u16, rows: u16) -> Self {
        let r = NonZeroU16::new(rows.max(1)).unwrap();
        let c = NonZeroU16::new(cols.max(1)).unwrap();
        Self {
            parser: vt100::Parser::new(r, c, 10_000),
            cols,
            rows,
            cursor_blink: 0.0,
        }
    }

    pub fn feed(&mut self, data: &[u8]) {
        self.parser.process(data);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols > 0 && rows > 0 && (cols != self.cols || rows != self.rows) {
            self.cols = cols;
            self.rows = rows;
            let r = NonZeroU16::new(rows.max(1)).unwrap();
            let c = NonZeroU16::new(cols.max(1)).unwrap();
            self.parser.set_size(r, c);
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn render(
        &mut self,
        ui: &mut egui::Ui,
        font_size: f32,
        focused: bool,
    ) -> (Option<Vec<u8>>, Option<(u16, u16)>) {
        let (rect, response) = ui.allocate_exact_size(
            ui.available_size(),
            egui::Sense::click_and_drag(),
        );

        let painter = ui.painter_at(rect);

        // Terminal background
        painter.rect_filled(rect, 0.0, Color32::from_rgb(10, 12, 16));

        let font_id = FontId::monospace(font_size);

        // Measure char width and line height
        let char_size = ui.fonts(|f| {
            let row_height = f.row_height(&font_id);
            let char_width = f.glyph_width(&font_id, 'M');
            Vec2::new(char_width, row_height)
        });

        // Recalculate cols and rows that can fit in this rect
        let available_cols = (rect.width() / char_size.x).floor() as u16;
        let available_rows = (rect.height() / char_size.y).floor() as u16;

        let mut resized_to = None;
        if available_cols >= 20 && available_rows >= 5 {
            if available_cols != self.cols || available_rows != self.rows {
                self.resize(available_cols, available_rows);
                resized_to = Some((available_cols, available_rows));
            }
        }

        let screen = self.parser.screen();
        let (cursor_row, cursor_col) = screen.cursor_position();
        let cursor_visible = !screen.hide_cursor();

        // Render visible grid cells
        for row in 0..self.rows {
            let y = rect.min.y + (row as f32) * char_size.y;
            if y + char_size.y > rect.max.y {
                break;
            }

            let mut col = 0;
            while col < self.cols {
                if let Some(cell) = screen.cell(row, col) {
                    let fg = vt_color_to_egui(cell.fgcolor(), Color32::from_rgb(220, 225, 235));
                    let bg = vt_bg_to_egui(cell.bgcolor());

                    // Find how many consecutive cells share the same fg and bg
                    let mut run_text = String::new();
                    let start_col = col;

                    while col < self.cols {
                        if let Some(c) = screen.cell(row, col) {
                            let cell_fg = vt_color_to_egui(c.fgcolor(), Color32::from_rgb(220, 225, 235));
                            let cell_bg = vt_bg_to_egui(c.bgcolor());
                            if cell_fg == fg && cell_bg == bg {
                                let ch = c.contents();
                                if ch.is_empty() {
                                    run_text.push(' ');
                                } else {
                                    run_text.push_str(ch);
                                }
                                col += 1;
                            } else {
                                break;
                            }
                        } else {
                            run_text.push(' ');
                            col += 1;
                        }
                    }

                    let x = rect.min.x + (start_col as f32) * char_size.x;
                    let width = ((col - start_col) as f32) * char_size.x;
                    let cell_rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(width, char_size.y));

                    // Draw custom background if present
                    if let Some(bg_color) = bg {
                        painter.rect_filled(cell_rect, 0.0, bg_color);
                    }

                    // Draw text
                    painter.text(
                        Pos2::new(x, y),
                        egui::Align2::LEFT_TOP,
                        &run_text,
                        font_id.clone(),
                        fg,
                    );
                } else {
                    col += 1;
                }
            }
        }

        // Draw cursor if focused and visible
        self.cursor_blink += ui.input(|i| i.stable_dt as f64);
        let cursor_on = (self.cursor_blink % 1.0) < 0.65;

        if focused && cursor_visible && cursor_on {
            let cursor_x = rect.min.x + (cursor_col as f32) * char_size.x;
            let cursor_y = rect.min.y + (cursor_row as f32) * char_size.y;
            let cursor_rect = Rect::from_min_size(
                Pos2::new(cursor_x, cursor_y),
                Vec2::new(char_size.x, char_size.y),
            );
            // Modern hollow/filled block cursor
            painter.rect_filled(
                cursor_rect,
                1.0,
                Color32::from_rgba_premultiplied(99, 102, 241, 180),
            );
        }

        // Collect keyboard input if focused
        let mut input_bytes = Vec::new();

        if focused || response.has_focus() {
            ui.input(|input| {
                let ctrl = input.modifiers.ctrl;

                // Handle text input (normal typing)
                for event in &input.events {
                    match event {
                        egui::Event::Text(text) => {
                            if !ctrl {
                                input_bytes.extend_from_slice(text.as_bytes());
                            }
                        }
                        egui::Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } => {
                            if modifiers.ctrl {
                                match key {
                                    egui::Key::C => input_bytes.push(0x03), // SIGINT Ctrl+C
                                    egui::Key::D => input_bytes.push(0x04), // EOF Ctrl+D
                                    egui::Key::Z => input_bytes.push(0x1a), // SIGTSTP
                                    egui::Key::L => input_bytes.push(0x0c), // Formfeed / Clear
                                    egui::Key::A => input_bytes.push(0x01), // Home
                                    egui::Key::E => input_bytes.push(0x05), // End
                                    egui::Key::K => input_bytes.push(0x0b), // Kill line
                                    egui::Key::U => input_bytes.push(0x15), // Clear line
                                    egui::Key::W => input_bytes.push(0x17), // Delete word
                                    egui::Key::R => input_bytes.push(0x12), // Reverse search
                                    _ => {}
                                }
                            } else {
                                match key {
                                    egui::Key::Enter => input_bytes.extend_from_slice(b"\r"),
                                    egui::Key::Backspace => input_bytes.push(0x08),
                                    egui::Key::Tab => input_bytes.push(0x09),
                                    egui::Key::Escape => input_bytes.push(0x1b),
                                    egui::Key::ArrowUp => input_bytes.extend_from_slice(b"\x1b[A"),
                                    egui::Key::ArrowDown => input_bytes.extend_from_slice(b"\x1b[B"),
                                    egui::Key::ArrowRight => input_bytes.extend_from_slice(b"\x1b[C"),
                                    egui::Key::ArrowLeft => input_bytes.extend_from_slice(b"\x1b[D"),
                                    egui::Key::Home => input_bytes.extend_from_slice(b"\x1b[H"),
                                    egui::Key::End => input_bytes.extend_from_slice(b"\x1b[F"),
                                    egui::Key::PageUp => input_bytes.extend_from_slice(b"\x1b[5~"),
                                    egui::Key::PageDown => input_bytes.extend_from_slice(b"\x1b[6~"),
                                    egui::Key::Delete => input_bytes.extend_from_slice(b"\x1b[3~"),
                                    egui::Key::Insert => input_bytes.extend_from_slice(b"\x1b[2~"),
                                    _ => {}
                                }
                            }
                        }
                        egui::Event::Paste(text) => {
                            input_bytes.extend_from_slice(text.as_bytes());
                        }
                        _ => {}
                    }
                }
            });
        }

        let input_opt = if !input_bytes.is_empty() {
            Some(input_bytes)
        } else {
            None
        };

        (input_opt, resized_to)
    }
}

pub type SharedTerminal = Arc<Mutex<TerminalEmulator>>;

fn vt_bg_to_egui(c: vt100::Color) -> Option<Color32> {
    match c {
        vt100::Color::Default => None,
        vt100::Color::Idx(i) => Some(ansi_16_color(i)),
        vt100::Color::Rgb(r, g, b) => Some(Color32::from_rgb(r, g, b)),
    }
}

fn vt_color_to_egui(c: vt100::Color, default: Color32) -> Color32 {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => ansi_16_color(i),
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
    }
}

fn ansi_16_color(i: u8) -> Color32 {
    match i {
        0 => Color32::from_rgb(20, 24, 34),       // Black
        1 => Color32::from_rgb(239, 68, 68),      // Red
        2 => Color32::from_rgb(34, 197, 94),      // Green
        3 => Color32::from_rgb(234, 179, 8),      // Yellow
        4 => Color32::from_rgb(59, 130, 246),     // Blue
        5 => Color32::from_rgb(168, 85, 247),     // Magenta
        6 => Color32::from_rgb(6, 182, 212),      // Cyan
        7 => Color32::from_rgb(209, 213, 219),    // White
        8 => Color32::from_rgb(75, 85, 99),       // Bright Black (Gray)
        9 => Color32::from_rgb(248, 113, 113),    // Bright Red
        10 => Color32::from_rgb(74, 222, 128),    // Bright Green
        11 => Color32::from_rgb(250, 204, 21),    // Bright Yellow
        12 => Color32::from_rgb(96, 165, 250),    // Bright Blue
        13 => Color32::from_rgb(192, 132, 252),   // Bright Magenta
        14 => Color32::from_rgb(56, 189, 248),    // Bright Cyan
        15 => Color32::from_rgb(243, 244, 246),   // Bright White
        _ => Color32::from_rgb(209, 213, 219),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emulator_feed_and_cursor() {
        let mut emu = TerminalEmulator::new(80, 24);
        emu.feed(b"Hello Pulento\r\n");

        let screen = emu.parser.screen();
        let (row, col) = screen.cursor_position();
        assert_eq!(row, 1);
        assert_eq!(col, 0);

        let cell = screen.cell(0, 0).expect("First cell exists");
        assert_eq!(cell.contents(), "H");
    }

    #[test]
    fn test_emulator_resize() {
        let mut emu = TerminalEmulator::new(80, 24);
        emu.resize(120, 40);
        assert_eq!(emu.cols(), 120);
        assert_eq!(emu.rows(), 40);
    }
}
