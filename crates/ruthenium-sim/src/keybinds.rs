//! The game's key bindings (vanilla's `KeyMapping`s in `Options`): every mapping 26.3
//! has, in its categories and with its default key; keys as the game names them
//! (`InputConstants`); the player's bindings and where two share a key; and the bindings
//! read from vanilla's `options.txt`, old ones included (its data fixers).
//!
//! Several mappings may share a key, as in vanilla: a press then presses all of them.
//! Ruthenium acts on the mappings it has an action for; the others (the inventory,
//! advancements, the debug keys...) are kept and rebindable all the same, so that an
//! imported `options.txt` and the conflicts it has are whole.

use ruthenium_protocol::text::{BoundKeys, KeyText, Lang, key_text};

/// A keyboard key (its SDL scancode, as vanilla stores it) or a mouse button (SDL's
/// numbering: 1 left, 2 middle, 3 right, 4 and 5 the side buttons). Values are Java
/// ints: a name in a file can give any of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Keyboard(i32),
    Mouse(i32),
}

/// What reading a key's name gave (`InputConstants.getKey`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parsed {
    Key(Key),
    /// Not a key's name (vanilla unbinds the mapping).
    Invalid,
    /// A name the game fails on outright (`key.keyboard` with nothing after it): vanilla
    /// stops reading its options there.
    Broken,
}

/// The keyboard's named keys (`InputConstants.Type.KEYBOARD`): name and scancode.
const KEYBOARD: &[(&str, i32)] = &[
    ("unknown", 0),
    ("a", 4),
    ("b", 5),
    ("c", 6),
    ("d", 7),
    ("e", 8),
    ("f", 9),
    ("g", 10),
    ("h", 11),
    ("i", 12),
    ("j", 13),
    ("k", 14),
    ("l", 15),
    ("m", 16),
    ("n", 17),
    ("o", 18),
    ("p", 19),
    ("q", 20),
    ("r", 21),
    ("s", 22),
    ("t", 23),
    ("u", 24),
    ("v", 25),
    ("w", 26),
    ("x", 27),
    ("y", 28),
    ("z", 29),
    ("1", 30),
    ("2", 31),
    ("3", 32),
    ("4", 33),
    ("5", 34),
    ("6", 35),
    ("7", 36),
    ("8", 37),
    ("9", 38),
    ("0", 39),
    ("enter", 40),
    ("escape", 41),
    ("backspace", 42),
    ("tab", 43),
    ("space", 44),
    ("minus", 45),
    ("equal", 46),
    ("left.bracket", 47),
    ("right.bracket", 48),
    ("backslash", 49),
    ("world.2", 50),
    ("semicolon", 51),
    ("apostrophe", 52),
    ("grave.accent", 53),
    ("comma", 54),
    ("period", 55),
    ("slash", 56),
    ("caps.lock", 57),
    ("f1", 58),
    ("f2", 59),
    ("f3", 60),
    ("f4", 61),
    ("f5", 62),
    ("f6", 63),
    ("f7", 64),
    ("f8", 65),
    ("f9", 66),
    ("f10", 67),
    ("f11", 68),
    ("f12", 69),
    ("print.screen", 70),
    ("scroll.lock", 71),
    ("pause", 72),
    ("insert", 73),
    ("home", 74),
    ("page.up", 75),
    ("delete", 76),
    ("end", 77),
    ("page.down", 78),
    ("right", 79),
    ("left", 80),
    ("down", 81),
    ("up", 82),
    ("num.lock", 83),
    ("keypad.divide", 84),
    ("keypad.multiply", 85),
    ("keypad.subtract", 86),
    ("keypad.add", 87),
    ("keypad.enter", 88),
    ("keypad.1", 89),
    ("keypad.2", 90),
    ("keypad.3", 91),
    ("keypad.4", 92),
    ("keypad.5", 93),
    ("keypad.6", 94),
    ("keypad.7", 95),
    ("keypad.8", 96),
    ("keypad.9", 97),
    ("keypad.0", 98),
    ("keypad.period", 99),
    ("world.1", 100),
    ("application", 101),
    ("power", 102),
    ("keypad.equal", 103),
    ("f13", 104),
    ("f14", 105),
    ("f15", 106),
    ("f16", 107),
    ("f17", 108),
    ("f18", 109),
    ("f19", 110),
    ("f20", 111),
    ("f21", 112),
    ("f22", 113),
    ("f23", 114),
    ("f24", 115),
    ("execute", 116),
    ("help", 117),
    ("menu", 118),
    ("select", 119),
    ("stop", 120),
    ("again", 121),
    ("undo", 122),
    ("cut", 123),
    ("copy", 124),
    ("paste", 125),
    ("find", 126),
    ("mute", 127),
    ("volume.up", 128),
    ("volume.down", 129),
    ("keypad.comma", 133),
    ("keypad.equals.as400", 134),
    ("international1", 135),
    ("international2", 136),
    ("international3", 137),
    ("international4", 138),
    ("international5", 139),
    ("international6", 140),
    ("international7", 141),
    ("international8", 142),
    ("international9", 143),
    ("lang1", 144),
    ("lang2", 145),
    ("lang3", 146),
    ("lang4", 147),
    ("lang5", 148),
    ("lang6", 149),
    ("lang7", 150),
    ("lang8", 151),
    ("lang9", 152),
    ("alternate.erase", 153),
    ("sys.req", 154),
    ("cancel", 155),
    ("clear", 156),
    ("prior", 157),
    ("enter2", 158),
    ("separator", 159),
    ("out", 160),
    ("oper", 161),
    ("clear.again", 162),
    ("crsel", 163),
    ("exsel", 164),
    ("keypad.00", 176),
    ("keypad.000", 177),
    ("thousands.separator", 178),
    ("decimal.separator", 179),
    ("currency.unit", 180),
    ("currency.subunit", 181),
    ("keypad.left.parenthesis", 182),
    ("keypad.right.parenthesis", 183),
    ("keypad.left.brace", 184),
    ("keypad.right.brace", 185),
    ("keypad.tab", 186),
    ("keypad.backspace", 187),
    ("keypad.a", 188),
    ("keypad.b", 189),
    ("keypad.c", 190),
    ("keypad.d", 191),
    ("keypad.e", 192),
    ("keypad.f", 193),
    ("keypad.xor", 194),
    ("keypad.power", 195),
    ("keypad.percent", 196),
    ("keypad.less", 197),
    ("keypad.greater", 198),
    ("keypad.ampersand", 199),
    ("keypad.double.ampersand", 200),
    ("keypad.vertical.bar", 201),
    ("keypad.double.vertical.bar", 202),
    ("keypad.colon", 203),
    ("keypad.hash", 204),
    ("keypad.space", 205),
    ("keypad.at", 206),
    ("keypad.exclamation", 207),
    ("keypad.memory.store", 208),
    ("keypad.memory.recall", 209),
    ("keypad.memory.clear", 210),
    ("keypad.memory.add", 211),
    ("keypad.memory.subtract", 212),
    ("keypad.memory.multiply", 213),
    ("keypad.memory.divide", 214),
    ("keypad.plus.minus", 215),
    ("keypad.clear", 216),
    ("keypad.clear.entry", 217),
    ("keypad.binary", 218),
    ("keypad.octal", 219),
    ("keypad.decimal", 220),
    ("keypad.hexadecimal", 221),
    ("left.control", 224),
    ("left.shift", 225),
    ("left.alt", 226),
    ("left.win", 227),
    ("right.control", 228),
    ("right.shift", 229),
    ("right.alt", 230),
    ("right.win", 231),
    ("mode", 257),
    ("sleep", 258),
    ("wake", 259),
    ("channel.up", 260),
    ("channel.down", 261),
    ("media.play", 262),
    ("media.pause", 263),
    ("media.record", 264),
    ("media.fast.forward", 265),
    ("media.rewind", 266),
    ("media.next.track", 267),
    ("media.previous.track", 268),
    ("media.stop", 269),
    ("media.eject", 270),
    ("media.play.pause", 271),
    ("media.select", 272),
    ("ac.new", 273),
    ("ac.open", 274),
    ("ac.close", 275),
    ("ac.exit", 276),
    ("ac.save", 277),
    ("ac.print", 278),
    ("ac.properties", 279),
    ("ac.search", 280),
    ("ac.home", 281),
    ("ac.back", 282),
    ("ac.forward", 283),
    ("ac.stop", 284),
    ("ac.refresh", 285),
    ("ac.bookmarks", 286),
    ("soft.left", 287),
    ("soft.right", 288),
    ("call", 289),
    ("end.call", 290),
];

