# rumd — Polish Pass: Design

Date: 2026-09-20
Status: Approved design, pending implementation plan

## Purpose

A full polish pass over rumd v1. v1 is a working read-only markdown viewer
(rendered + source views, dark/light theme, auto-reload, drag-and-drop
open). This pass improves how the app looks and adds the three features the
missing most: zoom, in-document search, and a split view. It also adds
persistence so preferences survive restarts.

## Scope

In scope:

1. **Split view** — rendered and source panes side by side (`Split` mode).
2. **In-document search** — `Ctrl+F` find bar; matches select in the source
   view and jump by section in the rendered view.
3. **Zoom** — `Ctrl+=` / `Ctrl+-` / `Ctrl+0`, persisted.
4. **Centered content column + typography** for the rendered view.
5. **App UI polish** — top bar tooltips, dismissable error banner, richer
   empty state.
6. **Preference persistence** — theme, zoom, and view mode restore on
   startup.

Out of scope: outline/TOC sidebar, recent files, editing, extra themes,
rendered-view inline match highlighting (explicitly declined during
brainstorming).

## Constraints

- Pure Rust, egui/eframe; no new heavy dependencies.
- Rendering stays on `egui_commonmark`; source view stays on
  `egui_code_editor`.
- All existing behavior and tests keep working (auto-reload, lossy badge,
  watcher failure banner, drop-open, CLI open).
- Headless `egui_kittest` tests must keep passing without a display server.

## Research findings (APIs this design relies on)

- `egui_code_editor::CodeEditor::show` returns
  `egui::TextEditOutput`, which exposes the `TextEditState` (mutable
  `ccursor_range`) and the laid-out `galley`. Setting `ccursor_range` to a
  match and storing the state back under the TextEdit's response id
  selects the match; `galley.pos_from_cursor` gives its pixel rect for
  `Ui::scroll_to_rect`. This gives precise search in the source view.
- `egui_commonmark::CommonMarkViewer` has `enable_scroll_to_heading`
  (rejected: only fires on link activation, which cannot be simulated),
  `default_width`, `max_image_width`, and
  `syntax_theme_light` / `syntax_theme_dark` (used for polish).
- `egui::Context::set_zoom_factor` exists, and **egui 0.36 already binds
  the keyboard zoom shortcuts natively** (`Options::zoom_with_keyboard`
  defaults to true; `Ctrl+=`/`Ctrl+-` step ±0.1, `Ctrl+0` resets, clamped
  to 0.2–5.0). The app therefore uses egui's native zoom and syncs
  `ctx.zoom_factor()` into app state for persistence instead of
  registering competing shortcuts.
- eframe's `persistence` provides `Storage`; preferences are stored as
  plain strings to avoid adding serde derives.

## Approaches considered for rendered-view search jumps

- **A. Section-split rendering (chosen).** Split the document into
  sections, render each section with its own `CommonMarkViewer::show`
  inside the scroll area, record each section's rect, and
  `Ui::scroll_to_rect` on the section containing the current match.
- **B. Single render + proportional scroll (rejected).** Jump scrolls to
  `(match_offset / doc_len) × content_height`. Zero render changes, but
  imprecise on long documents and multiple matches land in the same place.
- **C. Anchor injection via `enable_scroll_to_heading` (rejected).** The
  scroll only triggers when the user clicks a link with that anchor; there
  is no programmatic trigger.

## Feature designs

### 1. View modes

- `ViewMode` becomes `Rendered | Split | Source`.
- `Ctrl+E` cycles Rendered → Split → Source → Rendered. The top-bar
  buttons become a three-way segmented control; clicking a segment selects
  that mode directly.
- Split mode: `Ui::columns(2, ...)` — rendered left, source right — with
  independent scrolling per pane. The existing single-pane paths are
  reused as-is in each column.

### 2. Search

New module `src/search.rs`.

**State.** `SearchState { open: bool, query: String, matches: Vec<usize>,
current: usize, }` living on `App`. Matches are byte offsets into the raw
text, found with case-insensitive `str::match_indices` on lowercased
copies. An empty query yields no matches.

**Find bar.** A top panel shown directly below the top bar when open:
auto-focused query field, `current+1/match_count` label, previous/next
buttons, and a close button. Search is always case-insensitive in v1.
`Esc` closes and clears focus.
`Enter` moves to the next match, `Shift+Enter` to the previous, wrapping
around.

**Shortcut interplay.** `Ctrl+F` opens the bar and focuses the field
(refocuses if already open). `Enter`/`Shift+Enter`/`Esc` are handled
globally whenever the bar is open, regardless of keyboard focus, so they
keep working after the source view steals selection focus. Zoom keeps
egui's native behavior (no per-field suppression).

**Source view.** `source::show` gains an optional
`SearchTarget { byte_range: Range<usize> }`. After the editor renders, use
the returned `TextEditOutput`: compute the `CCursor` pair for the match
(char indices converted from byte offsets), set
`state.ccursor_range` and store it back via the response's widget id, then
`ui.scroll_to_rect(galley.pos_from_cursor(...))` to bring it into view.
The match appears as a text selection. Fallback if state re-storage proves
unreliable in practice: keep the scroll jump, drop the selection.

**Rendered view.** `RenderedView::show` gains an optional jump target.
Sections are computed once per document revision (see below); each frame
renders sections sequentially inside the existing vertical `ScrollArea`,
recording each section's rect via the `InnerResponse` of its
`CommonMarkViewer::show` call. When a jump is pending, `Ui::scroll_to_rect`
brings the target section's top into view.

**Section splitting.** Fence-aware line scan:
- If the document contains at least one ATX heading (a line starting with
  `#` outside a fence), sections split at heading lines only.
