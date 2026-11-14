//! The UI wake: how a worker thread tells the event loop that its result is ready.
//!
//! The application owns one [`Wake`] and binds it on the UI thread. Threads started through
//! [`spawn`] or [`Builder`] inherit that binding and wake it when they end; a long-lived
//! worker calls [`notify`] after each result it publishes, and a poller that still waits on
//! work calls [`notify_after`] so it is polled again. A thread with no binding (the CLI, a
//! unit test) notifies nobody, so nothing needs a UI to run.

use std::{
    cell::RefCell,
    io,
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};

type Hook = Box<dyn Fn(Duration) + Send + Sync>;

/// How long after a thread ends the owner is woken a second time, because a handle reports
/// `is_finished` only once the thread has fully unwound.
const LATE: Duration = Duration::from_millis(50);

/// A cloneable handle to the owner's repaint request.
#[derive(Clone, Default)]
pub struct Wake(Arc<Mutex<Option<Hook>>>);

thread_local! {
    static BOUND: RefCell<Option<Wake>> = const { RefCell::new(None) };
}

impl Wake {
    pub fn new() -> Self {
        Self::default()
    }
    /// Set what a wake does: repaint after the given delay. Safe to call from any thread,
    /// and again to replace it.
    pub fn set(&self, hook: impl Fn(Duration) + Send + Sync + 'static) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(Box::new(hook));
        }
    }
    /// Ask the owner to repaint now.
    pub fn wake(&self) {
        self.wake_after(Duration::ZERO);
    }
    /// Ask the owner to repaint after `delay`.
    pub fn wake_after(&self, delay: Duration) {
        if let Ok(slot) = self.0.lock()
            && let Some(hook) = slot.as_ref()
        {
            hook(delay);
        }
    }
    /// Make this the wake of the calling thread and of every thread it starts.
    pub fn bind(&self) {
        BOUND.with(|bound| *bound.borrow_mut() = Some(self.clone()));
    }
}

/// The calling thread's bound wake, if any.
pub fn current() -> Option<Wake> {
    BOUND.with(|bound| bound.borrow().clone())
}

/// Wake the calling thread's bound owner now. A no-op on an unbound thread.
pub fn notify() {
    if let Some(wake) = current() {
        wake.wake();
    }
}

/// Ask the calling thread's bound owner to repaint after `delay`. A no-op on an unbound thread.
pub fn notify_after(delay: Duration) {
    if let Some(wake) = current() {
        wake.wake_after(delay);
    }
}

fn run<T>(wake: Option<Wake>, work: impl FnOnce() -> T) -> T {
    if let Some(wake) = &wake {
        wake.bind();
    }
    let result = work();
    if let Some(wake) = &wake {
        wake.wake();
        wake.wake_after(LATE);
    }
    result
}

/// A named thread builder whose thread inherits the caller's wake and wakes it on exit.
pub struct Builder(thread::Builder);

impl Builder {
    pub fn new() -> Self {
        Self(thread::Builder::new())
    }
    pub fn name(self, name: String) -> Self {
        Self(self.0.name(name))
    }
    pub fn spawn<T: Send + 'static>(
        self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> io::Result<JoinHandle<T>> {
        let wake = current();
        self.0.spawn(move || run(wake, work))
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self::new()
    }
}

/// `std::thread::spawn` that inherits the caller's wake and wakes it on exit.
/// Panics like `std::thread::spawn` when the OS refuses a thread.
pub fn spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> JoinHandle<T> {
    let wake = current();
    thread::spawn(move || run(wake, work))
}

/// An unbounded channel whose sender wakes the creator's owner after every send.
pub fn channel<T>() -> (Sender<T>, mpsc::Receiver<T>) {
    let (send, receive) = mpsc::channel();
    (
        Sender {
            send,
            wake: current(),
        },
        receive,
    )
}

/// A bounded channel whose sender wakes the creator's owner after every send.
pub fn sync_channel<T>(bound: usize) -> (SyncSender<T>, mpsc::Receiver<T>) {
    let (send, receive) = mpsc::sync_channel(bound);
    (
        SyncSender {
            send,
            wake: current(),
        },
        receive,
    )
}

/// The sending half of [`channel`].
pub struct Sender<T> {
    send: mpsc::Sender<T>,
    wake: Option<Wake>,
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            send: self.send.clone(),
            wake: self.wake.clone(),
        }
    }
}

impl<T> Sender<T> {
    /// Send, then wake the owner whether or not the receiver is still there.
    pub fn send(&self, value: T) -> Result<(), mpsc::SendError<T>> {
        let sent = self.send.send(value);
        if let Some(wake) = &self.wake {
            wake.wake();
        }
        sent
    }
}

/// The sending half of [`sync_channel`].
pub struct SyncSender<T> {
    send: mpsc::SyncSender<T>,
    wake: Option<Wake>,
}

impl<T> Clone for SyncSender<T> {
    fn clone(&self) -> Self {
        Self {
            send: self.send.clone(),
            wake: self.wake.clone(),
        }
    }
}

impl<T> SyncSender<T> {
    /// Send (blocking while full), then wake the owner.
    pub fn send(&self, value: T) -> Result<(), mpsc::SendError<T>> {
        let sent = self.send.send(value);
        if let Some(wake) = &self.wake {
            wake.wake();
        }
        sent
    }
    /// Send without blocking, then wake the owner when the value was queued.
    pub fn try_send(&self, value: T) -> Result<(), mpsc::TrySendError<T>> {
        let sent = self.send.try_send(value);
        if sent.is_ok()
            && let Some(wake) = &self.wake
        {
            wake.wake();
        }
        sent
    }
}