/// The mouse's named buttons.
const MOUSE: &[(&str, i32)] = &[
    ("left", 1),
    ("middle", 2),
    ("right", 3),
    ("4", 4),
    ("5", 5),
    ("6", 6),
    ("7", 7),
    ("8", 8),
];

const KEYBOARD_PREFIX: &str = "key.keyboard";
const MOUSE_PREFIX: &str = "key.mouse";

impl Key {
    /// No key (`InputConstants.UNKNOWN`, `key.keyboard.unknown`).
    pub const UNBOUND: Key = Key::Keyboard(0);

    pub fn is_unbound(self) -> bool {
        self == Key::UNBOUND
    }

    /// The key a keyboard event names, by SDL scancode.
    pub fn scancode(scancode: u32) -> Key {
        Key::Keyboard(i32::try_from(scancode).unwrap_or(0))
    }

    /// The key a mouse button event names (SDL's numbering).
    pub fn button(button: u8) -> Key {
        Key::Mouse(i32::from(button))
    }

    /// Its name, as `options.txt` and `client.toml` keep it (`key.keyboard.w`,
    /// `key.mouse.left`, `key.keyboard.300` for one without a name).
    pub fn name(self) -> String {
        let (prefix, table, value) = match self {
            Key::Keyboard(v) => (KEYBOARD_PREFIX, KEYBOARD, v),
            Key::Mouse(v) => (MOUSE_PREFIX, MOUSE, v),
        };
        match table.iter().find(|(_, v)| *v == value) {
            Some((name, _)) => format!("{prefix}.{name}"),
            None => format!("{prefix}.{value}"),
        }
    }

    /// Reads a key's name as `InputConstants.getKey` does: a known name, else a
    /// type's prefix, any one character, and a Java int.
    pub fn parse(name: &str) -> Parsed {
        let known = |prefix: &str, table: &[(&str, i32)]| {
            let rest = name.strip_prefix(prefix)?.strip_prefix('.')?;
            table.iter().find(|(n, _)| *n == rest).map(|(_, v)| *v)
        };
        if let Some(v) = known(KEYBOARD_PREFIX, KEYBOARD) {
            return Parsed::Key(Key::Keyboard(v));
        }
        if let Some(v) = known(MOUSE_PREFIX, MOUSE) {
            return Parsed::Key(Key::Mouse(v));
        }
        for (prefix, mouse) in [(KEYBOARD_PREFIX, false), (MOUSE_PREFIX, true)] {
            let Some(rest) = name.strip_prefix(prefix) else {
                continue;
            };
            // `substring(prefix.length() + 1)`: past the end fails outright.
            let mut chars = rest.chars();
            if chars.next().is_none() {
                return Parsed::Broken;
            }
            return match java_int(chars.as_str()) {
                Some(v) if mouse => Parsed::Key(Key::Mouse(v)),
                Some(v) => Parsed::Key(Key::Keyboard(v)),
                None => Parsed::Invalid,
            };
        }
        Parsed::Invalid
    }
}

/// `Integer.parseInt`: an optional sign and decimal digits, in range.
fn java_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// The Controls list's headings (`KeyMapping.Category`), in their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Movement,
    Misc,
    Multiplayer,
    Gameplay,
    Inventory,
    Creative,
    Spectator,
    Debug,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::Movement,
        Category::Misc,
        Category::Multiplayer,
        Category::Gameplay,
        Category::Inventory,
        Category::Creative,
        Category::Spectator,
        Category::Debug,
    ];

    /// Its id's path (`minecraft:movement`).
    pub fn id(self) -> &'static str {
        match self {
            Category::Movement => "movement",
            Category::Misc => "misc",
            Category::Multiplayer => "multiplayer",
            Category::Gameplay => "gameplay",
            Category::Inventory => "inventory",
            Category::Creative => "creative",
            Category::Spectator => "spectator",
            Category::Debug => "debug",
        }
    }

    /// The language key of its heading.
    pub fn lang_key(self) -> String {
        format!("key.category.minecraft.{}", self.id())
    }

    /// Ruthenium's own words for it, without the game's language.
    pub fn label(self) -> &'static str {
        match self {
            Category::Movement => "Movement",
            Category::Misc => "Miscellaneous",
            Category::Multiplayer => "Multiplayer",
            Category::Gameplay => "Gameplay",
            Category::Inventory => "Inventory",
            Category::Creative => "Creative Mode",
            Category::Spectator => "Spectator",
            Category::Debug => "Debug",
        }
    }
}

