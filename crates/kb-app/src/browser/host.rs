//! The page worker: one thread that owns every page document.
//!
//! A `kb_engine::Page` is built on `Rc`, so it is not `Send`: each page is created on this thread and
//! used only here. The UI thread sends a job (a closure over the page slots) and gets its result back
//! as a value. Jobs run one at a time, in the order they were sent. The design is in
//! docs/06-page-worker.md.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use kb_engine::Page;
use kb_net::SyncNet;

use super::types::Pump;

/// Stack size of the worker. Layout of a large page recurses deeply; match the main thread's 8 MiB.
const STACK: usize = 8 * 1024 * 1024;
/// How often a build's wait checks its progress and the input.
const TICK: Duration = Duration::from_millis(60);

/// The pages the worker holds: the one on screen, and the one kept for Back.
pub(super) struct Slots {
    pub(super) net: Arc<SyncNet>,
    pub(super) page: Option<Page>,
    pub(super) previous: Option<Page>,
}

/// Facts about the page on screen that the UI thread reads without waiting. Refreshed as each job
/// completes, in the order the jobs ran.
#[derive(Clone, Default, Debug, PartialEq)]
pub(super) struct Mirror {
    pub(super) has_page: bool,
    pub(super) scroll_y: f64,
    pub(super) max_scroll: f64,
    pub(super) content_height: f64,
    pub(super) title: String,
}

/// Called by the worker after each job finishes (the device wakes its input loop with it).
pub(super) type Waker = Arc<dyn Fn() + Send + Sync>;

type Work = Box<dyn FnOnce(&mut Slots) -> Box<dyn Any + Send> + Send>;

struct Done {
    ticket: u64,
    value: Box<dyn Any + Send>,
    mirror: Mirror,
}

struct Inner {
    jobs: mpsc::Sender<(u64, Work)>,
    done: mpsc::Receiver<Done>,
    /// Finished jobs that nobody has taken yet, by ticket.
    finished: HashMap<u64, Done>,
    waker: Arc<Mutex<Option<Waker>>>,
    next: u64,
    mirror: Mirror,
    net: Arc<SyncNet>,
    pump: Option<Pump>,
    paint_delay: Duration,
}

/// The UI thread's handle on the page worker. Cloning it shares the same worker.
#[derive(Clone)]
pub(super) struct Host(Rc<RefCell<Inner>>);

/// Wrap a typed job so that its result travels as `Option<R>`: `None` when the job panicked.
fn wrap<R: Send + 'static>(f: impl FnOnce(&mut Slots) -> R + Send + 'static) -> Work {
    Box::new(move |slots: &mut Slots| {
        let value: Option<R> = catch_unwind(AssertUnwindSafe(|| f(slots))).ok();
        Box::new(value) as Box<dyn Any + Send>
    })
}

fn snapshot(slots: &Slots) -> Mirror {
    match slots.page.as_ref() {
        Some(p) => Mirror {
            has_page: true,
            scroll_y: p.scroll_y(),
            max_scroll: p.max_scroll(),
            content_height: p.content_height(),
            title: p.title.clone(),
        },
        None => Mirror::default(),
    }
}

impl Host {
    /// Start the worker. If the thread cannot start, every job fails at once (`None`), so the browser
    /// shows blank pages instead of waiting forever.
    pub(super) fn new(net: Arc<SyncNet>) -> Host {
        let (jobs, job_rx) = mpsc::channel::<(u64, Work)>();
        let (done_tx, done) = mpsc::channel::<Done>();
        let waker: Arc<Mutex<Option<Waker>>> = Arc::new(Mutex::new(None));
        let worker_waker = waker.clone();
        let worker_net = net.clone();
        let spawned = thread::Builder::new()
            .name("page-worker".into())
            .stack_size(STACK)
            .spawn(move || {
                let mut slots = Slots { net: worker_net, page: None, previous: None };
                for (ticket, work) in job_rx {
                    let value = work(&mut slots);
                    let mirror = snapshot(&slots);
                    if done_tx.send(Done { ticket, value, mirror }).is_err() {
                        break;
                    }
                    let wake = worker_waker.lock().ok().and_then(|g| g.clone());
                    if let Some(wake) = wake {
                        wake();
                    }
                }
            });
        if spawned.is_err() {
            eprintln!("[page] the page worker could not start");
        }
        Host(Rc::new(RefCell::new(Inner {
            jobs,
            done,
            finished: HashMap::new(),
            waker,
            next: 0,
            mirror: Mirror::default(),
            net,
            pump: None,
            paint_delay: Duration::ZERO,
        })))
    }

    /// The facts about the page on screen, as of the last job that finished.
    pub(super) fn mirror(&self) -> Mirror {
        self.0.borrow().mirror.clone()
    }

    pub(super) fn set_waker(&self, waker: Waker) {
        if let Ok(mut slot) = self.0.borrow().waker.lock() {
            *slot = Some(waker);
        }
    }

    pub(super) fn set_pump(&self, pump: Pump) {
        self.0.borrow_mut().pump = Some(pump);
    }

    /// Ask the pump (progress and input) on behalf of a wait. True when no pump is set.
    pub(super) fn pump_call(&self, fraction: f32, label: &str) -> bool {
        match self.0.borrow_mut().pump.as_mut() {
            Some(pump) => pump(fraction, label),
            None => true,
        }
    }

