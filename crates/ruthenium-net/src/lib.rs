//! Networking.
//!
//! - [`address`]: server addresses as players type them (`host`, `host:port`,
//!   `[v6]:port`).
//! - [`resolve`]: the system resolver with Minecraft's SRV redirect.
//! - [`ping`]: the server list's status ping, on a background thread.
//! - [`server_list`]: the saved list, `servers.dat`.

#![forbid(unsafe_code)]

pub mod address;
pub mod ping;
pub mod resolve;
pub mod server_list;

pub use address::ServerAddress;
/// The protocol version this client speaks (servers reporting another are incompatible).
pub use ruthenium_data::v26_3::version::PROTOCOL_VERSION;