/// Every key mapping, in `options.txt`'s order (`Options.keyMappings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Attack,
    Use,
    Forward,
    Left,
    Back,
    Right,
    Jump,
    Sneak,
    Sprint,
    Drop,
    Inventory,
    Chat,
    PlayerList,
    PickItem,
    Command,
    Friends,
    SocialInteractions,
    ToggleGui,
    ToggleSpectatorShaderEffects,
    Screenshot,
    TogglePerspective,
    SmoothCamera,
    Fullscreen,
    SpectatorOutlines,
    SpectatorHotbar,
    SwapOffhand,
    SaveToolbarActivator,
    LoadToolbarActivator,
    Advancements,
    QuickActions,
    DebugOverlay,
    DebugModifier,
    Hotbar1,
    Hotbar2,
    Hotbar3,
    Hotbar4,
    Hotbar5,
    Hotbar6,
    Hotbar7,
    Hotbar8,
    Hotbar9,
    DebugReloadChunk,
    DebugShowHitboxes,
    DebugClearChat,
    DebugCrash,
    DebugShowChunkBorders,
    DebugShowAdvancedTooltips,
    DebugCopyRecreateCommand,
    DebugSpectate,
    DebugSwitchGameMode,
    DebugDebugOptions,
    DebugFocusPause,
    DebugDumpDynamicTextures,
    DebugReloadResourcePacks,
    DebugProfiling,
    DebugCopyLocation,
    DebugDumpVersion,
    DebugProfilingChart,
    DebugFpsCharts,
    DebugNetworkCharts,
    DebugLightmapTexture,
    DebugImprovedTransparency,
}

/// How many mappings there are.
pub const COUNT: usize = 62;

/// One mapping: its name, heading, default key, place among its heading's (before the
/// names' order), and Ruthenium's own words for it.
struct Mapping {
    name: &'static str,
    category: Category,
    default: Key,
    order: i32,
    label: &'static str,
}

const fn m(name: &'static str, category: Category, default: Key, label: &'static str) -> Mapping {
    Mapping {
        name,
        category,
        default,
        order: 0,
        label,
    }
}

const fn ordered(mapping: Mapping, order: i32) -> Mapping {
    Mapping { order, ..mapping }
}

use Category as C;
use Key::{Keyboard as K, Mouse as M};

/// Indexed by [`Action`] (`Options`' mappings, `Options.java:660`).
const MAPPINGS: [Mapping; COUNT] = [
    m("key.attack", C::Gameplay, M(1), "Attack / Break"),
    m("key.use", C::Gameplay, M(3), "Use / Place"),
    m("key.forward", C::Movement, K(26), "Walk Forward"),
    m("key.left", C::Movement, K(4), "Strafe Left"),
    m("key.back", C::Movement, K(22), "Walk Back"),
    m("key.right", C::Movement, K(7), "Strafe Right"),
    m("key.jump", C::Movement, K(44), "Jump"),
    m("key.sneak", C::Movement, K(225), "Sneak"),
    m("key.sprint", C::Movement, K(224), "Sprint"),
    m("key.drop", C::Inventory, K(20), "Drop Item"),
    m("key.inventory", C::Inventory, K(8), "Inventory"),
    m("key.chat", C::Multiplayer, K(23), "Chat"),
    m("key.playerlist", C::Multiplayer, K(43), "Player List"),
    m("key.pickItem", C::Gameplay, M(2), "Pick Block"),
    m("key.command", C::Multiplayer, K(56), "Command"),
    m("key.friends", C::Multiplayer, K(18), "Friends"),
    m(
        "key.socialInteractions",
        C::Multiplayer,
        K(19),
        "Social Interactions",
    ),
    m("key.toggleGui", C::Misc, K(58), "Hide HUD"),
    m(
        "key.toggleSpectatorShaderEffects",
        C::Misc,
        K(61),
        "Spectator Shaders",
    ),
    m("key.screenshot", C::Misc, K(59), "Screenshot"),
    m("key.togglePerspective", C::Misc, K(62), "Perspective"),
    m("key.smoothCamera", C::Misc, K(0), "Cinematic Camera"),
    m("key.fullscreen", C::Misc, K(68), "Fullscreen"),
    m(
        "key.spectatorOutlines",
        C::Spectator,
        K(0),
        "Highlight Players",
    ),
    m("key.spectatorHotbar", C::Spectator, M(2), "Spectator Menu"),
    m("key.swapOffhand", C::Inventory, K(9), "Swap Hands"),
    m("key.saveToolbarActivator", C::Creative, K(6), "Save Hotbar"),
    m(
        "key.loadToolbarActivator",
        C::Creative,
        K(27),
        "Load Hotbar",
    ),
    m("key.advancements", C::Misc, K(15), "Advancements"),
    m("key.quickActions", C::Misc, K(10), "Quick Actions"),
    ordered(m("key.debug.overlay", C::Debug, K(60), "Debug Screen"), -2),
    ordered(
        m("key.debug.modifier", C::Debug, K(60), "Debug Modifier"),
        -1,
    ),
    m("key.hotbar.1", C::Inventory, K(30), "Hotbar Slot 1"),
    m("key.hotbar.2", C::Inventory, K(31), "Hotbar Slot 2"),
    m("key.hotbar.3", C::Inventory, K(32), "Hotbar Slot 3"),
    m("key.hotbar.4", C::Inventory, K(33), "Hotbar Slot 4"),
    m("key.hotbar.5", C::Inventory, K(34), "Hotbar Slot 5"),
    m("key.hotbar.6", C::Inventory, K(35), "Hotbar Slot 6"),
    m("key.hotbar.7", C::Inventory, K(36), "Hotbar Slot 7"),
    m("key.hotbar.8", C::Inventory, K(37), "Hotbar Slot 8"),
    m("key.hotbar.9", C::Inventory, K(38), "Hotbar Slot 9"),
    m("key.debug.reloadChunk", C::Debug, K(4), "Reload Chunks"),
    m("key.debug.showHitboxes", C::Debug, K(5), "Hitboxes"),
    m("key.debug.clearChat", C::Debug, K(7), "Clear Chat"),
    m("key.debug.crash", C::Debug, K(6), "Crash"),
    m(
        "key.debug.showChunkBorders",
        C::Debug,
        K(10),
        "Chunk Borders",
    ),
    m(
        "key.debug.showAdvancedTooltips",
        C::Debug,
        K(11),
        "Advanced Tooltips",
    ),
    m(
        "key.debug.copyRecreateCommand",
        C::Debug,
        K(12),
        "Copy Entity Command",
    ),
    m("key.debug.spectate", C::Debug, K(17), "Spectate"),
    m(
        "key.debug.switchGameMode",
        C::Debug,
        K(61),
        "Game Mode Switcher",
    ),
    m("key.debug.debugOptions", C::Debug, K(63), "Debug Options"),
    m(
        "key.debug.focusPause",
        C::Debug,
        K(19),
        "Pause on Focus Loss",
    ),
    m(
        "key.debug.dumpDynamicTextures",
        C::Debug,
        K(22),
        "Dump Textures",
    ),
    m(
        "key.debug.reloadResourcePacks",
        C::Debug,
        K(23),
        "Reload Resource Packs",
    ),
    m("key.debug.profiling", C::Debug, K(15), "Profiling"),
    m("key.debug.copyLocation", C::Debug, K(6), "Copy Location"),
    m("key.debug.dumpVersion", C::Debug, K(25), "Dump Version"),
    ordered(
        m(
            "key.debug.profilingChart",
            C::Debug,
            K(30),
            "Profiler Chart",
        ),
        1,
    ),
    ordered(m("key.debug.fpsCharts", C::Debug, K(31), "Frame Charts"), 2),
    ordered(
        m("key.debug.networkCharts", C::Debug, K(32), "Network Charts"),
        3,
    ),
    ordered(
        m(
            "key.debug.lightmapTexture",
            C::Debug,
            K(33),
            "Lightmap Texture",
        ),
        4,
    ),
    m(
        "key.debug.improvedTransparency",
        C::Debug,
        K(27),
        "Improved Transparency",
    ),
];

