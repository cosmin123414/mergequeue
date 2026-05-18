//! Engine event broadcaster. Multiple subscribers (TUI panels, tests,
//! debug printers) each get their own crossbeam-channel receiver.

use std::sync::{Mutex, RwLock};

use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::core::events::QueueEvent;

pub struct EventSubscriber {
    pub rx: Receiver<QueueEvent>,
}

#[derive(Default)]
pub struct EventBroadcaster {
    senders: RwLock<Vec<Sender<QueueEvent>>>,
    history: Mutex<Vec<QueueEvent>>,
    record_history: bool,
}

impl EventBroadcaster {
    pub fn new() -> Self {
        Self {
            senders: RwLock::new(Vec::new()),
            history: Mutex::new(Vec::new()),
            record_history: false,
        }
    }

    /// Construct a broadcaster that also records every event into an
    /// in-memory log (useful for tests).
    pub fn with_history() -> Self {
        Self {
            senders: RwLock::new(Vec::new()),
            history: Mutex::new(Vec::new()),
            record_history: true,
        }
    }

    pub fn subscribe(&self) -> EventSubscriber {
        let (tx, rx) = unbounded();
        self.senders
            .write()
            .expect("EventBroadcaster lock poisoned")
            .push(tx);
        EventSubscriber { rx }
    }

    pub fn emit(&self, ev: QueueEvent) {
        if self.record_history {
            if let Ok(mut h) = self.history.lock() {
                h.push(ev.clone());
            }
        }
        // Drop any senders whose receivers have hung up.
        let mut senders = self
            .senders
            .write()
            .expect("EventBroadcaster lock poisoned");
        senders.retain(|tx| tx.send(ev.clone()).is_ok());
    }

    /// Snapshot of the in-memory event log. Empty unless
    /// `with_history` was used at construction.
    pub fn history(&self) -> Vec<QueueEvent> {
        self.history.lock().map(|h| h.clone()).unwrap_or_default()
    }
}
