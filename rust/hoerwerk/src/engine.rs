//! One job engine for everything that runs in the background. Steps go to a queue with its own
//! workers: `encode` (ffmpeg; one worker – a conversion already uses all configured cores) and `io` (lookups,
//! tags, filing – short steps that must not wait behind a two-hour conversion). Two steps of one job never
//! run at the same time.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::text;

pub const QUEUES: [&str; 2] = ["encode", "io"];

type Step = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct State {
    queues: HashMap<&'static str, VecDeque<(String, Step)>>,
    running: HashMap<&'static str, Vec<String>>,
    started: bool,
    stopping: bool,
    done: u64,
}

pub struct Engine {
    workers: HashMap<&'static str, usize>,
    state: Mutex<State>,
    wake: Condvar,
    on_change: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

fn queue_name(q: &str) -> Option<&'static str> {
    QUEUES.iter().copied().find(|x| *x == q)
}

impl Engine {
    pub fn new(encode: usize, io: usize) -> Self {
        let mut st = State::default();
        for q in QUEUES {
            st.queues.insert(q, VecDeque::new());
            st.running.insert(q, Vec::new());
        }
        Engine {
            workers: HashMap::from([("encode", encode.max(1)), ("io", io.max(1))]),
            state: Mutex::new(st),
            wake: Condvar::new(),
            on_change: Mutex::new(None),
        }
    }

    pub fn on_change(&self, f: Box<dyn Fn() + Send + Sync>) {
        *self.on_change.lock().unwrap_or_else(|p| p.into_inner()) = Some(f);
    }

    fn changed(&self) {
        if let Some(f) = self.on_change.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
            f();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn start(self: &Arc<Self>) {
        {
            let mut st = self.lock();
            if st.started {
                return;
            }
            st.started = true;
        }
        for q in QUEUES {
            for n in 0..self.workers[q] {
                let me = self.clone();
                std::thread::Builder::new()
                    .name(format!("{q}-{}", n + 1))
                    .spawn(move || me.worker(q))
                    .expect("worker thread");
            }
        }
    }

    /// Queue a step; a key that is already waiting is not queued twice.
    pub fn submit(&self, queue: &str, key: &str, step: Step) -> bool {
        let Some(q) = queue_name(queue) else { return false };
        {
            let mut st = self.lock();
            let list = st.queues.get_mut(q).expect("queue");
            if list.iter().any(|(k, _)| k == key) {
                return false;
            }
            list.push_back((key.to_string(), step));
            self.wake.notify_all();
        }
        self.changed();
        true
    }

    /// Drop waiting steps of a job (cancelled or removed).
    pub fn discard(&self, key: &str) {
        let mut st = self.lock();
        for q in st.queues.values_mut() {
            q.retain(|(k, _)| k != key);
        }
    }

    pub fn is_waiting(&self, key: &str) -> bool {
        self.lock().queues.values().any(|q| q.iter().any(|(k, _)| k == key))
    }

    pub fn stats(&self) -> Value {
        let st = self.lock();
        let mut out = serde_json::Map::new();
        for q in QUEUES {
            out.insert(
                q.into(),
                json!({"workers": self.workers[q], "running": st.running[q].len(), "waiting": st.queues[q].len()}),
            );
        }
        Value::Object(out)
    }

    pub fn idle(&self) -> bool {
        let st = self.lock();
        st.queues.values().all(VecDeque::is_empty) && st.running.values().all(Vec::is_empty)
    }

    /// Shutting down: no new step starts; waits (up to `timeout`) for the running ones to end. Waiting steps
    /// stay – the jobs are persisted and queued again at the next start.
    pub fn stop(&self, timeout: Duration) -> bool {
        {
            let mut st = self.lock();
            st.stopping = true;
            self.wake.notify_all();
        }
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if self.lock().running.values().all(Vec::is_empty) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// The first waiting step whose job is not being worked on right now (in any queue).
    fn next(st: &mut State, q: &'static str) -> Option<(String, Step)> {
        if st.stopping {
            return None;
        }
        let busy: Vec<String> = st.running.values().flatten().cloned().collect();
        let list = st.queues.get_mut(q)?;
        let pos = list.iter().position(|(k, _)| !busy.contains(k))?;
        list.remove(pos)
    }

    fn worker(&self, q: &'static str) {
        loop {
            let (key, step) = {
                let mut st = self.lock();
                loop {
                    if let Some(item) = Self::next(&mut st, q) {
                        st.running.get_mut(q).expect("queue").push(item.0.clone());
                        break item;
                    }
                    st = self.wake.wait(st).unwrap_or_else(|p| p.into_inner());
                }
            };
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)).is_err() {
                text::error("engine", &format!("Fehler in Warteschlange {q} ({key})"));
            }
            {
                let mut st = self.lock();
                let running = st.running.get_mut(q).expect("queue");
                if let Some(i) = running.iter().position(|k| *k == key) {
                    running.remove(i);
                }
                st.done += 1;
                self.wake.notify_all(); // a step held back for this job may start now
            }
            self.changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn one_step_per_job_at_a_time() {
        let e = Arc::new(Engine::new(1, 2));
        e.start();
        let active = Arc::new(AtomicUsize::new(0));
        let max = Arc::new(AtomicUsize::new(0));
        for (q, key) in [("io", "a"), ("encode", "a"), ("io", "b")] {
            let (a, m) = (active.clone(), max.clone());
            let only_a = key == "a";
            e.submit(
                q,
                key,
                Box::new(move || {
                    if only_a {
                        let n = a.fetch_add(1, Ordering::SeqCst) + 1;
                        m.fetch_max(n, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(100));
                        a.fetch_sub(1, Ordering::SeqCst);
                    }
                }),
            );
        }
        let end = Instant::now() + Duration::from_secs(5);
        while !e.idle() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(e.idle());
        assert_eq!(max.load(Ordering::SeqCst), 1);
        // a panicking step does not kill the worker: the next step on the same queue still runs
        e.submit("io", "c", Box::new(|| panic!("boom")));
        let after = Arc::new(AtomicUsize::new(0));
        let a2 = after.clone();
        e.submit(
            "io",
            "d",
            Box::new(move || {
                a2.store(1, Ordering::SeqCst);
            }),
        );
        let end = Instant::now() + Duration::from_secs(5);
        while !(e.idle() && after.load(Ordering::SeqCst) == 1) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(after.load(Ordering::SeqCst), 1, "the worker survived the panic");
        assert!(e.idle());
        assert!(e.stop(Duration::from_secs(1)));
    }
}
