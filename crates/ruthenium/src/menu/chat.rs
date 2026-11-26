//! The chat screen (`ChatScreen`, its `EditBox`, and `ChatComponent`'s scrolling and
//! history): opened by the Chat key empty or by the Command key with a slash, it takes
//! typed text (`§` and control characters left out, at most 256 characters), moves the
//! caret and the selection as the edit box does, steps through the messages sent
//! (Up and Down), scrolls the chat (the wheel, seven lines a notch or one with Shift;
//! Page Up and Down, a page less a line), sends on Enter and closes, and closes on
//! Escape. It is drawn by the HUD, in vanilla's look ([`ChatInput`]).
//!
//! The messages sent are kept for the session's run (`recentChat`, the last 100);
//! commands are kept in `command_history.txt` too (the last 50), and come back from it.
//!
//! Not yet: tab completion and the command's suggestions, clicking in the edit box or on
//! the chat's links and hovering its text, drafts, and the chat restrictions.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use ruthenium_sim::hud_text::ChatMessage;
use ruthenium_ui::hud::{ChatInput, ChatShown};
use ruthenium_ui::modern::TextField;

use super::{
    Clipboard, SCANCODE_A, SCANCODE_BACKSPACE, SCANCODE_C, SCANCODE_DELETE, SCANCODE_DOWN,
    SCANCODE_END, SCANCODE_ESCAPE, SCANCODE_HOME, SCANCODE_KP_ENTER, SCANCODE_LEFT,
    SCANCODE_PAGE_DOWN, SCANCODE_PAGE_UP, SCANCODE_RETURN, SCANCODE_RIGHT, SCANCODE_UP, SCANCODE_V,
    SCANCODE_X,
};

/// The longest message, in UTF-16 units (`setMaxLength(256)`).
const MAX_LENGTH: usize = 256;
/// Messages kept for Up and Down (`ChatComponent.recentChat`).
const RECENT: usize = 100;
/// Commands kept in the file (`CommandHistory.MAX_PERSISTED_COMMAND_HISTORY`).
const PERSISTED: usize = 50;
/// Lines a notch of the wheel scrolls (`ChatScreen.MOUSE_SCROLL_SPEED`).
const SCROLL_SPEED: i32 = 7;
/// Lines the chat shows while it is typed in.
const LINES_PER_PAGE: i32 = 20;
/// How long the caret is on and then off (`TextCursorUtils`).
const BLINK_MS: u128 = 300;

/// The messages sent, for Up and Down.
#[derive(Debug, Default)]
pub struct ChatHistory {
    recent: VecDeque<String>,
    commands: VecDeque<String>,
    path: Option<PathBuf>,
}

impl ChatHistory {
    /// The history with the commands `path` keeps (`CommandHistory`), which come back
    /// first.
    pub fn load(path: Option<PathBuf>) -> ChatHistory {
        let mut commands = VecDeque::new();
        if let Some(text) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            for line in text.lines() {
                if commands.len() == PERSISTED {
                    commands.pop_front();
                }
                commands.push_back(line.to_owned());
            }
        }
        ChatHistory {
            recent: commands.clone(),
            commands,
            path,
        }
    }

    pub fn len(&self) -> usize {
        self.recent.len()
    }

    pub fn get(&self, i: usize) -> Option<&str> {
        self.recent.get(i).map(String::as_str)
    }

    /// `addRecentChat`: kept unless it is the last again; a command is written to the
    /// file as well.
    pub fn add(&mut self, message: &str) {
        if self.recent.back().map(String::as_str) != Some(message) {
            if self.recent.len() >= RECENT {
                self.recent.pop_front();
            }
            self.recent.push_back(message.to_owned());
        }
        if message.starts_with('/') && self.commands.back().map(String::as_str) != Some(message) {
            if self.commands.len() >= PERSISTED {
                self.commands.pop_front();
            }
            self.commands.push_back(message.to_owned());
            if let Some(path) = &self.path {
                let mut text = String::new();
                for c in &self.commands {
                    text.push_str(c);
                    text.push('\n');
                }
                if let Err(e) = std::fs::write(path, text) {
                    eprintln!(
                        "[ruthenium] couldn't write {}, the commands sent will be missing: {e}",
                        path.display()
                    );
                }
            }
        }
    }
}

