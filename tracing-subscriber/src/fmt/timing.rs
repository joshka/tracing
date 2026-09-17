//! Span timing state and its human-readable output representation.
//!
//! The formatter layer owns lifecycle callbacks, while this module owns the
//! state transitions and duration accounting those callbacks drive.

use core::{fmt, num::NonZeroU64};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

/// Counts live [`Timings`] extensions across all formatter layers.
///
/// Enter and exit callbacks use this as a conservative guard before looking up
/// timing state in a span's extensions. The count is process-wide because a
/// span can outlive the formatter layer or configuration that created its
/// timing state.
static LIVE_TIMED_SPANS: AtomicUsize = AtomicUsize::new(0);

/// Whether a span is idle or busy, including the nesting depth of active
/// enters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TimingState {
    Idle,
    Busy(NonZeroU64),
}

/// Accumulates the idle and busy durations of one span.
///
/// `state` determines whether the interval beginning at `last_transition` is
/// idle or busy.
pub(super) struct Timings {
    /// Idle time accumulated from completed idle intervals.
    ///
    /// This excludes the interval beginning at `last_transition`, which is
    /// included when a snapshot is taken or the span next becomes busy.
    idle_ns: u64,

    /// Busy time accumulated from completed busy intervals.
    ///
    /// This excludes the interval beginning at `last_transition`, which is
    /// included when a snapshot is taken or the span next becomes idle.
    busy_ns: u64,

    /// The beginning of the current idle or busy interval.
    last_transition: Instant,

    /// The current idle or busy state and any active enter nesting depth.
    state: TimingState,
}

impl Timings {
    /// Starts a timing state in its idle interval and registers it with the
    /// process-wide lifecycle-callback guard.
    pub(super) fn new(now: Instant) -> Self {
        LIVE_TIMED_SPANS.fetch_add(1, Ordering::AcqRel);
        Self {
            idle_ns: 0,
            busy_ns: 0,
            last_transition: now,
            state: TimingState::Idle,
        }
    }

    /// Returns whether any span may currently contain timing state.
    ///
    /// This is a process-wide conservative guard: `true` may refer to a span
    /// owned by another layer or subscriber, while `false` means an extension
    /// lookup can be skipped. Counting live timed spans rather than layer
    /// configurations keeps the guard valid when a reload replaces the layer
    /// that created a span's timing state.
    #[inline]
    pub(super) fn any_live() -> bool {
        LIVE_TIMED_SPANS.load(Ordering::Acquire) != 0
    }

    /// Returns whether this span currently has no active enters.
    pub(super) fn is_idle(&self) -> bool {
        matches!(self.state, TimingState::Idle)
    }

    /// Records an enter, changing the current interval from idle to busy only
    /// for the outermost enter.
    ///
    /// The clock is supplied lazily so nested enters do not sample it when no
    /// idle/busy transition occurs.
    #[inline]
    pub(super) fn enter(&mut self, now: impl FnOnce() -> Instant) {
        match self.state {
            TimingState::Idle => {
                let now = now();
                self.idle_ns += now.duration_since(self.last_transition).as_nanos() as u64;
                self.last_transition = now;
                self.state = TimingState::Busy(NonZeroU64::new(1).expect("one is non-zero"));
            }
            TimingState::Busy(depth) => {
                let depth = depth.checked_add(1).expect("span nesting depth overflow");
                self.state = TimingState::Busy(depth);
            }
        }
    }

    /// Records an exit, changing the current interval from busy to idle only
    /// when the outermost enter is exited.
    ///
    /// Each exit must have a matching enter. Lifecycle callbacks therefore
    /// continue updating existing timing state even while reloads disable the
    /// CLOSE events that caused that state to be created.
    #[inline]
    pub(super) fn exit(&mut self, now: impl FnOnce() -> Instant) {
        match self.state {
            TimingState::Idle => panic!("span exited without a matching enter"),
            TimingState::Busy(depth) if depth.get() == 1 => {
                let now = now();
                self.busy_ns += now.duration_since(self.last_transition).as_nanos() as u64;
                self.last_transition = now;
                self.state = TimingState::Idle;
            }
            TimingState::Busy(depth) => {
                let depth = NonZeroU64::new(depth.get() - 1)
                    .expect("a nested enter depth remains non-zero after an exit");
                self.state = TimingState::Busy(depth);
            }
        }
    }

