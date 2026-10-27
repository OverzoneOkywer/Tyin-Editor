use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use tyin_fs::Project;

use super::TyinApp;

impl TyinApp {
    pub(super) fn has_unsaved_changes(&self) -> bool {
        self.tabs.iter().any(|tab| tab.buffer.dirty)
    }

    pub(super) fn save_all_dirty(&mut self) -> bool {
        let dirty_tabs: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(idx, tab)| tab.buffer.dirty.then_some(idx))
            .collect();
        for idx in dirty_tabs {
            if !self.save_tab(idx) {
                return false;
            }
        }
        true
    }

    pub(super) fn request_open_workspace(&mut self, path: PathBuf) {
        if self.has_unsaved_changes() {
            self.pending_workspace = Some(path);
        } else {
            self.open_workspace(path);
        }
    }

    pub(super) fn open_workspace(&mut self, path: PathBuf) {
        self.start_workspace_scan(path, true);
    }

    pub(super) fn refresh_workspace(&mut self, path: PathBuf) {
        self.start_workspace_scan(path, false);
    }

    fn start_workspace_scan(&mut self, path: PathBuf, replaces: bool) {
        if self.workspace_scan.is_some() {
            if replaces
                && self
                    .pending_workspace_scan
                    .as_ref()
                    .is_some_and(|(_, pending_replaces)| !pending_replaces)
            {
                self.pending_workspace_scan = None;
            }
            match &self.pending_workspace_scan {
                Some((_, true)) if !replaces => {}
                _ => self.pending_workspace_scan = Some((path, replaces)),
            }
            if replaces {
                self.workspace_loading = true;
            }
            return;
        }

        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = Project::scan(path).map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
        self.workspace_scan = Some(receiver);
        self.workspace_scan_replaces = replaces;
        self.workspace_loading |= replaces;
    }

    pub(super) fn poll_workspace_scan(&mut self) {
        let result = self
            .workspace_scan
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("workspace scanner stopped unexpectedly".into()))
                }
            });
        let Some(result) = result else {
            return;
        };
        self.workspace_scan = None;
        let replaces = self.workspace_scan_replaces;

        if let Some((path, pending_replaces)) = self.pending_workspace_scan.take() {
            self.start_workspace_scan(path, pending_replaces);
            return;
        }

        match result {
            Ok((root, entries)) if replaces => {
                self.project.root = Some(root);
                self.tree_cache = entries;
                self.lsp.shutdown_all();
                self.expanded_dirs.clear();
                self.external_changes.clear();
                if self.watcher.take().is_some() {
                    self.resources.dec_watchers();
                }
                self.watcher_started = false;
                self.watcher_attempted = false;
                self.git_status = None;
                self.git_selected = None;
                self.git_diff = None;
                self.terminal = None;
                self.workspace_loading = false;
                self.error_message = None;
                self.refresh_git_status();
                if let Some(tab) = self.active_tab.and_then(|idx| self.tabs.get(idx)) {
                    if let Some(path) = tab.path.clone() {
                        let text = tab.edit_text.clone();
                        self.setup_lsp_for_file(&path, &text);
                    }
                }
            }
            Ok((root, entries)) if self.project.root.as_ref() == Some(&root) => {
                self.tree_cache = entries;
                self.expanded_dirs
                    .retain(|path| path.starts_with(&root) && path.is_dir());
                self.refresh_git_status();
            }
            Ok(_) => {}
            Err(error) => {
                self.workspace_loading = false;
                self.error_message = Some(format!("Gagal memindai workspace: {error}"));
            }
        }
    }

    pub(super) fn refresh_git_status(&mut self) {
        let Some(root) = self.project.root.as_deref() else {
            self.git_status = None;
            self.git_diff = None;
            return;
        };
        match tyin_git::status(root) {
            Ok(status) => self.git_status = Some(status),
            Err(error) => {
                self.git_status = None;
                if !error.to_string().contains("not a git repository") {
                    self.error_message = Some(format!("Gagal membaca status Git: {error}"));
                }
            }
        }
        self.refresh_git_diff();
    }
}