/// What a key did on the chat screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatOutcome {
    /// Nothing to do outside it.
    Stay,
    /// Escape: it closes.
    Close,
    /// Enter: this goes to the server (made tidy, maybe empty), and the screen closes.
    Send(String),
}

/// See the module docs.
#[derive(Debug)]
pub struct ChatScreen {
    field: TextField,
    /// Where Up and Down are in the history; at its length, the text being typed, kept
    /// in `buffer` while the history shows.
    history_pos: usize,
    buffer: String,
    /// When the edit box took the focus (the caret blinks from then).
    focused: Instant,
    scroll: i32,
    newest: Option<Arc<ChatMessage>>,
    new_since_scroll: bool,
    /// The chat's lines when it was last drawn.
    lines: usize,
}

impl ChatScreen {
    pub fn new(initial: &str, history: &ChatHistory) -> ChatScreen {
        ChatScreen {
            field: TextField::new(initial, MAX_LENGTH),
            history_pos: history.len(),
            buffer: String::new(),
            focused: Instant::now(),
            scroll: 0,
            newest: None,
            new_since_scroll: false,
            lines: 0,
        }
    }

    #[cfg(test)]
    pub fn text(&self) -> &str {
        self.field.value()
    }

    /// Typed text (`charTyped`): what the chat allows of it.
    pub fn type_text(&mut self, text: &str) {
        self.field.insert(text);
    }

    /// A key went down (or repeated): `ChatScreen.keyPressed`, the edit box's keys first.
    pub fn key(
        &mut self,
        scancode: u32,
        shift: bool,
        shortcut: bool,
        history: &ChatHistory,
        clipboard: &dyn Clipboard,
    ) -> ChatOutcome {
        let f = &mut self.field;
        match scancode {
            SCANCODE_ESCAPE => return ChatOutcome::Close,
            SCANCODE_BACKSPACE => f.backspace(shortcut),
            SCANCODE_DELETE => f.delete(shortcut),
            SCANCODE_HOME => f.home(shift),
            SCANCODE_END => f.end(shift),
            SCANCODE_RIGHT => f.step(true, shortcut, shift),
            SCANCODE_LEFT => f.step(false, shortcut, shift),
            SCANCODE_A if shortcut => f.select_all(),
            SCANCODE_C if shortcut => clipboard.set(f.selected_text()),
            SCANCODE_X if shortcut => {
                clipboard.set(f.selected_text());
                f.insert("");
            }
            SCANCODE_V if shortcut => {
                if let Some(text) = clipboard.get() {
                    f.insert(&text);
                }
            }
            SCANCODE_RETURN | SCANCODE_KP_ENTER => {
                return ChatOutcome::Send(self.field.value().to_owned());
            }
            SCANCODE_PAGE_UP => self.scroll_chat(LINES_PER_PAGE - 1),
            SCANCODE_PAGE_DOWN => self.scroll_chat(-LINES_PER_PAGE + 1),
            SCANCODE_UP => self.move_in_history(-1, history),
            SCANCODE_DOWN => self.move_in_history(1, history),
            _ => return ChatOutcome::Stay,
        }
        // (Anything that edits or moves the caret shows it at once.)
        if !matches!(scancode, SCANCODE_PAGE_UP | SCANCODE_PAGE_DOWN) {
            self.focused = Instant::now();
        }
        ChatOutcome::Stay
    }

    /// `moveInHistory`: back (-1) or on (1) through the messages sent; past the last,
    /// the text that was being typed.
    fn move_in_history(&mut self, dir: i32, history: &ChatHistory) {
        let max = history.len();
        let pos = (self.history_pos as i64 + i64::from(dir)).clamp(0, max as i64) as usize;
        if pos == self.history_pos {
            return;
        }
        if pos == max {
            self.history_pos = max;
            self.field = TextField::new(&self.buffer, MAX_LENGTH);
        } else {
            if self.history_pos == max {
                self.buffer = self.field.value().to_owned();
            }
            self.field = TextField::new(history.get(pos).unwrap_or(""), MAX_LENGTH);
            self.history_pos = pos;
        }
    }

