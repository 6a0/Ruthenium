//! Block changes the client predicts and the server confirms
//! (`BlockStatePredictionHandler` with `ClientLevel`'s part of it).
//!
//! An action that may change blocks (breaking, using an item on a block, using an item)
//! runs under a new sequence number, which its packet carries. A block the client
//! changes meanwhile keeps the state the server last gave it, and the player's
//! position then. Until the server acknowledges that sequence, its block updates for
//! those positions only replace the kept state; the acknowledgement puts the kept
//! states back where they differ, moving the player back to where it stood if the
//! restored block would hold it (unless a teleport came after the action).

use crate::blocks::Blocks;
use crate::player::LocalPlayer;
use crate::world::Level;

/// What the server last said a predicted block is (`ServerVerifiedState`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Verified {
    sequence: i32,
    state: u32,
    player_position: [f64; 3],
}

/// The client level's predictions.
#[derive(Debug, Clone, PartialEq)]
pub struct Predictions {
    /// In the order the positions were first predicted. (Vanilla keeps them in a hash
    /// map; the order only matters when one acknowledgement restores several blocks
    /// that each would move the player.)
    verified: Vec<([i32; 3], Verified)>,
    sequence: i32,
    predicting: bool,
    last_teleport_sequence: i32,
}

impl Default for Predictions {
    fn default() -> Predictions {
        Predictions {
            verified: Vec::new(),
            sequence: 0,
            predicting: false,
            last_teleport_sequence: -1,
        }
    }
}

impl Predictions {
    pub fn new() -> Predictions {
        Predictions::default()
    }

    /// The last sequence number given out.
    pub fn sequence(&self) -> i32 {
        self.sequence
    }

    /// Starts the count somewhere else (a replay that joins a running session).
    pub fn set_sequence(&mut self, sequence: i32) {
        self.sequence = sequence;
    }

    /// `startPredicting`: the next sequence number, for the action's packet.
    pub fn start(&mut self) -> i32 {
        self.sequence += 1;
        self.predicting = true;
        self.sequence
    }

    /// `close`: the action's prediction is over.
    pub fn end(&mut self) {
        self.predicting = false;
    }

    /// How many blocks wait for the server.
    pub fn pending(&self) -> usize {
        self.verified.len()
    }

    /// `ClientLevel.setBlock`: the client changes a block. While predicting, a change
    /// keeps the server's state (the first time the block changes) and moves the block
    /// on to this sequence. True if the block changed.
    pub fn set_block(
        &mut self,
        level: &mut Level,
        pos: [i32; 3],
        state: u32,
        player_position: [f64; 3],
    ) -> bool {
        let Some(old) = level.replace_block(pos[0], pos[1], pos[2], state) else {
            return false;
        };
        if self.predicting {
            let sequence = self.sequence;
            match self.verified.iter_mut().find(|(p, _)| *p == pos) {
                Some((_, v)) => v.sequence = sequence,
                None => self.verified.push((
                    pos,
                    Verified {
                        sequence,
                        state: old,
                        player_position,
                    },
                )),
            }
        }
        true
    }

    /// `setServerVerifiedBlockState`: a block update from the server. A predicted
    /// block keeps it for the acknowledgement; any other is set at once.
    pub fn server_block(&mut self, level: &mut Level, pos: [i32; 3], state: u32) {
        match self.verified.iter_mut().find(|(p, _)| *p == pos) {
            Some((_, v)) => v.state = state,
            None => level.set_block(pos[0], pos[1], pos[2], state),
        }
    }

    /// `endPredictionsUpTo` (`handleBlockChangedAck`): the blocks predicted up to
    /// `sequence` get the server's state back (`syncBlockState`).
    pub fn acknowledge(
        &mut self,
        sequence: i32,
        level: &mut Level,
        blocks: &Blocks,
        player: &mut LocalPlayer,
    ) {
        let snap = self.last_teleport_sequence < sequence;
        let mut i = 0;
        while i < self.verified.len() {
            let (pos, v) = self.verified[i];
            if v.sequence > sequence {
                i += 1;
                continue;
            }
            self.verified.remove(i);
            if level
                .replace_block(pos[0], pos[1], pos[2], v.state)
                .is_some()
                && snap
                && blocks
                    .collision(v.state, pos)
                    .is_some_and(|s| s.overlaps(&player.bounding_box()))
            {
                player.abs_snap_to(v.player_position);
            }
        }
    }

