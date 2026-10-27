use crossbeam_channel::{unbounded, Receiver, Sender};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub enum Event {
    OpenFolder(String),
    OpenFile(String),
    FocusTab(usize),
    CloseTab(usize),
    SaveActive,
    ToggleResourceMonitor,
    RequestIndex(String),
    FileChanged(String),
    WorkspaceChanged,
    ReloadActive,
}

#[derive(Clone)]
pub struct EventBus {
    tx: Sender<Event>,
    rx: Arc<Mutex<Receiver<Event>>>,
}

impl Default for EventBus {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
        }
    }
}

impl EventBus {
    pub fn send(&self, ev: Event) {
        let _ = self.tx.send(ev);
    }

    pub fn try_recv(&self) -> Option<Event> {
        if let Ok(rx) = self.rx.lock() {
            rx.try_recv().ok()
        } else {
            None
        }
    }
}
