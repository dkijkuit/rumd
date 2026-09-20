# rumd — Rust Markdown Viewer: Design

Date: 2026-09-20
Status: Approved design, pending implementation plan

## Purpose

rumd is a cross-platform (Linux, macOS, Windows) desktop markdown viewer
written in pure Rust. It shows a markdown file either rendered or as
syntax-highlighted source, and automatically refreshes the display when the
file changes on disk.

## Constraints and decisions

- **Pure Rust only.** No JavaScript, TypeScript, Vite, or webviews. The GUI
  is built entirely with Rust crates.
- **GUI toolkit: egui/eframe.** Chosen over iced and Slint for its mature
  cross-platform support and the availability of an existing markdown
  rendering crate.
- **Markdown rendering: `egui_commonmark`.** Chosen over a custom
  pulldown-cmark renderer: it provides GFM features, syntax highlighting,
  links, and images out of the box. A custom renderer was rejected as weeks
  of widget work for equal output.
- **Scope: core viewer.** GFM (tables, task lists, strikethrough), code block
  syntax highlighting, dark/light theme, source↔rendered toggle, file open
  (dialog, CLI argument, drag-and-drop), auto-reload on file change.

## Dependencies

| Crate | Purpose |
|---|---|
| `eframe` / `egui` | App shell, rendering, drag-and-drop events, system theme detection |
| `egui_commonmark` | Rendered view; features: pulldown-cmark GFM (tables, task lists, strikethrough) + `syntax-highlighting` (syntect) |
| `egui_code_editor` | Source view: monospace text, line numbers, syntect highlighting |
| `rfd` | Native open-file dialogs |
| `notify` | File watching for auto-reload |

## Project structure

```
rumd/
├── Cargo.toml
└── src/
    ├── main.rs        # entry: parse CLI arg, run eframe app
    ├── app.rs         # App state + update(): top bar, mode/theme toggles
    ├── viewer.rs      # rendered-mode painting via egui_commonmark
    ├── source.rs      # source-mode painting via egui_code_editor
    └── document.rs    # file loading, UTF-8 handling, watcher events
```

## Application state

The `App` struct holds:

- `path: Option<PathBuf>` — currently open file
- `raw: String` — file contents as a (possibly lossy-decoded) string
- `lossy: bool` — true if the source bytes were not valid UTF-8
- `mode: ViewMode` — `Rendered` or `Source`
- `theme: Theme` — `Dark` or `Light`, initialized from the system theme
- `watcher` — `notify` watcher handle plus debounce state
- `reload_pending: bool` — set by debounced watcher events, consumed by the
  next frame
- `error: Option<String>` — transient error banner message
- `commonmark_cache` — egui_commonmark's cache to avoid re-parsing per frame

## Data flow

1. A file is opened via CLI argument (`rumd README.md`), native dialog
   (`Ctrl/Cmd+O`), or drag-and-drop onto the window.
2. `document.rs` reads the bytes and decodes them with
   `String::from_utf8_lossy`. If any bytes were replaced, `lossy` is set and
   a badge is shown next to the filename.
3. Rendered view: egui_commonmark paints from `raw` each frame using its
   cache. Local relative image paths resolve against the open file's
   directory. Links open in the system browser.
4. Source view: `egui_code_editor` displays `raw` read-only with line
   numbers and syntect highlighting.
5. Auto-reload: a `notify` watcher on the open file fires events; these are
   debounced (~100 ms) into a `reload_pending` flag. The next frame re-reads
   the file and repaints.

## UI

- **Top bar:** Open button, filename, Rendered/Source toggle, theme toggle,
  refresh button.
- **Main area:** rendered markdown or source view, scrollable.
- **Empty state:** centered "Drop a .md file here or press Ctrl+O" hint.
- No menu bar; the top bar and keyboard shortcuts cover everything.

### Keyboard shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl/Cmd+O` | Open file |
| `Ctrl/Cmd+E` | Toggle rendered/source |
| `Ctrl/Cmd+D` | Toggle dark/light theme |
| `F5` | Manual refresh (re-read file) |

## Error handling

- **Read failure** (missing file, permission denied): transient error banner
  at the top of the window; the previously displayed document remains
  visible; the banner clears on the next user action.
- **Non-UTF-8 content:** decoded lossily; a persistent "not valid UTF-8"
  badge next to the filename prevents mistaking replacement characters for a
  rendering bug.
- **Watcher failure** (e.g. inotify watch limit): auto-reload degrades to
  the manual refresh button; a one-time info message tells the user.
- **Dropped files:** any file type is accepted and rendered — viewers should
  not be pedantic. The open dialog filters to `*.md`, `*.markdown`,
  `*.mdown`, `*.txt`.

## Testing

- **Unit tests** for `document.rs` (pure logic): lossy UTF-8 decoding,
  parent-directory resolution for relative images, watcher debounce state
  machine.
- **Snapshot tests** via `egui_kittest` for the app shell: open → render,
  toggle to source, theme switch. Headless, runnable in CI on all three
  platforms.
- **Golden-doc smoke tests:** a small set of markdown documents (tables, task
  lists, code fences, images) exercised through the rendering path to catch
  integration regressions.
- **Manual smoke per platform before release:** open dialog, drag-and-drop,
  auto-reload, shortcuts, HiDPI scaling.

## Packaging and distribution

Release phase, not v1-blocking:

- **Linux:** plain binary; README documents X11/Wayland dev-library build
  prerequisites. AppImage is a stretch goal.
- **macOS:** per-arch release builds merged into a universal binary with
  `lipo`. `.app` bundle and DMG are stretch goals.
- **Windows:** single `.exe`; code-signing and installer are stretch goals.
- **CI:** GitHub Actions builds all three platforms on release tags, added
  once the project is on git.

## Out of scope for v1

Search, sidebar file tree, math (KaTeX) and diagrams (mermaid), remote
(http) images, editing/saving, recent-files list, file associations and
installers.
