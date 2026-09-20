# rumd Polish Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a split view, in-document search (find bar + source selection + rendered section jumps), preference persistence, a centered reading column with better typography, and general UI polish to rumd.

**Architecture:** A new `src/search.rs` holds the pure text-processing core (case-insensitive match finder, fence-aware section splitter, `SearchState`). `app.rs` owns the find bar, mode cycling, and jump bookkeeping; `viewer.rs` renders per-section inside a centered column and consumes pending jumps; `source.rs` sets a selection on `egui_code_editor`'s `TextEditState` and scrolls to it. Zoom is egui's native keyboard zoom, mirrored into app state for persistence via `eframe::Storage`.

**Tech Stack:** Rust 2024, eframe/egui 0.36, egui_commonmark 0.25 (`better_syntax_highlighting`), egui_code_editor 0.4, egui_kittest 0.36 (headless UI tests), notify, rfd.

**Spec:** `docs/superpowers/specs/2026-09-20-rumd-polish-pass-design.md`

## Global Constraints

- No new dependencies. The only Cargo.toml change is adding the `persistence` feature to the existing `eframe` dependency (Task 8).
- All APIs used below were verified against the exact sources in `~/.cargo/registry/src/` for egui 0.36.2 / eframe 0.36.2 / egui_kittest 0.36.2 / egui_commonmark 0.25 / egui_commonmark_backend 0.25: `egui::CCursorRange::two`, `TextEditState { pub cursor: TextCursorState }` with `set_char_range`/`load`/`store`, `Galley::pos_from_cursor(CCursor) -> Rect`, `Ui::new_child(UiBuilder)`, `Ui::columns`, `Ui::scroll_to_rect(Rect, Option<Align>)`, `Context::all_styles_mut`, `Context::zoom_factor()`, `Frame::group(&Style)`, `eframe::Storage::{get_string,set_string,remove_string,flush}`, `eframe::App::save`, kittest `key_press`/`key_press_mods`, `CommonMarkViewer::{default_width(Option<usize>), syntax_theme_light, syntax_theme_dark}`.
- Search is always case-insensitive; matches are byte-offset `Range<usize>`s into the raw document.
- Centered column: width `min(800, available - 48)`, floor 320 (`COLUMN_WIDTH = 800.0`, `COLUMN_MARGIN = 24.0`, `COLUMN_MIN = 320.0`).
- Zoom: egui native (`Ctrl+=`/`Ctrl+-`/`Ctrl+0`); app mirrors `ctx.zoom_factor()` into `App::zoom` every frame; persisted values clamp to 0.2–5.0, garbage falls back to 1.0.
- Every v1 behavior and test keeps working (auto-reload, lossy badge, watcher-failure banner, refresh errors, drag-drop, CLI open). The v1 test `mode_toggle_button_switches_views` must keep passing unchanged.
- Headless tests only (`cargo test`); edition 2024; `cargo clippy --all-targets -- -D warnings` clean at every commit.
- Commit after every task (and at the noted steps inside tasks).

## Review Focus

Inputs no task test covers directly; each is pinned by the named test in the named task:

1. **A document that is one giant unclosed fence** — must render as a single section; search jumps must no-op, never panic. Pinned by `unclosed_fence_never_splits` (Task 2) and `single_fence_document_renders_as_one_section` (Task 6).
2. **Auto-reload while search is open** — counter must clamp to the new match count and never index out of bounds. Pinned by `reload_recomputes_matches_and_clamps_current` (Task 4).
3. **Multibyte characters before a match** — byte offsets from matching and char indices for egui cursors must stay aligned. Pinned by `offsets_are_byte_offsets_into_original`, `char_index_of_byte_handles_multibyte` (Task 1), and `source_selection_handles_multibyte_offsets` (Task 5).
4. **Tiny window in split mode** — both panes must shrink without panic. Pinned by `split_mode_tiny_window_does_not_panic` (Task 3).
5. **Garbage or missing persisted preferences** — defaults apply, zoom clamps. Pinned by `prefs_roundtrip_and_defaults` (Task 8).

---

### Task 1: Search core — case-insensitive match finder

**Files:**
- Create: `src/search.rs`
- Modify: `src/main.rs` (add `mod search;`)

**Interfaces:**
- Consumes: nothing (pure functions).
- Produces:
  - `pub fn find_matches(text: &str, query: &str) -> Vec<std::ops::Range<usize>>` — byte-offset ranges into `text`, case-insensitive, empty query → empty vec.
  - `pub fn char_index_of_byte(text: &str, byte: usize) -> usize` — char index for egui cursors.

- [ ] **Step 1: Create the module with implementation and failing-to-pass tests**

Create `src/search.rs`:

```rust
//! Search support: match finding and document section splitting.

use std::ops::Range;

/// Find all case-insensitive occurrences of `query` in `text`.
/// Returns byte-offset ranges into the original `text`.
/// An empty query yields no matches.
pub fn find_matches(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let fold = |c: char| -> Vec<char> { c.to_lowercase().collect() };
    let query_folded: Vec<Vec<char>> = query.chars().map(fold).collect();
    let text_folded: Vec<(usize, usize, Vec<char>)> = text
        .char_indices()
        .map(|(byte, c)| (byte, c.len_utf8(), fold(c)))
        .collect();
    let mut matches = Vec::new();
    if text_folded.len() < query_folded.len() {
        return matches;
    }
    'window: for start in 0..=(text_folded.len() - query_folded.len()) {
        for (step, q_chars) in query_folded.iter().enumerate() {
            if text_folded[start + step].2 != *q_chars {
                continue 'window;
            }
        }
        let last = start + query_folded.len() - 1;
        let end = text_folded[last].0 + text_folded[last].1;
        matches.push(text_folded[start].0..end);
    }
    matches
}

/// Convert a byte offset into a char index (egui cursors are char-based).
pub fn char_index_of_byte(text: &str, byte: usize) -> usize {
    text[..byte.min(text.len())].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_case_insensitive_matches() {
        assert_eq!(find_matches("AbC abc ABC", "abc"), vec![0..3, 4..7, 8..11]);
    }

    #[test]
    fn empty_query_yields_no_matches() {
        assert!(find_matches("hello", "").is_empty());
        assert!(find_matches("", "x").is_empty());
    }

    #[test]
    fn offsets_are_byte_offsets_into_original() {
        // 'é' is two bytes; the match range must still slice the original text.
        let text = "éx abc";
        let m = find_matches(text, "abc");
        assert_eq!(m, vec![3..6]);
        assert_eq!(&text[m[0].clone()], "abc");
    }

    #[test]
    fn overlapping_occurrences_count_separately() {
        assert_eq!(find_matches("aaa", "aa"), vec![0..2, 1..3]);
    }

    #[test]
    fn char_index_of_byte_handles_multibyte() {
        let text = "éx abc";
        assert_eq!(char_index_of_byte(text, 3), 2);
        assert_eq!(char_index_of_byte(text, 100), text.chars().count());
    }
}
```

