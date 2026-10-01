//! The analysis queue: which tracks get analyzed, in what order.
//!
//! Three lanes: tracks loaded on a deck first, then tracks the user asked
//! for (browser, file explorer), then the background pass over the
//! collection. A track is queued once; asking again with a higher priority
//! moves it up. Progress counts jobs when they *finish*, from the moment the
//! queue last became idle, so "12 / 340" means twelve are done.
//!
//! The time left is estimated from how long finished jobs took: the jobs
//! still waiting plus what the running ones probably still need, spread
//! over the workers.

use std::collections::{HashMap, VecDeque};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use rille_library::TrackId;

use crate::timing::{Stopwatch, Timing};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Background = 0,
    User = 1,
    Deck = 2,
}

/// Where a track stands, for the browser's status column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisState {
    /// A file shown by the explorer that is not in the collection.
    NotInCollection,
    NotAnalyzed,
    /// Analyzed by an older analyzer; will be redone.
    Stale,
    Queued,
    Running,
    Done,
    Failed,
}

/// A job handed to a worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Job {
    pub id: TrackId,
    /// Analyze even if a current result exists.
    pub force: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    pub done: usize,
    pub failed: usize,
    /// Jobs since the queue was last idle (done + failed + waiting + running).
    pub total: usize,
    pub running: Vec<TrackId>,
    pub paused: bool,
    pub timing: Timing,
}

/// A batch that just ended: every job queued since the last idle time ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BatchDone {
    pub done: usize,
    pub failed: usize,
    pub elapsed: Duration,
}

impl Progress {
    pub fn idle(&self) -> bool {
        self.done + self.failed >= self.total && self.running.is_empty()
    }
}

#[derive(Default)]
struct Inner {
    lanes: [VecDeque<TrackId>; 3],
    /// Queued tracks: lane and force flag.
    queued: HashMap<TrackId, (Priority, bool)>,
    /// Running tracks and when they started.
    running: HashMap<TrackId, Instant>,
    paused: bool,
    shutdown: bool,
    done: usize,
    failed: usize,
    total: usize,
    workers: usize,
    /// Time since the batch started, stopped while paused.
    clock: Stopwatch,
    /// Summed run time of the batch's finished jobs.
    busy: Duration,
    /// Mean job time of the previous batch, to estimate before the first
    /// job of a new one finishes.
    last_job: Option<Duration>,
}

impl Inner {
    fn is_idle(&self) -> bool {
        self.queued.is_empty() && self.running.is_empty()
    }

    /// A new batch after idle time starts a fresh progress count.
    fn start_batch_if_idle(&mut self) {
        if self.is_idle() {
            let now = Instant::now();
            self.done = 0;
            self.failed = 0;
            self.total = 0;
            self.busy = Duration::ZERO;
            self.clock = Stopwatch::started(now);
            if self.paused {
                self.clock.stop(now);
            }
        }
    }

    /// Mean job time: this batch's, leaning on the previous batch's while
    /// only a few jobs have finished.
    fn per_job(&self) -> Option<Duration> {
        let finished = (self.done + self.failed) as u32;
        match self.last_job {
            Some(prior) => Some((self.busy + prior * 2) / (finished + 2)),
            None if finished > 0 => Some(self.busy / finished),
            None => None,
        }
    }

    fn timing(&self, now: Instant) -> Timing {
        let remaining = if self.paused || self.is_idle() {
            None
        } else {
            self.per_job().map(|per_job| {
                let running = self.running.values().map(|&s| now.saturating_duration_since(s));
                estimate(per_job, self.queued.len(), running, self.workers)
            })
        };
        Timing { elapsed: self.clock.elapsed(now), remaining }
    }
}

/// Time left for `queued` jobs of `per_job` each plus the rest of the
/// running ones (which have run for the given times), over `workers`.
fn estimate(per_job: Duration, queued: usize, running: impl Iterator<Item = Duration>, workers: usize) -> Duration {
    let mut rest = Duration::ZERO;
    let mut longest = Duration::ZERO;
    for ran in running {
        let r = per_job.saturating_sub(ran);
        rest += r;
        longest = longest.max(r);
    }
    let work = per_job.mul_f64(queued as f64) + rest;
    (work / workers.max(1) as u32).max(longest)
}

#[derive(Default)]
pub struct AnalysisQueue {
    inner: Mutex<Inner>,
    wake: Condvar,
}

