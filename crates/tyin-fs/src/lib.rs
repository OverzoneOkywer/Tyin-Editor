use anyhow::{Context, Result};
use ignore::WalkBuilder;
use notify_debouncer_mini::new_debouncer;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::thread::JoinHandle;
use std::time::Duration;
use tracing::info;

#[derive(Debug, Clone)]
pub struct FsEntry {
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Default)]
pub struct Project {
    pub root: Option<PathBuf>,
}

impl Project {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open<P: AsRef<Path>>(&mut self, p: P) -> Result<Vec<FsEntry>> {
        let (root, entries) = Self::scan(p)?;
        self.root = Some(root);
        Ok(entries)
    }

    pub fn scan<P: AsRef<Path>>(p: P) -> Result<(PathBuf, Vec<FsEntry>)> {
        let root = p
            .as_ref()
            .canonicalize()
            .with_context(|| format!("cannot resolve workspace {}", p.as_ref().display()))?;
        if !root.is_dir() {
            anyhow::bail!("workspace path is not a directory: {}", root.display());
        }
        let mut out = Vec::new();
        let mut wb = WalkBuilder::new(&root);
        wb.hidden(false)
            .git_ignore(true)
            .git_exclude(true)
            .max_depth(Some(14));
        for dent in wb.build() {
            let dent = dent.with_context(|| "failed to scan workspace entry")?;
            let path = dent.path().to_path_buf();
            if path == root {
                continue;
            }
            out.push(FsEntry {
                is_dir: dent.file_type().map(|ft| ft.is_dir()).unwrap_or(false),
                path,
            });
        }
        out.sort_by(|a, b| tree_order(&root, a, b));
        Ok((root, out))
    }
}

fn tree_order(root: &Path, a: &FsEntry, b: &FsEntry) -> std::cmp::Ordering {
    let segments = |entry: &FsEntry| {
        let relative = entry.path.strip_prefix(root).unwrap_or(&entry.path);
        relative
            .components()
            .enumerate()
            .map(|(index, component)| {
                let name = component.as_os_str().to_string_lossy().to_lowercase();
                let is_file = index + 1 == relative.components().count() && !entry.is_dir;
                (is_file, name, component.as_os_str().to_os_string())
            })
            .collect::<Vec<_>>()
    };
    segments(a).cmp(&segments(b))
}

pub struct WatcherHandle {
    stop: Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn watch_path<P: AsRef<Path>, F>(path: P, mut on_change: F) -> Result<WatcherHandle>
where
    F: FnMut(PathBuf) + Send + 'static,
{
    let (tx, rx) = channel();
    let mut debouncer = new_debouncer(Duration::from_millis(300), tx)?;
    debouncer
        .watcher()
        .watch(path.as_ref(), notify::RecursiveMode::Recursive)?;
    let (stop, stop_rx) = channel();
    let thread = std::thread::spawn(move || {
        info!("tyin-fs: watcher started");
        loop {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            let res = match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(res) => res,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
            match res {
                Ok(events) => {
                    for ev in events {
                        on_change(ev.path);
                    }
                }
                Err(e) => tracing::warn!("watch error: {:?}", e),
            }
        }
        drop(debouncer);
    });
    Ok(WatcherHandle {
        stop,
        thread: Some(thread),
    })
}

#[cfg(test)]
mod tests {
    use super::{Project, WatcherHandle};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_workspace() -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tyin-fs-test-{}-{id}", std::process::id()))
    }

    #[test]
    fn scan_groups_directories_before_files_and_is_case_insensitive() {
        let root = temporary_workspace();
        fs::create_dir_all(root.join("z-dir")).unwrap();
        fs::create_dir_all(root.join("A-dir")).unwrap();
        fs::write(root.join("z-file"), "").unwrap();
        fs::write(root.join("A-file"), "").unwrap();

        let mut project = Project::new();
        let entries = project.open(&root).unwrap();
        let names = entries
            .iter()
            .map(|entry| {
                entry
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_lowercase()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["a-dir", "z-dir", "a-file", "z-file"]);
        assert_eq!(
            project.root.as_deref(),
            Some(root.canonicalize().unwrap().as_path())
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn case_ties_use_a_stable_original_name_order() {
        let root = temporary_workspace();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a-file"), "").unwrap();
        fs::write(root.join("A-file"), "").unwrap();

        let mut project = Project::new();
        let entries = project.open(&root).unwrap();
        let names = entries
            .iter()
            .map(|entry| {
                entry
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["A-file", "a-file"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_workspace_does_not_replace_open_root() {
        let root = temporary_workspace();
        fs::create_dir_all(&root).unwrap();
        let mut project = Project::new();
        project.open(&root).unwrap();
        let original = project.root.clone();

        assert!(project.open(root.join("missing")).is_err());
        assert_eq!(project.root, original);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn watcher_handle_owns_watcher_until_dropped() {
        let root = temporary_workspace();
        fs::create_dir_all(&root).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let _watcher: WatcherHandle = super::watch_path(&root, move |path| {
            let _ = tx.send(path);
        })
        .unwrap();
        fs::write(root.join("new.txt"), "watch").unwrap();

        assert!(rx.recv_timeout(std::time::Duration::from_secs(3)).is_ok());
        drop(_watcher);
        fs::remove_dir_all(root).unwrap();
    }
}
