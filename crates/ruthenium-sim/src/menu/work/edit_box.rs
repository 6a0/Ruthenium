//! `EditBox` as the anvil's name box runs it: the text (Java's UTF-16 units, as vanilla
//! counts them), the caret and the selection, typing, deleting and moving, the length
//! limit, and `setValue`. What the box says to its owner when the text changes
//! (`onValueChange`'s responder) is returned to the caller.

/// SDL's keycodes vanilla's `EditBox.keyPressed` reads (`shortcutKey`): backspace,
/// delete, home, end, right and left.
pub const KEY_BACKSPACE: u32 = 8;
pub const KEY_DELETE: u32 = 127;
pub const KEY_HOME: u32 = 1_073_741_898;
pub const KEY_END: u32 = 1_073_741_901;
pub const KEY_RIGHT: u32 = 1_073_741_903;
pub const KEY_LEFT: u32 = 1_073_741_904;

/// A key an edit box reads, as the window names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKey {
    Backspace,
    Delete,
    Home,
    End,
    Right,
    Left,
    SelectAll,
    Copy,
    Cut,
    Paste,
    /// Any other key.
    Other,
}

/// See the module docs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EditBox {
    value: Vec<u16>,
    cursor: usize,
    highlight: usize,
    max_length: usize,
    pub editable: bool,
    /// Bumped whenever the caret moves or the text changes (the caret shows again).
    pub edits: u32,
}

impl EditBox {
    pub fn new(max_length: usize) -> EditBox {
        EditBox {
            max_length,
            editable: true,
            ..EditBox::default()
        }
    }

    pub fn value(&self) -> String {
        String::from_utf16_lossy(&self.value)
    }

    /// The caret and the selection's other end, in UTF-16 units.
    pub fn cursor(&self) -> (usize, usize) {
        (self.cursor, self.highlight)
    }

    /// `getHighlighted`.
    pub fn highlighted(&self) -> String {
        let (a, b) = (
            self.cursor.min(self.highlight),
            self.cursor.max(self.highlight),
        );
        String::from_utf16_lossy(&self.value[a..b])
    }

    /// `setValue`: cut to the limit, caret at the end; the responder hears the value
    /// as given (not as cut).
    pub fn set_value(&mut self, value: &str) -> String {
        let v: Vec<u16> = value.encode_utf16().collect();
        self.value = if v.len() > self.max_length {
            v[..self.max_length].to_vec()
        } else {
            v
        };
        self.move_cursor_to(self.value.len(), false);
        self.highlight = self.cursor;
        value.to_owned()
    }

    /// `insertText`: the filtered text over the selection, as far as the limit allows.
    /// The responder's value if the text changed.
    pub fn insert_text(&mut self, input: &str) -> Option<String> {
        let start = self.cursor.min(self.highlight);
        let end = self.cursor.max(self.highlight);
        let room = self.max_length as i64 - self.value.len() as i64 - (start as i64 - end as i64);
        if room <= 0 {
            return None;
        }
        let mut text: Vec<u16> = super::filter_text(input).encode_utf16().collect();
        let mut room = room as usize;
        if room < text.len() {
            if (0xD800..0xDC00).contains(&text[room - 1]) {
                room -= 1;
            }
            text.truncate(room);
        }
        let n = text.len();
        self.value.splice(start..end, text);
        self.set_cursor(start + n);
        self.highlight = self.cursor;
        self.edits += 1;
        Some(self.value())
    }

    /// `keyPressed` (the box is focused): whether it took the key, and the responder's
    /// value if the text changed.
    pub fn key(
        &mut self,
        key: EditKey,
        shift: bool,
        ctrl: bool,
        clipboard: &str,
    ) -> (bool, Option<String>) {
        match key {
            EditKey::Backspace => (
                true,
                if self.editable {
                    self.delete(-1, ctrl)
                } else {
                    None
                },
            ),
            EditKey::Delete => (
                true,
                if self.editable {
                    self.delete(1, ctrl)
                } else {
                    None
                },
            ),
            EditKey::Home => {
                self.move_cursor_to(0, shift);
                (true, None)
            }
            EditKey::End => {
                self.move_cursor_to(self.value.len(), shift);
                (true, None)
            }
            EditKey::Right | EditKey::Left => {
                let dir = if key == EditKey::Right { 1 } else { -1 };
                let to = if ctrl {
                    self.word_position(dir, self.cursor)
                } else {
                    self.offset_by_codepoints(self.cursor, dir)
                };
                self.move_cursor_to(to, shift);
                (true, None)
            }
            EditKey::SelectAll => {
                self.move_cursor_to(self.value.len(), false);
                self.highlight = 0;
                (true, None)
            }
            EditKey::Copy => (true, None),
            EditKey::Paste => (
                true,
                if self.editable {
                    self.insert_text(clipboard)
                } else {
                    None
                },
            ),
            EditKey::Cut => (
                true,
                if self.editable {
                    self.insert_text("")
                } else {
                    None
                },
            ),
            EditKey::Other => (false, None),
        }
    }

