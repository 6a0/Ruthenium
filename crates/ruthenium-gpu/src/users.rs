//! Who still uses a range of bytes on the GPU: the last graphics frame that reads or
//! writes it and the last upload batches that copy from or into it. Frames and the two
//! kinds of upload batch are counted separately (a batch that has not been submitted yet
//! has no frame or queue submission to wait for), and a range is free again only when all
//! of them are done.

/// The last users of a range of bytes. Zero means none of that kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Users {
    /// The last graphics frame that may read or write it (the frame's number).
    pub frame: u64,
    /// The last batch of copies recorded into a frame that uses it
    /// ([`crate::UploadRing`] numbers them; a staged batch gets its frame when recorded).
    pub frame_batch: u64,
    /// The last batch of copies submitted on their own (the transfer queue, or a copy
    /// submission of the graphics queue) that uses it.
    pub copy_batch: u64,
}

/// How far each kind of work has got: everything up to these has finished on the GPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Completed {
    pub frame: u64,
    pub frame_batch: u64,
    pub copy_batch: u64,
}

impl Users {
    /// Used by nothing: free at once.
    pub const NONE: Users = Users {
        frame: 0,
        frame_batch: 0,
        copy_batch: 0,
    };

    /// Used by graphics frames up to `frame`.
    pub fn until_frame(frame: u64) -> Users {
        Users {
            frame,
            ..Users::NONE
        }
    }

    /// Both sets of users: the later of each kind.
    #[must_use]
    pub fn and(self, other: Users) -> Users {
        Users {
            frame: self.frame.max(other.frame),
            frame_batch: self.frame_batch.max(other.frame_batch),
            copy_batch: self.copy_batch.max(other.copy_batch),
        }
    }

    /// Whether every user has finished.
    pub fn done(&self, completed: &Completed) -> bool {
        self.frame <= completed.frame
            && self.frame_batch <= completed.frame_batch
            && self.copy_batch <= completed.copy_batch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_is_free_only_when_every_user_is_done() {
        let u = Users::until_frame(7).and(Users {
            copy_batch: 3,
            ..Users::NONE
        });
        let mut c = Completed {
            frame: 7,
            frame_batch: 0,
            copy_batch: 2,
        };
        assert!(!u.done(&c), "the copy has not finished");
        c.copy_batch = 3;
        assert!(u.done(&c));
        c.frame = 6;
        assert!(!u.done(&c), "a frame still draws it");
        assert!(Users::NONE.done(&Completed::default()));
    }
}
