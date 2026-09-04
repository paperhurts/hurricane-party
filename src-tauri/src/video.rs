//! The video window's switch ack (D68).
//!
//! `open_video` cannot learn from the dispatch whether an already-open video
//! window received the switch: `emit_to`, like `eval`, is fire-and-forget in
//! this build, and a dead webview drops the message silently (D67). So the
//! window says so itself, with a `video_ready` command, and the absence of
//! that ack within [`ACK_TIMEOUT`] is the failure signal. A freshly built
//! window acks the same way once it is up and listening, within
//! [`MOUNT_TIMEOUT`], and [`OpenLock`] holds every later click until it has.
//!
//! This module is the registry of waits in flight. There is no webview in it,
//! which is what makes it testable: the command layer in `lib.rs` is two
//! calls, `expect` before the emit or the build and `wait` after it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::oneshot;

/// The event the window listens for. Payload: the track id.
pub const SWITCH_EVENT: &str = "hp://switch-track";

/// How long `open_video` waits for the window to confirm a switch.
///
/// The round trip is emit, listener, `list_tracks`, `video_ready`: tens of
/// milliseconds on a warm webview, and the existing-window branch is warm by
/// definition. One second is an order of magnitude of headroom for a renderer
/// that is momentarily busy, and short enough that a click on a dead window
/// reports failure while the user is still looking at the button. `open_video`
/// logs the measured round trip on every switch so the hand test can check the
/// margin rather than trust this comment.
pub const ACK_TIMEOUT: Duration = Duration::from_millis(1000);

/// How long `open_video` waits for a freshly built window to say it is up.
///
/// The cold path is the native window, WebView2, the navigation, the bundle,
/// the mount, the listener registration and one `list_tracks`: hundreds of
/// milliseconds under Vite, more on the first WebView2 spin-up of a process.
/// Five seconds is headroom for a slow disk at launch; past it the window was
/// built and has not come up, which is worth a message rather than a longer
/// wait. Logged on every open so the hand test can check the margin.
pub const MOUNT_TIMEOUT: Duration = Duration::from_millis(5000);

/// One `open_video` at a time (issue #42).
///
/// `build()` returns the instant the window is *requested*: the label goes
/// into the map before the native window exists, let alone the page. Two
/// clicks in the microseconds between the label check and that insert both
/// build; a click in the hundreds of milliseconds after it takes the switch
/// branch and emits to a page that has not registered its listener, so the
/// event is dropped (D67) and the click reports a timeout over a working
/// window. Same gap, two faces. Holding this across the whole command, ack
/// wait included, means the second click always sees a window that has said
/// it is listening. Async because the wait is an await; a `std` mutex here
/// would park a runtime thread for up to `MOUNT_TIMEOUT`.
#[derive(Default)]
pub struct OpenLock(pub tokio::sync::Mutex<()>);

#[derive(Debug, thiserror::Error)]
pub enum SwitchError {
    #[error(
        "the video window did not confirm track {} within {} ms",
        .id,
        .timeout.as_millis()
    )]
    Timeout { id: i64, timeout: Duration },
    /// The registry was dropped under a live waiter: app teardown, in practice.
    #[error("the video window went away before confirming the switch to track {id}")]
    Gone { id: i64 },
}

/// Waits in flight, keyed by track id.
///
/// A Vec per id rather than one sender, because two clicks on the same track
/// inside the timeout are one switch as far as the window is concerned, and
/// both callers should hear the same ack. Newest-wins would turn a
/// double-click into a spurious error strip.
#[derive(Default)]
pub struct SwitchAcks(Mutex<HashMap<i64, Vec<oneshot::Sender<()>>>>);

/// One registered wait. Obtain it before the emit and consume it after, so an
/// ack that beats the wait is already in the channel rather than lost.
#[must_use = "a Pending dropped without wait() hears nothing"]
pub struct Pending {
    id: i64,
    rx: oneshot::Receiver<()>,
}

