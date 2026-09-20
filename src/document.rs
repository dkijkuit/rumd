use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

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

/// Watches a document's parent directory and reports changes to the
/// document file itself on a channel. Watching the parent (not the file)
/// keeps the watch alive when editors save by atomic rename.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    pub events: Receiver<()>,
}

impl FileWatcher {
    pub fn spawn(path: &Path) -> notify::Result<Self> {
        let (tx, rx) = channel();
        let target = path.to_path_buf();
        let mut watcher = notify::recommended_watcher(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    let touches_target = event.paths.iter().any(|p| p == &target);
                    let kind_matches = matches!(
                        event.kind,
                        EventKind::Modify(_) | EventKind::Create(_)
                    );
                    if touches_target && kind_matches {
                        let _ = tx.send(());
                    }
                }
            },
        )?;
        let watch_target = path.parent().unwrap_or(path).to_path_buf();
        watcher.watch(&watch_target, RecursiveMode::NonRecursive)?;
        Ok(FileWatcher {
            _watcher: watcher,
            events: rx,
        })
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

    use std::sync::mpsc::TryRecvError;

    #[test]
    fn watcher_spawns_for_existing_file() {
        let path = temp_path("watched.md");
        std::fs::write(&path, "v1").unwrap();
        let w = FileWatcher::spawn(&path).unwrap();
        assert!(matches!(w.events.try_recv(), Err(TryRecvError::Empty)));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn watcher_errors_for_missing_path() {
        assert!(FileWatcher::spawn(Path::new("/nonexistent/rumd/dir/x.md")).is_err());
    }

    #[test]
    fn watcher_reports_writes_and_atomic_replace() {
        let path = temp_path("events.md");
        std::fs::write(&path, "v1").unwrap();
        let w = FileWatcher::spawn(&path).unwrap();
        // Direct write (Modify in place)...
        std::fs::write(&path, "v2").unwrap();
        let mut seen_write = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if w.events.try_recv().is_ok() {
                seen_write = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(seen_write, "watcher did not report the direct write");
        // ...and atomic-style replace: write a sibling, rename over the file.
        let sibling = temp_path("events_new.md");
        std::fs::write(&sibling, "v3").unwrap();
        std::fs::rename(&sibling, &path).unwrap();
        let mut seen_replace = false;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if w.events.try_recv().is_ok() {
                seen_replace = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(seen_replace, "watcher did not report the rename-over save");
        std::fs::remove_file(&path).unwrap();
    }
}