impl Action {
    /// Every mapping, in `options.txt`'s order.
    pub const ALL: [Action; COUNT] = [
        Action::Attack,
        Action::Use,
        Action::Forward,
        Action::Left,
        Action::Back,
        Action::Right,
        Action::Jump,
        Action::Sneak,
        Action::Sprint,
        Action::Drop,
        Action::Inventory,
        Action::Chat,
        Action::PlayerList,
        Action::PickItem,
        Action::Command,
        Action::Friends,
        Action::SocialInteractions,
        Action::ToggleGui,
        Action::ToggleSpectatorShaderEffects,
        Action::Screenshot,
        Action::TogglePerspective,
        Action::SmoothCamera,
        Action::Fullscreen,
        Action::SpectatorOutlines,
        Action::SpectatorHotbar,
        Action::SwapOffhand,
        Action::SaveToolbarActivator,
        Action::LoadToolbarActivator,
        Action::Advancements,
        Action::QuickActions,
        Action::DebugOverlay,
        Action::DebugModifier,
        Action::Hotbar1,
        Action::Hotbar2,
        Action::Hotbar3,
        Action::Hotbar4,
        Action::Hotbar5,
        Action::Hotbar6,
        Action::Hotbar7,
        Action::Hotbar8,
        Action::Hotbar9,
        Action::DebugReloadChunk,
        Action::DebugShowHitboxes,
        Action::DebugClearChat,
        Action::DebugCrash,
        Action::DebugShowChunkBorders,
        Action::DebugShowAdvancedTooltips,
        Action::DebugCopyRecreateCommand,
        Action::DebugSpectate,
        Action::DebugSwitchGameMode,
        Action::DebugDebugOptions,
        Action::DebugFocusPause,
        Action::DebugDumpDynamicTextures,
        Action::DebugReloadResourcePacks,
        Action::DebugProfiling,
        Action::DebugCopyLocation,
        Action::DebugDumpVersion,
        Action::DebugProfilingChart,
        Action::DebugFpsCharts,
        Action::DebugNetworkCharts,
        Action::DebugLightmapTexture,
        Action::DebugImprovedTransparency,
    ];

    /// The Controls list's order: by heading, then by `order`, then by the names
    /// in English (`KeyMapping.compareTo` under `en_us`).
    pub const SORTED: [Action; COUNT] = [
        Action::Jump,
        Action::Sneak,
        Action::Sprint,
        Action::Left,
        Action::Right,
        Action::Back,
        Action::Forward,
        Action::Advancements,
        Action::QuickActions,
        Action::Screenshot,
        Action::SmoothCamera,
        Action::Fullscreen,
        Action::ToggleGui,
        Action::TogglePerspective,
        Action::ToggleSpectatorShaderEffects,
        Action::Friends,
        Action::PlayerList,
        Action::Chat,
        Action::Command,
        Action::SocialInteractions,
        Action::Attack,
        Action::PickItem,
        Action::Use,
        Action::Drop,
        Action::Hotbar1,
        Action::Hotbar2,
        Action::Hotbar3,
        Action::Hotbar4,
        Action::Hotbar5,
        Action::Hotbar6,
        Action::Hotbar7,
        Action::Hotbar8,
        Action::Hotbar9,
        Action::Inventory,
        Action::SwapOffhand,
        Action::LoadToolbarActivator,
        Action::SaveToolbarActivator,
        Action::SpectatorOutlines,
        Action::SpectatorHotbar,
        Action::DebugOverlay,
        Action::DebugModifier,
        Action::DebugClearChat,
        Action::DebugCopyRecreateCommand,
        Action::DebugCopyLocation,
        Action::DebugSpectate,
        Action::DebugCrash,
        Action::DebugDebugOptions,
        Action::DebugDumpDynamicTextures,
        Action::DebugDumpVersion,
        Action::DebugSwitchGameMode,
        Action::DebugReloadChunk,
        Action::DebugReloadResourcePacks,
        Action::DebugShowAdvancedTooltips,
        Action::DebugShowChunkBorders,
        Action::DebugShowHitboxes,
        Action::DebugProfiling,
        Action::DebugImprovedTransparency,
        Action::DebugFocusPause,
        Action::DebugProfilingChart,
        Action::DebugFpsCharts,
        Action::DebugNetworkCharts,
        Action::DebugLightmapTexture,
    ];

    /// The hotbar's keys, slot by slot.
    pub const HOTBAR: [Action; 9] = [
        Action::Hotbar1,
        Action::Hotbar2,
        Action::Hotbar3,
        Action::Hotbar4,
        Action::Hotbar5,
        Action::Hotbar6,
        Action::Hotbar7,
        Action::Hotbar8,
        Action::Hotbar9,
    ];

    const fn mapping(self) -> &'static Mapping {
        &MAPPINGS[self as usize]
    }

    /// Its name (`key.forward`), also its language key.
    pub const fn name(self) -> &'static str {
        self.mapping().name
    }

    pub const fn category(self) -> Category {
        self.mapping().category
    }

    pub const fn default_key(self) -> Key {
        self.mapping().default
    }

    /// Its place among its heading's, before the names' order (`KeyMapping.order`).
    pub const fn order(self) -> i32 {
        self.mapping().order
    }

    /// Ruthenium's own words for it, without the game's language.
    pub const fn label(self) -> &'static str {
        self.mapping().label
    }

    /// The mapping called `name`.
    pub fn by_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == name)
    }

    /// The hotbar slot it selects.
    pub fn hotbar_slot(self) -> Option<u8> {
        Action::HOTBAR
            .iter()
            .position(|&a| a == self)
            .map(|i| i as u8)
    }
}