- Otherwise, sections split at blank lines.
- In both cases, lines inside fenced code blocks (``` / ~~~ toggles) never
  split. A document that is one giant fence is a single section.

Sections are re-split whenever the document reloads. The split is pure
text processing, unit-testable without a UI.

**Reloads.** Search stays open across auto-reload; matches recompute
against the new text and `current` clamps to the new count.

**Known fidelity limitation (accepted).** Per-section rendering parses
each section independently, so CommonMark reference-style link
definitions (`[text]: url`) do not resolve when defined in a different
section, and cross-section GFM edge cases may render slightly
differently. Chosen because precise jumps require per-section geometry;
correctness impact is limited to reference-heavy documents.

### 3. Zoom

egui 0.36 provides the zoom shortcuts natively (`zoom_with_keyboard` is on
by default): `Ctrl+=` zoom in, `Ctrl+-` zoom out (±0.1 per press), and
`Ctrl+0` reset, clamped to 0.2–5.0. rumd does not register its own zoom
shortcuts (that would double-zoom). Instead, the app mirrors
`ctx.zoom_factor()` into app state every frame and persists it; a stored
value outside 0.2–5.0 clamps on load.

- Affects all UI including the source editor (its font size is set in
  points and scales with the zoom factor automatically).

### 4. Persistence

- eframe `Storage` (persistence feature; enable explicitly if not default).
- Stored keys, encoded as plain strings: theme preference
  (`dark`/`light`/`system`), zoom factor, view mode
  (`rendered`/`split`/`source`).
- Loaded in the app creator from `cc.storage`; saved in
  `eframe::App::save`. Unparseable values fall back to defaults.
- Window position/size uses eframe's built-in window persistence; no
  custom handling.

### 5. Centered column + typography

- Rendered content sits in a centered inner Ui of width
  `min(660, available_width − 80)` (~70 characters; raised from an
  initial 800px after visual review showed the column read as
  full-width at typical window sizes), built with `UiBuilder::max_rect`,
  with the viewer's `default_width` set to the actual column width so
  text wraps at the column edge. Source view keeps the full window
  width.
- Text style sizes raised for readability: Body ≈ 16, Heading ≈ 28,
  Monospace ≈ 14, Button ≈ 14 (final values eyeballed during
  implementation against both themes).
- Code block syntax themes matched to the app theme via
  `syntax_theme_light`/`syntax_theme_dark` on the viewer.

### 6. App UI polish

- **Top bar.** Every button gets `.on_hover_text` tooltips that include
  its shortcut (Open `Ctrl+O`, mode buttons `Ctrl+E`, theme `Ctrl+D`,
  Refresh `F5`). The filename label's tooltip shows the full path.
  Mode buttons are grouped in a `Frame::group` for a segmented-control
  look.
- **Error banner.** Warning-tinted frame with a Close button that clears
  the error; otherwise unchanged semantics (one-shot watcher notice,
  refresh errors kept).
- **Empty state.** Centered stack: large faded "rumd" wordmark, the
  existing drop/open hint line, and an `Open…` button wired to the same
  action as `Ctrl+O`.

## Shortcut summary

| Shortcut | Action |
|---|---|
| `Ctrl/Cmd+O` | Open file |
| `Ctrl/Cmd+E` | Cycle view mode |
| `Ctrl/Cmd+D` | Toggle theme |
| `Ctrl/Cmd+F` | Open/focus search |
| `Enter` / `Shift+Enter` | Next / previous match (search focused) |
| `Esc` | Close search (search focused) |
| `Ctrl/Cmd+=` / `Ctrl/Cmd+-` / `Ctrl/Cmd+0` | Zoom in / out / reset |
| `F5` | Refresh from disk |

## Error handling

- No matches: counter shows `0/0`; next/prev are no-ops.
- Zoom factor outside 0.2–5.0 in storage clamps on load; unparseable
  values fall back to 1.0.
- Search across reload: matches recomputed; `current` clamps; a stale jump
  target is dropped rather than scrolled.
- Section splitter must never panic on adversarial input (unclosed fences,
  fences with tildes, heading-only docs) — covered by unit tests.
- Split mode with no document open shows the empty state, not broken
  panes.

## Testing

- **Unit (no UI):** section splitter (headings, fences, blank-line
  fallback, adversarial input), match finding (case folding, empty query,
  byte→char index conversion), persistence encode/decode round-trip and
  clamping, zoom clamp.
- **Headless UI (egui_kittest):**
  - Mode cycling `Ctrl+E` through three modes; split mode shows both
    heading text and source content.
  - Search: open with `Ctrl+F`, type query via harness events, `n/m`
    label appears, Enter advances `current`, source selection/scroll
    fires without panic, rendered jump does not panic on heading-less
    docs.
  - Zoom: `Ctrl+=` raises `ctx.zoom_factor()`; the mirrored app value
    follows; reset returns to 1.0; persistence decode clamps and falls
    back on garbage.
  - Centered column: rendered text does not span full window width.
  - Persistence: save/load round-trip of prefs struct.
  - All v1 tests keep passing (error banner now has a Close button —
    update assertions accordingly; empty state still exposes the hint
    label).

## Project structure after the pass

```
src/
├── main.rs        # entry: CLI arg, persistence loading wired in creator
├── app.rs         # App state: modes, search state, zoom, prefs, top bar
├── search.rs      # NEW: match finding, section splitting, jump bookkeeping
├── viewer.rs      # rendered view: centered column, per-section render+jump
├── source.rs      # source view: optional match selection + scroll
└── document.rs    # unchanged
```
