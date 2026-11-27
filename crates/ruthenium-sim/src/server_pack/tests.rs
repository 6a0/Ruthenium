//! The manager's paths one at a time (the golden cases cover them in random mixes).

use super::*;

const A: u128 = 0xA;
const B: u128 = 0xB;

/// Packs whose file name has "broken" in it have no valid metadata.
fn packs() -> ServerPacks<u128> {
    ServerPacks::new(|list: &[(u128, PathBuf)]| {
        list.iter()
            .map(|(id, p)| (!p.to_string_lossy().contains("broken")).then_some(*id))
            .collect()
    })
}

fn allowed() -> ServerPacks<u128> {
    let mut p = packs();
    p.configure_for_server_control(PromptStatus::Allowed);
    p.run_tasks();
    p.take_effects();
    p
}

fn batch(effects: &[Effect<u128>]) -> u64 {
    effects
        .iter()
        .find_map(|e| match e {
            Effect::Download(b) => Some(b.batch),
            _ => None,
        })
        .expect("a download")
}

/// Pushed, downloaded and loaded: ACCEPTED at once, DOWNLOADED with the result, the
/// reload with the pack, and APPLIED once it is done.
#[test]
fn a_pack_goes_from_accepted_to_applied() {
    let mut p = allowed();
    p.push(A, "http://h/a".into(), None);
    assert_eq!(p.take_effects(), [Effect::Feedback(A, Feedback::Accepted)]);
    // Nothing happens before the tasks run.
    p.run_tasks();
    let e = p.take_effects();
    assert_eq!(
        e,
        [Effect::Download(Batch {
            batch: 0,
            requests: vec![Request {
                id: A,
                url: "http://h/a".into(),
                hash: None
            }]
        })]
    );
    p.downloaded(0, vec![(A, Ok("/c/a".into()))]);
    assert!(p.take_effects().is_empty(), "a result waits for the tasks");
    p.run_tasks();
    assert_eq!(
        p.take_effects(),
        [
            Effect::Feedback(A, Feedback::Downloaded),
            Effect::Reload(Reload {
                packs: vec![A],
                recovery: false
            })
        ]
    );
    assert!(p.reloading());
    p.reloaded(true);
    assert_eq!(p.take_effects(), [Effect::Feedback(A, Feedback::Applied)]);
    p.run_tasks();
    assert!(p.take_effects().is_empty());
}

/// One pack of a batch failed: it is FAILED_DOWNLOAD, the other (not active) is
/// DISCARDED, and nothing is reloaded.
#[test]
fn a_failed_download_discards_the_batch() {
    let mut p = allowed();
    p.push(A, "http://h/a".into(), None);
    p.push(B, "http://h/b".into(), None);
    p.run_tasks();
    let b = batch(&p.take_effects());
    p.downloaded(b, vec![(A, Ok("/c/a".into())), (B, Err(()))]);
    p.run_tasks();
    assert_eq!(
        p.take_effects(),
        [
            Effect::Feedback(A, Feedback::Discarded),
            Effect::Feedback(B, Feedback::DownloadFailed),
        ]
    );
}

/// A pack whose metadata is not valid fails its reload at once: FAILED_RELOAD, and the
/// reload goes on with the packs that were active.
#[test]
fn a_pack_without_metadata_fails_to_activate() {
    let mut p = allowed();
    p.push(A, "http://h/a".into(), None);
    p.run_tasks();
    let b = batch(&p.take_effects());
    p.downloaded(b, vec![(A, Ok("/c/a".into()))]);
    p.run_tasks();
    p.take_effects();
    p.reloaded(true);
    assert_eq!(p.take_effects(), [Effect::Feedback(A, Feedback::Applied)]);
    p.push(B, "http://h/b".into(), None);
    p.run_tasks();
    let b = batch(&p.take_effects());
    p.downloaded(b, vec![(B, Ok("/c/broken".into()))]);
    p.run_tasks();
    assert_eq!(
        p.take_effects(),
        [
            Effect::Feedback(B, Feedback::Downloaded),
            Effect::Reload(Reload {
                packs: vec![A],
                recovery: false
            }),
            Effect::Feedback(B, Feedback::ActivationFailed),
        ]
    );
}

/// A reload that fails is made again with the packs that were active (a recovery);
/// one that fails too ends the session.
#[test]
fn a_failed_reload_recovers_and_a_failed_recovery_aborts() {
    let mut p = allowed();
    p.push(A, "http://h/a".into(), None);
    p.run_tasks();
    let b = batch(&p.take_effects());
    p.downloaded(b, vec![(A, Ok("/c/a".into()))]);
    p.run_tasks();
    p.take_effects();
    p.reloaded(false);
    assert_eq!(
        p.take_effects(),
        [Effect::Reload(Reload {
            packs: vec![],
            recovery: true
        })]
    );
    p.run_tasks();
    assert_eq!(
        p.take_effects(),
        [Effect::Feedback(A, Feedback::ActivationFailed)]
    );
    p.reloaded(false);
    assert_eq!(p.take_effects(), [Effect::Abort]);
    assert!(!p.reloading());
}