/// The player's bindings: a key for each mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinds {
    keys: [Key; COUNT],
}

impl Default for KeyBinds {
    fn default() -> KeyBinds {
        KeyBinds::DEFAULT
    }
}

impl KeyBinds {
    /// Vanilla's.
    pub const DEFAULT: KeyBinds = {
        let mut keys = [Key::UNBOUND; COUNT];
        let mut i = 0;
        while i < COUNT {
            keys[i] = MAPPINGS[i].default;
            i += 1;
        }
        KeyBinds { keys }
    };

    pub fn get(&self, action: Action) -> Key {
        self.keys[action as usize]
    }

    pub fn set(&mut self, action: Action, key: Key) {
        self.keys[action as usize] = key;
    }

    /// Back to its default (a row's Reset).
    pub fn reset(&mut self, action: Action) {
        self.set(action, action.default_key());
    }

    /// Every one back to its default (Reset Keys).
    pub fn reset_all(&mut self) {
        *self = KeyBinds::DEFAULT;
    }

    pub fn is_default(&self, action: Action) -> bool {
        self.get(action) == action.default_key()
    }

    /// How many are not on their defaults.
    pub fn changed(&self) -> usize {
        Action::ALL.iter().filter(|&&a| !self.is_default(a)).count()
    }

    /// Whether `key` presses `action` (`KeyMapping.matches`: no key presses an unbound
    /// one).
    pub fn matches(&self, action: Action, key: Key) -> bool {
        !key.is_unbound() && self.get(action) == key
    }

    /// The mappings `key` presses.
    pub fn on(&self, key: Key) -> impl Iterator<Item = Action> + '_ {
        Action::ALL
            .into_iter()
            .filter(move |&a| self.matches(a, key))
    }

    /// The mappings that share `action`'s key, as the Controls list marks them
    /// (`KeyBindsList.KeyEntry.refreshEntry`): none for an unbound one, and none where
    /// both are on their defaults (the debug keys share F3 by design).
    pub fn conflicts(&self, action: Action) -> impl Iterator<Item = Action> + '_ {
        let key = self.get(action);
        let default = self.is_default(action);
        Action::ALL.into_iter().filter(move |&other| {
            other != action
                && !key.is_unbound()
                && self.get(other) == key
                && (!self.is_default(other) || !default)
        })
    }

    /// Sets a mapping by names, as a file keeps them: false if either name is unknown
    /// (the mapping keeps its key).
    pub fn set_by_name(&mut self, action: &str, key: &str) -> bool {
        match (Action::by_name(action), Key::parse(key)) {
            (Some(a), Parsed::Key(k)) => {
                self.set(a, k);
                true
            }
            _ => false,
        }
    }

    /// The bindings for text that names them (`keybind` components), with what the
    /// keyboard's layout calls each keyboard key (`system`, by scancode).
    pub fn bound_keys(&self, system: &dyn Fn(i32) -> Option<String>) -> BoundKeys {
        let mut b = BoundKeys::default();
        for a in Action::ALL {
            let key = self.get(a);
            let (value, mouse) = match key {
                Key::Keyboard(v) => (v, false),
                Key::Mouse(v) => (v, true),
            };
            let named = if mouse { None } else { system(value) };
            b.push(a.name(), &key.name(), value, mouse, named);
        }
        b
    }
}

/// How a key's name is shown (`InputConstants.Key.getDisplayName`): what the keyboard's
/// layout calls it when that is one character (`system`, from SDL), else the game's
/// name for it, translated with `lang` when it has one.
pub fn key_label(key: Key, system: Option<&str>, lang: Option<&dyn Lang>) -> KeyText {
    let (value, mouse) = match key {
        Key::Keyboard(v) => (v, false),
        Key::Mouse(v) => (v, true),
    };
    key_text(&key.name(), value, mouse, system, lang.unwrap_or(&()))
}

/// A key's name as the Controls page shows it: as the game would with its language, and
/// without one in Ruthenium's own words (what SDL calls a keyboard key, `Button 4`).
pub fn key_display(key: Key, system: Option<&str>, lang: Option<&dyn Lang>) -> String {
    let label = key_label(key, system, lang);
    if let Some(lang) = lang {
        return label.resolve(lang);
    }
    match (label, key) {
        (KeyText::Literal(s), _) => s,
        (_, k) if k.is_unbound() => "Not Bound".into(),
        (_, Key::Mouse(1)) => "Left Button".into(),
        (_, Key::Mouse(2)) => "Middle Button".into(),
        (_, Key::Mouse(3)) => "Right Button".into(),
        (_, Key::Mouse(v)) => format!("Button {v}"),
        (_, Key::Keyboard(v)) => system
            .filter(|s| !s.is_empty())
            .map_or_else(|| format!("Key {v}"), str::to_owned),
    }
}

/// Vanilla's `options.txt` (`Options.load`): the lines read with the data fixers for
/// old files (LWJGL 2's key numbers before data version 1344, names without
/// `keyboard` before 1446, `swapHands` before 2558), then every mapping's
/// `key_<name>` line in order; a name that is not a key unbinds its mapping. Returns
/// the bindings, and whether the file had any key lines.
pub fn read_options(text: &str) -> (KeyBinds, bool) {
    let mut binds = KeyBinds::DEFAULT;
    // `Splitter.on(':').limit(2)` into a map: the last line of a name wins.
    let mut lines: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        match lines.iter_mut().find(|(n, _)| n == name) {
            Some(entry) => entry.1 = value.to_owned(),
            None => lines.push((name.to_owned(), value.to_owned())),
        }
    }
    let any = lines.iter().any(|(n, _)| n.starts_with("key_"));
    let version = lines
        .iter()
        .find(|(n, _)| n == "version")
        .and_then(|(_, v)| java_int(v))
        .unwrap_or(0);
    if fix_options(&mut lines, version).is_none() {
        // A fixer failed: vanilla reads nothing of the file.
        return (binds, any);
    }
    for a in Action::ALL {
        let Some((_, value)) = lines
            .iter()
            .find(|(n, _)| n.strip_prefix("key_") == Some(a.name()))
        else {
            continue;
        };
        if *value == binds.get(a).name() {
            continue;
        }
        match Key::parse(value) {
            Parsed::Key(k) => binds.set(a, k),
            Parsed::Invalid => binds.set(a, Key::UNBOUND),
            Parsed::Broken => break,
        }
    }
    (binds, any)
}