    /// `onTeleport`: acknowledgements of actions up to now no longer move the player.
    pub fn on_teleport(&mut self) {
        self.last_teleport_sequence = self.sequence;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruthenium_prepare::exporters::{BlockPhysics, ShapeData, StatePhysics};

    const STONE: u32 = 1;
    const UNDER: [i32; 3] = [0, -61, 0];

    /// Air and a full block, in one chunk with stone under the player at 0.5 -60 0.5.
    fn world() -> (Level, Blocks, LocalPlayer) {
        let state = |shape| StatePhysics {
            shape,
            flags: 0,
            friction: 0.6,
            speed_factor: 1.0,
            jump_factor: 1.0,
            bounce: 0.0,
            fluid: 0,
            sturdy: 0,
            offset: None,
            outline: shape,
            interaction_shape: 0,
            interaction: 0,
            destroy_time: 1.5,
        };
        let cube = |full| ShapeData {
            coords: [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
            full: vec![full],
        };
        let blocks = Blocks::new(&BlockPhysics {
            shapes: vec![cube(false), cube(true)],
            states: vec![state(0), state(1)],
        });
        let mut level = Level::new(-64, 384);
        level.add_empty_chunk(0, 0);
        level.set_block(UNDER[0], UNDER[1], UNDER[2], STONE);
        let mut p = LocalPlayer::new();
        p.position = [0.5, -60.0, 0.5];
        (level, blocks, p)
    }

    /// The client breaks the block under the player as action `sequence`.
    fn break_under(pr: &mut Predictions, level: &mut Level, p: &LocalPlayer) -> i32 {
        let sequence = pr.start();
        assert!(pr.set_block(level, UNDER, 0, p.position));
        pr.end();
        sequence
    }

    #[test]
    fn a_refused_break_comes_back_and_moves_the_player_back() {
        let (mut level, blocks, mut p) = world();
        let mut pr = Predictions::new();
        let s = break_under(&mut pr, &mut level, &p);
        assert_eq!((s, pr.pending()), (1, 1));
        // The player falls into the hole; the server says the block is still there.
        p.position = [0.5, -60.5, 0.5];
        pr.server_block(&mut level, UNDER, STONE);
        assert_eq!(level.block(0, -61, 0), 0, "kept until the acknowledgement");
        pr.acknowledge(s, &mut level, &blocks, &mut p);
        assert_eq!(level.block(0, -61, 0), STONE);
        assert_eq!(p.position, [0.5, -60.0, 0.5]);
        assert_eq!(p.old_position, p.position);
        assert_eq!(pr.pending(), 0);
    }

    #[test]
    fn an_accepted_break_stays_and_a_teleport_stops_the_snap() {
        let (mut level, blocks, mut p) = world();
        let mut pr = Predictions::new();
        let s = break_under(&mut pr, &mut level, &p);
        pr.server_block(&mut level, UNDER, 0);
        p.position = [0.5, -60.5, 0.5];
        pr.acknowledge(s, &mut level, &blocks, &mut p);
        assert_eq!(level.block(0, -61, 0), 0);
        assert_eq!(p.position, [0.5, -60.5, 0.5]);

        // Refused, but a teleport came after the action: the block returns, the player
        // stays.
        level.set_block(UNDER[0], UNDER[1], UNDER[2], STONE);
        let s = break_under(&mut pr, &mut level, &p);
        pr.on_teleport();
        pr.server_block(&mut level, UNDER, STONE);
        pr.acknowledge(s, &mut level, &blocks, &mut p);
        assert_eq!(level.block(0, -61, 0), STONE);
        assert_eq!(p.position, [0.5, -60.5, 0.5]);
    }

    #[test]
    fn later_predictions_hold_the_block_and_other_updates_apply_at_once() {
        let (mut level, blocks, mut p) = world();
        let mut pr = Predictions::new();
        let first = break_under(&mut pr, &mut level, &p);
        // Predicted again (put back, say) under the next action.
        let second = pr.start();
        assert!(pr.set_block(&mut level, UNDER, STONE, p.position));
        // The same state again isn't a change.
        assert!(!pr.set_block(&mut level, UNDER, STONE, p.position));
        pr.end();
        pr.acknowledge(first, &mut level, &blocks, &mut p);
        assert_eq!(pr.pending(), 1, "still held by the later action");
        pr.acknowledge(second, &mut level, &blocks, &mut p);
        assert_eq!(pr.pending(), 0);
        assert_eq!(level.block(0, -61, 0), STONE, "the server's own state");
        // Not predicted: a server update lands at once. Outside predictions, nothing is
        // kept.
        pr.server_block(&mut level, [1, -61, 0], STONE);
        assert_eq!(level.block(1, -61, 0), STONE);
        assert!(pr.set_block(&mut level, [2, -61, 0], STONE, p.position));
        assert_eq!(pr.pending(), 0);
        // No chunk there: nothing changes.
        assert!(!pr.set_block(&mut level, [64, -61, 0], STONE, p.position));
    }
}