    /// Returns idle and busy durations as of `now` without changing the live
    /// timing state.
    ///
    /// The interval since `last_transition` is classified using the current
    /// nesting depth, so snapshots are well-defined in both idle and busy
    /// states.
    #[inline]
    pub(super) fn snapshot(&self, now: Instant) -> TimingSnapshot {
        let mut timings = TimingSnapshot {
            idle: Duration::from_nanos(self.idle_ns),
            busy: Duration::from_nanos(self.busy_ns),
        };
        let current = now.duration_since(self.last_transition);
        match self.state {
            TimingState::Idle => timings.idle += current,
            TimingState::Busy(_) => timings.busy += current,
        }
        timings
    }
}

impl Drop for Timings {
    fn drop(&mut self) {
        let previous = LIVE_TIMED_SPANS.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
    }
}

/// The idle and busy durations observed when a span's timing state is sampled.
///
/// A snapshot includes the current idle or busy interval without completing
/// that interval or otherwise changing the span's live timing state.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct TimingSnapshot {
    idle: Duration,
    busy: Duration,
}

impl TimingSnapshot {
    /// Returns the total duration for which the span was not entered.
    ///
    /// This includes the current interval when the span was idle at the time
    /// this snapshot was taken.
    pub(super) fn idle(&self) -> Duration {
        self.idle
    }

    /// Returns the total duration for which the span was entered.
    ///
    /// Nested enters are measured as one continuous busy interval rather than
    /// counting overlapping time more than once. This includes the current
    /// interval when the span was busy at the time this snapshot was taken.
    pub(super) fn busy(&self) -> Duration {
        self.busy
    }
}

/// Formats a duration using approximately three significant digits and an
/// appropriate nanosecond-through-second unit.
pub(super) struct TimingDisplay(pub(super) Duration);

impl fmt::Display for TimingDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut t = self.0.as_nanos() as f64;
        for unit in ["ns", "µs", "ms", "s"].iter() {
            if t < 10.0 {
                return write!(f, "{:.2}{}", t, unit);
            } else if t < 100.0 {
                return write!(f, "{:.1}{}", t, unit);
            } else if t < 1000.0 {
                return write!(f, "{:.0}{}", t, unit);
            }
            t /= 1000.0;
        }
        write!(f, "{:.0}s", t * 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};

    use super::*;

    #[test]
    fn tracks_nested_enters() {
        let start = Instant::now();
        let mut timings = Timings::new(start);

        timings.enter(|| start + Duration::from_secs(1));
        timings.enter(|| start + Duration::from_secs(2));
        timings.exit(|| start + Duration::from_secs(3));

        let snapshot = timings.snapshot(start + Duration::from_secs(4));
        assert_eq!(snapshot.idle(), Duration::from_secs(1));
        assert_eq!(snapshot.busy(), Duration::from_secs(3));

        timings.exit(|| start + Duration::from_secs(5));
        let snapshot = timings.snapshot(start + Duration::from_secs(7));
        assert_eq!(snapshot.idle(), Duration::from_secs(3));
        assert_eq!(snapshot.busy(), Duration::from_secs(4));
    }

    #[test]
    fn formats_durations() {
        fn fmt(t: u64) -> String {
            TimingDisplay(Duration::from_nanos(t)).to_string()
        }

        assert_eq!(fmt(1), "1.00ns");
        assert_eq!(fmt(12), "12.0ns");
        assert_eq!(fmt(123), "123ns");
        assert_eq!(fmt(1234), "1.23µs");
        assert_eq!(fmt(12345), "12.3µs");
        assert_eq!(fmt(123456), "123µs");
        assert_eq!(fmt(1234567), "1.23ms");
        assert_eq!(fmt(12345678), "12.3ms");
        assert_eq!(fmt(123456789), "123ms");
        assert_eq!(fmt(1234567890), "1.23s");
        assert_eq!(fmt(12345678901), "12.3s");
        assert_eq!(fmt(123456789012), "123s");
        assert_eq!(fmt(1234567890123), "1235s");
    }
}
