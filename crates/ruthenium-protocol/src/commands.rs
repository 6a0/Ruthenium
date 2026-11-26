//! The server's commands (`ClientboundCommandsPacket`): a graph of nodes, each a literal
//! word or an argument of some type, with the nodes that may follow it and the node it
//! redirects to. The client parses what the player types against it to find the
//! arguments to sign (`SignableCommand`).
//!
//! Every argument type of 26.3's `command_argument_type` registry is read with its
//! properties; a node of an unknown type makes the packet unreadable, as it makes
//! vanilla's reading go wrong.

use ruthenium_data::v26_3::registries::CommandArgumentType as T;

use crate::wire::{MAX_STRING, Reader, WireError};

/// An argument type and the properties the server sent with it.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgumentType {
    Bool,
    Float {
        min: f32,
        max: f32,
    },
    Double {
        min: f64,
        max: f64,
    },
    Integer {
        min: i32,
        max: i32,
    },
    Long {
        min: i64,
        max: i64,
    },
    String(StringKind),
    Entity {
        single: bool,
        players_only: bool,
    },
    ScoreHolder {
        multiple: bool,
    },
    Time {
        min: i32,
    },
    /// The `resource`, `resource_key`, `resource_or_tag`, `resource_or_tag_key` and
    /// `resource_selector` types, with the registry they name.
    Resource {
        kind: T,
        registry: String,
    },
    /// A type without properties.
    Plain(T),
}

/// `StringArgumentType.StringType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringKind {
    /// One unquoted word.
    Word,
    /// A word, or a quoted phrase.
    Phrase,
    /// Everything left.
    Greedy,
}

/// What a node is.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeKind {
    Root,
    Literal(String),
    Argument {
        name: String,
        kind: ArgumentType,
        /// The suggestion provider it asks the server or the client for.
        suggestions: Option<String>,
    },
}

/// One node of the graph.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandNode {
    pub kind: NodeKind,
    /// Whether a command may end here.
    pub executable: bool,
    /// Only players allowed restricted commands may use it.
    pub restricted: bool,
    /// The nodes that may follow, as indices of the tree's nodes.
    pub children: Vec<usize>,
    /// Where parsing goes on instead of the children.
    pub redirect: Option<usize>,
}

/// The server's command graph.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommandTree {
    pub nodes: Vec<CommandNode>,
    pub root: usize,
}

const MASK_TYPE: u8 = 3;
const FLAG_EXECUTABLE: u8 = 4;
const FLAG_REDIRECT: u8 = 8;
const FLAG_SUGGESTIONS: u8 = 16;
const FLAG_RESTRICTED: u8 = 32;

impl CommandTree {
    /// The packet's body.
    pub fn read(r: &mut Reader<'_>) -> Result<CommandTree, WireError> {
        let n = r.count(r.remaining())?;
        let mut nodes = Vec::with_capacity(n);
        for _ in 0..n {
            nodes.push(read_node(r)?);
        }
        let root = index(r.varint()?, n)?;
        let tree = CommandTree { nodes, root };
        let in_range = |i: &usize| *i < n;
        if !tree
            .nodes
            .iter()
            .all(|node| node.children.iter().all(in_range) && node.redirect.iter().all(in_range))
        {
            return Err(WireError::Invalid("command node index"));
        }
        if tree.nodes[root].kind != NodeKind::Root {
            return Err(WireError::Invalid("command tree root"));
        }
        Ok(tree)
    }
}

fn index(i: i32, n: usize) -> Result<usize, WireError> {
    usize::try_from(i)
        .ok()
        .filter(|i| *i < n)
        .ok_or(WireError::Invalid("command node index"))
}