In `src/main.rs`, add the module below the existing ones:

```rust
mod search;
```

- [ ] **Step 2: Run the tests**

Run: `cargo test search`
Expected: PASS (new module + 5 tests; all other tests still pass).

- [ ] **Step 3: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/search.rs src/main.rs
git commit -m "feat: search core with case-insensitive match finder"
```

---

### Task 2: Section splitter for rendered-view jumps

**Files:**
- Modify: `src/search.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub fn split_sections(text: &str) -> Vec<Range<usize>>` — fence-aware sections: ATX-heading boundaries when any heading exists outside fences, otherwise blank-line boundaries.
  - `pub fn section_containing(sections: &[Range<usize>], at: usize) -> Option<usize>`.

- [ ] **Step 1: Add the splitter, the lookup helper, and tests**

Append to `src/search.rs` (after `char_index_of_byte`, before the test module):

```rust
/// Split `text` into sections for rendered-view jumps.
///
/// Boundaries are never placed inside fenced code blocks (``` or ~~~).
/// If any ATX heading (`#`…`) line exists outside a fence, sections start
/// at heading lines; otherwise at blank lines that precede content.
/// A document with no boundaries is a single section.
pub fn split_sections(text: &str) -> Vec<Range<usize>> {
    let lines: Vec<(usize, &str)> = {
        let mut v = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            v.push((offset, line));
            offset += line.len();
        }
        v
    };

    let mut fence: Option<char> = None;
    let mut headings = Vec::new();
    let mut blanks = Vec::new();
    for (i, &(offset, line)) in lines.iter().enumerate() {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        if let Some(marker) = fence {
            if trimmed.starts_with(marker) {
                fence = None;
            }
            continue;
        }
        if trimmed.starts_with("```") {
            fence = Some('`');
            continue;
        }
        if trimmed.starts_with("~~~") {
            fence = Some('~');
            continue;
        }
        if trimmed.starts_with('#') {
            headings.push(offset);
        }
        let next_is_content = lines
            .get(i + 1)
            .is_some_and(|&(_, l)| !l.trim().is_empty());
        if content.trim().is_empty() && next_is_content {
            blanks.push(offset);
        }
    }

    let boundaries: &[usize] = if headings.is_empty() { &blanks } else { &headings };
    let mut sections = Vec::new();
    let mut start = 0;
    for &b in boundaries {
        if b > start {
            sections.push(start..b);
            start = b;
        }
    }
    if start < text.len() {
        sections.push(start..text.len());
    }
    sections
}

/// Index of the section containing byte offset `at`.
pub fn section_containing(sections: &[Range<usize>], at: usize) -> Option<usize> {
    sections.iter().position(|r| r.start <= at && at < r.end)
}
```

Append inside the existing `mod tests`:

```rust
    #[test]
    fn splits_at_headings_outside_fences() {
        let md = "intro\n\n# One\ntext\n\n## Two\nmore";
        let s = split_sections(md);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], 0..7);
        assert_eq!(&md[s[1].clone()], "# One\ntext\n\n");
        assert_eq!(&md[s[2].clone()], "## Two\nmore");
    }

    #[test]
    fn heading_inside_fence_is_not_a_boundary() {
        let md = "a\n\n```rust\n# not a heading\n```\n\nb";
        // No headings outside fences → blank-line fallback applies.
        assert_eq!(split_sections(md).len(), 2);
    }

    #[test]
    fn tilde_fences_are_respected() {
        let md = "a\n\n~~~\n## not a heading\n~~~\n\nb";
        assert_eq!(split_sections(md).len(), 2);
    }

    #[test]
    fn unclosed_fence_never_splits() {
        let md = "a\n\n```\n# heading inside\nmore\n\nstill fenced";
        let s = split_sections(md);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0], 0..md.len());
    }

    #[test]
    fn blank_line_fallback_when_no_headings() {
        let md = "para one\n\npara two\n\npara three";
        let s = split_sections(md);
        assert_eq!(s.len(), 3);
        assert_eq!(s[2], 19..md.len());
    }

    #[test]
    fn empty_text_has_no_sections() {
        assert!(split_sections("").is_empty());
    }

    #[test]
    fn section_containing_finds_and_rejects() {
        let sections = vec![0..5, 5..10];
        assert_eq!(section_containing(&sections, 3), Some(0));
        assert_eq!(section_containing(&sections, 7), Some(1));
        assert_eq!(section_containing(&sections, 10), None);
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test search`
Expected: PASS (12 tests in `search::tests` total).

- [ ] **Step 3: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/search.rs
git commit -m "feat: fence-aware section splitter for rendered search jumps"
```

---

### Task 3: Split view mode, cycling, and segmented top bar

**Files:**
- Modify: `src/app.rs` (ViewMode enum, cycle logic, top bar, CentralPanel)

**Interfaces:**
- Consumes: existing `RenderedView::show(ui, markdown)` and `source::show(ui, doc, theme)` signatures (unchanged in this task).
- Produces:
  - `pub enum ViewMode { Rendered, Split, Source }` in `app.rs`.
  - `Action::ToggleMode` cycles Rendered → Split → Source → Rendered.
  - Top bar gains a "Split" button inside a `Frame::group` with "Rendered"/"Source".
  - Split layout: `ui.columns(2, ...)` rendered left, source right.

- [ ] **Step 1: Write the failing tests**

In `src/app.rs` `mod tests`, add (with the existing helpers `temp_path` and `CWD_LOCK`):

```rust
    #[test]
    fn ctrl_e_cycles_three_modes() {
        let doc = temp_path("cycle.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Cycle Heading\n\nbody").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(900.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        assert_eq!(app.borrow().mode, ViewMode::Rendered);
        harness.key_press_mods(egui::Modifiers::CTRL, egui::Key::E);
        harness.run();
        assert_eq!(app.borrow().mode, ViewMode::Split);
        harness.key_press_mods(egui::Modifiers::CTRL, egui::Key::E);
        harness.run();
        assert_eq!(app.borrow().mode, ViewMode::Source);
        harness.key_press_mods(egui::Modifiers::CTRL, egui::Key::E);
        harness.run();
        assert_eq!(app.borrow().mode, ViewMode::Rendered);
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn split_button_shows_both_panes() {
        let doc = temp_path("splitpanes.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Split Heading\n\nbody").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_label("Split").click();
        harness.run();
        harness.run();
        // Rendered pane shows the heading; the app is in Split mode.
        harness.get_by_label("Split Heading");
        assert_eq!(app.borrow().mode, ViewMode::Split);
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn split_mode_tiny_window_does_not_panic() {
        let doc = temp_path("splittiny.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Tiny\n\nbody").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().mode = ViewMode::Split;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(300.0, 200.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.run();
        std::fs::remove_file(&doc).unwrap();
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test`
Expected: FAIL to compile — `ViewMode::Split` does not exist.

- [ ] **Step 3: Implement**

In `src/app.rs`:

1. Extend the enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Rendered,
    Split,
    Source,
}
```

2. Change the `ToggleMode` handling to a cycle — there are two places that map `ToggleMode` (in `handle_events` and in `show_top_bar`). Replace the inner `match` in both with:

```rust
self.mode = match self.mode {
    ViewMode::Rendered => ViewMode::Split,
    ViewMode::Split => ViewMode::Source,
    ViewMode::Source => ViewMode::Rendered,
};
```

3. In `show_top_bar`, wrap the two existing mode buttons in a grouped frame and add the Split button. Replace the block from `if ui.selectable_label(self.mode == ViewMode::Rendered, "Rendered")` through the end of the `Source` button with:

```rust
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        // Right-to-left layout: add in reverse to read
                        // Rendered | Split | Source left-to-right.
                        if ui
                            .selectable_label(self.mode == ViewMode::Source, "Source")
                            .clicked()
                            && self.mode != ViewMode::Source
                        {
                            action = Some(Action::ToggleMode);
                        }
                        if ui
                            .selectable_label(self.mode == ViewMode::Split, "Split")
                            .clicked()
                            && self.mode != ViewMode::Split
                        {
                            action = Some(Action::ToggleMode);
                        }
                        if ui
                            .selectable_label(self.mode == ViewMode::Rendered, "Rendered")
                            .clicked()
                            && self.mode != ViewMode::Rendered
                        {
                            action = Some(Action::ToggleMode);
                        }
                    });
