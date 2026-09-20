# rumd

A cross-platform markdown viewer in pure Rust. View a file rendered or as
syntax-highlighted source, and it refreshes automatically when the file
changes on disk.

## Features

- Rendered markdown: GFM tables, task lists, strikethrough, syntax
  highlighted code blocks, clickable links, local images (relative paths
  resolve against the document's directory)
- Source view: read-only, line numbers, markdown highlighting
- Dark/light theme, follows the system theme by default
- Auto-reload on file changes (degrades to manual refresh if the OS
  watch limit is hit)
- Open via drag-and-drop, file dialog, or command line: `rumd README.md`

## Shortcuts

| Shortcut | Action |
|---|---|
| `Ctrl/Cmd+O` | Open file |
| `Ctrl/Cmd+E` | Toggle rendered/source |
| `Ctrl/Cmd+D` | Toggle dark/light theme |
| `F5` | Refresh from disk |

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
