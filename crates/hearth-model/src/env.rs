//! State shared by the app, its connections and buffers: the clock, the
//! effect queue, timers, and change counters. Cloning an `Env` is cheap.

use crate::effect::{Effect, ReqId};
use crate::time::system_now;
use crate::types::{Line, LineType};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Delayed work. Scheduling an equal timer again replaces the earlier one
/// (a debounce), which is what every use here wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Timer {
    SaveBuffers { conn: String },
    MarkRead { conn: String, key: String },
    Rejoin { conn: String },
    FetchTargets { conn: String },
    MetadataSync { conn: String, target: String },
    ToastExpire { id: u64 },
    SaveSettings,
    SearchDebounce { run: u64 },
    ClearFlash { line: u64 },
}

struct Inner {
    manual: AtomicBool,
    now: AtomicI64,
    rev: AtomicU64,
    seq: AtomicU64,
    effects: Mutex<Vec<Effect>>,
    timers: Mutex<Vec<(i64, Timer)>>,
}

#[derive(Clone)]
pub struct Env(Arc<Inner>);

impl Default for Env {
    fn default() -> Self {
        Env::new()
    }
}

impl Env {
    pub fn new() -> Env {
        Env(Arc::new(Inner {
            manual: AtomicBool::new(false),
            now: AtomicI64::new(0),
            rev: AtomicU64::new(1),
            seq: AtomicU64::new(0),
            effects: Mutex::new(Vec::new()),
            timers: Mutex::new(Vec::new()),
        }))
    }

    /// Epoch milliseconds: the system clock until [`Env::set_now`] pins it.
    pub fn now(&self) -> i64 {
        if self.0.manual.load(Ordering::Relaxed) {
            self.0.now.load(Ordering::Relaxed)
        } else {
            system_now()
        }
    }

    /// Pin the clock (tests, scripted mock). It stays put until set again.
    pub fn set_now(&self, ms: i64) {
        self.0.now.store(ms, Ordering::Relaxed);
        self.0.manual.store(true, Ordering::Relaxed);
    }

    pub fn emit(&self, e: Effect) {
        self.touch();
        self.0.effects.lock().unwrap().push(e);
    }

    pub fn take_effects(&self) -> Vec<Effect> {
        std::mem::take(&mut *self.0.effects.lock().unwrap())
    }

    pub fn has_effects(&self) -> bool {
        !self.0.effects.lock().unwrap().is_empty()
    }

    /// Marks the state as changed, so the UI knows to redraw.
    pub fn touch(&self) {
        self.0.rev.fetch_add(1, Ordering::Relaxed);
    }

    pub fn revision(&self) -> u64 {
        self.0.rev.load(Ordering::Relaxed)
    }

    /// A fresh id: unique across lines, requests, labels and toasts.
    pub fn next_seq(&self) -> u64 {
        self.0.seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn next_req(&self) -> ReqId {
        self.next_seq()
    }

    /// A new line stamped with the current time.
    pub fn line(&self, kind: LineType) -> Line {
        Line::new(self.next_seq(), self.now(), kind, "", "")
    }

    pub fn schedule(&self, delay_ms: i64, t: Timer) {
        let due = self.now() + delay_ms;
        let mut timers = self.0.timers.lock().unwrap();
        timers.retain(|(_, x)| *x != t);
        timers.push((due, t));
    }

    pub fn cancel(&self, t: &Timer) {
        self.0.timers.lock().unwrap().retain(|(_, x)| x != t);
    }

    /// Timers due by now, oldest first.
    pub fn take_due(&self) -> Vec<Timer> {
        let now = self.now();
        let mut timers = self.0.timers.lock().unwrap();
        let mut due: Vec<(i64, Timer)> = Vec::new();
        timers.retain(|(at, t)| {
            if *at <= now {
                due.push((*at, t.clone()));
                false
            } else {
                true
            }
        });
        due.sort_by_key(|(at, _)| *at);
        due.into_iter().map(|(_, t)| t).collect()
    }

    pub fn next_timer(&self) -> Option<i64> {
        self.0.timers.lock().unwrap().iter().map(|(at, _)| *at).min()
    }
}
