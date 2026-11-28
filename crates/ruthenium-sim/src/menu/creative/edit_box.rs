//! vanilla's `EditBox` as the creative inventory's search uses it: the text in UTF-16
//! units as Java holds it, the caret and the other end of the selection, typing
//! (`charTyped`, `insertText` with `StringUtil.filterText` and the length limit),
//! deleting by character or word, moving by code point or word, Home and End, select
//! all, copy, cut and paste. The keys come as SDL scancodes, as the chat's edit box reads
//! them (so the shortcuts sit where Ctrl+A, C, V and X are on a US layout).

use std::time::Instant;

/// SDL scancodes the edit box reads.
pub const SCANCODE_A: u32 = 4;
pub const SCANCODE_C: u32 = 6;
pub const SCANCODE_V: u32 = 25;
pub const SCANCODE_X: u32 = 27;
pub const SCANCODE_BACKSPACE: u32 = 42;
pub const SCANCODE_HOME: u32 = 74;
pub const SCANCODE_DELETE: u32 = 76;
pub const SCANCODE_END: u32 = 77;
pub const SCANCODE_RIGHT: u32 = 79;
pub const SCANCODE_LEFT: u32 = 80;

/// A key as the edit box reads it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeyPress {
    pub scancode: u32,
    pub shift: bool,
    /// The shortcut modifier (`hasControlDownWithQuirk`: Ctrl, Cmd on macOS).
    pub shortcut: bool,
    /// What the clipboard held, for a paste.
    pub clipboard: Option<String>,
}

/// See the module docs.
#[derive(Debug, Clone)]
pub struct EditBox {
    value: Vec<u16>,
    cursor: usize,
    highlight: usize,
    max_length: usize,
    pub visible: bool,
    focused: bool,
    can_lose_focus: bool,
    /// When it took the focus (`focusedTime`: the caret blinks from then).
    pub focused_at: Instant,
}

impl EditBox {
    pub fn new(max_length: usize) -> EditBox {
        EditBox {
            value: Vec::new(),
            cursor: 0,
            highlight: 0,
            max_length,
            visible: true,
            focused: false,
            can_lose_focus: true,
            focused_at: Instant::now(),
        }
    }

    pub fn value(&self) -> String {
        String::from_utf16_lossy(&self.value)
    }

    pub fn units(&self) -> &[u16] {
        &self.value
    }