```

Note: the segmented control cycles rather than selecting directly; this keeps `Action::ToggleMode` semantics and the v1 test (`mode_toggle_button_switches_views`) passing.

4. In `App::show`, replace the CentralPanel body to add the Split arm (note `let ctx = ui.ctx().clone();` already exists at the top of `show`):

```rust
        egui::CentralPanel::default().show(ui, |ui| match &self.doc {
            None => self.show_empty_state(ui),
            Some(doc) => match self.mode {
                ViewMode::Rendered => self.rendered.show(ui, &doc.raw),
                ViewMode::Source => {
                    let theme = source::code_theme(self.theme_pref, &ctx);
                    source::show(ui, doc, theme);
                }
                ViewMode::Split => {
                    let theme = source::code_theme(self.theme_pref, &ctx);
                    ui.columns(2, |columns| {
                        self.rendered.show(&mut columns[0], &doc.raw);
                        source::show(&mut columns[1], doc, theme);
                    });
                }
            },
        });
```

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: PASS (3 new tests + all v1 tests, including `mode_toggle_button_switches_views`).

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/app.rs
git commit -m "feat: split view mode with three-way cycle and segmented mode control"
```

---

### Task 4: Search state, find bar, and global search keys

**Files:**
- Modify: `src/search.rs` (SearchState), `src/app.rs` (Action::Search, find bar, recompute)

**Interfaces:**
- Consumes: `search::find_matches` (Task 1).
- Produces:
  - `pub struct SearchState { pub open: bool, pub query: String, pub matches: Vec<Range<usize>>, pub current: usize }` with `current_match(&self) -> Option<&Range<usize>>` and `step(&mut self, delta: isize)`.
  - `Action::Search` mapped from `Ctrl/Cmd+F`.
  - `App { search: SearchState }`, `App::recompute_matches(&mut self)` (also called from `open_path` on reload).
  - Find-bar panel id `"rumd_search_bar"`, field id constant `SEARCH_FIELD: &str = "rumd_search_field"`.
  - Global keys while open: `Enter` next, `Shift+Enter` prev, `Esc` close.
  - Counter label `"{current+1}/{total}"` (weak text), buttons `<`, `>`, `Close`.

- [ ] **Step 1: Add SearchState with tests**

Append to `src/search.rs` (before the test module):

```rust
/// Live find-bar state.
#[derive(Default)]
pub struct SearchState {
    pub open: bool,
    pub query: String,
    /// Byte-offset ranges into the current document.
    pub matches: Vec<Range<usize>>,
    /// Index of the current match into `matches`.
    pub current: usize,
}

impl SearchState {
    pub fn current_match(&self) -> Option<&Range<usize>> {
        self.matches.get(self.current)
    }

    /// Advance `delta` matches, wrapping at both ends.
    pub fn step(&mut self, delta: isize) {
        if self.matches.is_empty() {
            self.current = 0;
            return;
        }
        let n = self.matches.len() as isize;
        self.current = (self.current as isize + delta).rem_euclid(n) as usize;
    }
}
```

Append inside `mod tests`:

```rust
    #[test]
    fn search_state_steps_with_wraparound() {
        let mut s = SearchState {
            open: true,
            query: "a".into(),
            matches: vec![0..1, 2..3, 4..5],
            current: 0,
        };
        s.step(1);
        assert_eq!(s.current, 1);
        s.step(-1);
        assert_eq!(s.current, 0);
        s.step(-1);
        assert_eq!(s.current, 2);
        s.step(1);
        assert_eq!(s.current, 0);
    }

    #[test]
    fn search_state_step_on_empty_is_noop() {
        let mut s = SearchState::default();
        s.step(1);
        s.step(-1);
        assert_eq!(s.current, 0);
        assert!(s.current_match().is_none());
    }
```

- [ ] **Step 2: Write the failing app tests**

In `src/app.rs` `mod tests`:

```rust
    #[test]
    fn ctrl_f_opens_search_and_enter_advances() {
        let doc = temp_path("findbar.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "alpha and alpha").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().search.query = "alpha".into();
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        assert!(harness.query_by_label("0/0").is_none(), "bar starts closed");
        harness.key_press_mods(egui::Modifiers::CTRL, egui::Key::F);
        harness.run();
        harness.get_by_label("1/2");
        harness.key_press(egui::Key::Enter);
        harness.run();
        harness.get_by_label("2/2");
        harness.key_press(egui::Key::Enter);
        harness.run();
        harness.get_by_label("1/2");
        harness.key_press(egui::Key::Escape);
        harness.run();
        assert!(harness.query_by_label("1/2").is_none(), "esc closes the bar");
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn reload_recomputes_matches_and_clamps_current() {
        let path = temp_path("findreload.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "one two one two one").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        {
            let mut a = app.borrow_mut();
            a.search.open = true;
            a.search.query = "two".into();
            a.recompute_matches();
            a.search.step(1);
        }
        assert_eq!(app.borrow().search.matches.len(), 2);
        assert_eq!(app.borrow().search.current, 1);

        std::fs::write(&path, "one").unwrap();
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        let mut reloaded = false;
        for _ in 0..100 {
            harness.run_steps(1);
            if app.borrow().search.matches.len() == 1 {
                reloaded = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(reloaded, "reload never recomputed matches");
        assert_eq!(app.borrow().search.current, 0, "current must clamp");
        std::fs::remove_file(&path).unwrap();
    }
```

