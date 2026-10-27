use anyhow::Result;
use ropey::Rope;
use std::path::PathBuf;

const MAX_UNDO_STEPS: usize = 100;

#[derive(Debug, Clone)]
pub struct Buffer {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub version: u64,
    saved_rope: Rope,
    undo_stack: Vec<Rope>,
    redo_stack: Vec<Rope>,
}

impl Buffer {
    pub fn from_str(s: &str) -> Self {
        let rope = Rope::from_str(s);
        Self {
            rope: rope.clone(),
            path: None,
            dirty: false,
            version: 1,
            saved_rope: rope,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
    pub fn from_file(path: PathBuf) -> Result<Self> {
        let s = std::fs::read_to_string(&path)?;
        let rope = Rope::from_str(&s);
        Ok(Self {
            rope: rope.clone(),
            path: Some(path),
            dirty: false,
            version: 1,
            saved_rope: rope,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    pub fn set_text(&mut self, text: &str) {
        let next = Rope::from_str(text);
        if self.rope == next {
            return;
        }

        self.undo_stack.push(self.rope.clone());
        if self.undo_stack.len() > MAX_UNDO_STEPS {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
        self.rope = next;
        self.update_state();
    }

    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(self.rope.clone());
        self.rope = previous;
        self.update_state();
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(self.rope.clone());
        self.rope = next;
        self.update_state();
        true
    }

    pub fn mark_saved(&mut self) {
        self.saved_rope = self.rope.clone();
        self.dirty = false;
    }

    pub fn save_to(&mut self, path: PathBuf) -> Result<()> {
        std::fs::write(&path, self.as_string())?;
        self.path = Some(path);
        self.mark_saved();
        Ok(())
    }

    pub fn reload_from_disk(&mut self) -> Result<()> {
        let path = self
            .path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("buffer has no file path"))?;
        *self = Self::from_file(path)?;
        Ok(())
    }

    fn update_state(&mut self) {
        self.version = self.version.wrapping_add(1);
        self.dirty = self.rope != self.saved_rope;
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }
    pub fn as_string(&self) -> String {
        self.rope.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{Buffer, MAX_UNDO_STEPS};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_file() -> std::path::PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("tyin-buffer-test-{}-{id}", std::process::id()))
    }

    #[test]
    fn undo_redo_tracks_dirty_state_and_clears_redo_after_new_edit() {
        let mut buffer = Buffer::from_str("first");

        buffer.set_text("second");
        assert!(buffer.dirty);
        assert!(buffer.undo());
        assert_eq!(buffer.as_string(), "first");
        assert!(!buffer.dirty);

        assert!(buffer.redo());
        assert_eq!(buffer.as_string(), "second");
        assert!(buffer.dirty);

        buffer.set_text("third");
        assert!(!buffer.redo());
        assert_eq!(buffer.as_string(), "third");
    }

    #[test]
    fn marking_saved_resets_the_dirty_baseline() {
        let mut buffer = Buffer::from_str("before");
        buffer.set_text("after");
        buffer.mark_saved();

        assert!(!buffer.dirty);
        assert!(buffer.undo());
        assert!(buffer.dirty);
        assert!(buffer.redo());
        assert!(!buffer.dirty);
    }

    #[test]
    fn setting_identical_text_does_not_create_history() {
        let mut buffer = Buffer::from_str("same");
        let version = buffer.version;

        buffer.set_text("same");

        assert_eq!(buffer.version, version);
        assert!(!buffer.undo());
    }

    #[test]
    fn undo_history_is_bounded() {
        let mut buffer = Buffer::from_str("0");
        for value in 1..=MAX_UNDO_STEPS + 1 {
            buffer.set_text(&value.to_string());
        }

        for _ in 0..MAX_UNDO_STEPS {
            assert!(buffer.undo());
        }
        assert!(!buffer.undo());
        assert_eq!(buffer.as_string(), "1");
    }

    #[test]
    fn save_and_reload_preserve_path_and_saved_state() {
        let path = temporary_file();
        let mut buffer = Buffer::from_str("initial");
        buffer.set_text("saved");
        buffer.save_to(path.clone()).unwrap();

        assert_eq!(buffer.path.as_deref(), Some(path.as_path()));
        assert!(!buffer.dirty);
        assert_eq!(fs::read_to_string(&path).unwrap(), "saved");

        fs::write(&path, "external").unwrap();
        buffer.reload_from_disk().unwrap();
        assert_eq!(buffer.as_string(), "external");
        assert!(!buffer.dirty);
        fs::remove_file(path).unwrap();
    }
}
