use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the file system must stay quiet after a change before reloading.
pub const RELOAD_DEBOUNCE: Duration = Duration::from_millis(100);

/// A markdown document loaded into memory.
pub struct Document {
    pub path: PathBuf,
    pub raw: String,
    /// True only when the source bytes were not valid UTF-8.
    pub lossy: bool,
}

impl Document {
    pub fn load(path: &Path) -> io::Result<Document> {
        let bytes = fs::read(path)?;
        let lossy = std::str::from_utf8(&bytes).is_err();
        let raw = String::from_utf8_lossy(&bytes).into_owned();
        Ok(Document {
            path: path.to_path_buf(),
            raw,
            lossy,
        })
    }

    /// Directory containing the document; base for relative image paths.
    pub fn dir(&self) -> Option<&Path> {
        self.path.parent()
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }
}

/// True when at least `threshold` has passed since `last_event`.
pub fn debounce_ready(last_event: Option<Instant>, now: Instant, threshold: Duration) -> bool {
    match last_event {
        Some(t) => now.duration_since(t) >= threshold,
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rumd_test_{}_{}", std::process::id(), name))
    }

    #[test]
    fn load_reads_valid_utf8() {
        let path = temp_path("utf8.md");
        std::fs::write(&path, "# Hello\nworld").unwrap();
        let doc = Document::load(&path).unwrap();
        assert_eq!(doc.raw, "# Hello\nworld");
        assert!(!doc.lossy);
        assert_eq!(doc.dir(), Some(path.parent().unwrap()));
        assert_eq!(doc.file_name(), path.file_name().unwrap().to_string_lossy());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn load_flags_invalid_utf8_as_lossy() {
        let path = temp_path("binary.md");
        std::fs::write(&path, [b'#', b' ', 0xFF, 0xFE]).unwrap();
        let doc = Document::load(&path).unwrap();
        assert!(doc.lossy);
        assert!(doc.raw.contains('\u{FFFD}'));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn literal_replacement_char_is_not_lossy() {
        let path = temp_path("literal_fffd.md");
        std::fs::write(&path, "\u{FFFD} already there").unwrap();
        let doc = Document::load(&path).unwrap();
        assert!(!doc.lossy);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn load_missing_file_is_error() {
        assert!(Document::load(Path::new("/nonexistent/rumd/nope.md")).is_err());
    }

    #[test]
    fn debounce_not_ready_without_event() {
        let now = Instant::now();
        assert!(!debounce_ready(None, now, Duration::from_millis(100)));
    }

    #[test]
    fn debounce_not_ready_while_recent() {
        let now = Instant::now();
        let event_at = now - Duration::from_millis(50);
        assert!(!debounce_ready(Some(event_at), now, Duration::from_millis(100)));
    }

    #[test]
    fn debounce_ready_after_quiet_period() {
        let now = Instant::now();
        let event_at = now - Duration::from_millis(150);
        assert!(debounce_ready(Some(event_at), now, Duration::from_millis(100)));
    }
}