Also update `shortcut_actions_map_correctly` — add after the F5 assertions:

```rust
        assert_eq!(shortcut_action(ctrl, egui::Key::F), Some(Action::Search));
        assert_eq!(shortcut_action(cmd, egui::Key::F), Some(Action::Search));
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test`
Expected: FAIL to compile — no field `search`, no variant `Search`, no method `recompute_matches`.

- [ ] **Step 4: Implement in app.rs**

1. Import and constants (top of file):

```rust
use crate::search;
```

(add `search` to the existing `use crate::{...}` block style used in the file)

```rust
const SEARCH_FIELD: &str = "rumd_search_field";
```

2. Add the action variant and shortcut mapping:

```rust
pub enum Action {
    Open,
    ToggleMode,
    ToggleTheme,
    Refresh,
    Search,
}
```

and in `shortcut_action`:

```rust
        (true, egui::Key::F) => Some(Action::Search),
```

3. Add the field to `App` (initialize `search: search::SearchState::default()` in `App::new`):

```rust
    pub search: search::SearchState,
```

4. In `open_path`, after `self.doc = Some(doc);` and `self.changed_at = None;` add:

```rust
                self.recompute_matches();
```

5. In `handle_events`, extend the match:

```rust
                Some(Action::Search) => {
                    self.search.open = true;
                    self.recompute_matches();
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new(SEARCH_FIELD)));
                    self.error = None;
                }
```

and after the `for event in &events { ... }` loop add the global search keys:

```rust
        if self.search.open {
            let (enter, shift, esc) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.modifiers.shift,
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if esc {
                self.search.open = false;
            } else if enter {
                self.search.step(if shift { -1 } else { 1 });
            }
        }
```

6. Add the recompute method to `impl App`:

```rust
    /// Recompute search matches against the current document and clamp
    /// the current index (used on query edits, open, and reloads).
    pub fn recompute_matches(&mut self) {
        let text = self.doc.as_ref().map(|d| d.raw.as_str()).unwrap_or("");
        self.search.matches = search::find_matches(text, &self.search.query);
        if self.search.current >= self.search.matches.len() {
            self.search.current = self.search.matches.len().saturating_sub(1);
        }
    }
```

7. In `show`, after `self.show_top_bar(ui);` add:

```rust
        self.show_search_bar(ui);
```

and add the method:

```rust
    fn show_search_bar(&mut self, ui: &mut egui::Ui) {
        if !self.search.open {
            return;
        }
        egui::Panel::top("rumd_search_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let response = egui::TextEdit::singleline(&mut self.search.query)
                    .id(egui::Id::new(SEARCH_FIELD))
                    .hint_text("Search (case-insensitive)")
                    .desired_width(240.0)
                    .show(ui)
                    .response;
                if response.changed() {
                    self.recompute_matches();
                }
                let count = if self.search.matches.is_empty() {
                    "0/0".to_string()
                } else {
                    format!("{}/{}", self.search.current + 1, self.search.matches.len())
                };
                ui.weak(count);
                if ui.button("<").clicked() {
                    self.search.step(-1);
                }
                if ui.button(">").clicked() {
                    self.search.step(1);
                }
                if ui.button("Close").clicked() {
                    self.search.open = false;
                }
            });
        });
    }
```

- [ ] **Step 5: Run the full test suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 6: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/search.rs src/app.rs
git commit -m "feat: find bar with case-insensitive search, n/m counter, wrap-around navigation"
```

---

### Task 5: Source-view match selection and scroll

**Files:**
- Modify: `src/source.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `search::char_index_of_byte`, `SearchState` (Tasks 1, 4).
- Produces:
  - `pub fn show(ui: &mut egui::Ui, raw: &str, theme: ColorTheme, jump: Option<Range<usize>>)` — `jump` is a **byte** range; converted to char indices internally. `ReadOnlyBuffer` now wraps `&str`.
  - `App { pending_source_match: Option<Range<usize>> }` + `App::jump_source_to_current(&mut self)`; called from Enter nav and `<`/`>` buttons; consumed (`.take()`) when the source pane renders.

- [ ] **Step 1: Write the failing tests**

In `src/app.rs` `mod tests`:

```rust
    #[test]
    fn search_jump_in_source_mode_selects_without_panic() {
        let doc = temp_path("srcjump.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "line one\nneedle here\nline three\nneedle again\n").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().mode = ViewMode::Source;
        {
            let mut a = app.borrow_mut();
            a.search.query = "needle".into();
            a.recompute_matches();
            a.search.step(1);
            a.jump_source_to_current();
        }
        assert!(app.borrow().pending_source_match.is_some());
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(3);
        assert!(
            app.borrow().pending_source_match.is_none(),
            "pending jump must be consumed on render"
        );
        std::fs::remove_file(&doc).unwrap();
    }
```

In `src/source.rs` `mod tests`, replace `source_view_runs_headless` with:

```rust
    #[test]
    fn source_view_runs_headless() {
        use egui_kittest::Harness;
        let raw = "# T\n\nbody";
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show(ui, raw, theme, None));
        harness.run();
    }

    #[test]
    fn source_selection_handles_multibyte_offsets() {
        use egui_kittest::Harness;
        let raw = "é\nneedle here\n";
        let jump = crate::search::find_matches(raw, "needle")[0].clone();
        let ctx = egui::Context::default();
        let theme = code_theme(egui::ThemePreference::Dark, &ctx);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| show(ui, raw, theme, Some(jump)));
        harness.run();
        harness.run();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test`
Expected: FAIL to compile — `show` takes 3 args; no `jump_source_to_current`.

- [ ] **Step 3: Implement**

In `src/source.rs`:

1. Update imports at top:

```rust
use std::ops::Range;

use eframe::egui;
use egui_code_editor::{CodeEditor, ColorTheme, Syntax};

use crate::document::Document;
use crate::search;
```

(`Document` stays imported for the tests that construct one — remove it if clippy reports it unused after the change below.)

2. Change `ReadOnlyBuffer` to wrap `&str`:

```rust
struct ReadOnlyBuffer<'a>(&'a str);
```

(its `as_str` body `self.0` is unchanged; the `#[test] fn read_only_buffer_exposes_text` keeps working since `&String` derefs — if not, pass `text.as_str()` in that test.)

3. Replace `show`:

