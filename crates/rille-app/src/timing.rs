//! How long a long task (analysis, import, scan) has run and how long it
//! still needs.

use std::time::{Duration, Instant};

/// Elapsed time and an estimate of the rest, for a progress display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timing {
    pub elapsed: Duration,
    /// `None` while there is nothing to estimate from, or while paused.
    pub remaining: Option<Duration>,
}

impl Timing {
    /// "3:21 elapsed · ~14:05 left".
    pub fn describe(&self, paused: bool) -> String {
        let rest = match self.remaining {
            _ if paused => String::new(),
            Some(r) if r < Duration::from_secs(1) => " · finishing".into(),
            Some(r) => format!(" · ~{} left", format_duration(r)),
            None => " · estimating…".into(),
        };
        format!("{} elapsed{rest}", format_duration(self.elapsed))
    }
}

/// "0:07", "12:03", "1:02:03".
pub fn format_duration(d: Duration) -> String {
    let s = d.as_secs_f64().round() as u64;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// Time left for a sequential task that took `elapsed` for `done` of
/// `total` items.
pub fn linear_estimate(elapsed: Duration, done: usize, total: usize) -> Option<Duration> {
    (done > 0).then(|| elapsed.mul_f64(total.saturating_sub(done) as f64 / done as f64))
}

/// A clock that can be paused.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stopwatch {
    banked: Duration,
    since: Option<Instant>,
}

impl Stopwatch {
    pub fn started(now: Instant) -> Self {
        Self { banked: Duration::ZERO, since: Some(now) }
    }

    pub fn stop(&mut self, now: Instant) {
        if let Some(s) = self.since.take() {
            self.banked += now.saturating_duration_since(s);
        }
    }

    pub fn resume(&mut self, now: Instant) {
        self.since.get_or_insert(now);
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.banked + self.since.map_or(Duration::ZERO, |s| now.saturating_duration_since(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    #[test]
    fn formats() {
        assert_eq!(format_duration(Duration::from_millis(6_600)), "0:07");
        assert_eq!(format_duration(S * 723), "12:03");
        assert_eq!(format_duration(S * 3723), "1:02:03");
    }

    #[test]
    fn describes() {
        let t = Timing { elapsed: S * 201, remaining: Some(S * 845) };
        assert_eq!(t.describe(false), "3:21 elapsed · ~14:05 left");
        assert_eq!(t.describe(true), "3:21 elapsed");
        assert_eq!(Timing { elapsed: S, remaining: None }.describe(false), "0:01 elapsed · estimating…");
        assert_eq!(Timing { elapsed: S, remaining: Some(Duration::ZERO) }.describe(false), "0:01 elapsed · finishing");
    }

    #[test]
    fn linear() {
        assert_eq!(linear_estimate(S * 10, 0, 100), None);
        assert_eq!(linear_estimate(S * 10, 20, 100), Some(S * 40));
        assert_eq!(linear_estimate(S * 10, 100, 100), Some(Duration::ZERO));
    }

    #[test]
    fn stopwatch_pauses() {
        let t0 = Instant::now();
        let mut w = Stopwatch::started(t0);
        w.stop(t0 + S * 5);
        assert_eq!(w.elapsed(t0 + S * 60), S * 5);
        w.resume(t0 + S * 60);
        assert_eq!(w.elapsed(t0 + S * 62), S * 7);
    }
}
