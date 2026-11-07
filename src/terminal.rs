//! An agent running inside the app: a pseudo-terminal, a screen model, and an egui view.

use eframe::egui::{self, Color32};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtyPair, PtySize};
use crate::path_env;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_FG: Color32 = Color32::from_rgb(214, 214, 214);
const DEFAULT_BG: Color32 = Color32::from_rgb(20, 20, 24);
const SELECTION_BG: Color32 = Color32::from_rgb(58, 90, 140);
/// Lines kept above the screen. Kept modest so memory and redraws stay light.
const SCROLLBACK: usize = 1000;
/// How often the window redraws while an agent is running, so output appears without input.
const REPAINT_EVERY: Duration = Duration::from_millis(33);
/// Two Escape presses within this window count as a double press.
const DOUBLE_ESCAPE_WINDOW: Duration = Duration::from_millis(500);
/// Ctrl+U: deletes the input line. Sent on a double Escape to clear the agent's input.
const CLEAR_INPUT: &[u8] = b"\x15";

/// Bracketed paste markers. An agent that turns bracketed paste on (Claude Code does)
/// receives a paste as one block and can fold a large paste into a placeholder.
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// A (row, column) position on the visible screen.
type Pos = (u16, u16);
/// Selection endpoints: where the drag started and where it is now.
type Selection = (Pos, Pos);

/// The 16 ANSI colors, matching common dark terminal palettes.
const ANSI: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (205, 49, 49),
    (13, 188, 121),
    (229, 229, 16),
    (36, 114, 200),
    (188, 63, 188),
    (17, 168, 205),
    (229, 229, 229),
    (102, 102, 102),
    (241, 76, 76),
    (35, 209, 139),
    (245, 245, 67),
    (59, 142, 234),
    (214, 112, 214),
    (41, 184, 219),
    (255, 255, 255),
];

pub struct EmbeddedTerminal {
    screen: Arc<Mutex<vt100::Parser>>,
    /// Bytes queued for the agent. A writer thread drains this so large pastes never block the UI.
    input: Sender<Vec<u8>>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    exited: Arc<AtomicBool>,
    rows: u16,
    cols: u16,
    selection: Option<Selection>,
    /// Lines scrolled back from the live bottom. 0 means the view follows the agent.
    scroll: usize,
    /// Fractional wheel movement not yet worth a whole line.
    scroll_acc: f32,
    /// When the last Escape arrived, for detecting a double press.
    last_escape: Option<Instant>,
}

