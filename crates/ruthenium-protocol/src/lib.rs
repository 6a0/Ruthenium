//! Wire and file formats shared by the client.
//!
//! - [`wire`]: VarInts, length-prefixed strings and packet frames.
//! - [`nbt`]: named binary tags as Java writes them to files (`servers.dat`), with
//!   Java's modified UTF-8.
//! - [`text`]: text components, flattened into styled spans for drawing.
//! - [`status`]: the server list's status response.
//! - [`item`]: item stacks and their data components, as play packets carry them.
//! - [`profile`]: profiles whose skin is shown.
//! - [`packets`]: the login and configuration packets, and those play shares with them.
//! - [`commands`]: the server's command graph.

#![forbid(unsafe_code)]

pub mod commands;
pub mod item;
pub mod nbt;
pub mod packets;
pub mod profile;
pub mod status;
pub mod text;
pub mod wire;
