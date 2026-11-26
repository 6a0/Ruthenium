//! Which arguments of a command are signed (`SignableCommand.of` over
//! `CommandDispatcher.parse`): the command is parsed against the server's command graph
//! as brigadier parses it, and the `message` arguments on the way it takes are the ones
//! a chat session signs, by name, with the text they cover.
//!
//! Brigadier tries every argument that may come next (or the one literal the next word
//! spells), each from where the last ended; an argument must end at a space or at the
//! end, and a redirect goes on from another node in a context of its own. Of the ways
//! that parse, it takes the first that read everything, else the first without errors.
//! Arguments in a context that redirects back to the root (`execute ... run`) are not
//! signed.
//!
//! The simple types read exactly as vanilla reads them (booleans, numbers with their
//! bounds, strings, coordinates, angles, swizzles, times, entity names and UUIDs,
//! selectors' types and their `limit` and `type`); the rest are taken as a token up to
//! the next space outside quotes and brackets, which is what each of them reads when
//! it is well formed, without checking its contents. A selector is allowed when the
//! player's permission level is 2 or more (`commands/entity_selectors`).

use ruthenium_data::v26_3::registries::CommandArgumentType as T;
use ruthenium_protocol::commands::{ArgumentType, CommandNode, CommandTree, NodeKind, StringKind};

use crate::chat::MAX_MESSAGE;

/// The arguments of `command` (without its slash) to sign, in order: their names and
/// what they say.
pub fn signable_arguments(
    tree: &CommandTree,
    command: &str,
    permission_level: i32,
) -> Vec<(String, String)> {
    let chars: Vec<char> = command.chars().collect();
    let parser = Parser {
        tree,
        chars: &chars,
        selectors: permission_level >= 2,
    };
    let parsed = parser.parse_nodes(tree.root, 0, Context::new(tree.root));
    let mut out = Vec::new();
    let mut context = Some(&parsed.context);
    while let Some(c) = context {
        for &(node, start, end) in &c.arguments {
            if let NodeKind::Argument {
                name,
                kind: ArgumentType::Plain(T::Message),
                ..
            } = &tree.nodes[node].kind
            {
                // (`getArguments().get(name)`: the last of that name in the context.)
                let value = c
                    .arguments
                    .iter()
                    .rev()
                    .find(|&&(n, ..)| node_name(&tree.nodes[n]) == Some(name))
                    .map_or((start, end), |&(_, s, e)| (s, e));
                out.push((name.clone(), chars[value.0..value.1].iter().collect()));
            }
        }
        context = c.child.as_deref().filter(|child| child.root != tree.root);
    }
    out
}

fn node_name(node: &CommandNode) -> Option<&String> {
    match &node.kind {
        NodeKind::Argument { name, .. } => Some(name),
        _ => None,
    }
}

/// `CommandContextBuilder`, for what signing needs: the arguments parsed (node and
/// range) and the context a redirect went on in.
#[derive(Debug, Clone)]
struct Context {
    root: usize,
    arguments: Vec<(usize, usize, usize)>,
    child: Option<Box<Context>>,
}

impl Context {
    fn new(root: usize) -> Context {
        Context {
            root,
            arguments: Vec::new(),
            child: None,
        }
    }
}

/// `ParseResults`: the context, where the reader stopped and whether anything failed.
#[derive(Debug)]
struct Parsed {
    context: Context,
    cursor: usize,
    errors: bool,
}

/// A parse that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fail {
    /// `@` with nothing or something unknown after it.
    SelectorType,
    Other,
}

type Result<T> = std::result::Result<T, Fail>;

struct Parser<'a> {
    tree: &'a CommandTree,
    chars: &'a [char],
    selectors: bool,
}