    /// Extra time each paint takes on the worker. Tests use it to hold a paint in flight.
    pub(super) fn paint_delay(&self) -> Duration {
        self.0.borrow().paint_delay
    }

    #[cfg(test)]
    pub(super) fn set_paint_delay(&self, delay: Duration) {
        self.0.borrow_mut().paint_delay = delay;
    }

    /// Run `f` on the worker and wait for its result. Waits behind any job already queued, so a
    /// paint in progress delays it. `None` when the job panicked or the worker is gone.
    pub(super) fn ask<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Slots) -> R + Send + 'static,
    ) -> Option<R> {
        let ticket = self.post(f)?;
        self.wait(ticket)
    }

    /// Queue `f` on the worker and return at once with its ticket.
    pub(super) fn post<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Slots) -> R + Send + 'static,
    ) -> Option<u64> {
        let mut inner = self.0.borrow_mut();
        inner.next += 1;
        let ticket = inner.next;
        inner.jobs.send((ticket, wrap(f))).ok().map(|_| ticket)
    }

    /// Take in every job that has finished, without waiting.
    pub(super) fn poll(&self) {
        let mut inner = self.0.borrow_mut();
        while let Ok(done) = inner.done.try_recv() {
            inner.absorb(done);
        }
    }

    /// The result of a posted job if it has finished: `None` while it runs, `Some(None)` if it panicked.
    pub(super) fn take<R: 'static>(&self, ticket: u64) -> Option<Option<R>> {
        self.poll();
        self.take_finished(ticket)
    }

    /// Wait for a posted job and return its result (`None` if it panicked or the worker is gone).
    pub(super) fn wait<R: 'static>(&self, ticket: u64) -> Option<R> {
        loop {
            if let Some(result) = self.take_finished::<R>(ticket) {
                return result;
            }
            let received = self.0.borrow().done.recv();
            match received {
                Ok(done) => self.0.borrow_mut().absorb(done),
                Err(_) => return None,
            }
        }
    }

    /// Build a page on the worker, waiting for it with the pump: the pump reports progress and can
    /// cancel the wait (its `false` sets the flag the build checks). Returns false if the worker is gone.
    pub(super) fn build(
        &self,
        work: impl FnOnce(&mut Slots, &AtomicBool) + Send + 'static,
    ) -> bool {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let Some(ticket) = self.post(move |slots: &mut Slots| work(slots, &flag)) else {
            return false;
        };
        let net = self.0.borrow().net.clone();
        loop {
            if self.take_finished::<()>(ticket).is_some() {
                return true;
            }
            let received = self.0.borrow().done.recv_timeout(TICK);
            match received {
                Ok(done) => self.0.borrow_mut().absorb(done),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let s = net.stats();
                    let req = s.requested.load(Ordering::Relaxed);
                    let done = s.done.load(Ordering::Relaxed);
                    let fraction = if req == 0 { 0.3 } else { 0.3 + 0.6 * done as f32 / req as f32 };
                    if !self.pump_call(fraction, "Loading") {
                        cancel.store(true, Ordering::Relaxed);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    fn take_finished<R: 'static>(&self, ticket: u64) -> Option<Option<R>> {
        let done = self.0.borrow_mut().finished.remove(&ticket)?;
        Some(match done.value.downcast::<Option<R>>() {
            Ok(value) => *value,
            Err(_) => None,
        })
    }
}

impl Inner {
    fn absorb(&mut self, done: Done) {
        self.mirror = done.mirror.clone();
        self.finished.insert(done.ticket, done);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn host() -> Host {
        Host::new(Arc::new(SyncNet::new(1024 * 1024)))
    }

    #[test]
    fn a_job_runs_on_the_worker_thread_and_returns_its_value() {
        let host = host();
        let here = thread::current().id();
        let there = host.ask(move |_| thread::current().id()).expect("the worker answers");
        assert_ne!(here, there, "the job ran off the calling thread");
        assert_eq!(host.ask(|_| 7u32), Some(7));
    }

    #[test]
    fn a_panicking_job_gives_none_and_the_worker_goes_on() {
        let host = host();
        let failed: Option<u32> = host.ask(|_| panic!("a job that fails"));
        assert_eq!(failed, None);
        assert_eq!(host.ask(|_| 3u32), Some(3), "the next job still runs");
    }

    #[test]
    fn jobs_run_in_the_order_they_were_sent() {
        let host = host();
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut tickets = Vec::new();
        for i in 0..5 {
            let log = log.clone();
            tickets.push(host.post(move |_| log.lock().unwrap().push(i)).expect("posted"));
        }
        for t in tickets {
            host.wait::<()>(t);
        }
        assert_eq!(*log.lock().unwrap(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_posted_job_is_not_done_until_the_worker_finishes_it() {
        let host = host();
        let slow = host
            .post(|_| std::thread::sleep(Duration::from_millis(400)))
            .expect("posted");
        assert!(host.take::<()>(slow).is_none(), "still running, so not taken");
        host.wait::<()>(slow);
        assert_eq!(host.take::<()>(slow), None, "taken once: the result is gone");
    }

    #[test]
    fn the_waker_runs_after_each_job() {
        let host = host();
        let woken = Arc::new(AtomicUsize::new(0));
        let counter = woken.clone();
        host.set_waker(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        let t = host.post(|_| 1u8).expect("posted");
        host.wait::<u8>(t);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while woken.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(woken.load(Ordering::SeqCst) >= 1);
    }
}
