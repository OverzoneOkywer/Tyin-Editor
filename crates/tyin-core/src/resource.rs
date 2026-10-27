use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct ResourceManager {
    tabs_open: AtomicU64,
    lsp_alive: AtomicU64,
    watchers: AtomicU64,
    started: Instant,
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self {
            tabs_open: AtomicU64::new(0),
            lsp_alive: AtomicU64::new(0),
            watchers: AtomicU64::new(0),
            started: Instant::now(),
        }
    }
}

impl ResourceManager {
    pub fn inc_tabs(&self) {
        self.tabs_open.fetch_add(1, Ordering::Relaxed);
    }
    pub fn dec_tabs(&self) {
        self.tabs_open.fetch_sub(1, Ordering::Relaxed);
    }
    pub fn inc_lsp(&self) {
        self.lsp_alive.fetch_add(1, Ordering::Relaxed);
    }
    pub fn dec_lsp(&self) {
        self.lsp_alive.fetch_sub(1, Ordering::Relaxed);
    }
    pub fn set_lsp_count(&self, count: usize) {
        self.lsp_alive.store(count as u64, Ordering::Relaxed);
    }
    pub fn inc_watchers(&self) {
        self.watchers.fetch_add(1, Ordering::Relaxed);
    }
    pub fn dec_watchers(&self) {
        self.watchers.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn stats(&self) -> ResourceStats {
        ResourceStats {
            tabs_open: self.tabs_open.load(Ordering::Relaxed),
            lsp_alive: self.lsp_alive.load(Ordering::Relaxed),
            watchers: self.watchers.load(Ordering::Relaxed),
            uptime: self.started.elapsed(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResourceStats {
    pub tabs_open: u64,
    pub lsp_alive: u64,
    pub watchers: u64,
    pub uptime: Duration,
}
