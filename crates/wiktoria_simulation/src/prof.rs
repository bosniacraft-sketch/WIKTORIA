//! Wall-time marks for finding slow spots inside a long routine, switched on by the run CLI's
//! `--profile`. Off by default: a mark then costs one thread-local read.
//!
//! `start()` sets the clock at the top of a routine, `mark("name")` adds the time since the last
//! mark (or start) to `name`. Marks are kept per thread, so only the thread that drives the day is
//! measured; work it hands to other threads counts under the mark that waits for it.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

thread_local! {
    static MARKS: RefCell<Option<(Instant, BTreeMap<&'static str, Duration>)>> = const { RefCell::new(None) };
}

/// Switches the marks on for this thread.
pub fn enable() {
    MARKS.with(|m| *m.borrow_mut() = Some((Instant::now(), BTreeMap::new())));
}

/// Sets the clock: time before this call is not counted under the next mark.
pub fn start() {
    MARKS.with(|m| {
        if let Some((t, _)) = m.borrow_mut().as_mut() {
            *t = Instant::now();
        }
    });
}

/// Adds the time since the last mark or start to `name`.
pub fn mark(name: &'static str) {
    MARKS.with(|m| {
        if let Some((t, sums)) = m.borrow_mut().as_mut() {
            let now = Instant::now();
            *sums.entry(name).or_default() += now - *t;
            *t = now;
        }
    });
}

/// The sums so far (empty when the marks are off).
pub fn sums() -> BTreeMap<&'static str, Duration> {
    MARKS.with(|m| m.borrow().as_ref().map(|(_, sums)| sums.clone()).unwrap_or_default())
}