/// Declined (the prompt's No, or the server list's Disabled): every push is DECLINED.
#[test]
fn declined_packs_are_answered_at_once() {
    let mut p = packs();
    p.configure_for_server_control(PromptStatus::Declined);
    p.push(A, "http://h/a".into(), None);
    assert_eq!(p.take_effects(), [Effect::Feedback(A, Feedback::Declined)]);
    p.run_tasks();
    assert!(p.take_effects().is_empty());
}

/// Pending: nothing is downloaded until the prompt is answered; Yes accepts what was
/// pushed meanwhile.
#[test]
fn pending_packs_wait_for_the_answer() {
    let mut p = packs();
    p.configure_for_server_control(PromptStatus::Pending);
    p.push(A, "http://h/a".into(), None);
    p.run_tasks();
    assert!(p.take_effects().is_empty());
    p.allow();
    assert_eq!(p.take_effects(), [Effect::Feedback(A, Feedback::Accepted)]);
    p.run_tasks();
    assert!(matches!(p.take_effects()[..], [Effect::Download(_)]));
    assert_eq!(p.prompt_status(), PromptStatus::Allowed);
}

/// A pack the server pops is unloaded, without a final status; a disconnect unloads
/// the rest and tells the server nothing.
#[test]
fn popped_packs_unload_quietly() {
    let mut p = allowed();
    for (id, url) in [(A, "http://h/a"), (B, "http://h/b")] {
        p.push(id, url.into(), None);
    }
    p.run_tasks();
    let b = batch(&p.take_effects());
    p.downloaded(b, vec![(A, Ok("/c/a".into())), (B, Ok("/c/b".into()))]);
    p.run_tasks();
    assert_eq!(
        p.take_effects().last(),
        Some(&Effect::Reload(Reload {
            packs: vec![A, B],
            recovery: false
        }))
    );
    p.reloaded(true);
    p.take_effects();
    p.pop(A);
    p.run_tasks();
    assert_eq!(
        p.take_effects(),
        [Effect::Reload(Reload {
            packs: vec![B],
            recovery: false
        })]
    );
    p.reloaded(true);
    assert_eq!(p.take_effects(), [Effect::Feedback(B, Feedback::Applied)]);
    p.cleanup_after_disconnect();
    assert_eq!(
        p.take_effects(),
        [Effect::Reload(Reload {
            packs: vec![],
            recovery: false
        })]
    );
    p.reloaded(true);
    p.run_tasks();
    assert!(p.take_effects().is_empty(), "nothing goes to the server");
}

/// The same id pushed again replaces the pack, which goes without a status.
#[test]
fn a_pack_pushed_again_replaces_the_old_one() {
    let mut p = allowed();
    p.push(A, "http://h/one".into(), None);
    p.run_tasks();
    let first = batch(&p.take_effects());
    p.push(A, "http://h/two".into(), None);
    assert_eq!(p.take_effects(), [Effect::Feedback(A, Feedback::Accepted)]);
    p.run_tasks();
    let e = p.take_effects();
    let Effect::Download(second) = &e[0] else {
        panic!("{e:?}")
    };
    assert_eq!(second.requests[0].url, "http://h/two");
    // The first's result is for an entry that is gone.
    p.downloaded(first, vec![(A, Ok("/c/one".into()))]);
    p.run_tasks();
    assert!(p.take_effects().is_empty());
    p.downloaded(second.batch, vec![(A, Ok("/c/two".into()))]);
    p.run_tasks();
    assert_eq!(
        p.take_effects()[0],
        Effect::Feedback(A, Feedback::Downloaded)
    );
}

/// Vanilla's `HashMap` order for UUID keys: `UUID.hashCode`, spread, by bucket.
#[test]
fn batches_follow_javas_hash_map_order() {
    // new UUID(1, 2).hashCode() == 3, new UUID(0, 0x10).hashCode() == 16 (bucket 0).
    assert_eq!(java_uuid_hash((1u128 << 64) | 2), 3);
    let r = |id: u128| Request {
        id,
        url: String::new(),
        hash: None,
    };
    let order: Vec<u128> = java_hash_map_order(vec![r((1 << 64) | 2), r(0x10), r(1)])
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(order, [0x10, 1, (1 << 64) | 2]);
}

/// Pushing and immediately popping a pack while its download batch is still in flight
/// keeps it counted in `tracked()` until the batch completes.
#[test]
fn push_and_pop_while_downloading_still_counts_toward_tracked() {
    let mut p = allowed();
    p.push(A, "http://h/a".into(), None);
    p.run_tasks();
    let b = batch(&p.take_effects());
    assert_eq!(p.tracked(), 1);
    p.pop(A);
    p.run_tasks();
    // Entry is removed from `packs`, but still pending in `out` while download `b` runs.
    assert_eq!(p.tracked(), 1);
    p.downloaded(b, vec![(A, Err(()))]);
    p.run_tasks();
    assert_eq!(p.tracked(), 0);
}