```rust
pub fn show(ui: &mut egui::Ui, raw: &str, theme: ColorTheme, jump: Option<Range<usize>>) {
    let mut buffer = ReadOnlyBuffer(raw);
    let (output, _tokens) = CodeEditor::default()
        .id_source("rumd_source")
        .with_rows(24)
        .with_fontsize(14.0)
        .with_theme(theme)
        .with_numlines(true)
        .with_clickable_links(true)
        .vscroll(true)
        .show(ui, &mut buffer, &markdown_syntax());

    // Select and scroll to the searched match. The state is stored back
    // under the TextEdit's own widget id (taken from its output), so it
    // reloads next frame and the selection appears.
    if let Some(byte_range) = jump {
        let start = search::char_index_of_byte(raw, byte_range.start);
        let end = search::char_index_of_byte(raw, byte_range.end);
        let start_cc = egui::text::CCursor::new(egui::text::CharIndex(start));
        let end_cc = egui::text::CCursor::new(egui::text::CharIndex(end));
        let mut state = output.state;
        state
            .cursor
            .set_char_range(Some(egui::CCursorRange::two(start_cc, end_cc)));
        let id = output.response.response.id;
        state.store(ui.ctx(), id);
        let local = output.galley.pos_from_cursor(start_cc);
        let world = local.translate(output.galley_pos.to_vec2());
        ui.scroll_to_rect(world, None);
    }
}
```

In `src/app.rs`:

4. Add the field to `App` (init `pending_source_match: None` in `App::new`):

```rust
    pending_source_match: Option<std::ops::Range<usize>>,
```

5. Add the queue method:

```rust
    fn jump_source_to_current(&mut self) {
        self.pending_source_match = self.search.current_match().cloned();
    }
```

6. Call it on navigation — in `handle_events` (`else if enter` branch):

```rust
            } else if enter {
                self.search.step(if shift { -1 } else { 1 });
                self.jump_source_to_current();
            }
```

and in `show_search_bar` for both buttons:

```rust
                if ui.button("<").clicked() {
                    self.search.step(-1);
                    self.jump_source_to_current();
                }
                if ui.button(">").clicked() {
                    self.search.step(1);
                    self.jump_source_to_current();
                }
```

7. Update the Source arm in `show`:

```rust
                ViewMode::Source => {
                    let theme = source::code_theme(self.theme_pref, &ctx);
                    let jump = self.pending_source_match.take();
                    source::show(ui, &doc.raw, theme, jump);
                }
```

(The Split arm passes `None` for now: `source::show(&mut columns[1], &doc.raw, theme, None);` — Task 6 does not change this; split-mode source jumps are covered by the same `take()` in a later cleanup if needed. Keep `None` here.)

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/source.rs src/app.rs
git commit -m "feat: search selects and scrolls to matches in the source view"
```

---

### Task 6: Rendered-view per-section rendering and jump

**Files:**
- Modify: `src/viewer.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `search::split_sections`, `search::section_containing` (Task 2).
- Produces:
  - `RenderedView::show(&mut self, ui: &mut egui::Ui, markdown: &str, sections: &[Range<usize>], pending_jump: &mut Option<usize>)`.
  - `App { sections: Vec<Range<usize>>, pending_render_jump: Option<usize> }`; `jump_source_to_current` becomes `queue_jumps` (sets both pending fields); `App::sections` recomputed in `open_path`.
  - The old 2-arg `RenderedView::show` is gone; all callers/tests updated.

- [ ] **Step 1: Write the failing tests**

In `src/viewer.rs` `mod tests`, update the three existing tests to the new signature by wrapping the call:

```rust
    fn show_whole(view: &mut RenderedView, ui: &mut egui::Ui, markdown: &str) {
        let sections = crate::search::split_sections(markdown);
        view.show(ui, markdown, &sections, &mut None);
    }
```

(replace each `view.show(ui, GOLDEN)` / `view.show(ui, &owned)` / `view.show(ui, &md)` with `show_whole(&mut view, ui, GOLDEN)` etc.) and add:

```rust
    #[test]
    fn pending_jump_to_section_consumes_and_does_not_panic() {
        let md = "# A\n\nword\n\n# B\n\nword\n".to_string();
        let sections = crate::search::split_sections(&md);
        assert_eq!(sections.len(), 2);
        let mut view = RenderedView::new();
        let jump = Rc::new(RefCell::new(Some(1usize)));
        let jump_for_ui = jump.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut jump_for_ui.borrow_mut());
            });
        harness.run();
        harness.run();
        assert!(jump.borrow().is_none(), "pending jump must be consumed");
    }

    #[test]
    fn single_fence_document_renders_as_one_section() {
        let md = "```\n# not heading\n\nstill fenced\n".to_string();
        let sections = crate::search::split_sections(&md);
        assert_eq!(sections.len(), 1);
        let mut view = RenderedView::new();
        let jump = Rc::new(RefCell::new(Some(0usize)));
        let jump_for_ui = jump.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(600.0, 400.0))
            .build_ui(move |ui| {
                view.show(ui, &md, &sections, &mut jump_for_ui.borrow_mut());
            });
        harness.run();
        harness.run();
        assert!(jump.borrow().is_none());
    }
```

(add `use std::cell::RefCell; use std::rc::Rc;` to the viewer test module.)

In `src/app.rs` `mod tests`, add:

```rust
    #[test]
    fn rendered_jump_fires_on_enter() {
        let doc = temp_path("renderjump.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# One\n\nalpha\n\n# Two\n\nalpha\n").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        {
            let mut a = app.borrow_mut();
            a.search.open = true;
            a.search.query = "alpha".into();
            a.recompute_matches();
            a.search.step(1);
            a.queue_jumps();
        }
        assert!(app.borrow().pending_render_jump.is_some());
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(3);
        assert!(app.borrow().pending_render_jump.is_none(), "jump consumed by rendered view");
        std::fs::remove_file(&doc).unwrap();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test`
Expected: FAIL to compile — no `queue_jumps`/`pending_render_jump`/`sections`; viewer signature mismatch.

- [ ] **Step 3: Implement**

In `src/viewer.rs` — new imports and `show`:

```rust
use std::ops::Range;

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
```

```rust
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        markdown: &str,
        sections: &[Range<usize>],
        pending_jump: &mut Option<usize>,
    ) {
        // CentralPanel clips overflow, so long documents need a ScrollArea.
        // (egui_commonmark's show_scrollable assumes static markdown and
        // requires manual cache clearing on every change.)
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // Sections are rendered separately so a pending search jump
                // can scroll to the exact section containing the match.
                // The cache is documented to support multiple source ids.
                for (i, range) in sections.iter().enumerate() {
                    let response = CommonMarkViewer::new()
                        .show(ui, &mut self.cache, &markdown[range.clone()]);
                    if *pending_jump == Some(i) {
                        *pending_jump = None;
                        ui.scroll_to_rect(response.response.rect, None);
                    }
                }
            });
    }