impl Parser<'_> {
    /// `CommandDispatcher.parseNodes`.
    fn parse_nodes(&self, node: usize, cursor: usize, context: Context) -> Parsed {
        let mut errors = false;
        let mut potentials: Vec<Parsed> = Vec::new();
        for child in self.relevant(node, cursor) {
            let n = &self.tree.nodes[child];
            let mut context = context.clone();
            let mut r = Reader {
                chars: self.chars,
                cursor,
            };
            let ok = match &n.kind {
                NodeKind::Root => Err(Fail::Other),
                NodeKind::Literal(word) => r.literal(word),
                NodeKind::Argument { kind, .. } => {
                    let start = r.cursor;
                    self.argument(&mut r, kind).map(|()| {
                        context.arguments.push((child, start, r.cursor));
                    })
                }
            }
            .and_then(|()| {
                if r.can_read(1) && r.peek() != ' ' {
                    Err(Fail::Other)
                } else {
                    Ok(())
                }
            });
            if ok.is_err() {
                errors = true;
                continue;
            }
            if r.can_read(if n.redirect.is_none() { 2 } else { 1 }) {
                r.cursor += 1;
                if let Some(redirect) = n.redirect {
                    let parsed = self.parse_nodes(redirect, r.cursor, Context::new(redirect));
                    context.child = Some(Box::new(parsed.context));
                    return Parsed {
                        context,
                        cursor: parsed.cursor,
                        errors: parsed.errors,
                    };
                }
                potentials.push(self.parse_nodes(child, r.cursor, context));
            } else {
                potentials.push(Parsed {
                    context,
                    cursor: r.cursor,
                    errors: false,
                });
            }
        }
        let len = self.chars.len();
        // (A stable sort: the first that read everything, then the first without errors.)
        potentials.sort_by_key(|p| (p.cursor < len, p.errors));
        match potentials.into_iter().next() {
            Some(best) => best,
            None => Parsed {
                context,
                cursor,
                errors,
            },
        }
    }

    /// `CommandNode.getRelevantNodes`: the literal the next word spells, else every
    /// argument.
    fn relevant(&self, node: usize, cursor: usize) -> Vec<usize> {
        let children = &self.tree.nodes[node].children;
        let word_end = self.chars[cursor..]
            .iter()
            .position(|&c| c == ' ')
            .map_or(self.chars.len(), |i| cursor + i);
        let word: String = self.chars[cursor..word_end].iter().collect();
        let mut literals = false;
        for &c in children {
            if let NodeKind::Literal(l) = &self.tree.nodes[c].kind {
                literals = true;
                if *l == word {
                    return vec![c];
                }
            }
        }
        let _ = literals;
        children
            .iter()
            .copied()
            .filter(|&c| matches!(self.tree.nodes[c].kind, NodeKind::Argument { .. }))
            .collect()
    }

    /// `ArgumentType.parse`.
    fn argument(&self, r: &mut Reader<'_>, kind: &ArgumentType) -> Result<()> {
        match kind {
            ArgumentType::Bool => r.boolean(),
            ArgumentType::Float { min, max } => {
                let v = r.number::<f32>()?;
                bounded(r, v, *min, *max)
            }
            ArgumentType::Double { min, max } => {
                let v = r.number::<f64>()?;
                bounded(r, v, *min, *max)
            }
            ArgumentType::Integer { min, max } => {
                let v = r.number::<i32>()?;
                bounded(r, v, *min, *max)
            }
            ArgumentType::Long { min, max } => {
                let v = r.number::<i64>()?;
                bounded(r, v, *min, *max)
            }
            ArgumentType::String(StringKind::Word) => {
                r.unquoted();
                Ok(())
            }
            ArgumentType::String(StringKind::Phrase) => r.string().map(drop),
            ArgumentType::String(StringKind::Greedy) => {
                r.cursor = r.chars.len();
                Ok(())
            }
            ArgumentType::Entity {
                single,
                players_only,
            } => {
                let s = self.selector(r)?;
                if (s.max_results > 1 && *single)
                    || (s.includes_entities && *players_only && !s.own)
                {
                    return Err(Fail::Other);
                }
                Ok(())
            }
            ArgumentType::ScoreHolder { multiple } => {
                if r.can_read(1) && r.peek() == '@' {
                    let s = self.selector(r)?;
                    if !multiple && s.max_results > 1 {
                        return Err(Fail::Other);
                    }
                } else {
                    r.skip_to_space();
                }
                Ok(())
            }
            ArgumentType::Time { min } => {
                let v = r.number::<f32>()?;
                let factor = match r.unquoted().as_str() {
                    "" | "t" => 1.0,
                    "s" => 20.0,
                    "d" => 24000.0,
                    _ => return Err(Fail::Other),
                };
                // (`Math.round`.)
                if ((v * factor + 0.5).floor() as i32) < *min {
                    return Err(Fail::Other);
                }
                Ok(())
            }
            ArgumentType::Resource { .. } => r.token(),
            ArgumentType::Plain(t) => self.plain(r, *t),
        }
    }

    fn plain(&self, r: &mut Reader<'_>, t: T) -> Result<()> {
        match t {
            T::Message => self.message(r),
            T::GameProfile => {
                if r.can_read(1) && r.peek() == '@' {
                    if self.selector(r)?.includes_entities {
                        return Err(Fail::Other);
                    }
                } else {
                    r.skip_to_space();
                }
                Ok(())
            }
            T::BlockPos => r.coordinates(3, true),
            T::Vec3 => r.coordinates(3, false),
            T::ColumnPos => r.world_coordinates(2, true),
            T::Vec2 | T::Rotation => r.world_coordinates(2, false),
            T::Angle => {
                if !r.can_read(1) {
                    return Err(Fail::Other);
                }
                if r.peek() == '~' {
                    r.cursor += 1;
                }
                if r.can_read(1) && r.peek() != ' ' {
                    let v = r.number::<f32>()?;
                    if !v.is_finite() {
                        return Err(Fail::Other);
                    }
                }
                Ok(())
            }
            T::Swizzle => {
                let mut seen = [false; 3];
                while r.can_read(1) && r.peek() != ' ' {
                    let axis = match r.peek() {
                        'x' => 0,
                        'y' => 1,
                        'z' => 2,
                        _ => return Err(Fail::Other),
                    };
                    r.cursor += 1;
                    if std::mem::replace(&mut seen[axis], true) {
                        return Err(Fail::Other);
                    }
                }
                Ok(())
            }
            T::Uuid => {
                let start = r.cursor;
                while r.can_read(1) && (r.peek().is_ascii_hexdigit() || r.peek() == '-') {
                    r.cursor += 1;
                }
                let s: String = r.chars[start..r.cursor].iter().collect();
                if s.split('-').count() != 5 || s.split('-').any(str::is_empty) {
                    r.cursor = start;
                    return Err(Fail::Other);
                }
                Ok(())
            }
            _ => r.token(),
        }
    }

    /// `MessageArgument.Message.parseText`: everything left, at most 256 characters;
    /// with selectors allowed, those in it must read.
    fn message(&self, r: &mut Reader<'_>) -> Result<()> {
        let left = r.chars.len() - r.cursor;
        let units: usize = r.chars[r.cursor..].iter().map(|c| c.len_utf16()).sum();
        if units > MAX_MESSAGE {
            return Err(Fail::Other);
        }
        if !self.selectors {
            r.cursor += left;
            return Ok(());
        }
        while r.can_read(1) {
            if r.peek() != '@' {
                r.cursor += 1;
                continue;
            }
            let start = r.cursor;
            match self.selector(r) {
                Ok(_) => {}
                Err(Fail::SelectorType) => r.cursor = start + 1,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// `EntitySelectorParser.parse`.
    fn selector(&self, r: &mut Reader<'_>) -> Result<Selector> {
        if !(r.can_read(1) && r.peek() == '@') {
            return r.name_or_uuid();
        }
        if !self.selectors {
            return Err(Fail::Other);
        }
        r.cursor += 1;
        if !r.can_read(1) {
            return Err(Fail::SelectorType);
        }
        let (max_results, includes_entities, own) = match r.peek() {
            'a' => (i32::MAX, false, false),
            'e' => (i32::MAX, true, false),
            'n' => (1, true, false),
            'p' | 'r' => (1, false, false),
            's' => (1, true, true),
            _ => return Err(Fail::SelectorType),
        };
        r.cursor += 1;
        let mut s = Selector {
            max_results,
            includes_entities,
            own,
        };
        if r.can_read(1) && r.peek() == '[' {
            r.cursor += 1;
            r.options(&mut s)?;
        }
        Ok(s)
    }
}

/// What a selector picks, as far as the argument types check it.
#[derive(Debug, Clone, Copy)]
struct Selector {
    max_results: i32,
    includes_entities: bool,
    /// `@s`.
    own: bool,
}

/// Fails if `v` is outside `min..=max`.
fn bounded<N: PartialOrd>(_: &mut Reader<'_>, v: N, min: N, max: N) -> Result<()> {
    if v < min || v > max {
        return Err(Fail::Other);
    }
    Ok(())
}

/// Brigadier's `StringReader`, over the command's characters.
struct Reader<'a> {
    chars: &'a [char],
    cursor: usize,
}

/// The selector options `EntitySelectorOptions` knows.
const OPTIONS: [&str; 21] = [
    "name",
    "distance",
    "level",
    "x",
    "y",
    "z",
    "dx",
    "dy",
    "dz",
    "x_rotation",
    "y_rotation",
    "limit",
    "sort",
    "gamemode",
    "team",
    "type",
    "tag",
    "nbt",
    "scores",
    "advancements",
    "predicate",
];

impl Reader<'_> {
    /// `canRead(n)`.
    fn can_read(&self, n: usize) -> bool {
        self.cursor + n <= self.chars.len()
    }

    fn peek(&self) -> char {
        self.chars[self.cursor]
    }

    fn text(&self, start: usize) -> String {
        self.chars[start..self.cursor].iter().collect()
    }

    fn skip_whitespace(&mut self) {
        while self.can_read(1) && self.peek().is_whitespace() {
            self.cursor += 1;
        }
    }

    fn skip_to_space(&mut self) {
        while self.can_read(1) && self.peek() != ' ' {
            self.cursor += 1;
        }
    }

    /// `LiteralCommandNode.parse`.
    fn literal(&mut self, word: &str) -> Result<()> {
        let w: Vec<char> = word.chars().collect();
        if self.chars[self.cursor..].starts_with(&w) {
            let end = self.cursor + w.len();
            if end == self.chars.len() || self.chars[end] == ' ' {
                self.cursor = end;
                return Ok(());
            }
        }
        Err(Fail::Other)
    }

    /// `readUnquotedString`.
    fn unquoted(&mut self) -> String {
        let start = self.cursor;
        while self.can_read(1)
            && matches!(self.peek(), '0'..='9' | 'A'..='Z' | 'a'..='z' | '_' | '-' | '.' | '+')
        {
            self.cursor += 1;
        }
        self.text(start)
    }

    /// `readString`: quoted, or a word.
    fn string(&mut self) -> Result<String> {
        if !self.can_read(1) {
            return Ok(String::new());
        }
        let q = self.peek();
        if q == '"' || q == '\'' {
            self.cursor += 1;
            return self.until(q);
        }
        Ok(self.unquoted())
    }

    /// `readStringUntil`: to the closing quote, backslashes escaping it and themselves.
    fn until(&mut self, quote: char) -> Result<String> {
        let mut out = String::new();
        let mut escaped = false;
        while self.can_read(1) {
            let c = self.peek();
            self.cursor += 1;
            if escaped {
                if c != quote && c != '\\' {
                    self.cursor -= 1;
                    return Err(Fail::Other);
                }
                out.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == quote {
                return Ok(out);
            } else {
                out.push(c);
            }
        }
        Err(Fail::Other)
    }

    /// `readBoolean`.
    fn boolean(&mut self) -> Result<()> {
        let start = self.cursor;
        match self.string()?.as_str() {
            "true" | "false" => Ok(()),
            _ => {
                self.cursor = start;
                Err(Fail::Other)
            }
        }
    }

    /// `readInt`, `readLong`, `readFloat`, `readDouble`: the number's characters
    /// (digits, `.` and `-`), which must parse.
    fn number<N: std::str::FromStr>(&mut self) -> Result<N> {
        let start = self.cursor;
        while self.can_read(1) && matches!(self.peek(), '0'..='9' | '.' | '-') {
            self.cursor += 1;
        }
        let s = self.text(start);
        // (Java's parsers take no `+`, and none is read here.)
        s.parse().map_err(|_| {
            self.cursor = start;
            Fail::Other
        })
    }

    /// `BlockPosArgument` (`ints`) and `Vec3Argument`: three local (`^`) or world
    /// coordinates.
    fn coordinates(&mut self, n: usize, ints: bool) -> Result<()> {
        if self.can_read(1) && self.peek() == '^' {
            let start = self.cursor;
            for i in 0..n {
                if i > 0 {
                    if !(self.can_read(1) && self.peek() == ' ') {
                        self.cursor = start;
                        return Err(Fail::Other);
                    }
                    self.cursor += 1;
                }
                if !self.can_read(1) || self.peek() != '^' {
                    self.cursor = start;
                    return Err(Fail::Other);
                }
                self.cursor += 1;
                if self.can_read(1) && self.peek() != ' ' {
                    self.number::<f64>()?;
                }
            }
            return Ok(());
        }
        self.world_coordinates(n, ints)
    }

    /// `WorldCoordinates` (and the two-coordinate arguments): each `~` and a number,
    /// either, or a number (whole for block positions), a space between.
    fn world_coordinates(&mut self, n: usize, ints: bool) -> Result<()> {
        let start = self.cursor;
        if !self.can_read(1) {
            return Err(Fail::Other);
        }
        for i in 0..n {
            if i > 0 {
                if !(self.can_read(1) && self.peek() == ' ') {
                    self.cursor = start;
                    return Err(Fail::Other);
                }
                self.cursor += 1;
            }
            if !self.can_read(1) || self.peek() == '^' {
                return Err(Fail::Other);
            }
            let relative = self.peek() == '~';
            if relative {
                self.cursor += 1;
            }
            if self.can_read(1) && self.peek() != ' ' {
                if ints && !relative {
                    self.number::<i32>()?;
                } else {
                    self.number::<f64>()?;
                }
            }
        }
        Ok(())
    }

    /// `parseNameOrUUID`: a name of 1 to 16 characters, or a UUID.
    fn name_or_uuid(&mut self) -> Result<Selector> {
        let start = self.cursor;
        let name = self.string()?;
        let uuid = name.split('-').count() == 5
            && name
                .split('-')
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_hexdigit()));
        if uuid {
            return Ok(Selector {
                max_results: 1,
                includes_entities: true,
                own: false,
            });
        }
        if name.is_empty() || name.encode_utf16().count() > 16 {
            self.cursor = start;
            return Err(Fail::Other);
        }
        Ok(Selector {
            max_results: 1,
            includes_entities: false,
            own: false,
        })
    }

    /// `parseOptions` after the `[`: `key=value` pairs and the `]`. Keys must be known;
    /// values are tokens, of which `limit` and `type` are read.
    fn options(&mut self, s: &mut Selector) -> Result<()> {
        self.skip_whitespace();
        while self.can_read(1) && self.peek() != ']' {
            self.skip_whitespace();
            let key = self.string()?;
            if !OPTIONS.contains(&key.as_str()) {
                return Err(Fail::Other);
            }
            self.skip_whitespace();
            if !(self.can_read(1) && self.peek() == '=') {
                return Err(Fail::Other);
            }
            self.cursor += 1;
            self.skip_whitespace();
            let start = self.cursor;
            self.value()?;
            let value = self.text(start);
            match key.as_str() {
                "limit" => {
                    let n: i32 = value.parse().map_err(|_| Fail::Other)?;
                    if n < 1 {
                        return Err(Fail::Other);
                    }
                    s.max_results = n;
                }
                "type" if !value.starts_with('!') && !value.starts_with('#') => {
                    s.includes_entities = !matches!(value.as_str(), "player" | "minecraft:player");
                }
                _ => {}
            }
            self.skip_whitespace();
            if self.can_read(1) {
                match self.peek() {
                    ',' => self.cursor += 1,
                    ']' => break,
                    _ => return Err(Fail::Other),
                }
            }
        }
        if !self.can_read(1) {
            return Err(Fail::Other);
        }
        self.cursor += 1;
        Ok(())
    }

    /// An option's value: up to a `,`, `]` or whitespace outside quotes and brackets.
    fn value(&mut self) -> Result<()> {
        self.balanced(|c| c == ',' || c == ']' || c.is_whitespace())
    }

    /// An argument read as a token: up to a space outside quotes and brackets, not
    /// empty.
    fn token(&mut self) -> Result<()> {
        let start = self.cursor;
        self.balanced(|c| c == ' ')?;
        if self.cursor == start {
            return Err(Fail::Other);
        }
        Ok(())
    }

    /// Reads up to a character `stop` says ends it, outside quotes and brackets (a
    /// closing bracket with none open ends it too).
    fn balanced(&mut self, stop: impl Fn(char) -> bool) -> Result<()> {
        let mut depth = 0u32;
        while self.can_read(1) {
            let c = self.peek();
            if depth == 0 && stop(c) {
                break;
            }
            match c {
                '"' | '\'' => {
                    self.cursor += 1;
                    self.until(c)?;
                    continue;
                }
                '[' | '{' | '(' => depth += 1,
                ']' | '}' | ')' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.cursor += 1;
        }
        if depth > 0 {
            return Err(Fail::Other);
        }
        Ok(())
    }
}
