# rumd Project Website — Design Spec

Date: 2026-09-28
Status: Approved design, pending implementation plan

## Goal

A professional, self-contained static website promoting rumd, the
cross-platform markdown viewer in pure Rust. The site presents the logo,
real application screenshots, the advantages of choosing rumd, its feature
set, keyboard shortcuts, and build instructions. It must be ready to publish
as-is (e.g. GitHub Pages) once the repository URL is known.

## Non-goals

- No build step, no JS framework, no package manager for the site itself.
- No blog, changelog, releases, or download mirrors (no releases exist yet).
- No analytics, cookies, or external network requests (self-hosted assets
  only; system font stack instead of web fonts).

## Deliverables

```
website/
  index.html      # the entire site, one page
  style.css       # all styling, dark theme, responsive
  assets/
    logo_without_bg.png   # copied from logo/
    dark-rendered.png     # real app screenshot
    dark-split.png        # real app screenshot
    light-rendered.png    # real app screenshot
    dark-search.png       # real app screenshot (see Screenshots)
```

## Screenshots (real captures)

No full-app screenshots exist; the site requires them. They are produced by
a throwaway test using the existing `egui_kittest::Harness` pattern from
`top_bar_snapshot` (src/app.rs:1542), at 1280×800, writing PNGs directly
into `website/assets/`. The test is deleted after capture; only the PNGs
remain.

Setup common to all captures:

- A sample document exercising the renderer: H1/H2/H3 headings, bold and
  italic, strikethrough, a GFM table, a task list, a fenced Rust code
  block, a blockquote, and links. No local image references.
- `app.open_path(&doc)` opens it; two `harness.run()` calls let the UI
  settle (same as existing snapshot tests).

Captures:

1. `dark-rendered.png` — `mode = Rendered`, dark theme (default).
2. `dark-split.png` — `mode = Split`, dark theme.
3. `light-rendered.png` — `mode = Rendered`, `theme_pref = Light`.
4. `dark-search.png` — `mode = Rendered`, dark theme, search opened with a
   query producing visible highlights. Attempted via harness key input; if
   driving the search UI proves unreliable, this capture is dropped
   without ceremony (the site design works with three screenshots).

The snapshot output path is set via `SnapshotOptions::output_path` pointed
at `website/assets/` (same mechanism the existing tests use for
`tests/snapshots`).

## Site structure (single page, top to bottom)

1. **Nav** (sticky): logo mark + "rumd" wordmark; anchor links Features,
   Screenshots, Shortcuts, Get started; a GitHub button.
   - The GitHub URL is a placeholder (`https://github.com/TODO-rumd/rumd`)
     marked with an HTML comment `TODO: replace with the real repository
     URL before publishing`, since the project is not yet published.
2. **Hero**: H1 tagline "A cross-platform markdown viewer in pure Rust";
   one-sentence sub-line covering live reload and rendered/source views;
   CTA buttons "Get started" (anchor) and "View source" (GitHub link);
   `dark-rendered.png` presented in a subtle browser-window-style frame.
3. **Why rumd** (advantages — the *why*, distinct from the feature list):
   - Single native binary, pure Rust — no Electron, no browser engine, low
     memory footprint.
   - Cross-platform: Linux, macOS, Windows.
   - Fully local and private: no accounts, no telemetry; files never leave
     the machine.
   - Edits are never overwritten: conflict banner when the file changes on
     disk under unsaved edits.
   - Quality backed by headless UI tests that run on every platform
     without a display server.
   - Opens anywhere: drag-and-drop, file dialog, or CLI argument.
4. **Features grid** (the *what*, from README):
   - Rendered markdown: GFM tables, task lists, strikethrough,
     syntax-highlighted code blocks, clickable links, local images.
   - Split view: rendered and source side by side.
   - Live editing: source pane editable, rendered pane follows as you type;
     save with Ctrl/Cmd+S; unsaved-changes dot in the title.
   - Search: highlights every match in both views; Enter/Shift+Enter to
     step through.
   - Dark/light theme, follows system theme by default.
   - Zoom and view-mode preferences persist between runs.
   - Auto-reload on file changes (degrades gracefully at OS watch limits).
   - Line numbers and markdown highlighting in source view.
5. **Screenshots**: gallery of the captures with one-line captions.
6. **Shortcuts**: the shortcut table from README, restyled.
7. **Get started**: prerequisites per OS (Linux X11/Wayland dev libraries;
   none for macOS/Windows), `cargo build --release`, run
   `./target/release/rumd README.md`. Rust 1.95+ noted.
8. **Footer**: "rumd — markdown viewing, the Rust way", link back to top,
   GitHub link.

## Visual design

- Palette from the logo: page background dark navy `#0d1420` (with slightly
  lighter raised surfaces ~`#151d2e`), text near-white `#e8ecf4`, accent
  blue `#3b82f6`, muted text `#94a3b8`.
- Light screenshots sit on raised light cards so they don't glare against
  the dark page.
- System font stack (`system-ui`, …) for prose; monospace stack for code
  and shortcut keys.
- Responsive: single-column under 720px, nav collapses to a simple wrapped
  row (no hamburger JS).
- Styling is pure CSS in `style.css`; the site ships zero JavaScript.

## Acceptance criteria

- `website/index.html` opens correctly from `file://` with all assets
  resolving (relative paths).
- All four (or three, if search capture dropped) screenshots appear and
  are genuine app captures.
- Page is usable at 360px, 768px, and 1280px widths.
- Every feature claim on the site is traceable to README.md; nothing
  invented.
- No external network requests (fonts, CDNs, analytics).
- The placeholder GitHub URL is a single string, replaced with one
  find-and-replace; its first occurrence carries an HTML comment marking
  it as a placeholder.
- The throwaway screenshot test is removed from the codebase after capture.
