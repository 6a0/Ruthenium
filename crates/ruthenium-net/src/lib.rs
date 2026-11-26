//! Networking.
//!
//! - [`address`]: server addresses as players type them (`host`, `host:port`,
//!   `[v6]:port`).
//! - [`resolve`]: the system resolver with Minecraft's SRV redirect and Mojang's list
//!   of blocked servers.
//! - [`ping`]: the server list's status ping (and the legacy one), on a background thread.
//! - [`lan`]: games open to the local network, heard on their multicast group.
//! - [`server_list`]: the saved list, `servers.dat`.
//! - [`connection`]: a connection for playing, from the handshake into play, on its own
//!   thread.
//! - [`crypt`], [`transport`]: the login's cryptography; framing, compression and
//!   encryption.

#![forbid(unsafe_code)]

pub mod address;
pub mod connection;
pub mod crypt;
pub mod lan;
pub mod ping;
pub mod resolve;
pub mod server_list;
pub mod transport;

pub use address::ServerAddress;
/// The protocol version this client speaks (servers reporting another are incompatible).
pub use ruthenium_data::v26_3::version::PROTOCOL_VERSION;