impl EmbeddedTerminal {
    /// Starts `program` in a pseudo-terminal of the given size, inside `cwd`.
    pub fn spawn(
        program: &str,
        args: &[String],
        cwd: &str,
        rows: u16,
        cols: u16,
    ) -> Result<Self, String> {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| e.to_string())?;

        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.cwd(cwd);
        if let Some(path) = path_env::agent_path() {
            cmd.env("PATH", path);
        }
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("could not start {program}: {e}"))?;

        // Dropping the slave end lets the reader see EOF once the agent exits.
        let PtyPair { master, slave } = pair;
        drop(slave);

        let mut reader = master.try_clone_reader().map_err(|e| e.to_string())?;
        let mut writer = master.take_writer().map_err(|e| e.to_string())?;

        let (input, queued) = mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            for chunk in queued {
                if writer.write_all(&chunk).and_then(|_| writer.flush()).is_err() {
                    break;
                }
            }
        });

        let screen = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, SCROLLBACK)));
        let exited = Arc::new(AtomicBool::new(false));
        let (reader_screen, reader_exited) = (Arc::clone(&screen), Arc::clone(&exited));
        thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut parser) = reader_screen.lock() {
                            parser.process(&buf[..n]);
                        }
                    }
                }
            }
            reader_exited.store(true, Ordering::SeqCst);
        });

        Ok(Self {
            screen,
            input,
            master,
            child,
            exited,
            rows,
            cols,
            selection: None,
            scroll: 0,
            scroll_acc: 0.0,
            last_escape: None,
        })
    }

    pub fn exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    fn send(&self, bytes: Vec<u8>) {
        let _ = self.input.send(bytes);
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        if rows == 0 || cols == 0 || (rows, cols) == (self.rows, self.cols) {
            return;
        }
        self.rows = rows;
        self.cols = cols;
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut parser) = self.screen.lock() {
            parser.set_size(rows, cols);
        }
    }

    fn with_screen<T>(&self, f: impl FnOnce(&vt100::Screen) -> T) -> Option<T> {
        let parser = self.screen.lock().ok()?;
        Some(f(parser.screen()))
    }

    /// Moves the view to `lines` above the bottom. The parser clamps it to the history it has.
    fn set_scroll(&mut self, lines: usize) {
        let before = self.scroll;
        if let Ok(mut parser) = self.screen.lock() {
            parser.set_scrollback(lines);
            self.scroll = parser.screen().scrollback();
        }
        if self.scroll != before {
            // The text under a selection changed when the view moved, so the selection is stale.
            self.selection = None;
        }
    }

    /// Fills the available area, sizes the pseudo-terminal to match, and handles mouse and keyboard.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        if !self.exited() {
            ui.ctx().request_repaint_after(REPAINT_EVERY);
        }

        let font = egui::FontId::monospace(14.0);
        let (cell_w, cell_h) = ui.fonts(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let avail = ui.available_size();
        let cols = ((avail.x / cell_w).floor() as u16).max(20);
        let rows = ((avail.y / cell_h).floor() as u16).max(5);
        self.resize(rows, cols);

        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(cols as f32 * cell_w, rows as f32 * cell_h),
            egui::Sense::click_and_drag(),
        );

        // Mouse wheel scrolls the history. Wheel movement under one line is carried over.
        if response.hovered() {
            let dy = ui.input(|i| i.raw_scroll_delta.y);
            if dy != 0.0 {
                self.scroll_acc += dy / cell_h;
                let lines = self.scroll_acc.trunc();
                if lines != 0.0 {
                    self.scroll_acc -= lines;
                    // Positive wheel movement is scrolling up, into older output.
                    let target = (self.scroll as f32 + lines).max(0.0) as usize;
                    self.set_scroll(target);
                }
            }
        }

        // Left-drag selects text. A plain click clears the selection.
        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                let p = self.cell_at(pos, rect, cell_w, cell_h);
                self.selection = Some((p, p));
            }
        } else if response.dragged() {
            if let (Some(pos), Some((anchor, _))) = (response.interact_pointer_pos(), self.selection) {
                let p = self.cell_at(pos, rect, cell_w, cell_h);
                self.selection = Some((anchor, p));
            }
        }
        if response.clicked() {
            self.selection = None;
            response.request_focus();
        }

        // Right-click copies the selection if there is one, otherwise pastes the clipboard.
        if response.secondary_clicked() {
            let copied = self
                .selection
                .take()
                .map(|sel| self.selected_text(sel))
                .filter(|t| !t.is_empty());
            match copied {
                Some(text) => set_clipboard(&text),
                None => {
                    self.set_scroll(0);
                    self.paste_from_clipboard();
                }
            }
        }

        // Give the terminal focus whenever no other widget holds it, so typing goes to the agent.
        if ui.memory(|m| m.focused().is_none()) {
            response.request_focus();
        }
        let focused = response.has_focus();
        if focused {
            // Keep Tab and arrow keys for the agent instead of egui focus navigation.
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            // Typing or pasting jumps back to the live bottom, as in a normal terminal.
            let typed = ui.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Text(_)
                            | egui::Event::Paste(_)
                            | egui::Event::Key { pressed: true, .. }
                    )
                })
            });
            if typed {
                self.set_scroll(0);
            }
            self.handle_input(ui);
        }

        let selection = self.ordered_selection();
        self.paint(ui, rect, &font, cell_w, cell_h, focused, selection);
    }

    /// The selection with its start before its end, in reading order.
    fn ordered_selection(&self) -> Option<Selection> {
        self.selection.map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
    }

    /// Maps a pointer position to the screen cell under it, clamped to the screen.
    fn cell_at(&self, pos: egui::Pos2, rect: egui::Rect, cell_w: f32, cell_h: f32) -> Pos {
        let col = ((pos.x - rect.min.x) / cell_w)
            .floor()
            .clamp(0.0, (self.cols - 1) as f32) as u16;
        let row = ((pos.y - rect.min.y) / cell_h)
            .floor()
            .clamp(0.0, (self.rows - 1) as f32) as u16;
        (row, col)
    }

    /// The text inside a selection. Each line has its trailing spaces removed.
    fn selected_text(&self, (start, end): Selection) -> String {
        self.with_screen(|screen| {
            let cols = screen.size().1;
            let mut out = String::new();
            for row in start.0..=end.0 {
                let from = if row == start.0 { start.1 } else { 0 };
                let to = if row == end.0 { end.1 } else { cols.saturating_sub(1) };
                let mut line = String::new();
                for col in from..=to {
                    let Some(cell) = screen.cell(row, col) else {
                        continue;
                    };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let contents = cell.contents();
                    if contents.is_empty() {
                        line.push(' ');
                    } else {
                        line.push_str(&contents);
                    }
                }
                out.push_str(line.trim_end());
                if row != end.0 {
                    out.push('\n');
                }
            }
            out
        })
        .unwrap_or_default()
    }

    fn paste_from_clipboard(&self) {
        let text = arboard::Clipboard::new()
            .and_then(|mut clip| clip.get_text())
            .unwrap_or_default();
        self.paste_text(&text);
    }

    /// Sends pasted text the way a terminal does. If the agent has bracketed paste on,
    /// the text is wrapped in markers so the agent treats it as one paste, not typed keys.
    fn paste_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let text = text.replace("\r\n", "\n");
        let bracketed = self.with_screen(|s| s.bracketed_paste()).unwrap_or(false);
        let bytes = if bracketed {
            // Strip any end marker inside the paste so it cannot end the paste early.
            let body = text.replace("\x1b[201~", "");
            let mut bytes = Vec::with_capacity(body.len() + PASTE_START.len() + PASTE_END.len());
            bytes.extend_from_slice(PASTE_START);
            bytes.extend_from_slice(body.as_bytes());
            bytes.extend_from_slice(PASTE_END);
            bytes
        } else {
            // Without bracketed paste, a newline is Enter, which terminals send as carriage return.
            text.replace('\n', "\r").into_bytes()
        };
        self.send(bytes);
    }

    /// The first Escape goes to the agent, since it may interrupt a turn. A second Escape
    /// within the window sends Ctrl+U instead, clearing the input line.
    fn escape(&mut self) {
        let now = Instant::now();
        let double = self
            .last_escape
            .is_some_and(|t| now.duration_since(t) <= DOUBLE_ESCAPE_WINDOW);
        if double {
            self.last_escape = None;
            self.send(CLEAR_INPUT.to_vec());
        } else {
            self.last_escape = Some(now);
            self.send(b"\x1b".to_vec());
        }
    }

    fn handle_input(&mut self, ui: &egui::Ui) {
        let (events, mods) = ui.input(|i| (i.events.clone(), i.modifiers));
        // Ctrl/Cmd chords arrive as key events; plain text arrives as Text events.
        let chord = (mods.ctrl || mods.command) && !mods.alt;
        for event in events {
            match event {
                egui::Event::Text(text) if !chord => self.send(text.into_bytes()),
                // Ctrl+V arrives as a Paste event with the clipboard text.
                egui::Event::Paste(text) => self.paste_text(&text),
                egui::Event::Key {
                    key: egui::Key::Escape,
                    pressed: true,
                    ..
                } => self.escape(),
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if let Some(bytes) = key_bytes(key, modifiers) {
                        self.send(bytes);
                    }
                }
                _ => {}
            }
        }
    }

    fn paint(
        &self,
        ui: &egui::Ui,
        rect: egui::Rect,
        font: &egui::FontId,
        cell_w: f32,
        cell_h: f32,
        focused: bool,
        selection: Option<Selection>,
    ) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, DEFAULT_BG);
        self.with_screen(|screen| {
            let (rows, cols) = screen.size();
            for row in 0..rows {
                // One galley per row keeps the number of draw calls low.
                let mut job = egui::text::LayoutJob::default();
                for col in 0..cols {
                    let Some(cell) = screen.cell(row, col) else {
                        continue;
                    };
                    if cell.is_wide_continuation() {
                        continue;
                    }
                    let mut fg = to_color(cell.fgcolor(), DEFAULT_FG);
                    let mut bg = to_color(cell.bgcolor(), DEFAULT_BG);
                    if cell.inverse() {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    let selected = selection.is_some_and(|sel| in_selection(row, col, sel));
                    let background = if selected {
                        SELECTION_BG
                    } else if bg == DEFAULT_BG {
                        Color32::TRANSPARENT
                    } else {
                        bg
                    };
                    let contents = cell.contents();
                    let text = if contents.is_empty() {
                        " ".to_string()
                    } else {
                        contents
                    };
                    job.append(
                        &text,
                        0.0,
                        egui::TextFormat {
                            font_id: font.clone(),
                            color: fg,
                            background,
                            italics: cell.italic(),
                            underline: if cell.underline() {
                                egui::Stroke::new(1.0_f32, fg)
                            } else {
                                egui::Stroke::NONE
                            },
                            ..Default::default()
                        },
                    );
                }
                let galley = ui.fonts(|f| f.layout_job(job));
                painter.galley(
                    egui::pos2(rect.min.x, rect.min.y + row as f32 * cell_h),
                    galley,
                    DEFAULT_FG,
                );
            }

            if focused && !screen.hide_cursor() && self.scroll == 0 {
                let (crow, ccol) = screen.cursor_position();
                let min = egui::pos2(
                    rect.min.x + ccol as f32 * cell_w,
                    rect.min.y + crow as f32 * cell_h,
                );
                painter.rect_filled(
                    egui::Rect::from_min_size(min, egui::vec2(cell_w, cell_h)),
                    0.0,
                    Color32::from_rgba_unmultiplied(220, 220, 220, 120),
                );
            }
        });

        // Shows that the view is not at the live bottom.
        if self.scroll > 0 {
            painter.text(
                egui::pos2(rect.max.x - 8.0, rect.min.y + 4.0),
                egui::Align2::RIGHT_TOP,
                format!("scrolled back {} lines · type to return", self.scroll),
                egui::FontId::proportional(12.0),
                Color32::from_rgb(240, 200, 90),
            );
        }
    }
}

