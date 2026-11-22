//! The game thread: everything vanilla's main thread does with a server connection.
//!
//! - [`session`]: one session with a server, from connecting into play: the
//!   configuration phase (registries, tags, known packs, features, cookies, resource-pack
//!   answers, the code of conduct) and the switch into play and back.
//!
//! The tick scheduler, physics and the local player come here next (PLAN Stage 2).

#![forbid(unsafe_code)]

pub mod session;