    /// The caret, in UTF-16 units.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The other end of the selection, in UTF-16 units.
    pub fn highlight(&self) -> usize {
        self.highlight
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn set_can_lose_focus(&mut self, can: bool) {
        self.can_lose_focus = can;
    }

    /// `setFocused`: it can't let go of the focus while `canLoseFocus` is off.
    pub fn set_focused(&mut self, focused: bool) {
        if self.can_lose_focus || focused {
            self.focused = focused;
            if focused {
                self.focused_at = Instant::now();
            }
        }
    }

    /// `canConsumeInput` (`capturesInput`): shown and focused.
    pub fn captures_input(&self) -> bool {
        self.visible && self.focused
    }

    /// `setValue`: cut to the length limit, the caret at the end.
    pub fn set_value(&mut self, value: &str) {
        let mut units: Vec<u16> = value.encode_utf16().collect();
        units.truncate(self.max_length);
        self.value = units;
        self.move_cursor_to_end(false);
        self.highlight = self.cursor;
    }

    /// `getHighlighted`.
    pub fn highlighted(&self) -> String {
        let (s, e) = (
            self.cursor.min(self.highlight),
            self.cursor.max(self.highlight),
        );
        String::from_utf16_lossy(&self.value[s..e])
    }

    /// `setHighlightPos` (clamped as vanilla's).
    pub fn set_highlight_pos(&mut self, pos: usize) {
        self.highlight = pos.min(self.value.len());
    }

    fn set_cursor_position(&mut self, pos: isize) {
        self.cursor = pos.clamp(0, self.value.len() as isize) as usize;
    }

    /// `moveCursorTo`.
    pub fn move_cursor_to(&mut self, pos: isize, extend: bool) {
        self.set_cursor_position(pos);
        if !extend {
            self.highlight = self.cursor;
        }
    }

    pub fn move_cursor_to_end(&mut self, extend: bool) {
        self.move_cursor_to(self.value.len() as isize, extend);
    }

    /// `Util.offsetByCodepoints` from the caret.
    fn cursor_pos(&self, dir: i32) -> isize {
        let mut pos = self.cursor;
        let is_low = |u: u16| (0xDC00..0xE000).contains(&u);
        let is_high = |u: u16| (0xD800..0xDC00).contains(&u);
        if dir >= 0 {
            for _ in 0..dir {
                if pos >= self.value.len() {
                    break;
                }
                pos += 1;
                if pos < self.value.len() && is_high(self.value[pos - 1]) && is_low(self.value[pos])
                {
                    pos += 1;
                }
            }
        } else {
            for _ in 0..-dir {
                if pos == 0 {
                    break;
                }
                pos -= 1;
                if pos > 0 && is_low(self.value[pos]) && is_high(self.value[pos - 1]) {
                    pos -= 1;
                }
            }
        }
        pos as isize
    }

    /// `getWordPosition(dir)` from the caret, spaces skipped.
    fn word_position(&self, dir: i32) -> isize {
        let space = u16::from(b' ');
        let mut result = self.cursor;
        let len = self.value.len();
        for _ in 0..dir.unsigned_abs() {
            if dir >= 0 {
                match self.value[result.min(len)..]
                    .iter()
                    .position(|&c| c == space)
                {
                    None => result = len,
                    Some(i) => {
                        result += i;
                        while result < len && self.value[result] == space {
                            result += 1;
                        }
                    }
                }
            } else {
                while result > 0 && self.value[result - 1] == space {
                    result -= 1;
                }
                while result > 0 && self.value[result - 1] != space {
                    result -= 1;
                }
            }
        }
        result as isize
    }

    /// `insertText`: what the chat allows of `input` takes the selection's place, as
    /// much as the limit leaves room for (a pair cut in half loses its first half).
    pub fn insert_text(&mut self, input: &str) {
        let start = self.cursor.min(self.highlight);
        let end = self.cursor.max(self.highlight);
        let room =
            self.max_length as isize - self.value.len() as isize - (start as isize - end as isize);
        if room <= 0 {
            return;
        }
        let mut text: Vec<u16> = input
            .encode_utf16()
            .filter(|&u| u != 0xA7 && u >= 0x20 && u != 0x7F)
            .collect();
        let mut room = room as usize;
        if room < text.len() {
            if (0xD800..0xDC00).contains(&text[room - 1]) {
                room -= 1;
            }
            text.truncate(room);
        }
        let n = text.len();
        self.value.splice(start..end, text);
        self.set_cursor_position((start + n) as isize);
        self.highlight = self.cursor;
    }

    fn delete_chars_to_pos(&mut self, pos: isize) {
        if self.value.is_empty() {
            return;
        }
        if self.highlight != self.cursor {
            self.insert_text("");
            return;
        }
        let pos = pos.clamp(0, self.value.len() as isize) as usize;
        let (start, end) = (pos.min(self.cursor), pos.max(self.cursor));
        if start != end {
            self.value.drain(start..end);
            self.set_cursor_position(start as isize);
            self.move_cursor_to(start as isize, false);
        }
    }

    /// `deleteText`: a character (a code point) or a word, before or after the caret.
    fn delete_text(&mut self, dir: i32, word: bool) {
        if word {
            if self.value.is_empty() {
                return;
            }
            if self.highlight != self.cursor {
                self.insert_text("");
            } else {
                self.delete_chars_to_pos(self.word_position(dir));
            }
        } else {
            self.delete_chars_to_pos(self.cursor_pos(dir));
        }
    }

    /// `keyPressed` while shown and focused: whether it took the key, and what goes to
    /// the clipboard (a copy or a cut).
    pub fn key_pressed(&mut self, k: &KeyPress) -> (bool, Option<String>) {
        if !self.captures_input() {
            return (false, None);
        }
        match k.scancode {
            SCANCODE_BACKSPACE => self.delete_text(-1, k.shortcut),
            SCANCODE_DELETE => self.delete_text(1, k.shortcut),
            SCANCODE_HOME => self.move_cursor_to(0, k.shift),
            SCANCODE_END => self.move_cursor_to_end(k.shift),
            SCANCODE_RIGHT => {
                let to = if k.shortcut {
                    self.word_position(1)
                } else {
                    self.cursor_pos(1)
                };
                self.move_cursor_to(to, k.shift);
            }
            SCANCODE_LEFT => {
                let to = if k.shortcut {
                    self.word_position(-1)
                } else {
                    self.cursor_pos(-1)
                };
                self.move_cursor_to(to, k.shift);
            }
            SCANCODE_A if k.shortcut && !k.shift => {
                self.move_cursor_to_end(false);
                self.highlight = 0;
            }
            SCANCODE_C if k.shortcut && !k.shift => return (true, Some(self.highlighted())),
            SCANCODE_V if k.shortcut && !k.shift => {
                if let Some(t) = &k.clipboard {
                    self.insert_text(t);
                }
            }
            SCANCODE_X if k.shortcut && !k.shift => {
                let cut = self.highlighted();
                self.insert_text("");
                return (true, Some(cut));
            }
            _ => return (false, None),
        }
        (true, None)
    }

    /// `charTyped`: a code point the chat allows is inserted.
    pub fn char_typed(&mut self, c: char) -> bool {
        if !self.captures_input() {
            return false;
        }
        if crate::chat::allowed_character(c) {
            let mut buf = [0u8; 4];
            self.insert_text(c.encode_utf8(&mut buf));
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focused(text: &str) -> EditBox {
        let mut b = EditBox::new(50);
        b.set_focused(true);
        b.set_value(text);
        b
    }

    fn key(scancode: u32, shift: bool, shortcut: bool) -> KeyPress {
        KeyPress {
            scancode,
            shift,
            shortcut,
            clipboard: None,
        }
    }

    #[test]
    fn typing_keeps_to_fifty_and_drops_what_chat_refuses() {
        let mut b = focused("");
        for c in "a§b\u{7f}c".chars() {
            b.char_typed(c);
        }
        assert_eq!(b.value(), "abc");
        b.insert_text(&"x".repeat(60));
        assert_eq!(b.units().len(), 50);
        // A pair that doesn't fit isn't cut in half.
        let mut b = focused(&"x".repeat(49));
        b.insert_text("😀");
        assert_eq!(b.units().len(), 49);
    }

    #[test]
    fn words_and_selection() {
        let mut b = focused("oak planks");
        b.key_pressed(&key(SCANCODE_BACKSPACE, false, true));
        assert_eq!(b.value(), "oak ");
        b.key_pressed(&key(SCANCODE_LEFT, true, true));
        assert_eq!(b.highlighted(), "oak ");
        b.char_typed('x');
        assert_eq!(b.value(), "x");
        b.set_value("stone");
        b.key_pressed(&key(SCANCODE_A, false, true));
        let (_, copied) = b.key_pressed(&key(SCANCODE_X, false, true));
        assert_eq!((copied.as_deref(), b.value().as_str()), (Some("stone"), ""));
    }

    #[test]
    fn it_keeps_the_focus_while_it_can_not_lose_it() {
        let mut b = EditBox::new(50);
        b.set_can_lose_focus(false);
        b.set_focused(true);
        b.set_focused(false);
        assert!(b.is_focused());
        b.set_can_lose_focus(true);
        b.set_focused(false);
        assert!(!b.is_focused());
        assert!(!b.key_pressed(&key(SCANCODE_LEFT, false, false)).0);
    }
}