```

In `src/app.rs`:

1. Add fields (init `sections: Vec::new()`, `pending_render_jump: None` in `App::new`):

```rust
    pub sections: Vec<std::ops::Range<usize>>,
    pending_render_jump: Option<usize>,
```

2. In `open_path`, before `self.doc = Some(doc);` add:

```rust
                self.sections = search::split_sections(&doc.raw);
```

3. Rename `jump_source_to_current` to `queue_jumps` and set both targets:

```rust
    fn queue_jumps(&mut self) {
        self.pending_source_match = self.search.current_match().cloned();
        self.pending_render_jump = self
            .search
            .current_match()
            .and_then(|m| search::section_containing(&self.sections, m.start));
    }
```

Update the three call sites (`Enter` branch, `<` button, `>` button) to `self.queue_jumps();`.

4. Update the CentralPanel arms in `show`:

```rust
                ViewMode::Rendered => {
                    self.rendered
                        .show(ui, &doc.raw, &self.sections, &mut self.pending_render_jump);
                }
```

```rust
                ViewMode::Split => {
                    let theme = source::code_theme(self.theme_pref, &ctx);
                    let sections = self.sections.clone();
                    let mut render_jump = self.pending_render_jump;
                    let source_jump = self.pending_source_match.take();
                    ui.columns(2, |columns| {
                        self.rendered
                            .show(&mut columns[0], &doc.raw, &sections, &mut render_jump);
                        source::show(&mut columns[1], &doc.raw, theme, source_jump);
                    });
                    self.pending_render_jump = render_jump;
                }
```

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/viewer.rs src/app.rs
git commit -m "feat: rendered view jumps to the section containing the current match"
```

---

### Task 7: Centered column, typography, and code syntax themes

**Files:**
- Modify: `src/viewer.rs`, `src/app.rs`

**Interfaces:**
- Consumes: Task 6 `show` signature.
- Produces:
  - `pub const COLUMN_WIDTH: f32 = 800.0;` and `pub fn column_width(available: f32) -> f32` in `viewer.rs`.
  - Rendered content inside a centered inner Ui (`ui.new_child(egui::UiBuilder::new().max_rect(rect))`), viewer configured with `.default_width(Some(800))`, `.syntax_theme_light("InspiredGitHub")`, `.syntax_theme_dark("base16-ocean.dark")`.
  - `App::apply_typography(ctx)` called every frame: Body 16, Heading 28, Monospace 14, Button 14, Small 12, `item_spacing (8, 10)`.

- [ ] **Step 1: Write the failing test**

In `src/viewer.rs` `mod tests`:

```rust
    #[test]
    fn column_width_clamps() {
        assert_eq!(super::column_width(2000.0), 800.0);
        assert_eq!(super::column_width(600.0), 552.0); // available - 2*margin
        assert_eq!(super::column_width(300.0), 300.0); // never exceeds available
        assert_eq!(super::column_width(100.0), 100.0);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test column_width_clamps`
Expected: FAIL (`column_width` not defined).

- [ ] **Step 3: Implement**

In `src/viewer.rs`:

1. Constants and helper (module level):

```rust
/// Target width of the centered rendered-content column.
pub const COLUMN_WIDTH: f32 = 800.0;
const COLUMN_MARGIN: f32 = 24.0;
const COLUMN_MIN: f32 = 320.0;
const SYNTAX_LIGHT: &str = "InspiredGitHub";
const SYNTAX_DARK: &str = "base16-ocean.dark";

/// Column width for a given available width: at most [`COLUMN_WIDTH`],
/// shrinking with the window, never exceeding the available width.
pub fn column_width(available: f32) -> f32 {
    COLUMN_WIDTH
        .min(available - 2.0 * COLUMN_MARGIN)
        .max(COLUMN_MIN.min(available))
}
```

2. Replace the body of `RenderedView::show`'s ScrollArea closure:

```rust
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let available = ui.available_width();
                let width = column_width(available);
                let full = ui.max_rect();
                let left = full.left() + (available - width) / 2.0;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(left, full.top()),
                    egui::vec2(width, full.height()),
                );
                let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                let viewer = CommonMarkViewer::new()
                    .default_width(Some(COLUMN_WIDTH as usize))
                    .syntax_theme_light(SYNTAX_LIGHT)
                    .syntax_theme_dark(SYNTAX_DARK);
                for (i, range) in sections.iter().enumerate() {
                    let response = viewer.show(&mut inner, &mut self.cache, &markdown[range.clone()]);
                    if *pending_jump == Some(i) {
                        *pending_jump = None;
                        inner.scroll_to_rect(response.response.rect, None);
                    }
                }
            });
```

In `src/app.rs`:

3. Add to `impl App` and call it in `show` right after `self.apply_theme(&ctx);`:

```rust
        Self::apply_typography(&ctx);
```

```rust
    fn apply_typography(ctx: &egui::Context) {
        ctx.all_styles_mut(|style| {
            style.text_styles = egui::BTreeMap::from([
                (egui::TextStyle::Small, egui::FontId::proportional(12.0)),
                (egui::TextStyle::Body, egui::FontId::proportional(16.0)),
                (egui::TextStyle::Button, egui::FontId::proportional(14.0)),
                (egui::TextStyle::Heading, egui::FontId::proportional(28.0)),
                (egui::TextStyle::Monospace, egui::FontId::monospace(14.0)),
            ]);
            style.spacing.item_spacing = egui::vec2(8.0, 10.0);
        });
    }
```

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 5: Manual sanity check (optional but recommended)**

Run: `cargo run` and open a doc with a wide code block.
Expected: text wrapped at ~800px centered column; code blocks use a GitHub-like light theme / dark ocean theme matching the app theme.

- [ ] **Step 6: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/viewer.rs src/app.rs
git commit -m "feat: centered reading column, larger typography, theme-matched code blocks"
```

---

### Task 8: Preference persistence (theme, zoom, view mode)

**Files:**
- Modify: `Cargo.toml`, `src/app.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `eframe::Storage` (string keys), `Context::zoom_factor()`.
- Produces:
  - `eframe` dependency gains `features = ["persistence"]`.
  - Storage keys: `"rumd.theme"` (`dark`/`light`/`system`), `"rumd.zoom"` (`f32` as string), `"rumd.mode"` (`rendered`/`split`/`source`).
  - `pub fn clamp_zoom(z: f32) -> f32` (0.2–5.0), `encode_theme/decode_theme`, `encode_mode/decode_mode`, `decode_zoom(Option<&str>) -> f32`.
  - `App { zoom: f32 }` mirrored from `ctx.zoom_factor()` each frame; `App::new_with_cc(initial, cc)` loads prefs; `App::save` writes them.
  - `main.rs` calls `App::new_with_cc(path, cc)`.

