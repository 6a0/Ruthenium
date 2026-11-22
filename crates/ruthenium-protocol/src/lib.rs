//! Wire and file formats shared by the client.
//!
//! - [`wire`]: VarInts, length-prefixed strings and packet frames.
//! - [`nbt`]: named binary tags as Java writes them to files (`servers.dat`), with
//!   Java's modified UTF-8.
//! - [`text`]: text components, flattened into styled spans for drawing.
//! - [`status`]: the server list's status response.
//! - [`packets`]: the login and configuration packets, and those play shares with them.

#![forbid(unsafe_code)]

pub mod nbt;
pub mod packets;
pub mod status;
pub mod text;
pub mod wire;