impl AnalysisQueue {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queues tracks (skipping running ones). A queued track asked for with
    /// a higher priority moves to that lane. Returns how many were added.
    pub fn push(&self, ids: &[TrackId], prio: Priority, force: bool) -> usize {
        let mut q = self.lock();
        q.start_batch_if_idle();
        let mut added = 0;
        for &id in ids {
            if q.running.contains_key(&id) {
                continue;
            }
            match q.queued.get(&id).copied() {
                Some((p, f)) if p >= prio => {
                    q.queued.insert(id, (p, f || force));
                }
                Some((p, f)) => {
                    q.lanes[p as usize].retain(|x| *x != id);
                    q.lanes[prio as usize].push_back(id);
                    q.queued.insert(id, (prio, f || force));
                }
                None => {
                    q.lanes[prio as usize].push_back(id);
                    q.queued.insert(id, (prio, force));
                    q.total += 1;
                    added += 1;
                }
            }
        }
        drop(q);
        self.wake.notify_all();
        added
    }

    /// The next job, waiting while the queue is empty or paused. `None`
    /// after [`shutdown`](Self::shutdown).
    pub fn take(&self) -> Option<Job> {
        let mut q = self.lock();
        loop {
            if q.shutdown {
                return None;
            }
            if !q.paused
                && let Some(job) = Self::pop(&mut q)
            {
                return Some(job);
            }
            q = self.wake.wait(q).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Like [`take`](Self::take) but never waits.
    pub fn try_take(&self) -> Option<Job> {
        let mut q = self.lock();
        if q.paused || q.shutdown { None } else { Self::pop(&mut q) }
    }

    fn pop(q: &mut Inner) -> Option<Job> {
        for lane in (0..3).rev() {
            if let Some(id) = q.lanes[lane].pop_front() {
                let (_, force) = q.queued.remove(&id).unwrap_or((Priority::Background, false));
                q.running.insert(id, Instant::now());
                return Some(Job { id, force });
            }
        }
        None
    }

    /// A deck load analyzes the track itself: take it out of the queue and
    /// show it as running. `false` if a worker is already on it.
    pub fn claim(&self, id: TrackId) -> bool {
        let mut q = self.lock();
        if q.running.contains_key(&id) {
            return false;
        }
        if let Some((p, _)) = q.queued.remove(&id) {
            q.lanes[p as usize].retain(|x| *x != id);
        } else {
            q.start_batch_if_idle();
            q.total += 1;
        }
        q.running.insert(id, Instant::now());
        true
    }

    /// A job (taken or claimed) ended. Returns the batch's totals when this
    /// was its last job.
    pub fn finish(&self, id: TrackId, ok: bool) -> Option<BatchDone> {
        let mut q = self.lock();
        let now = Instant::now();
        let mut ended = None;
        if let Some(started) = q.running.remove(&id) {
            q.busy += now.saturating_duration_since(started);
            if ok {
                q.done += 1;
            } else {
                q.failed += 1;
            }
            if q.is_idle() {
                q.clock.stop(now);
                q.last_job = Some(q.busy / (q.done + q.failed) as u32);
                ended = Some(BatchDone { done: q.done, failed: q.failed, elapsed: q.clock.elapsed(now) });
            }
        }
        drop(q);
        self.wake.notify_all();
        ended
    }

    /// How many workers take jobs, for the time estimate.
    pub fn set_workers(&self, n: usize) {
        self.lock().workers = n;
    }

    /// Drops everything waiting (running jobs finish).
    pub fn cancel(&self) {
        let mut q = self.lock();
        let dropped = q.queued.len();
        q.queued.clear();
        for lane in &mut q.lanes {
            lane.clear();
        }
        q.total -= dropped;
    }

    pub fn set_paused(&self, paused: bool) {
        let mut q = self.lock();
        let now = Instant::now();
        q.paused = paused;
        if paused || q.is_idle() {
            q.clock.stop(now);
        } else {
            q.clock.resume(now);
        }
        drop(q);
        self.wake.notify_all();
    }

    pub fn shutdown(&self) {
        self.lock().shutdown = true;
        self.wake.notify_all();
    }

    /// `Queued` or `Running` if the queue knows the track.
    pub fn state(&self, id: TrackId) -> Option<AnalysisState> {
        let q = self.lock();
        if q.running.contains_key(&id) {
            Some(AnalysisState::Running)
        } else if q.queued.contains_key(&id) {
            Some(AnalysisState::Queued)
        } else {
            None
        }
    }

    pub fn progress(&self) -> Progress {
        let q = self.lock();
        let mut running: Vec<TrackId> = q.running.keys().copied().collect();
        running.sort_unstable();
        Progress {
            done: q.done,
            failed: q.failed,
            total: q.total,
            running,
            paused: q.paused,
            timing: q.timing(Instant::now()),
        }
    }

    /// Just the time, cheap enough for every frame.
    pub fn timing(&self) -> Timing {
        self.lock().timing(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deck_before_user_before_background() {
        let q = AnalysisQueue::default();
        q.push(&[1, 2, 3], Priority::Background, false);
        q.push(&[4], Priority::User, false);
        q.push(&[5], Priority::Deck, false);
        let order: Vec<TrackId> = std::iter::from_fn(|| q.try_take()).map(|j| j.id).collect();
        assert_eq!(order, [5, 4, 1, 2, 3]);
    }

    #[test]
    fn dedupe_and_upgrade() {
        let q = AnalysisQueue::default();
        assert_eq!(q.push(&[1, 2, 3], Priority::Background, false), 3);
        // Asking again adds nothing; a higher priority moves the track up.
        assert_eq!(q.push(&[2], Priority::Background, false), 0);
        assert_eq!(q.push(&[3], Priority::User, true), 0);
        assert_eq!(q.progress().total, 3);
        assert_eq!(q.try_take(), Some(Job { id: 3, force: true }));
        assert_eq!(q.try_take(), Some(Job { id: 1, force: false }));
        // A running track is not queued again.
        assert_eq!(q.push(&[1], Priority::User, false), 0);
        assert_eq!(q.state(1), Some(AnalysisState::Running));
        assert_eq!(q.state(2), Some(AnalysisState::Queued));
    }

    #[test]
    fn progress_counts_finished_jobs() {
        let q = AnalysisQueue::default();
        q.push(&[1, 2, 3], Priority::Background, false);
        let a = q.try_take().unwrap();
        let p = q.progress();
        assert_eq!((p.done, p.failed, p.total, p.running, p.paused), (0, 0, 3, vec![1], false));
        // Nothing finished yet: no estimate.
        assert_eq!(p.timing.remaining, None);
        assert_eq!(q.finish(a.id, true), None);
        let b = q.try_take().unwrap();
        assert_eq!(q.finish(b.id, false), None);
        let p = q.progress();
        assert_eq!((p.done, p.failed, p.total), (1, 1, 3));
        assert!(p.timing.remaining.is_some());
        let c = q.try_take().unwrap();
        let end = q.finish(c.id, true).expect("batch ended");
        assert_eq!((end.done, end.failed), (2, 1));
        assert!(q.progress().idle());
        // After idle time a new batch counts from zero.
        q.push(&[9], Priority::User, false);
        let p = q.progress();
        assert_eq!((p.done, p.failed, p.total), (0, 0, 1));
    }

    #[test]
    fn claim_pause_cancel() {
        let q = AnalysisQueue::default();
        q.push(&[1, 2, 3], Priority::Background, false);
        // A deck load takes track 2 out of the queue.
        assert!(q.claim(2));
        assert!(!q.claim(2));
        q.set_paused(true);
        assert_eq!(q.try_take(), None);
        q.set_paused(false);
        assert_eq!(q.try_take().map(|j| j.id), Some(1));
        q.cancel();
        assert_eq!(q.try_take(), None);
        let p = q.progress();
        assert_eq!((p.total, p.running.len()), (2, 2));
        q.finish(1, true);
        q.finish(2, true);
        assert!(q.progress().idle());
    }

    #[test]
    fn estimate_spreads_over_workers() {
        let s = Duration::from_secs(1);
        // 8 waiting jobs of 10 s on 4 workers, the running ones half done.
        let running = [s * 5; 4].into_iter();
        assert_eq!(estimate(s * 10, 8, running, 4), s * 25);
        // Nothing waiting: the slowest running job decides.
        assert_eq!(estimate(s * 10, 0, [s * 2, s * 9].into_iter(), 4), s * 8);
        // A job over the mean counts as about to finish.
        assert_eq!(estimate(s * 10, 0, [s * 30].into_iter(), 4), Duration::ZERO);
    }

    #[test]
    fn take_waits_and_shutdown_releases() {
        let q = std::sync::Arc::new(AnalysisQueue::default());
        let worker = {
            let q = q.clone();
            std::thread::spawn(move || q.take())
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        q.push(&[7], Priority::User, false);
        assert_eq!(worker.join().unwrap().map(|j| j.id), Some(7));
        let waiting = {
            let q = q.clone();
            std::thread::spawn(move || q.take())
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        q.shutdown();
        assert_eq!(waiting.join().unwrap(), None);
    }
}