fn read_node(r: &mut Reader<'_>) -> Result<CommandNode, WireError> {
    let flags = r.u8()?;
    let count = r.count(r.remaining())?;
    let mut children = Vec::with_capacity(count);
    for _ in 0..count {
        children.push(
            usize::try_from(r.varint()?).map_err(|_| WireError::Invalid("command node index"))?,
        );
    }
    let redirect = if flags & FLAG_REDIRECT != 0 {
        Some(usize::try_from(r.varint()?).map_err(|_| WireError::Invalid("command node index"))?)
    } else {
        None
    };
    let kind = match flags & MASK_TYPE {
        0 => NodeKind::Root,
        1 => NodeKind::Literal(r.string(MAX_STRING)?.to_owned()),
        2 => {
            let name = r.string(MAX_STRING)?.to_owned();
            let kind = argument_type(r)?;
            let suggestions = if flags & FLAG_SUGGESTIONS != 0 {
                Some(r.identifier()?)
            } else {
                None
            };
            NodeKind::Argument {
                name,
                kind,
                suggestions,
            }
        }
        _ => return Err(WireError::Invalid("command node type")),
    };
    Ok(CommandNode {
        kind,
        executable: flags & FLAG_EXECUTABLE != 0,
        restricted: flags & FLAG_RESTRICTED != 0,
        children,
        redirect,
    })
}