impl Drop for EmbeddedTerminal {
    /// Closing a tab ends its agent process.
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Whether a cell lies between the two ends of a selection, in reading order.
fn in_selection(row: u16, col: u16, (start, end): Selection) -> bool {
    (row, col) >= start && (row, col) <= end
}

fn set_clipboard(text: &str) {
    if let Ok(mut clip) = arboard::Clipboard::new() {
        let _ = clip.set_text(text.to_string());
    }
}

/// Escape sequence for a non-text key, or the control byte for Ctrl+letter.
fn key_bytes(key: egui::Key, mods: egui::Modifiers) -> Option<Vec<u8>> {
    if mods.ctrl && !mods.alt {
        // Ctrl+V arrives as Event::Paste, so it is not sent as a control byte.
        if key == egui::Key::V {
            return None;
        }
        let name = key.name().as_bytes();
        if name.len() == 1 && name[0].is_ascii_uppercase() {
            return Some(vec![name[0] - b'A' + 1]);
        }
        return None;
    }
    let seq: &[u8] = match key {
        egui::Key::Enter => b"\r",
        egui::Key::Backspace => b"\x7f",
        egui::Key::Tab => b"\t",
        egui::Key::Escape => b"\x1b",
        egui::Key::ArrowUp => b"\x1b[A",
        egui::Key::ArrowDown => b"\x1b[B",
        egui::Key::ArrowRight => b"\x1b[C",
        egui::Key::ArrowLeft => b"\x1b[D",
        egui::Key::Home => b"\x1b[H",
        egui::Key::End => b"\x1b[F",
        egui::Key::Delete => b"\x1b[3~",
        egui::Key::PageUp => b"\x1b[5~",
        egui::Key::PageDown => b"\x1b[6~",
        _ => return None,
    };
    Some(seq.to_vec())
}

fn to_color(color: vt100::Color, default: Color32) -> Color32 {
    match color {
        vt100::Color::Default => default,
        vt100::Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        vt100::Color::Idx(i) => indexed(i),
    }
}

/// xterm 256-color palette: 16 ANSI colors, a 6x6x6 cube, then a gray ramp.
fn indexed(i: u8) -> Color32 {
    match i {
        0..=15 => {
            let (r, g, b) = ANSI[i as usize];
            Color32::from_rgb(r, g, b)
        }
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(n / 36), level((n / 6) % 6), level(n % 6))
        }
        232..=255 => {
            let g = 8 + (i - 232) * 10;
            Color32::from_rgb(g, g, g)
        }
    }
}
