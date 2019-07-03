use std::cmp::max;
use std::collections::HashMap;
use std::io::Write;
use futures::future::Future;

use failure::Error;
use termion::clear::CurrentLine as ClearLine;
use termion::cursor::Goto;
use termion::event::{MouseButton, MouseEvent};
use xrl::{ConfigChanges, Line, LineCache, Style, Update};
use serde_json::Value;

use crate::core::Command;

use super::cfg::ViewConfig;
use super::client::Client;
use super::style::{reset_style, set_style};
use super::window::Window;

#[derive(Debug, Default)]
pub struct Cursor {
    pub line: u64,
    pub column: u64,
}

pub struct View {
    cache: LineCache,
    cursor: Cursor,
    window: Window,
    file: Option<String>,
    client: Client,
    cfg: ViewConfig,

    search_in_progress: bool,
}

impl View {
    pub fn new(client: Client, file: Option<String>) -> View {
        View {
            cache: LineCache::default(),
            cursor: Default::default(),
            window: Window::new(),
            cfg: ViewConfig::default(),
            client,
            file,
            search_in_progress: false,
        }
    }

    pub fn update_cache(&mut self, update: Update) {
        info!("updating cache");
        self.cache.update(update)
    }

    pub fn set_cursor(&mut self, line: u64, column: u64) {
        self.cursor = Cursor { line, column };
        self.window.set_cursor(&self.cursor);
    }

    pub fn config_changed(&mut self, changes: ConfigChanges) {
        if let Some(tab_size) = changes.tab_size {
            self.cfg.tab_size = tab_size as u16;
        }
    }

    pub fn render<W: Write>(
        &mut self,
        w: &mut W,
        styles: &HashMap<u64, Style>,
    ) -> Result<(), Error> {
        self.update_window();
        self.render_lines(w, styles)?;
        self.render_cursor(w);
        Ok(())
    }

    pub fn resize(&mut self, height: u16) {
        self.window.resize(height);
        self.update_window();
        let top = self.cache.before() + self.window.start();
        let bottom = self.cache.after() + self.window.end();
        self.client.scroll(top, bottom);
    }

    pub fn save(&mut self) {
        self.client.save(self.file.as_ref().unwrap())
    }

    pub fn toggle_line_numbers(&mut self) {
        self.cfg.display_gutter = !self.cfg.display_gutter;
    }

    fn update_window(&mut self) {
        if self.cursor.line < self.cache.before() {
            error!(
                "cursor is on line {} but there are {} invalid lines in cache.",
                self.cursor.line,
                self.cache.before()
            );
            return;
        }
        let cursor_line = self.cursor.line - self.cache.before();
        let nb_lines = self.cache.lines().len() as u64;
        let gutter_size = (self.cache.before() + nb_lines + self.cache.after())
            .to_string()
            .len() as u16;
        let gutter_size = gutter_size + 1; // Space between line number and content
        self.cfg.gutter_size = max(gutter_size, 4); //  min gutter width 4
        self.window.update(cursor_line, nb_lines);
    }

    fn get_click_location(&self, x: u64, y: u64) -> (u64, u64) {
        let lineno = x + self.cache.before() + self.window.start();
        if let Some(line) = self.cache.lines().get(x as usize) {
            if y < u64::from(self.cfg.gutter_size) {
                return (lineno, 0);
            }
            let mut text_len: u16 = 0;
            for (idx, c) in line.text.chars().enumerate() {
                let char_width = self.translate_char_width(text_len, c);
                text_len += char_width;
                if u64::from(text_len) >= y {
                    // If the character at idx is wider than one column,
                    // the click occurred within the character. Otherwise,
                    // the click occurred on the character at idx + 1
                    if char_width > 1 {
                        return (lineno as u64, (idx - self.cfg.gutter_size as usize) as u64);
                    } else {
                        return (
                            lineno as u64,
                            (idx - self.cfg.gutter_size as usize) as u64 + 1,
                        );
                    }
                }
            }
            return (lineno, line.text.len() as u64 + 1);
        } else {
            warn!("no line at index {} found in cache", x);
            return (x, y);
        }
    }

    fn click(&mut self, x: u64, y: u64) {
        let (line, column) = self.get_click_location(x, y);
        self.client.click(line, column);
    }

    fn click_cursor_extend(&mut self, x: u64, y: u64) {
        let (line, column) = self.get_click_location(x, y);
        self.client.click_cursor_extend(line, column);
    }

    fn drag(&mut self, x: u64, y: u64) {
        let (line, column) = self.get_click_location(x, y);
        self.client.drag(line, column);
    }

