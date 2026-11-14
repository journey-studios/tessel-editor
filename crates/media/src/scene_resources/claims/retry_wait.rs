//! Transient attempts yield CPU while there is still time in the caller's original budget.

use std::time::{Duration, Instant};

const PAUSE: Duration = Duration::from_micros(500);

pub(super) fn pause(deadline: Instant) {
    let delay = PAUSE.min(deadline.saturating_duration_since(Instant::now()));
    if delay.is_zero() {
        return;
    }
    #[cfg(test)]
    if SLEEP.with(|hook| {
        hook.borrow_mut().as_mut().is_some_and(|sleep| {
            sleep(delay);
            true
        })
    }) {
        return;
    }
    std::thread::sleep(delay);
}

#[cfg(test)]
type Sleep = Box<dyn FnMut(Duration)>;

#[cfg(test)]
thread_local! {
    /// Replaces this thread's sleep, so retry control can be tested without scheduler timing.
    pub(super) static SLEEP: std::cell::RefCell<Option<Sleep>> =
        const { std::cell::RefCell::new(None) };
}
