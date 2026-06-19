//! Engine event broadcaster. Workers emit `QueueEvent`s here; a future
//! TUI subscriber can fan them out to live panels. For now the TUI polls
//! the store directly, so emitted events are dropped unless a sender is
//! registered.

use std::sync::RwLock;

use crossbeam_channel::Sender;

use crate::core::events::QueueEvent;

#[derive(Default)]
pub struct EventBroadcaster {
    senders: RwLock<Vec<Sender<QueueEvent>>>,
}

impl EventBroadcaster {
    pub fn new() -> Self {
        Self {
            senders: RwLock::new(Vec::new()),
        }
    }

    pub fn emit(&self, ev: QueueEvent) {
        // Drop any senders whose receivers have hung up.
        let mut senders = self
            .senders
            .write()
            .expect("EventBroadcaster lock poisoned");
        senders.retain(|tx| tx.send(ev.clone()).is_ok());
    }
}