- [ ] **Step 1: Enable the feature and write the failing tests**

In `Cargo.toml` change the eframe line to:

```toml
eframe = { version = "0.36", features = ["persistence"] }
```

In `src/app.rs` `mod tests`:

```rust
    #[derive(Default)]
    struct MemStorage(std::collections::HashMap<String, String>);

    impl eframe::Storage for MemStorage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.to_owned(), value);
        }
        fn remove_string(&mut self, key: &str) {
            self.0.remove(key);
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn prefs_roundtrip_and_defaults() {
        assert_eq!(
            decode_theme(encode_theme(egui::ThemePreference::Dark)),
            egui::ThemePreference::Dark
        );
        assert_eq!(decode_theme("garbage"), egui::ThemePreference::System);
        assert_eq!(decode_mode(encode_mode(ViewMode::Split)), ViewMode::Split);
        assert_eq!(decode_mode("junk"), ViewMode::Rendered);
        assert_eq!(decode_zoom(Some("1.25")), 1.25);
        assert_eq!(decode_zoom(Some("99.0")), ZOOM_MAX);
        assert_eq!(decode_zoom(Some("0.01")), ZOOM_MIN);
        assert_eq!(decode_zoom(Some("abc")), 1.0);
        assert_eq!(decode_zoom(None), 1.0);
    }

    #[test]
    fn save_writes_pref_keys() {
        let mut storage = MemStorage::default();
        let app = Rc::new(RefCell::new(App::new(None)));
        {
            let mut a = app.borrow_mut();
            a.mode = ViewMode::Split;
            a.theme_pref = egui::ThemePreference::Light;
            a.zoom = 1.5;
        }
        eframe::App::save(&mut app.borrow_mut(), &mut storage);
        assert_eq!(storage.get_string(KEY_MODE).as_deref(), Some("split"));
        assert_eq!(storage.get_string(KEY_THEME).as_deref(), Some("light"));
        assert_eq!(storage.get_string(KEY_ZOOM).as_deref(), Some("1.5"));
    }

    #[test]
    fn zoom_shortcuts_update_mirrored_zoom() {
        let app = Rc::new(RefCell::new(App::new(None)));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.key_press_mods(egui::Modifiers::CTRL, egui::Key::Equals);
        harness.run();
        assert!(
            app.borrow().zoom > 1.0,
            "ctrl+= must zoom in natively, zoom={}",
            app.borrow().zoom
        );
        harness.key_press_mods(egui::Modifiers::CTRL, egui::Key::Num0);
        harness.run();
        assert!(
            (app.borrow().zoom - 1.0).abs() < 1e-6,
            "ctrl+0 must reset zoom, zoom={}",
            app.borrow().zoom
        );
    }
```

(For the key constants to be visible, they must be declared `pub const` at the module level of `app.rs` — see implementation below; the test module's `use super::*` puts them in scope.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test`
Expected: FAIL to compile — helper functions/fields not defined; `KEY_*` missing.

- [ ] **Step 3: Implement**

In `src/app.rs`:

1. Constants and helpers at module level:

```rust
pub const KEY_THEME: &str = "rumd.theme";
pub const KEY_ZOOM: &str = "rumd.zoom";
pub const KEY_MODE: &str = "rumd.mode";
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 5.0;

pub fn clamp_zoom(zoom: f32) -> f32 {
    zoom.clamp(ZOOM_MIN, ZOOM_MAX)
}

pub fn encode_theme(pref: egui::ThemePreference) -> &'static str {
    match pref {
        egui::ThemePreference::Dark => "dark",
        egui::ThemePreference::Light => "light",
        egui::ThemePreference::System => "system",
    }
}

pub fn decode_theme(s: &str) -> egui::ThemePreference {
    match s {
        "dark" => egui::ThemePreference::Dark,
        "light" => egui::ThemePreference::Light,
        _ => egui::ThemePreference::System,
    }
}

pub fn encode_mode(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Rendered => "rendered",
        ViewMode::Split => "split",
        ViewMode::Source => "source",
    }
}

pub fn decode_mode(s: &str) -> ViewMode {
    match s {
        "split" => ViewMode::Split,
        "source" => ViewMode::Source,
        _ => ViewMode::Rendered,
    }
}

pub fn decode_zoom(raw: Option<&str>) -> f32 {
    raw.and_then(|s| s.parse::<f32>().ok())
        .map(clamp_zoom)
        .unwrap_or(1.0)
}
```

(If the compiler reports `ThemePreference` as non-exhaustive in `encode_theme`, add a `_ => "system"` arm and fold `System` into it.)

2. Add the field `zoom: f32` to `App` (init `zoom: 1.0` in `App::new`) and mirror it in `show` right after `let ctx = ui.ctx().clone();`:

```rust
        self.zoom = ctx.zoom_factor();
```

3. Loader — add to `impl App`:

```rust
    /// Load persisted preferences (theme, zoom, view mode) from storage.
    pub fn new_with_cc(initial: Option<PathBuf>, cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = App::new(initial);
        if let Some(storage) = cc.storage {
            app.theme_pref =
                decode_theme(&storage.get_string(KEY_THEME).unwrap_or_default());
            app.zoom = decode_zoom(storage.get_string(KEY_ZOOM).as_deref());
            app.mode = decode_mode(&storage.get_string(KEY_MODE).unwrap_or_default());
        }
        cc.egui_ctx.set_zoom_factor(app.zoom);
        app
    }
```

4. Saver — extend the existing `impl eframe::App for App`:

```rust
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(KEY_THEME, encode_theme(self.theme_pref).to_owned());
        storage.set_string(KEY_ZOOM, format!("{}", self.zoom));
        storage.set_string(KEY_MODE, encode_mode(self.mode).to_owned());
    }
```

In `src/main.rs`:

5. Replace the app construction (the test module lives inside `app.rs` with
   `use super::*`, so the `KEY_*` constants are in scope for the tests
   without any re-export — in `save_writes_pref_keys` use `KEY_MODE`,
   `KEY_THEME`, `KEY_ZOOM` directly instead of `crate::KEY_*`):

```rust
    eframe::run_native(
        "rumd",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new_with_cc(path, cc)))),
    )
```

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: PASS. (`zoom_shortcuts_update_mirrored_zoom` verifies egui's native zoom actually fires headless and is mirrored.)

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add Cargo.toml Cargo.lock src/app.rs src/main.rs
git commit -m "feat: persist theme, zoom, and view mode across restarts"
```

---

### Task 9: UI polish — tooltips, dismissable banner, empty state

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: existing App state.
- Produces:
  - Tooltips with shortcuts on all top-bar buttons; filename tooltip shows the full path.
  - Error banner gains a working `Close` button.
  - `show_empty_state(&mut self, ...)` — centered "rumd" wordmark, hint line, `Open…` button wired to `open_requested`; `show`'s CentralPanel restructured to a `has_doc` check so the empty state can mutate.

- [ ] **Step 1: Write the failing test**

In `src/app.rs` `mod tests`:

```rust
    #[test]
    fn empty_state_offers_open_button() {
        let app = Rc::new(RefCell::new(App::new(None)));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.get_by_label("rumd");
        // One run: the click is processed and sets the request flag. Do NOT
        // run another frame — the next frame would open the real (blocking)
        // rfd file dialog, which headless tests must avoid.
        harness.get_by_label("Open…").click();
        harness.run();
        assert!(
            app.borrow().open_requested,
            "Open… button must request the file dialog"
        );
    }

    #[test]
    fn error_banner_close_clears_message() {
        let good = temp_path("banner.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&good, "# Banner Doc").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&good);
        app.borrow_mut()
            .open_path(Path::new("/nonexistent/rumd/missing.md"));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        assert!(harness.query_by_label_contains("Failed to open").is_some());
        harness.get_by_label("Close").click();
        harness.run();
        assert!(app.borrow().error.is_none(), "Close must clear the error");
        std::fs::remove_file(&good).unwrap();
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test empty_state_offers error_banner_close`
Expected: FAIL (no "rumd"/"Open…" labels; no "Close" button).

- [ ] **Step 3: Implement**

1. Tooltips in `show_top_bar` (labels unchanged — tests query them):

```rust
                if ui.button("Open").on_hover_text("Open a file (Ctrl+O)").clicked() {
                    action = Some(Action::Open);
                }
                if let Some(doc) = &self.doc {
                    ui.strong(doc.file_name())
                        .on_hover_text(doc.path.display().to_string());
                    if doc.lossy {
                        ui.label(
                            egui::RichText::new(LOSSY_BADGE)
                                .small()
                                .color(ui.visuals().warn_fg_color),
                        );
                    }
                }
```

```rust
                    if ui.button("Refresh").on_hover_text("Reload from disk (F5)").clicked() {
                        action = Some(Action::Refresh);
                    }
```

theme button:

```rust
                    let theme_label = match ctx.theme() {
                        egui::Theme::Dark => "Light theme",
                        egui::Theme::Light => "Dark theme",
                    };
                    if ui
                        .button(theme_label)
                        .on_hover_text("Switch theme (Ctrl+D)")
                        .clicked()
                    {
                        action = Some(Action::ToggleTheme);
                    }
```

mode buttons inside the group each get `.on_hover_text("Cycle view mode (Ctrl+E)")`.

2. Error banner with close:

```rust
    fn show_error_banner(&mut self, ui: &mut egui::Ui) {
        if let Some(message) = self.error.clone() {
            egui::Panel::top("error_banner").show(ui, |ui| {
                egui::Frame::default()
                    .fill(ui.visuals().warn_fg_color.gamma_multiply(0.15))
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Close").clicked() {
                                self.error = None;
                            }
                            ui.colored_label(ui.visuals().warn_fg_color, message);
                        });
                    });
            });
        }
    }
