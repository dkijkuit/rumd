# rumd

A cross-platform markdown viewer in pure Rust. View a file rendered or as
syntax-highlighted source, and it refreshes automatically when the file
changes on disk.

## Features

- Rendered markdown: GFM tables, task lists, strikethrough, syntax
  highlighted code blocks, clickable links, local images (relative paths
  resolve against the document's directory)
- Centered reading column with adjusted typography
- Split view: rendered and source side by side
- Editing: the source pane is always editable (Source and Split views) and
  the rendered view updates live as you type, following the edited section;
  save with `Ctrl/Cmd+S`. Unsaved changes show a `•` in the window title
- If the file changes on disk while you have unsaved edits, a banner lets
  you keep editing or reload from disk — your edits are never overwritten
- Files that are not valid UTF-8 stay read-only (saving is disabled to
  avoid corrupting them)
- Search (`Ctrl/Cmd+F`): every visible occurrence is highlighted in both
  views — the current match in yellow with black text, the others with a
  subtle wash (`Enter`/`Shift+Enter` steps through matches)
- Source view: line numbers, markdown highlighting
- Dark/light theme, follows the system theme by default
- Zoom (`Ctrl/Cmd+=`, `Ctrl/Cmd+-`, `Ctrl/Cmd+0`)
- Auto-reload on file changes (degrades to manual refresh if the OS
  watch limit is hit)
- Theme, zoom, and view mode persist between runs
- Open via drag-and-drop, file dialog, or command line: `rumd README.md`

## Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl/Cmd+O` | Open file |
| `Ctrl/Cmd+S` | Save edits |
| `Ctrl/Cmd+E` | Cycle rendered/split/source |
| `Ctrl/Cmd+D` | Toggle dark/light theme |
| `Ctrl/Cmd+F` | Search |
| `Enter` / `Shift+Enter` | Next / previous match |
| `Esc` | Close search |
| `Ctrl/Cmd+=` / `Ctrl/Cmd+-` / `Ctrl/Cmd+0` | Zoom in / out / reset |
| `F5` | Reload from disk (asks first if there are unsaved edits) |

## Building

Requires a recent stable Rust toolchain (1.95 or newer).

- Linux: X11/Wayland development libraries are needed at build time, e.g.
  on Debian/Ubuntu: `sudo apt install libxkbcommon-dev libwayland-dev libx11-dev`
- macOS: no extra prerequisites
- Windows: no extra prerequisites

```sh
cargo build --release
```

The binary is at `target/release/rumd`.

## Testing

```sh
cargo test
```

Headless UI tests run on all platforms without a display server.