/// An argument type's id and properties (`ArgumentTypeInfo.deserializeFromNetwork`).
fn argument_type(r: &mut Reader<'_>) -> Result<ArgumentType, WireError> {
    let id = u8::try_from(r.varint()?).map_err(|_| WireError::Invalid("argument type"))?;
    let kind = T::from_id(id.into()).ok_or(WireError::Invalid("argument type"))?;
    // `ArgumentUtils.createNumberFlags`: bit 0 a minimum, bit 1 a maximum.
    let bounds = |r: &mut Reader<'_>| -> Result<(bool, bool), WireError> {
        let flags = r.u8()?;
        Ok((flags & 1 != 0, flags & 2 != 0))
    };
    Ok(match kind {
        T::BrigadierBool => ArgumentType::Bool,
        T::BrigadierFloat => {
            let (lo, hi) = bounds(r)?;
            ArgumentType::Float {
                min: if lo { r.f32()? } else { -f32::MAX },
                max: if hi { r.f32()? } else { f32::MAX },
            }
        }
        T::BrigadierDouble => {
            let (lo, hi) = bounds(r)?;
            ArgumentType::Double {
                min: if lo { r.f64()? } else { -f64::MAX },
                max: if hi { r.f64()? } else { f64::MAX },
            }
        }
        T::BrigadierInteger => {
            let (lo, hi) = bounds(r)?;
            ArgumentType::Integer {
                min: if lo { r.i32()? } else { i32::MIN },
                max: if hi { r.i32()? } else { i32::MAX },
            }
        }
        T::BrigadierLong => {
            let (lo, hi) = bounds(r)?;
            ArgumentType::Long {
                min: if lo { r.i64()? } else { i64::MIN },
                max: if hi { r.i64()? } else { i64::MAX },
            }
        }
        T::BrigadierString => ArgumentType::String(match r.varint()? {
            0 => StringKind::Word,
            1 => StringKind::Phrase,
            2 => StringKind::Greedy,
            _ => return Err(WireError::Invalid("string argument type")),
        }),
        T::Entity => {
            let flags = r.u8()?;
            ArgumentType::Entity {
                single: flags & 1 != 0,
                players_only: flags & 2 != 0,
            }
        }
        T::ScoreHolder => ArgumentType::ScoreHolder {
            multiple: r.u8()? & 1 != 0,
        },
        T::Time => ArgumentType::Time { min: r.i32()? },
        T::ResourceOrTag
        | T::ResourceOrTagKey
        | T::Resource
        | T::ResourceKey
        | T::ResourceSelector => ArgumentType::Resource {
            kind,
            registry: r.identifier()?,
        },
        other => ArgumentType::Plain(other),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{write_string, write_varint};

    fn read(bytes: &[u8]) -> Result<CommandTree, WireError> {
        let mut r = Reader::new(bytes);
        let t = CommandTree::read(&mut r)?;
        r.finish()?;
        Ok(t)
    }

    /// `count` nodes, the root first: `say <message>` and `n <i: 1..64> <w: word>`.
    fn tree(argument_id: i32, last_child: i32) -> Vec<u8> {
        let mut o = Vec::new();
        write_varint(&mut o, 6);
        // The root.
        o.extend_from_slice(&[0, 2, 1, 3]);
        // say: executable no, one child.
        o.extend_from_slice(&[1, 1, 2]);
        write_string(&mut o, "say");
        // <message>: an executable argument.
        o.extend_from_slice(&[2 | 4, 0]);
        write_string(&mut o, "message");
        write_varint(&mut o, argument_id);
        // n, redirecting to the root, restricted.
        o.extend_from_slice(&[1 | 8 | 32, 1, 4, 0]);
        write_string(&mut o, "n");
        // <i>: an integer from 1 to 64.
        o.extend_from_slice(&[2, 1]);
        write_varint(&mut o, last_child);
        write_string(&mut o, "i");
        o.extend_from_slice(&[3, 3, 0, 0, 0, 1, 0, 0, 0, 64]);
        // <w>: a word, with suggestions.
        o.extend_from_slice(&[2 | 4 | 16, 0]);
        write_string(&mut o, "w");
        o.extend_from_slice(&[5, 0]);
        write_string(&mut o, "ask_server");
        // The root's index.
        o.push(0);
        o
    }

    #[test]
    fn reads_nodes_with_their_arguments_properties() {
        let t = read(&tree(20, 5)).unwrap();
        assert_eq!((t.nodes.len(), t.root), (6, 0));
        assert_eq!(t.nodes[0].children, [1, 3]);
        assert_eq!(t.nodes[1].kind, NodeKind::Literal("say".into()));
        assert!(t.nodes[2].executable && !t.nodes[1].executable);
        assert_eq!(
            t.nodes[2].kind,
            NodeKind::Argument {
                name: "message".into(),
                kind: ArgumentType::Plain(T::Message),
                suggestions: None,
            }
        );
        assert_eq!(
            (t.nodes[3].redirect, t.nodes[3].restricted),
            (Some(0), true)
        );
        assert_eq!(
            t.nodes[4].kind,
            NodeKind::Argument {
                name: "i".into(),
                kind: ArgumentType::Integer { min: 1, max: 64 },
                suggestions: None,
            }
        );
        assert_eq!(
            t.nodes[5].kind,
            NodeKind::Argument {
                name: "w".into(),
                kind: ArgumentType::String(StringKind::Word),
                suggestions: Some("minecraft:ask_server".into()),
            }
        );
        // An argument type the registry doesn't have, a child that isn't there.
        assert!(read(&tree(62, 5)).is_err());
        assert!(read(&tree(20, 6)).is_err());
        // A root that is a literal.
        let mut not_root = tree(20, 5);
        *not_root.last_mut().unwrap() = 1;
        assert!(read(&not_root).is_err());
    }

    #[test]
    fn numbers_without_bounds_take_their_types_extremes() {
        let one = |id: u8, props: &[u8]| {
            // Two nodes: the argument, and the root with it as its child.
            let mut o = vec![2, 2, 0];
            write_string(&mut o, "x");
            o.push(id);
            o.extend_from_slice(props);
            o.extend_from_slice(&[0, 1, 0, 1]);
            o
        };
        let kind = |b: Vec<u8>| match read(&b).unwrap().nodes[0].kind.clone() {
            NodeKind::Argument { kind, .. } => kind,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            kind(one(1, &[0])),
            ArgumentType::Float {
                min: -f32::MAX,
                max: f32::MAX
            }
        );
        assert_eq!(
            kind(one(2, &[2, 0x40, 0x24, 0, 0, 0, 0, 0, 0])),
            ArgumentType::Double {
                min: -f64::MAX,
                max: 10.0
            }
        );
        assert_eq!(
            kind(one(4, &[1, 0, 0, 0, 0, 0, 0, 0, 7])),
            ArgumentType::Long {
                min: 7,
                max: i64::MAX
            }
        );
        assert_eq!(
            kind(one(6, &[3])),
            ArgumentType::Entity {
                single: true,
                players_only: true
            }
        );
        assert_eq!(
            kind(one(31, &[1])),
            ArgumentType::ScoreHolder { multiple: true }
        );
        assert_eq!(
            kind(one(43, &[0, 0, 0, 20])),
            ArgumentType::Time { min: 20 }
        );
        let mut res = vec![];
        write_string(&mut res, "minecraft:item");
        assert_eq!(
            kind(one(46, &res)),
            ArgumentType::Resource {
                kind: T::Resource,
                registry: "minecraft:item".into()
            }
        );
        assert!(read(&one(5, &[3])).is_err());
    }
}