    /// `deleteText`: a character or a word before (-1) or after (1).
    fn delete(&mut self, dir: i32, words: bool) -> Option<String> {
        if self.value.is_empty() {
            return None;
        }
        if self.highlight != self.cursor {
            return self.insert_text("");
        }
        let to = if words {
            self.word_position(dir, self.cursor)
        } else {
            self.offset_by_codepoints(self.cursor, dir)
        };
        let (start, end) = (to.min(self.cursor), to.max(self.cursor));
        if start == end {
            return None;
        }
        self.value.drain(start..end);
        self.set_cursor(start);
        let v = self.value();
        self.move_cursor_to(start, false);
        self.edits += 1;
        Some(v)
    }

    /// `getWordPosition` (spaces stripped).
    fn word_position(&self, dir: i32, from: usize) -> usize {
        let space = u16::from(b' ');
        let mut r = from;
        let len = self.value.len();
        if dir > 0 {
            match self.value[r.min(len)..].iter().position(|&c| c == space) {
                None => r = len,
                Some(p) => {
                    r += p;
                    while r < len && self.value[r] == space {
                        r += 1;
                    }
                }
            }
        } else {
            while r > 0 && self.value[r - 1] == space {
                r -= 1;
            }
            while r > 0 && self.value[r - 1] != space {
                r -= 1;
            }
        }
        r
    }

    /// `Util.offsetByCodepoints`.
    fn offset_by_codepoints(&self, at: usize, dir: i32) -> usize {
        let low = |c: u16| (0xDC00..0xE000).contains(&c);
        let high = |c: u16| (0xD800..0xDC00).contains(&c);
        let mut i = at.min(self.value.len());
        if dir > 0 {
            if i < self.value.len() {
                i += 1;
                if i < self.value.len() && high(self.value[i - 1]) && low(self.value[i]) {
                    i += 1;
                }
            }
        } else if i > 0 {
            i -= 1;
            if i > 0 && low(self.value[i]) && high(self.value[i - 1]) {
                i -= 1;
            }
        }
        i
    }

    /// `moveCursorTo`.
    fn move_cursor_to(&mut self, at: usize, extend: bool) {
        self.set_cursor(at);
        if !extend {
            self.highlight = self.cursor;
        }
        self.edits += 1;
    }

    fn set_cursor(&mut self, at: usize) {
        self.cursor = at.min(self.value.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_stops_at_the_limit_and_filters() {
        let mut b = EditBox::new(5);
        assert_eq!(b.insert_text("a§b\u{7}c"), Some("abc".into()));
        assert_eq!(b.insert_text("defg"), Some("abcde".into()));
        assert_eq!(b.insert_text("x"), None);
        // A pair cut in half at the limit is dropped whole.
        let mut b = EditBox::new(2);
        b.insert_text("a");
        assert_eq!(b.insert_text("😀"), Some("a".into()));
        // `setValue` cuts the text but tells the responder what it was given.
        let mut b = EditBox::new(3);
        assert_eq!(b.set_value("abcdef"), "abcdef");
        assert_eq!(b.value(), "abc");
        assert_eq!(b.cursor(), (3, 3));
    }

    #[test]
    fn keys_move_select_and_delete() {
        let mut b = EditBox::new(50);
        b.set_value("one two  three");
        let (taken, changed) = b.key(EditKey::Backspace, false, true, "");
        assert!(taken);
        assert_eq!(changed, Some("one two  ".into()));
        b.key(EditKey::Left, false, true, "");
        assert_eq!(b.cursor().0, 4);
        b.key(EditKey::Right, true, false, "");
        assert_eq!(b.highlighted(), "t");
        assert_eq!(
            b.key(EditKey::Cut, false, false, "").1,
            Some("one wo  ".into())
        );
        b.key(EditKey::SelectAll, false, false, "");
        assert_eq!(b.highlighted(), "one wo  ");
        assert_eq!(b.key(EditKey::Paste, false, false, "z").1, Some("z".into()));
        b.editable = false;
        assert_eq!(b.key(EditKey::Backspace, false, false, ""), (true, None));
        assert_eq!(b.key(EditKey::Other, false, false, ""), (false, None));
    }
}