    fn find_under_expand(&mut self) {
        if self.search_in_progress {
            self.client.find_under_expand_next()
        } else {
            self.search_in_progress = true;
            self.client.find_under_expand()
        }
    }

    pub fn paste(&mut self, text: &str) {
        self.client.paste(text)
    }

    pub fn copy(&mut self) -> impl Future<Item = Value, Error = xrl::ClientError> {
        self.client.copy()
    }

    pub fn cut(&mut self) -> impl Future<Item = Value, Error = xrl::ClientError> {
        self.client.cut()
    }

    pub fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::ToggleLineNumbers => self.toggle_line_numbers(),
            Command::FindUnderExpand => self.find_under_expand(),
            Command::Cancel => { self.search_in_progress = false; self.client.collapse_selections() },
            client_command => self.client.handle_command(client_command),
        }
    }

    pub fn handle_mouse_event(&mut self, mouse_event: MouseEvent) {
        match mouse_event {
            MouseEvent::Press(press_event, y, x) => match press_event {
                MouseButton::Left => self.click(u64::from(x) - 1, u64::from(y) - 1),
                MouseButton::Middle => self.click_cursor_extend(u64::from(x) - 1, u64::from(y) - 1),
                MouseButton::WheelUp => self.client.up(false),
                MouseButton::WheelDown => self.client.down(false),
                button => error!("un-handled button {:?}", button),
            },
            MouseEvent::Release(..) => {}
            MouseEvent::Hold(y, x) => self.drag(u64::from(x) - 1, u64::from(y) - 1),
        }
    }

    fn render_lines<W: Write>(&self, w: &mut W, styles: &HashMap<u64, Style>) -> Result<(), Error> {
        debug!("rendering lines");
        trace!("current cache\n{:?}", self.cache);

        // Get the lines that are within the displayed window
        let lines = self
            .cache
            .lines()
            .iter()
            .skip(self.window.start() as usize)
            .take(self.window.size() as usize);

        // Draw the valid lines within this range
        let mut line_strings = String::new();
        let mut line_no = self.cache.before() + self.window.start();
        for (line_index, line) in lines.enumerate() {
            line_strings.push_str(&self.render_line_str(line, Some(line_no), line_index, styles));
            line_no += 1;
        }

        // If the number of lines is less than window height
        // render empty lines to fill the view window.
        let line_count = self.cache.lines().len() as u16;
        let win_size = self.window.size();
        if win_size > line_count {
            for num in line_count..win_size {
                line_strings.push_str(&self.render_line_str(
                    &Line::default(),
                    None,
                    num as usize,
                    styles,
                ));
            }
        }
        w.write_all(line_strings.as_bytes())?;

        Ok(())
    }

    // Next tab stop, assuming 0-based indexing
    fn tab_width_at_position(&self, position: u16) -> u16 {
        self.cfg.tab_size - (position % self.cfg.tab_size)
    }

    fn render_line_str(
        &self,
        line: &Line,
        lineno: Option<u64>,
        line_index: usize,
        styles: &HashMap<u64, Style>,
    ) -> String {
        let text = self.escape_control_and_add_styles(styles, line);
        if let Some(line_no) = lineno {
            if self.cfg.display_gutter {
                let line_no = (line_no + 1).to_string();
                let line_no_offset = self.cfg.gutter_size - line_no.len() as u16;
                format!(
                    "{}{}{}{}{}",
                    Goto(line_no_offset, line_index as u16 + 1),
                    ClearLine,
                    line_no,
                    Goto(self.cfg.gutter_size + 1, line_index as u16 + 1),
                    &text
                )
            } else {
                format!("{}{}{}", Goto(0, line_index as u16 + 1), ClearLine, &text)
            }
        } else {
            format!(
                "{}{}{}",
                Goto(self.cfg.gutter_size + 1, line_index as u16 + 1),
                ClearLine,
                &text
            )
        }
    }

    fn escape_control_and_add_styles(&self, styles: &HashMap<u64, Style>, line: &Line) -> String {
        let mut position: u16 = 0;
        let mut text = String::with_capacity(line.text.capacity());
        for c in line.text.chars() {
            match c {
                '\x00'..='\x08' | '\x0a'..='\x1f' | '\x7f' => {
                    // Render in caret notation, i.e. '\x02' is rendered as '^B'
                    text.push('^');
                    text.push((c as u8 ^ 0x40u8) as char);
                    position += 2;
                }
                '\t' => {
                    let tab_width = self.tab_width_at_position(position);
                    text.push_str(&" ".repeat(tab_width as usize));
                    position += tab_width;
                }
                _ => {
                    text.push(c);
                    position += 1;
                }
            }
        }
        if line.styles.is_empty() {
            return text;
        }
        let mut style_sequences = self.get_style_sequences(styles, line);
        for style in style_sequences.drain(..) {
            trace!("inserting style: {:?}", style);
            if style.0 >= text.len() {
                text.push_str(&style.1);
            } else {
                text.insert_str(style.0, &style.1);
            }
        }
        trace!("styled line: {:?}", text);
        text
    }

    fn get_style_sequences(
        &self,
        styles: &HashMap<u64, Style>,
        line: &Line,
    ) -> Vec<(usize, String)> {
        let mut style_sequences: Vec<(usize, String)> = Vec::new();
        let mut prev_style_end: usize = 0;
        for style_def in &line.styles {
            let start_idx = if style_def.offset >= 0 {
                (prev_style_end + style_def.offset as usize)
            } else {
                // FIXME: does that actually work?
                (prev_style_end - ((-style_def.offset) as usize))
            };
            let end_idx = start_idx + style_def.length as usize;
            prev_style_end = end_idx;

            if let Some(style) = styles.get(&style_def.style_id) {
                let start_sequence = match set_style(style) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("could not get CSI sequence to set style {:?}: {}", style, e);
                        continue;
                    }
                };
                let end_sequence = match reset_style(style) {
                    Ok(s) => s,
                    Err(e) => {
                        error!(
                            "could not get CSI sequence to reset style {:?}: {}",
                            style, e
                        );
                        continue;
                    }
                };
                style_sequences.push((start_idx, start_sequence));
                style_sequences.push((end_idx, end_sequence));
            } else {
                error!(
                    "no style ID {} found. Not applying style.",
                    style_def.style_id
                );
            };
        }
        // Note that we sort the vector in *reverse* order, so that we apply style starting from
        // the end of the line, and we don't have to worry about the indices changing.
        style_sequences.sort_by(|a, b| a.0.cmp(&b.0));
        style_sequences.reverse();
        trace!("{:?}", style_sequences);
        style_sequences
    }

    fn render_cursor<W: Write>(&self, w: &mut W) {
        info!("rendering cursor");
        if self.cache.is_empty() {
            info!("cache is empty, rendering cursor at the top left corner");
            if let Err(e) = write!(w, "{}", Goto(1, 1)) {
                error!("failed to render cursor: {}", e);
            }
            return;
        }

        if self.cursor.line < self.cache.before() {
            error!(
                "the cursor is on line {} which is marked invalid in the cache",
                self.cursor.line
            );
            return;
        }
        // Get the line that has the cursor
        let line_idx = self.cursor.line - self.cache.before();
        let line = match self.cache.lines().get(line_idx as usize) {
            Some(line) => line,
            None => {
                error!("no valid line at cursor index {}", self.cursor.line);
                return;
            }
        };

        if line_idx < self.window.start() {
            error!(
                "the line that has the cursor (nb={}, cache_idx={}) not within the displayed window ({:?})",
                self.cursor.line,
                line_idx,
                self.window
            );
            return;
        }
        // Get the line vertical offset so that we know where to draw it.
        let line_pos = line_idx - self.window.start();

        // Calculate the cursor position on the line. The trick is that we know the position within
        // the string, but characters may have various lengths. For the moment, we only handle
        // control characters and tabs. We assume control characters (0x00-0x1f, excluding 0x09 ==
        // tab) are rendered in caret notation and are thus two columns wide. Tabs are
        // variable-width, rounding up to the next tab stop. All other characters are assumed to be
        // one column wide.
        let column: u16 = line
            .text
            .chars()
            .take(self.cursor.column as usize)
            .fold(0, |acc, c| acc + self.translate_char_width(acc, c));

        // Draw the cursor
        let cursor_pos = Goto(self.cfg.gutter_size + column + 1, line_pos as u16 + 1);
        if let Err(e) = write!(w, "{}", cursor_pos) {
            error!("failed to render cursor: {}", e);
        }
        info!("Cursor rendered at ({}, {})", line_pos, column);
    }

    fn translate_char_width(&self, position: u16, c: char) -> u16 {
        match c {
            // Caret notation means non-tab control characters are two columns wide
            '\x00'..='\x08' | '\x0a'..='\x1f' | '\x7f' => 2,
            '\t' => self.tab_width_at_position(position),
            _ => 1,
        }
    }
}