    /// `ChatComponent.scrollChat`, in range of the lines last drawn.
    fn scroll_chat(&mut self, lines: i32) {
        self.scroll += lines;
        let max = self.lines as i32 - LINES_PER_PAGE;
        if self.scroll > max {
            self.scroll = max;
        }
        if self.scroll <= 0 {
            self.scroll = 0;
            self.new_since_scroll = false;
        }
    }

    /// The wheel (`mouseScrolled`): a notch at a time, seven lines unless Shift is held.
    pub fn wheel(&mut self, notches: f32, shift: bool) {
        let n = notches.clamp(-1.0, 1.0);
        let n = if shift { n } else { n * SCROLL_SPEED as f32 };
        self.scroll_chat(n as i32);
    }

    /// How the HUD drew the chat: its lines, and those that came since (each scrolls it
    /// up one more while it is up). Taken in once.
    pub fn shown(&mut self, shown: ChatShown) {
        self.lines = shown.lines;
        let same = match (&self.newest, &shown.newest) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        if self.scroll > 0 && shown.added > 0 {
            self.new_since_scroll = true;
            self.scroll_chat(shown.added);
        }
        self.newest = shown.newest;
    }

    /// What the HUD draws of it at `now`.
    pub fn input(&self, now: Instant) -> ChatInput {
        let blink = now.saturating_duration_since(self.focused).as_millis() / BLINK_MS;
        let selection = self.field.selection();
        let caret = self.field.caret();
        ChatInput {
            text: self.field.value().to_owned(),
            caret,
            anchor: if selection.start == caret {
                selection.end
            } else {
                selection.start
            },
            caret_on: blink.is_multiple_of(2),
            scroll: self.scroll,
            newest: self.newest.clone(),
            new_since_scroll: self.new_since_scroll,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Clip(RefCell<Option<String>>);

    impl Clipboard for Clip {
        fn get(&self) -> Option<String> {
            self.0.borrow().clone()
        }
        fn set(&self, text: &str) {
            *self.0.borrow_mut() = Some(text.to_owned());
        }
    }

    fn history(items: &[&str]) -> ChatHistory {
        let mut h = ChatHistory::default();
        for i in items {
            h.add(i);
        }
        h
    }

    #[test]
    fn typing_keeps_what_the_chat_allows_up_to_256() {
        let h = ChatHistory::default();
        let mut s = ChatScreen::new("/", &h);
        s.type_text("say a§b\tc\u{7F}é😀");
        assert_eq!(s.text(), "/say abcé😀");
        s.type_text(&"x".repeat(300));
        assert_eq!(s.text().encode_utf16().count(), 256);
        // A pair that does not fit is not cut in half.
        let mut s = ChatScreen::new(&"x".repeat(255), &h);
        s.type_text("😀y");
        assert_eq!(s.text(), "x".repeat(255));
    }

    #[test]
    fn keys_edit_as_the_edit_box_does() {
        let h = ChatHistory::default();
        let clip = Clip::default();
        let mut s = ChatScreen::new("hello big world", &h);
        let key =
            |s: &mut ChatScreen, code, shift, shortcut| s.key(code, shift, shortcut, &h, &clip);
        key(&mut s, SCANCODE_LEFT, false, true);
        key(&mut s, SCANCODE_BACKSPACE, false, true);
        assert_eq!(s.text(), "hello world");
        key(&mut s, SCANCODE_HOME, false, false);
        key(&mut s, SCANCODE_RIGHT, true, true);
        key(&mut s, SCANCODE_X, false, true);
        assert_eq!((s.text(), clip.get().as_deref()), ("world", Some("hello ")));
        key(&mut s, SCANCODE_END, false, false);
        key(&mut s, SCANCODE_V, false, true);
        assert_eq!(s.text(), "worldhello ");
        key(&mut s, SCANCODE_A, false, true);
        key(&mut s, SCANCODE_DELETE, false, false);
        assert_eq!(s.text(), "");
        assert_eq!(
            key(&mut s, SCANCODE_ESCAPE, false, false),
            ChatOutcome::Close
        );
        s.type_text("hi");
        assert_eq!(
            key(&mut s, SCANCODE_KP_ENTER, false, false),
            ChatOutcome::Send("hi".into())
        );
    }

    #[test]
    fn the_same_message_twice_is_kept_once() {
        let h = history(&["a", "a", "/b", "/b", "a"]);
        assert_eq!(h.len(), 3);
        assert_eq!(
            (h.get(0), h.get(1), h.get(2)),
            (Some("a"), Some("/b"), Some("a"))
        );
    }

    #[test]
    fn up_and_down_step_through_what_was_sent() {
        let h = history(&["one", "/two", "three"]);
        let clip = Clip::default();
        let mut s = ChatScreen::new("", &h);
        s.type_text("draft");
        s.key(SCANCODE_UP, false, false, &h, &clip);
        assert_eq!(s.text(), "three");
        s.key(SCANCODE_UP, false, false, &h, &clip);
        s.key(SCANCODE_UP, false, false, &h, &clip);
        s.key(SCANCODE_UP, false, false, &h, &clip);
        assert_eq!(s.text(), "one");
        s.key(SCANCODE_DOWN, false, false, &h, &clip);
        assert_eq!(s.text(), "/two");
        s.key(SCANCODE_DOWN, false, false, &h, &clip);
        s.key(SCANCODE_DOWN, false, false, &h, &clip);
        assert_eq!(s.text(), "draft");
        s.key(SCANCODE_DOWN, false, false, &h, &clip);
        assert_eq!(s.text(), "draft");
    }

    #[test]
    fn the_history_keeps_100_and_the_file_50_commands() {
        let dir = std::env::temp_dir().join(format!("ruthenium-chat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("command_history.txt");
        let mut h = ChatHistory::load(Some(path.clone()));
        for i in 0..120 {
            h.add(&format!("/c{i}"));
            h.add(&format!("m{i}"));
        }
        h.add("m119");
        assert_eq!(h.len(), 100);
        assert_eq!(h.get(99), Some("m119"));
        let again = ChatHistory::load(Some(path.clone()));
        assert_eq!(again.len(), 50);
        assert_eq!((again.get(0), again.get(49)), (Some("/c70"), Some("/c119")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_chat_scrolls_within_its_lines() {
        let h = ChatHistory::default();
        let clip = Clip::default();
        let mut s = ChatScreen::new("", &h);
        s.shown(ChatShown {
            lines: 50,
            ..ChatShown::default()
        });
        assert_eq!(s.input(Instant::now()).scroll, 0);
        s.key(SCANCODE_PAGE_UP, false, false, &h, &clip);
        assert_eq!(s.input(Instant::now()).scroll, 19);
        s.wheel(3.0, false);
        assert_eq!(s.input(Instant::now()).scroll, 26);
        s.wheel(1.0, false);
        assert_eq!(s.input(Instant::now()).scroll, 30);
        s.wheel(-1.0, true);
        assert_eq!(s.input(Instant::now()).scroll, 29);
        // Two lines came: they push it up two, once.
        let message = |n: i32| {
            let mut state = ruthenium_sim::hud_text::HudTextState::default();
            state.system_message(n, ruthenium_protocol::nbt::Tag::String(n.to_string()));
            let mut texts = ruthenium_sim::hud_text::HudTexts::default();
            let none = Default::default();
            state.fill(
                &mut texts,
                0,
                "",
                false,
                false,
                &none,
                &Default::default(),
                &Default::default(),
            );
            texts.chat[0].clone()
        };
        let new = Some(message(1));
        let shown = ChatShown {
            lines: 52,
            added: 2,
            newest: new.clone(),
        };
        s.shown(shown.clone());
        s.shown(shown);
        let i = s.input(Instant::now());
        assert_eq!((i.scroll, i.new_since_scroll), (31, true));
        assert!(
            i.newest
                .is_some_and(|n| Arc::ptr_eq(&n, new.as_ref().unwrap()))
        );
        s.key(SCANCODE_PAGE_DOWN, false, false, &h, &clip);
        s.key(SCANCODE_PAGE_DOWN, false, false, &h, &clip);
        let i = s.input(Instant::now());
        assert_eq!((i.scroll, i.new_since_scroll), (0, false));
        // At the bottom, new lines leave it there.
        s.shown(ChatShown {
            lines: 53,
            added: 1,
            newest: Some(message(2)),
        });
        assert_eq!(s.input(Instant::now()).scroll, 0);
    }
}