/// The data fixers that touch key lines, for a file of data version `version`; `None`
/// where one fails (a value that is not a number, or too short).
fn fix_options(lines: &mut [(String, String)], version: i32) -> Option<()> {
    fn keys(lines: &mut [(String, String)]) -> impl Iterator<Item = &mut String> {
        lines
            .iter_mut()
            .filter(|(n, _)| n.starts_with("key_"))
            .map(|(_, v)| v)
    }
    if version < 1344 {
        // `OptionsKeyLwjgl3Fix`: LWJGL 2's numbers, mouse buttons below zero.
        for v in keys(lines) {
            let old = java_int(v)?;
            *v = if old < 0 {
                match old + 100 {
                    0 => "key.mouse.left".into(),
                    1 => "key.mouse.right".into(),
                    2 => "key.mouse.middle".into(),
                    b => format!("key.mouse.{}", b + 1),
                }
            } else {
                LWJGL2
                    .iter()
                    .find(|(n, _)| *n == old)
                    .map_or("key.unknown", |(_, name)| name)
                    .to_owned()
            };
        }
    }
    if version < 1446 {
        // `OptionsKeyTranslationFix`: `key.w` became `key.keyboard.w`.
        for v in keys(lines) {
            if !v.starts_with("key.mouse") && !v.starts_with("scancode.") {
                let rest = v.get("key.".len()..)?;
                *v = format!("key.keyboard.{rest}");
            }
        }
    }
    if version < 2558 {
        // `OptionsRenameFieldFix`: `key_key.swapHands` became `key_key.swapOffhand`.
        let old = lines.iter().position(|(n, _)| n == "key_key.swapHands");
        if let Some(i) = old {
            let value = lines[i].1.clone();
            for (n, v) in lines.iter_mut() {
                if n == "key_key.swapOffhand" {
                    *v = value.clone();
                }
            }
            if lines.iter().all(|(n, _)| n != "key_key.swapOffhand") {
                lines[i].0 = "key_key.swapOffhand".into();
            } else {
                lines[i].0.clear();
            }
        }
    }
    Some(())
}

/// LWJGL 2's key numbers and the names `OptionsKeyLwjgl3Fix` gives them.
const LWJGL2: &[(i32, &str)] = &[
    (0, "key.unknown"),
    (11, "key.0"),
    (2, "key.1"),
    (3, "key.2"),
    (4, "key.3"),
    (5, "key.4"),
    (6, "key.5"),
    (7, "key.6"),
    (8, "key.7"),
    (9, "key.8"),
    (10, "key.9"),
    (30, "key.a"),
    (40, "key.apostrophe"),
    (48, "key.b"),
    (43, "key.backslash"),
    (14, "key.backspace"),
    (46, "key.c"),
    (58, "key.caps.lock"),
    (51, "key.comma"),
    (32, "key.d"),
    (211, "key.delete"),
    (208, "key.down"),
    (18, "key.e"),
    (207, "key.end"),
    (28, "key.enter"),
    (13, "key.equal"),
    (1, "key.escape"),
    (33, "key.f"),
    (59, "key.f1"),
    (68, "key.f10"),
    (87, "key.f11"),
    (88, "key.f12"),
    (100, "key.f13"),
    (101, "key.f14"),
    (102, "key.f15"),
    (103, "key.f16"),
    (104, "key.f17"),
    (105, "key.f18"),
    (113, "key.f19"),
    (60, "key.f2"),
    (61, "key.f3"),
    (62, "key.f4"),
    (63, "key.f5"),
    (64, "key.f6"),
    (65, "key.f7"),
    (66, "key.f8"),
    (67, "key.f9"),
    (34, "key.g"),
    (41, "key.grave.accent"),
    (35, "key.h"),
    (199, "key.home"),
    (23, "key.i"),
    (210, "key.insert"),
    (36, "key.j"),
    (37, "key.k"),
    (82, "key.keypad.0"),
    (79, "key.keypad.1"),
    (80, "key.keypad.2"),
    (81, "key.keypad.3"),
    (75, "key.keypad.4"),
    (76, "key.keypad.5"),
    (77, "key.keypad.6"),
    (71, "key.keypad.7"),
    (72, "key.keypad.8"),
    (73, "key.keypad.9"),
    (78, "key.keypad.add"),
    (83, "key.keypad.decimal"),
    (181, "key.keypad.divide"),
    (156, "key.keypad.enter"),
    (141, "key.keypad.equal"),
    (55, "key.keypad.multiply"),
    (74, "key.keypad.subtract"),
    (38, "key.l"),
    (203, "key.left"),
    (56, "key.left.alt"),
    (26, "key.left.bracket"),
    (29, "key.left.control"),
    (42, "key.left.shift"),
    (219, "key.left.win"),
    (50, "key.m"),
    (12, "key.minus"),
    (49, "key.n"),
    (69, "key.num.lock"),
    (24, "key.o"),
    (25, "key.p"),
    (209, "key.page.down"),
    (201, "key.page.up"),
    (197, "key.pause"),
    (52, "key.period"),
    (183, "key.print.screen"),
    (16, "key.q"),
    (19, "key.r"),
    (205, "key.right"),
    (184, "key.right.alt"),
    (27, "key.right.bracket"),
    (157, "key.right.control"),
    (54, "key.right.shift"),
    (220, "key.right.win"),
    (31, "key.s"),
    (70, "key.scroll.lock"),
    (39, "key.semicolon"),
    (53, "key.slash"),
    (57, "key.space"),
    (20, "key.t"),
    (15, "key.tab"),
    (22, "key.u"),
    (200, "key.up"),
    (47, "key.v"),
    (17, "key.w"),
    (45, "key.x"),
    (21, "key.y"),
    (44, "key.z"),
];