```

3. Restructure the CentralPanel body in `show` so the empty state can mutate (replace the current `match &self.doc { ... }` block):

```rust
        egui::CentralPanel::default().show(ui, |ui| {
            if self.doc.is_none() {
                self.show_empty_state(ui);
                return;
            }
            let doc = self.doc.as_ref().unwrap();
            match self.mode {
                ViewMode::Rendered => {
                    self.rendered
                        .show(ui, &doc.raw, &self.sections, &mut self.pending_render_jump);
                }
                ViewMode::Source => {
                    let theme = source::code_theme(self.theme_pref, &ctx);
                    let jump = self.pending_source_match.take();
                    source::show(ui, &doc.raw, theme, jump);
                }
                ViewMode::Split => {
                    let theme = source::code_theme(self.theme_pref, &ctx);
                    let sections = self.sections.clone();
                    let mut render_jump = self.pending_render_jump;
                    let source_jump = self.pending_source_match.take();
                    ui.columns(2, |columns| {
                        self.rendered
                            .show(&mut columns[0], &doc.raw, &sections, &mut render_jump);
                        source::show(&mut columns[1], &doc.raw, theme, source_jump);
                    });
                    self.pending_render_jump = render_jump;
                }
            }
        });
```

4. Empty state (replace existing method):

```rust
    fn show_empty_state(&mut self, ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(48.0);
                ui.label(egui::RichText::new("rumd").size(56.0).weak());
                ui.add_space(8.0);
                ui.label(EMPTY_HINT);
                ui.add_space(16.0);
                if ui.button("Open…").clicked() {
                    self.open_requested = true;
                }
            });
        });
    }
```

- [ ] **Step 4: Run the full test suite**

Run: `cargo test`
Expected: PASS (v1 tests `empty_state_shows_hint` and `open_failure_shows_error_banner_keeps_previous_doc` still pass — labels preserved).

- [ ] **Step 5: Lint and commit**

Run: `cargo clippy --all-targets -- -D warnings`

```bash
git add src/app.rs
git commit -m "feat: UI polish — tooltips, dismissable error banner, richer empty state"
```

---

### Task 10: README, full suite, clippy

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: everything above.
- Produces: accurate README; green suite; clean clippy.

- [ ] **Step 1: Update README**

Features section — add:

```markdown
- Split view: rendered and source side by side
- Search (`Ctrl/Cmd+F`): matches select in the source view and jump by
  section in the rendered view
- Zoom (`Ctrl/Cmd+=`, `Ctrl/Cmd+-`, `Ctrl/Cmd+0`)
- Centered reading column with adjusted typography
- Preferences (theme, zoom, view mode) persist between runs
```

Shortcuts table — add rows:

```markdown
| `Ctrl/Cmd+F` | Search |
| `Enter` / `Shift+Enter` | Next / previous match |
| `Esc` | Close search |
| `Ctrl/Cmd+=` / `Ctrl/Cmd+-` / `Ctrl/Cmd+0` | Zoom in / out / reset |
```

- [ ] **Step 2: Run the entire suite and lints**

Run: `cargo test && cargo clippy --all-targets -- -D warnings`
Expected: all green.

- [ ] **Step 3: Manual smoke test**

Run: `cargo run -- README.md`
Check: rendered column centered; `Ctrl+E` cycles 3 modes; `Ctrl+F`, type `md`, `Enter` walks matches in both source and rendered modes; `Esc` closes; zoom in/out/reset works; restart restores theme/zoom/mode.

- [ ] **Step 4: Commit**

```bash
git add README.md
git commit -m "docs: document split view, search, zoom, and persistence"
```