impl SwitchAcks {
    /// Register interest in the ack for `id`.
    pub fn expect(&self, id: i64) -> Pending {
        let (tx, rx) = oneshot::channel();
        let mut map = self.0.lock().unwrap();
        // Sweep waiters whose receiver is gone: timed out, or the caller was
        // dropped. Here rather than on a timer keeps the map bounded without
        // a task of its own; a dead window's failed clicks are cleaned up by
        // the next click.
        map.retain(|_, waiters| {
            waiters.retain(|tx| !tx.is_closed());
            !waiters.is_empty()
        });
        map.entry(id).or_default().push(tx);
        Pending { id, rx }
    }

    /// The window confirmed `id`. Returns whether anyone was still waiting,
    /// which the command ignores and the tests do not.
    pub fn complete(&self, id: i64) -> bool {
        let Some(waiters) = self.0.lock().unwrap().remove(&id) else {
            return false;
        };
        // A closed receiver is a waiter that already timed out; its click has
        // reported failure and there is nothing left to tell it.
        waiters
            .into_iter()
            .fold(false, |any, tx| tx.send(()).is_ok() | any)
    }

    #[cfg(test)]
    fn pending_ids(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

impl Pending {
    /// Block until the ack arrives or `timeout` elapses.
    pub async fn wait(self, timeout: Duration) -> Result<(), SwitchError> {
        let Pending { id, rx } = self;
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(SwitchError::Gone { id }),
            Err(_) => Err(SwitchError::Timeout { id, timeout }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHORT: Duration = Duration::from_millis(20);

    #[tokio::test]
    async fn an_ack_completes_the_wait() {
        // Ack before wait: the ordering open_video relies on. Registered, then
        // emitted, then waited; an ack between the last two is not lost.
        let acks = SwitchAcks::default();
        let pending = acks.expect(7);
        assert!(acks.complete(7));
        assert!(pending.wait(SHORT).await.is_ok());
    }

    #[tokio::test]
    async fn no_ack_times_out() {
        let acks = SwitchAcks::default();
        let pending = acks.expect(7);
        assert!(matches!(
            pending.wait(SHORT).await,
            Err(SwitchError::Timeout { id: 7, .. })
        ));
    }

    #[tokio::test]
    async fn an_ack_for_another_track_does_not_count() {
        let acks = SwitchAcks::default();
        let pending = acks.expect(7);
        assert!(!acks.complete(8));
        assert!(matches!(
            pending.wait(SHORT).await,
            Err(SwitchError::Timeout { id: 7, .. })
        ));
    }

    #[tokio::test]
    async fn two_clicks_on_one_track_both_hear_the_ack() {
        let acks = SwitchAcks::default();
        let first = acks.expect(7);
        let second = acks.expect(7);
        assert!(acks.complete(7));
        assert!(first.wait(SHORT).await.is_ok());
        assert!(second.wait(SHORT).await.is_ok());
    }

    #[tokio::test]
    async fn a_late_ack_finds_nobody() {
        let acks = SwitchAcks::default();
        let pending = acks.expect(7);
        assert!(pending.wait(SHORT).await.is_err());
        assert!(!acks.complete(7));
    }

    #[tokio::test]
    async fn timed_out_waits_are_swept_by_the_next_expect() {
        let acks = SwitchAcks::default();
        assert!(acks.expect(7).wait(SHORT).await.is_err());
        assert_eq!(acks.pending_ids(), 1, "dead waiter sits until swept");
        let _live = acks.expect(8);
        assert_eq!(acks.pending_ids(), 1, "7 swept, 8 registered");
    }

    #[tokio::test]
    async fn the_registry_going_away_is_reported_not_hung() {
        let acks = SwitchAcks::default();
        let pending = acks.expect(7);
        drop(acks);
        assert!(matches!(
            pending.wait(SHORT).await,
            Err(SwitchError::Gone { id: 7 })
        ));
    }
}