#[cfg(test)]
#[allow(clippy::type_complexity)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// What a key's display name is made of, as the golden file writes it.
    #[derive(Debug)]
    enum Shown {
        Literal(&'static str),
        Translate(&'static str, &'static [&'static str]),
    }

    include!("keybinds_golden.rs");

    #[test]
    fn every_mapping_is_vanillas_with_its_heading_default_and_order() {
        assert_eq!(MAPPINGS.len(), self::MAPPINGS.len());
        for (i, &(name, category, default, order)) in self::MAPPINGS.iter().enumerate() {
            let a = Action::ALL[i];
            assert_eq!(a as usize, i);
            assert_eq!(a.name(), name);
            assert_eq!(a.category().id(), category, "{name}");
            assert_eq!(a.default_key().name(), default, "{name}");
            assert_eq!(a.order(), order, "{name}");
            assert_eq!(Action::by_name(name), Some(a));
            assert!(!a.label().is_empty());
        }
        let sorted: Vec<&str> = Action::SORTED.iter().map(|a| a.name()).collect();
        assert_eq!(sorted, SORTED);
        // Headings come in their order, each once.
        let mut seen: Vec<Category> = Vec::new();
        for a in Action::SORTED {
            if seen.last() != Some(&a.category()) {
                assert!(!seen.contains(&a.category()));
                seen.push(a.category());
            }
        }
        assert_eq!(seen, Category::ALL);
        assert_eq!(Action::by_name("key.mystery"), None);
        assert_eq!(Action::Hotbar7.hotbar_slot(), Some(6));
        assert_eq!(Action::Jump.hotbar_slot(), None);
    }

    /// With the prepared game (`RUTHENIUM_GAME_DIR`): sorting by heading, order and the
    /// game's English names gives the list's order.
    #[test]
    fn the_games_english_names_sort_as_the_list() {
        let Some(game) = std::env::var_os("RUTHENIUM_GAME_DIR") else {
            eprintln!("RUTHENIUM_GAME_DIR not set; skipped");
            return;
        };
        let jar = std::path::PathBuf::from(game).join("versions/26.3/client.jar");
        let jar = ruthenium_assets::zip::Zip::open(&jar).unwrap();
        let i = jar.find("assets/minecraft/lang/en_us.json").unwrap();
        let lang: HashMap<String, String> = serde_json::from_slice(&jar.read(i).unwrap()).unwrap();
        let mut all = Action::ALL;
        let heading = |a: &Action| Category::ALL.iter().position(|&c| c == a.category());
        // (Java compares strings by UTF-16 units; these are ASCII.)
        all.sort_by(|a, b| {
            (heading(a), a.order(), &lang[a.name()]).cmp(&(heading(b), b.order(), &lang[b.name()]))
        });
        assert_eq!(all, Action::SORTED);
        for c in Category::ALL {
            assert!(lang.contains_key(&c.lang_key()), "{c:?}");
        }
    }

    #[test]
    fn keys_are_named_and_read_as_the_game_does() {
        // Every named key, both ways.
        for &(name, mouse, value) in NAMES {
            let key = if mouse {
                Key::Mouse(value as i32)
            } else {
                Key::Keyboard(value as i32)
            };
            assert_eq!(key.name(), name);
            assert_eq!(Key::parse(name), Parsed::Key(key), "{name}");
        }
        assert_eq!(NAMES.len(), KEYBOARD.len() + MOUSE.len());
        for &(name, want) in PARSED {
            let got = match Key::parse(name) {
                Parsed::Key(Key::Keyboard(v)) => Some((false, i64::from(v))),
                Parsed::Key(Key::Mouse(v)) => Some((true, i64::from(v))),
                _ => None,
            };
            assert_eq!(got, want, "{name:?}");
        }
        assert_eq!(Key::parse("key.keyboard"), Parsed::Broken);
        assert_eq!(Key::parse("key.mouse"), Parsed::Broken);
        assert_eq!(Key::parse("key.keyboard."), Parsed::Invalid);
        assert_eq!(Key::Keyboard(300).name(), "key.keyboard.300");
        assert_eq!(Key::Mouse(9).name(), "key.mouse.9");
        assert_eq!(Key::Keyboard(-3).name(), "key.keyboard.-3");
        assert_eq!(Key::scancode(26), Key::Keyboard(26));
        assert_eq!(Key::button(5), Key::Mouse(5));
        assert!(Key::UNBOUND.is_unbound() && !Key::Mouse(0).is_unbound());
    }

    #[test]
    fn options_txt_reads_as_the_game_reads_it() {
        for &(text, want, failed) in IMPORTED {
            let (binds, any) = read_options(text);
            let got: Vec<(&str, String)> = Action::ALL
                .iter()
                .filter(|&&a| !binds.is_default(a))
                .map(|&a| (a.name(), binds.get(a).name()))
                .collect();
            let mut want: Vec<(&str, String)> =
                want.iter().map(|&(a, k)| (a, k.to_owned())).collect();
            want.sort_by_key(|(a, _)| Action::by_name(a).unwrap() as usize);
            assert_eq!(got, want, "{text:?} (failed: {failed})");
            assert!(any);
        }
        assert_eq!(
            read_options("version:4671\nfov:0.5\n"),
            (KeyBinds::DEFAULT, false)
        );
        assert_eq!(read_options(""), (KeyBinds::DEFAULT, false));
    }

    #[test]
    fn display_names_are_made_as_the_game_makes_them() {
        // The language has names for the mouse's first three buttons only.
        let mut lang = HashMap::new();
        for (_, _, shown) in DISPLAYED {
            if let Shown::Translate(key, []) = shown {
                lang.insert(key.to_string(), format!("<{key}>"));
            }
        }
        // The golden file was written on Linux. The game names a few keys by the system
        // it runs on (InputQuirks.keyboardTranslationKey): Linux's names there, for the
        // keys it calls Windows keys, are the Command keys on a Mac and the Windows keys on
        // Windows, and a Mac's Alt keys are its Option keys.
        let here = |linux: &'static str| -> &'static str {
            let names: &[(&str, &str)] = if cfg!(target_os = "macos") {
                &[
                    ("key.keyboard.left.alt", "key.keyboard.left.option"),
                    ("key.keyboard.right.alt", "key.keyboard.right.option"),
                    ("key.keyboard.left.meta", "key.keyboard.left.command"),
                    ("key.keyboard.right.meta", "key.keyboard.right.command"),
                ]
            } else if cfg!(windows) {
                &[
                    ("key.keyboard.left.meta", "key.keyboard.left.windows"),
                    ("key.keyboard.right.meta", "key.keyboard.right.windows"),
                ]
            } else {
                &[]
            };
            names
                .iter()
                .find(|(from, _)| *from == linux)
                .map_or(linux, |(_, to)| to)
        };
        for &(name, system, ref shown) in DISPLAYED {
            let key = match Key::parse(name) {
                Parsed::Key(k) => k,
                other => panic!("{name}: {other:?}"),
            };
            let system = Some(system).filter(|s| !s.is_empty());
            let got = key_label(key, system, Some(&lang));
            let want = match shown {
                Shown::Literal(s) => KeyText::Literal(s.to_string()),
                Shown::Translate("key.mouse", [n]) => KeyText::Button(n.parse().unwrap()),
                Shown::Translate(k, []) => KeyText::Translate(here(k).to_string()),
                other => panic!("{other:?}"),
            };
            assert_eq!(got, want, "{name}");
        }
        // Another layout's letters, in capitals; names longer than a character are the
        // game's.
        assert_eq!(
            key_label(Key::Keyboard(26), Some("z"), None),
            KeyText::Literal("Z".into())
        );
        assert_eq!(
            key_label(Key::Keyboard(51), Some("ö"), None),
            KeyText::Literal("Ö".into())
        );
        assert_eq!(
            key_label(Key::Keyboard(225), Some("Left Shift"), None),
            KeyText::Translate("key.keyboard.left.shift".into())
        );
        // No key is its own name, whatever the layout says.
        assert_eq!(
            key_label(Key::UNBOUND, Some("x"), None),
            KeyText::Translate("key.keyboard.unknown".into())
        );
    }

    #[test]
    fn the_controls_page_names_keys_without_the_games_language() {
        let shown = |k, system| key_display(k, system, None);
        assert_eq!(shown(Key::Keyboard(26), Some("W")), "W");
        assert_eq!(shown(Key::Keyboard(225), Some("Left Shift")), "Left Shift");
        assert_eq!(shown(Key::UNBOUND, Some("")), "Not Bound");
        assert_eq!(shown(Key::Mouse(1), None), "Left Button");
        assert_eq!(shown(Key::Mouse(3), None), "Right Button");
        assert_eq!(shown(Key::Mouse(2), None), "Middle Button");
        assert_eq!(shown(Key::Mouse(4), None), "Button 4");
        assert_eq!(shown(Key::Keyboard(300), None), "Key 300");
        // With the language, the game's own.
        let mut lang = HashMap::new();
        lang.insert(
            "key.keyboard.left.shift".to_owned(),
            "Shift (left)".to_owned(),
        );
        lang.insert("key.mouse".to_owned(), "Mouse %1$s".to_owned());
        let with = |k, system| key_display(k, system, Some(&lang));
        assert_eq!(with(Key::Keyboard(225), Some("Left Shift")), "Shift (left)");
        assert_eq!(with(Key::Mouse(5), None), "Mouse 5");
        assert_eq!(with(Key::Keyboard(26), Some("w")), "W");
    }

    #[test]
    fn conflicts_are_marked_as_the_controls_list_marks_them() {
        let mut b = KeyBinds::DEFAULT;
        let of = |b: &KeyBinds, a| b.conflicts(a).collect::<Vec<_>>();
        // Defaults that share a key are not marked (F3, the middle button, C and X).
        for a in Action::ALL {
            assert_eq!(of(&b, a), [], "{a:?}");
        }
        b.set(Action::Jump, Key::Keyboard(26));
        assert_eq!(of(&b, Action::Jump), [Action::Forward]);
        assert_eq!(of(&b, Action::Forward), [Action::Jump]);
        // A changed one on a default pair marks both of them, and they it.
        b.set(Action::Drop, Key::Keyboard(60));
        assert_eq!(
            of(&b, Action::Drop),
            [Action::DebugOverlay, Action::DebugModifier]
        );
        assert_eq!(of(&b, Action::DebugOverlay), [Action::Drop]);
        // Unbound ones never are.
        b.set(Action::Chat, Key::UNBOUND);
        b.set(Action::Command, Key::UNBOUND);
        assert_eq!(of(&b, Action::Chat), []);
        assert_eq!(of(&b, Action::SmoothCamera), []);
        // A mouse button is not the key of the same number.
        b.set(Action::Use, Key::Mouse(26));
        assert_eq!(of(&b, Action::Use), []);
        assert_eq!(b.changed(), 5);
        b.reset(Action::Jump);
        assert!(b.is_default(Action::Jump) && of(&b, Action::Forward).is_empty());
        b.reset_all();
        assert_eq!(b, KeyBinds::DEFAULT);
    }

    #[test]
    fn a_key_presses_every_mapping_on_it() {
        let mut b = KeyBinds::DEFAULT;
        let on = |b: &KeyBinds, k| b.on(k).collect::<Vec<_>>();
        assert_eq!(
            on(&b, Key::Keyboard(60)),
            [Action::DebugOverlay, Action::DebugModifier]
        );
        assert_eq!(
            on(&b, Key::Mouse(2)),
            [Action::PickItem, Action::SpectatorHotbar]
        );
        assert_eq!(on(&b, Key::Keyboard(26)), [Action::Forward]);
        // Nothing presses unbound ones.
        assert_eq!(on(&b, Key::UNBOUND), []);
        b.set(Action::Attack, Key::Keyboard(13));
        assert_eq!(on(&b, Key::Keyboard(13)), [Action::Attack]);
        assert_eq!(on(&b, Key::Mouse(1)), []);
        assert!(b.matches(Action::Attack, Key::Keyboard(13)));
        assert!(!b.matches(Action::SmoothCamera, Key::UNBOUND));
        assert!(b.set_by_name("key.use", "key.mouse.4"));
        assert_eq!(b.get(Action::Use), Key::Mouse(4));
        assert!(!b.set_by_name("key.use", "nonsense"));
        assert!(!b.set_by_name("key.nothing", "key.mouse.4"));
        assert_eq!(b.get(Action::Use), Key::Mouse(4));
    }

    #[test]
    fn text_that_names_a_binding_shows_its_key() {
        use ruthenium_protocol::text::{StyledText, WithKeys};
        let mut b = KeyBinds::DEFAULT;
        b.set(Action::Sneak, Key::Keyboard(29));
        b.set(Action::Use, Key::Mouse(5));
        let system = |v: i32| (v == 29).then(|| "Y".to_owned());
        let keys = b.bound_keys(&system);
        let mut lang = HashMap::new();
        lang.insert("key.mouse".to_owned(), "Button %1$s".to_owned());
        lang.insert("key.keyboard.space".to_owned(), "Space".to_owned());
        let with = WithKeys {
            lang: &lang,
            keys: &keys,
        };
        let shown = |binding: &str| {
            StyledText::from_json_in(&serde_json::json!({ "keybind": binding }), &with).plain()
        };
        // A German layout's Z is where the US one has Y.
        assert_eq!(shown("key.sneak"), "Y");
        assert_eq!(shown("key.use"), "Button 5");
        assert_eq!(shown("key.jump"), "Space");
        assert_eq!(shown("key.mystery"), "key.mystery");
        // The defaults the text module falls back on are the game's.
        let defaults = KeyBinds::DEFAULT.bound_keys(&|_| None);
        for a in Action::ALL {
            if let Some(name) = ruthenium_protocol::text::default_key(a.name()) {
                assert_eq!(defaults.get(a.name()).unwrap().name, name, "{a:?}");
            }
        }
    }
}
