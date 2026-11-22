//! When the player counts as loaded into a level (`LevelLoadTracker`): after the server
//! says chunks are coming (`LEVEL_CHUNKS_LOAD_START`), once the chunk section the camera
//! is in is ready, or 30 seconds after the level started, whichever comes first. Until
//! then the local player doesn't tick or send movement; the tick it is ready, the client
//! sends `player_loaded`.
//!
//! Vanilla waits for the camera's section to be compiled for drawing; Ruthenium has no
//! terrain meshes yet, so the chunk having arrived stands in for it.

/// How long the client waits before letting the player in anyway (`CLIENT_WAIT_TIMEOUT_MS`).
pub const CLIENT_WAIT_TIMEOUT_MS: i64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelLoad {
    WaitingForServer { timeout_at: i64 },
    WaitingForPlayerChunk { timeout_at: i64 },
    Ready { at: i64 },
}

impl LevelLoad {
    /// A new level (`startClientLoad`).
    pub fn start(now_ms: i64) -> LevelLoad {
        LevelLoad::WaitingForServer {
            timeout_at: now_ms + CLIENT_WAIT_TIMEOUT_MS,
        }
    }

    /// The server's `LEVEL_CHUNKS_LOAD_START` game event.
    pub fn loading_packets_received(&mut self) {
        if let LevelLoad::WaitingForServer { timeout_at } = *self {
            *self = LevelLoad::WaitingForPlayerChunk { timeout_at };
        }
    }

    /// A tick (`tickClientLoad`). `no_wait`: the camera is outside the build height, or
    /// the player is a spectator or dead, so there is no chunk to wait for.
    pub fn tick(&mut self, now_ms: i64, player_chunk_ready: bool, no_wait: bool) {
        if let LevelLoad::WaitingForPlayerChunk { timeout_at } = *self {
            let timed_out = now_ms > timeout_at;
            if timed_out {
                eprintln!(
                    "[session] timed out waiting for the player's chunk; letting the player in anyway"
                );
            }
            if timed_out || no_wait || player_chunk_ready {
                *self = LevelLoad::Ready { at: now_ms };
            }
        }
    }

    /// `isLevelReady` (vanilla's multiplayer tracker has no close delay).
    pub fn is_ready(&self, now_ms: i64) -> bool {
        matches!(*self, LevelLoad::Ready { at } if now_ms >= at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_the_server_then_the_chunk() {
        let mut l = LevelLoad::start(0);
        // The chunk alone isn't enough before the server says loading started.
        l.tick(10, true, false);
        assert!(!l.is_ready(10));
        l.loading_packets_received();
        l.tick(20, false, false);
        assert!(!l.is_ready(20));
        l.tick(70, true, false);
        assert!(l.is_ready(70));
    }

    #[test]
    fn gives_up_waiting_after_30_seconds() {
        let mut l = LevelLoad::start(0);
        l.loading_packets_received();
        l.tick(30_000, false, false);
        assert!(!l.is_ready(30_000));
        l.tick(30_001, false, false);
        assert!(l.is_ready(30_001));
        // Without the server's event, the timeout never applies (as in vanilla).
        let mut l = LevelLoad::start(0);
        l.tick(99_999, false, true);
        assert!(!l.is_ready(99_999));
    }
}
