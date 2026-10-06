use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::document::{Document, FileWatcher, RELOAD_DEBOUNCE, debounce_ready};
use crate::icons;
use crate::search;
use crate::source;
use crate::toc;
use crate::viewer::RenderedView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Rendered,
    Split,
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    ToggleMode,
    ToggleTheme,
    Refresh,
    Search,
    Save,
    ToggleToc,
}

/// Map a modifier+key combination to a global action.
pub fn shortcut_action(mods: egui::Modifiers, key: egui::Key) -> Option<Action> {
    let primary = mods.ctrl || mods.command;
    match (primary, key) {
        (true, egui::Key::O) => Some(Action::Open),
        (true, egui::Key::E) => Some(Action::ToggleMode),
        (true, egui::Key::D) => Some(Action::ToggleTheme),
        (true, egui::Key::F) => Some(Action::Search),
        (true, egui::Key::S) => Some(Action::Save),
        (true, egui::Key::T) => Some(Action::ToggleToc),
        (false, egui::Key::F5) => Some(Action::Refresh),
        _ => None,
    }
}

/// Map an egui event to a global action (fresh key presses only).
pub fn event_action(event: &egui::Event) -> Option<Action> {
    match event {
        egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } => shortcut_action(*modifiers, *key),
        _ => None,
    }
}

/// Open a dropped file set: the first path wins.
pub fn first_drop(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.first().cloned()
}

const OPEN_FILTER_NAME: &str = "Markdown";
const OPEN_FILTER_EXTS: &[&str] = &["md", "markdown", "mdown", "txt"];

/// The real open-file dialog; injectable so headless tests never block.
fn rfd_open_dialog() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter(OPEN_FILTER_NAME, OPEN_FILTER_EXTS)
        .pick_file()
}

const EMPTY_HINT: &str = "Drop a .md file here or press Ctrl+O";
const LOSSY_BADGE: &str = "not valid UTF-8";
const SEARCH_FIELD: &str = "rumd_search_field";

const KEY_THEME: &str = "theme";
const KEY_ZOOM: &str = "zoom";
const KEY_MODE: &str = "mode";
const KEY_TOC: &str = "toc";
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

/// Platform-appropriate location of the prefs file, if determinable.
pub fn prefs_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA").map(PathBuf::from)?;
        Some(base.join("rumd").join("prefs.txt"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("rumd").join("prefs.txt"))
    }
}

pub fn encode_toc(open: bool) -> &'static str {
    if open { "open" } else { "closed" }
}

pub fn decode_toc(raw: Option<&str>) -> bool {
    raw == Some("open")
}

pub struct App {
    pub doc: Option<Document>,
    /// Text as last loaded from / saved to disk; `doc.raw` drifts from it
    /// while the user edits.
    saved_raw: Option<String>,
    pub mode: ViewMode,
    pub theme_pref: egui::ThemePreference,
    pub error: Option<String>,
    pub watcher: Option<FileWatcher>,
    pub watcher_failed: bool,
    pub changed_at: Option<Instant>,
    /// When our own save last wrote; watcher events right after it are
    /// self-caused and must not trigger a reload.
    last_save_at: Option<Instant>,
    /// The file changed on disk while there were unsaved edits; a banner
    /// lets the user keep their edits or reload from disk.
    conflict: bool,
    /// A refresh (F5 / toolbar) was requested with unsaved edits; a banner
    /// asks for confirmation before discarding them.
    refresh_confirm: bool,
    pub search: search::SearchState,
    /// Mirrored from `ctx.zoom_factor()` every frame (egui's native zoom).
    pub zoom: f32,
    /// Match currently highlighted in the source view; persists while the
    /// find bar stays open.
    source_highlight: Option<std::ops::Range<usize>>,
    pub sections: Vec<std::ops::Range<usize>>,
    pending_render_jump: Option<usize>,
    /// Cached table of contents for the current document text; recomputed
    /// wherever `sections` is.
    toc_entries: Vec<toc::TocEntry>,
    /// Which ToC groups (by outline path) are collapsed.
    toc_collapsed: std::collections::HashSet<toc::EntryPath>,
    /// ToC sidebar visibility; toggled with Ctrl/Cmd+T or the toolbar icon.
    toc_open: bool,
    /// Byte offset of a ToC entry awaiting its jump (rendered view scrolls
    /// to its section; the source caret parks at the heading line).
    pending_toc_jump: Option<usize>,
    /// Where prefs are persisted; `None` (tests) disables writing.
    prefs_file: Option<PathBuf>,
    /// Open-file dialog hook (overridden by tests to avoid blocking).
    open_dialog: Box<dyn Fn() -> Option<PathBuf>>,
    saved_theme: Option<egui::ThemePreference>,
    saved_mode: Option<ViewMode>,
    saved_zoom: f32,
    saved_toc: bool,
    rendered: RenderedView,
    open_requested: bool,
    applied_theme: Option<egui::ThemePreference>,
    last_title: Option<String>,
}

impl App {
    /// How long after our own save watcher events are treated as
    /// self-caused and dropped. An external change inside this window is
    /// picked up by a manual refresh (F5).
    const SAVE_RELOAD_SUPPRESSION: std::time::Duration = std::time::Duration::from_secs(1);

    pub fn new(initial: Option<PathBuf>) -> Self {
        let mut app = App {
            doc: None,
            saved_raw: None,
            mode: ViewMode::Rendered,
            theme_pref: egui::ThemePreference::System,
            error: None,
            watcher: None,
            watcher_failed: false,
            changed_at: None,
            last_save_at: None,
            conflict: false,
            refresh_confirm: false,
            search: search::SearchState::default(),
            zoom: 1.0,
            source_highlight: None,
            sections: Vec::new(),
            pending_render_jump: None,
            toc_entries: Vec::new(),
            toc_collapsed: std::collections::HashSet::new(),
            toc_open: false,
            pending_toc_jump: None,
            prefs_file: None,
            open_dialog: Box::new(rfd_open_dialog),
            saved_theme: None,
            saved_mode: None,
            saved_zoom: 1.0,
            saved_toc: false,
            rendered: RenderedView::new(),
            open_requested: false,
            applied_theme: None,
            last_title: None,
        };
        if let Some(path) = initial {
            app.open_path(&path);
        }
        app
    }

    /// Load a document, replacing the current one. On failure the previous
    /// document stays visible and the error banner explains what happened.
    pub fn open_path(&mut self, path: &Path) {
        // Canonicalize so the stored path is absolute and matches the paths
        // notify reports in events (it canonicalizes too), and so refresh
        // and auto-reload resolve correctly after the working directory
        // changes below.
        let canonical = match path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                self.error = Some(format!("Failed to open {}: {}", path.display(), e));
                return;
            }
        };
        match Document::load(&canonical) {
            Ok(doc) => {
                // egui resolves relative image paths against the process
                // working directory, so follow the document.
                if let Some(dir) = doc.dir() {
                    let _ = std::env::set_current_dir(dir);
                }
                self.error = None;
                match FileWatcher::spawn(&canonical) {
                    Ok(w) => {
                        self.watcher = Some(w);
                        self.watcher_failed = false;
                    }
                    Err(e) => {
                        self.watcher = None;
                        if !self.watcher_failed {
                            self.error =
                                Some(format!("Auto-reload unavailable: {e}. Use Refresh."));
                        }
                        self.watcher_failed = true;
                    }
                }
                self.sections = search::split_sections(&doc.raw);
                self.toc_entries = toc::headings(&doc.raw);
                self.saved_raw = Some(doc.raw.clone());
                self.conflict = false;
                // Section heights come from the previous document; reset so
                // virtualized placeholders measure the new text.
                self.rendered.clear();
                self.doc = Some(doc);
                self.changed_at = None;
                self.recompute_matches();
            }
            Err(e) => {
                self.error = Some(format!("Failed to open {}: {}", path.display(), e));
            }
        }
    }

    /// Drain watcher events; reload once the file has been quiet for
    /// RELOAD_DEBOUNCE. The file system watcher cannot wake the event loop,
    /// so while a change is pending this schedules repaints itself.
    /// Events within [`Self::SAVE_RELOAD_SUPPRESSION`] of our own last save
    /// are self-caused and dropped: reloading them would overwrite
    /// keystrokes typed right after the save.
    fn poll_watcher(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        if let Some(w) = &self.watcher {
            while let Ok(()) = w.events.try_recv() {
                changed = true;
            }
        }
        if changed
            && self
                .last_save_at
                .is_some_and(|t| t.elapsed() < Self::SAVE_RELOAD_SUPPRESSION)
        {
            changed = false;
        }
        if changed {
            self.changed_at = Some(Instant::now());
        }
        if let Some(t) = self.changed_at {
            if debounce_ready(Some(t), Instant::now(), RELOAD_DEBOUNCE) {
                self.changed_at = None;
                if self.is_dirty() {
                    // Unsaved edits must never be clobbered: hold the
                    // reload and ask the user instead.
                    self.conflict = true;
                } else if let Some(path) = self.doc.as_ref().map(|d| d.path.clone()) {
                    self.open_path(&path);
                }
            } else {
                // egui subtracts the predicted frame time from the delay
                // to avoid oversleeping; pre-compensate so the effective
                // delay is the exact remainder (and never collapses to an
                // immediate repaint when predicted_dt > remaining, which
                // would spin repaint loops under kittest).
                let predicted = Duration::from_secs_f32(ctx.input(|i| i.predicted_dt));
                ctx.request_repaint_after(RELOAD_DEBOUNCE - t.elapsed() + predicted);
            }
        }
    }

    /// Recompute search matches against the current document and clamp
    /// the current index (used on query edits, open, and reloads).
    /// Pending jumps queued against the previous text are dropped — the
    /// spec forbids scrolling to a stale target.
    /// True while `doc.raw` differs from the text on disk.
    pub fn is_dirty(&self) -> bool {
        match (&self.doc, &self.saved_raw) {
            (Some(doc), Some(saved)) => doc.raw != *saved,
            _ => false,
        }
    }

    /// The user edited the source text this frame: recompute everything
    /// derived from it. Stale jumps and highlights are dropped, matching
    /// reload behavior. `cursor_byte` then queues a fresh preview jump so
    /// the rendered pane follows the edited spot.
    fn on_document_edited(&mut self, cursor_byte: Option<usize>) {
        if let Some(doc) = &self.doc {
            self.sections = search::split_sections(&doc.raw);
            self.toc_entries = toc::headings(&doc.raw);
        }
        self.recompute_matches();
        if let Some(byte) = cursor_byte {
            self.pending_render_jump = search::section_for_cursor(&self.sections, byte);
        }
    }

    pub fn recompute_matches(&mut self) {
        let text = self.doc.as_ref().map(|d| d.raw.as_str()).unwrap_or("");
        self.search.matches = search::find_matches(text, &self.search.query);
        if self.search.current >= self.search.matches.len() {
            self.search.current = self.search.matches.len().saturating_sub(1);
        }
        self.source_highlight = None;
        self.pending_render_jump = None;
    }

    fn queue_jumps(&mut self) {
        self.source_highlight = self.search.current_match().cloned();
        self.pending_render_jump = self
            .search
            .current_match()
            .and_then(|m| search::section_containing(&self.sections, m.start));
    }

    /// What to highlight in rendered views: only while the find bar is
    /// open, the query non-empty, and there is a current match whose
    /// section can be located.
    fn search_open_highlight(&self) -> Option<crate::viewer::SearchHighlight> {
        if !self.search.open || self.search.query.is_empty() {
            return None;
        }
        let m = self.search.current_match()?;
        let section = search::section_containing(&self.sections, m.start)?;
        Some(crate::viewer::SearchHighlight {
            query: self.search.query.clone(),
            section,
            match_byte: m.start,
        })
    }

    /// Close the find bar and forget the on-screen highlight.
    fn close_search(&mut self) {
        self.search.open = false;
        self.source_highlight = None;
    }

    /// Opt in to preference persistence (main only; tests leave `None`).
    pub fn set_prefs_file(&mut self, path: Option<PathBuf>) {
        self.prefs_file = path;
    }

    /// Serialize the current preferences as `key=value` lines.
    pub fn prefs_to_string(&self) -> String {
        format!(
            "{}={}\n{}={}\n{}={}\n{}={}\n",
            KEY_THEME,
            encode_theme(self.theme_pref),
            KEY_ZOOM,
            self.zoom,
            KEY_MODE,
            encode_mode(self.mode),
            KEY_TOC,
            encode_toc(self.toc_open)
        )
    }

    /// Apply `key=value` lines; unknown keys/values are ignored.
    pub fn apply_prefs_string(&mut self, s: &str) {
        for line in s.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                KEY_THEME => self.theme_pref = decode_theme(value.trim()),
                KEY_ZOOM => self.zoom = decode_zoom(Some(value.trim())),
                KEY_MODE => self.mode = decode_mode(value.trim()),
                KEY_TOC => self.toc_open = decode_toc(Some(value.trim())),
                _ => {}
            }
        }
    }

    /// Read preferences from disk if a prefs file is configured.
    pub fn load_prefs(&mut self) {
        if let Some(path) = &self.prefs_file {
            if let Ok(s) = std::fs::read_to_string(path) {
                self.apply_prefs_string(&s);
            }
            self.saved_theme = Some(self.theme_pref);
            self.saved_mode = Some(self.mode);
            self.saved_zoom = self.zoom;
            self.saved_toc = self.toc_open;
        }
    }

    /// Apply the loaded zoom to the egui context (call once at startup,
    /// after [`Self::load_prefs`], before the first UI pass).
    pub fn apply_zoom_to_ctx(&self, ctx: &egui::Context) {
        // Direct option write: `set_zoom_factor` only defers to the next
        // pass, but at startup there is no pass yet to consume it.
        ctx.memory_mut(|mem| mem.options.zoom_factor = self.zoom);
    }

    /// Write preferences when they changed since the last write. Called
    /// every frame; zoom mirroring makes zoom changes visible here.
    fn persist_prefs_if_changed(&mut self) {
        let changed = self.saved_theme != Some(self.theme_pref)
            || self.saved_mode != Some(self.mode)
            || (self.saved_zoom - self.zoom).abs() > f32::EPSILON
            || self.saved_toc != self.toc_open;
        if !changed {
            return;
        }
        if let Some(path) = &self.prefs_file {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if std::fs::write(path, self.prefs_to_string()).is_ok() {
                self.saved_theme = Some(self.theme_pref);
                self.saved_mode = Some(self.mode);
                self.saved_zoom = self.zoom;
                self.saved_toc = self.toc_open;
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.handle_events(&ctx);
        self.poll_watcher(&ctx);
        self.apply_theme(&ctx);
        Self::apply_typography(&ctx);
        // egui smooths ctrl/cmd+wheel over several frames, which would
        // rebuild the glyph atlas per intermediate zoom level; instead a
        // wheel notch steps once, like Ctrl+= (and browsers). Only pinch
        // gestures stay continuous.
        let (notches, zoom_event) = ctx.input(|i| {
            let notches: f32 = i
                .raw
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Line,
                        delta,
                        modifiers,
                        ..
                    } if modifiers.matches_any(egui::Modifiers::COMMAND) => Some(delta.y.signum()),
                    _ => None,
                })
                .sum();
            let zoom_event = i.multi_touch().is_some()
                || i.raw
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Zoom(_)));
            (notches, zoom_event)
        });
        if notches != 0.0 {
            let stepped = (ctx.zoom_factor() * 10.0).round() / 10.0 + notches / 10.0;
            let zoomed = clamp_zoom(stepped);
            ctx.memory_mut(|mem| mem.options.zoom_factor = zoomed);
        } else if zoom_event {
            let zoom_delta = ctx.input(|i| i.zoom_delta());
            if zoom_delta != 1.0 {
                let zoomed = clamp_zoom(ctx.zoom_factor() * zoom_delta);
                ctx.memory_mut(|mem| mem.options.zoom_factor = zoomed);
            }
        }
        self.zoom = ctx.zoom_factor();
        self.persist_prefs_if_changed();
        self.show_error_banner(ui);
        self.show_conflict_banner(ui);
        self.show_refresh_confirm_banner(ui);
        self.show_top_bar(ui);
        self.show_search_bar(ui);
        self.show_toc_panel(ui);
        self.update_window_title(&ctx);

        egui::CentralPanel::default().show(ui, |ui| {
            if self.doc.is_none() {
                self.show_empty_state(ui);
                return;
            }
            let search = self.search_open_highlight();
            let theme = source::code_theme(self.theme_pref, &ctx);
            let source_query = self.search.open.then(|| self.search.query.clone());
            let sections = self.sections.clone();
            let doc = self.doc.as_mut().unwrap();
            let mut render_jump = self.pending_render_jump;
            // A pending ToC jump goes to the rendered section containing
            // the heading (the view then pins it to the top) — except in
            // Source view, where it parks the caret on the heading line by
            // reusing the search-jump plumbing with a zero-width highlight
            // (and retires any search caret park).
            let toc_byte = self.pending_toc_jump.take();
            let highlight = match toc_byte {
                Some(byte) if self.mode == ViewMode::Source => {
                    self.source_highlight = None;
                    Some(byte..byte)
                }
                _ => self.source_highlight.clone(),
            };
            let mut toc_jump =
                toc_byte.and_then(|byte| search::section_containing(&sections, byte));
            let mut edited = false;
            let mut edit_cursor = None;
            match self.mode {
                ViewMode::Rendered => {
                    self.rendered.show(
                        ui,
                        &doc.raw,
                        &sections,
                        &mut render_jump,
                        &mut toc_jump,
                        search.as_ref(),
                    );
                }
                ViewMode::Source => {
                    let (changed, cursor) = source::show(
                        ui,
                        &mut doc.raw,
                        doc.lossy,
                        theme,
                        source_query.as_deref(),
                        highlight,
                    );
                    if changed {
                        edited = true;
                        edit_cursor = cursor;
                    }
                }
                ViewMode::Split => {
                    let raw = &mut doc.raw;
                    ui.columns(2, |columns| {
                        self.rendered.show(
                            &mut columns[0],
                            raw,
                            &sections,
                            &mut render_jump,
                            &mut toc_jump,
                            search.as_ref(),
                        );
                        let (changed, cursor) = source::show(
                            &mut columns[1],
                            raw,
                            doc.lossy,
                            theme,
                            source_query.as_deref(),
                            highlight,
                        );
                        if changed {
                            edited = true;
                            edit_cursor = cursor;
                        }
                    });
                }
            }
            self.pending_render_jump = render_jump;
            if edited {
                self.on_document_edited(edit_cursor);
            }
        });
    }

    fn handle_events(&mut self, ctx: &egui::Context) {
        let events: Vec<egui::Event> = ctx.input(|i| i.events.clone());
        for event in &events {
            match event_action(event) {
                Some(Action::Open) => self.open_requested = true,
                Some(Action::ToggleMode) => {
                    self.mode = match self.mode {
                        ViewMode::Rendered => ViewMode::Split,
                        ViewMode::Split => ViewMode::Source,
                        ViewMode::Source => ViewMode::Rendered,
                    };
                    self.error = None;
                }
                Some(Action::ToggleTheme) => {
                    self.toggle_theme(ctx);
                    self.error = None;
                }
                Some(Action::Refresh) => self.refresh(),
                Some(Action::Save) => self.save_document(),
                Some(Action::ToggleToc) => {
                    if self.doc.is_some() {
                        self.toc_open = !self.toc_open;
                    }
                }
                Some(Action::Search) => {
                    self.search.open = true;
                    self.recompute_matches();
                    ctx.memory_mut(|m| m.request_focus(egui::Id::new(SEARCH_FIELD)));
                    self.error = None;
                }
                None => {}
            }
        }

        if self.search.open {
            let (enter, shift, esc) = ctx.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.modifiers.shift,
                    i.key_pressed(egui::Key::Escape),
                )
            });
            if esc {
                self.close_search();
            } else if enter {
                self.search.step(if shift { -1 } else { 1 });
                self.queue_jumps();
            }
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(path) = first_drop(&dropped) {
            self.open_path(&path);
        }

        if self.open_requested {
            self.open_requested = false;
            if let Some(path) = (self.open_dialog)() {
                self.open_path(&path);
            }
        }
    }

    fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.theme_pref = match ctx.theme() {
            egui::Theme::Dark => egui::ThemePreference::Light,
            egui::Theme::Light => egui::ThemePreference::Dark,
        };
    }

    fn apply_theme(&mut self, ctx: &egui::Context) {
        if self.applied_theme != Some(self.theme_pref) {
            ctx.set_theme(self.theme_pref);
            self.applied_theme = Some(self.theme_pref);
        }
    }

    fn apply_typography(ctx: &egui::Context) {
        ctx.all_styles_mut(|style| {
            style.text_styles = std::collections::BTreeMap::from([
                (egui::TextStyle::Small, egui::FontId::proportional(12.0)),
                (egui::TextStyle::Body, egui::FontId::proportional(16.0)),
                (egui::TextStyle::Button, egui::FontId::proportional(14.0)),
                (egui::TextStyle::Heading, egui::FontId::proportional(28.0)),
                (egui::TextStyle::Monospace, egui::FontId::monospace(14.0)),
            ]);
            style.spacing.item_spacing = egui::vec2(8.0, 10.0);
        });
    }

    fn refresh(&mut self) {
        if self.is_dirty() {
            // Never discard unsaved edits without asking.
            self.refresh_confirm = true;
            return;
        }
        self.refresh_now();
    }

    /// Unconditionally reload from disk, discarding unsaved edits.
    fn refresh_now(&mut self) {
        self.refresh_confirm = false;
        match &self.doc {
            Some(doc) => {
                let path = doc.path.clone();
                self.open_path(&path);
            }
            None => self.error = None,
        }
    }

    /// Shown after F5 / the Refresh button while there are unsaved edits:
    /// reloading would discard them, so ask first.
    fn show_refresh_confirm_banner(&mut self, ui: &mut egui::Ui) {
        if !self.refresh_confirm {
            return;
        }
        egui::Panel::top("refresh_confirm_banner").show(ui, |ui| {
            egui::Frame::default()
                .fill(ui.visuals().warn_fg_color.gamma_multiply(0.15))
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Cancel").clicked() {
                            self.refresh_confirm = false;
                        }
                        if ui.button("Discard & reload").clicked() {
                            self.refresh_now();
                        }
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "Discard unsaved changes and reload from disk?",
                        );
                    });
                });
        });
    }

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

    /// Shown while the file changed on disk and unsaved edits exist. The
    /// user either reloads from disk (discarding edits) or keeps editing.
    fn show_conflict_banner(&mut self, ui: &mut egui::Ui) {
        if !self.conflict {
            return;
        }
        egui::Panel::top("conflict_banner").show(ui, |ui| {
            egui::Frame::default()
                .fill(ui.visuals().warn_fg_color.gamma_multiply(0.15))
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reload from disk").clicked() {
                            self.conflict = false;
                            // Already an explicit choice made after a
                            // warning — no further confirmation.
                            self.refresh_now();
                        }
                        if ui.button("Keep editing").clicked() {
                            self.conflict = false;
                        }
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "The file changed on disk — you have unsaved edits.",
                        );
                    });
                });
        });
    }

    fn show_top_bar(&mut self, ui: &mut egui::Ui) {
        use icons::Icon;
        let ctx = ui.ctx().clone();
        let mut action: Option<Action> = None;
        let mut mode_clicked = false;
        let doc_info = self
            .doc
            .as_ref()
            .map(|doc| (doc.file_name(), doc.path.display().to_string(), doc.lossy));
        egui::Panel::top("top_bar")
            .frame(
                egui::Frame::default()
                    .fill(ui.visuals().window_fill)
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ui, |ui| {
                let bar_h = 32.0;
                let full = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width(), bar_h),
                );
                // Left group: open, refresh, mode segments.
                let left_rect = {
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(full)
                            .id_salt("top_bar_left")
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    );
                    if icons::icon_button(&mut child, Icon::Folder, "Open", true)
                        .on_hover_text("Open a file (Ctrl+O)")
                        .clicked()
                    {
                        action = Some(Action::Open);
                    }
                    if icons::icon_button(&mut child, Icon::Refresh, "Refresh", doc_info.is_some())
                        .on_hover_text("Reload from disk (F5)")
                        .clicked()
                    {
                        action = Some(Action::Refresh);
                    }
                    if icons::icon_button(&mut child, Icon::Save, "Save", self.is_dirty())
                        .on_hover_text("Save (Ctrl+S)")
                        .clicked()
                    {
                        action = Some(Action::Save);
                    }
                    if icons::icon_button(
                        &mut child,
                        Icon::Toc,
                        "Table of contents",
                        doc_info.is_some(),
                    )
                    .on_hover_text("Toggle the table of contents (Ctrl+T)")
                    .clicked()
                    {
                        action = Some(Action::ToggleToc);
                    }
                    child.separator();
                    // Clicking a segment selects that mode directly
                    // (Ctrl+E cycles through the same three modes).
                    if let Some(mode) = Self::mode_switcher(&mut child, self.mode)
                        && mode != self.mode
                    {
                        self.mode = mode;
                        mode_clicked = true;
                    }
                    child.min_rect()
                };
                // Right group: search and theme icons (plus lossy badge).
                let right_rect = {
                    let mut child = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(full)
                            .id_salt("top_bar_right")
                            .layout(egui::Layout::right_to_left(egui::Align::Center)),
                    );
                    let theme_icon = match ctx.theme() {
                        egui::Theme::Dark => Icon::Sun,
                        egui::Theme::Light => Icon::Moon,
                    };
                    if icons::icon_button(&mut child, theme_icon, "Toggle theme", true)
                        .on_hover_text("Switch theme (Ctrl+D)")
                        .clicked()
                    {
                        action = Some(Action::ToggleTheme);
                    }
                    if icons::icon_button(&mut child, Icon::Search, "Search", doc_info.is_some())
                        .on_hover_text("Search (Ctrl+F)")
                        .clicked()
                    {
                        action = Some(Action::Search);
                    }
                    if let Some((_, _, lossy)) = &doc_info
                        && *lossy
                    {
                        child.label(
                            egui::RichText::new(LOSSY_BADGE)
                                .small()
                                .color(child.visuals().warn_fg_color),
                        );
                    }
                    child.min_rect()
                };
                // Center: the file name, centered between the two groups.
                let title_rect = egui::Rect::from_min_max(
                    egui::pos2(left_rect.right() + 8.0, full.top()),
                    egui::pos2(right_rect.left() - 8.0, full.bottom()),
                );
                if let Some((name, path, _)) = &doc_info
                    && title_rect.width() > 60.0
                {
                    let mut title = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(title_rect)
                            .id_salt("top_bar_title")
                            .layout(egui::Layout::centered_and_justified(
                                egui::Direction::TopDown,
                            )),
                    );
                    title
                        .add(
                            egui::Label::new(egui::RichText::new(name.clone()).strong())
                                .halign(egui::Align::Center)
                                .truncate(),
                        )
                        .on_hover_text(path.clone());
                }
                ui.expand_to_include_rect(full);
            });
        if action.is_some() || mode_clicked {
            self.error = None;
        }
        match action {
            Some(Action::Open) => self.open_requested = true,
            Some(Action::ToggleMode) => {
                self.mode = match self.mode {
                    ViewMode::Rendered => ViewMode::Split,
                    ViewMode::Split => ViewMode::Source,
                    ViewMode::Source => ViewMode::Rendered,
                };
            }
            Some(Action::ToggleTheme) => self.toggle_theme(&ctx),
            Some(Action::Refresh) => self.refresh(),
            Some(Action::Save) => self.save_document(),
            Some(Action::Search) => {
                self.search.open = true;
                self.recompute_matches();
                ctx.memory_mut(|m| m.request_focus(egui::Id::new(SEARCH_FIELD)));
            }
            Some(Action::ToggleToc) if self.doc.is_some() => {
                self.toc_open = !self.toc_open;
            }
            Some(Action::ToggleToc) | None => {}
        }
    }

    /// Write the current text to disk; on success the dirty flag clears
    /// and self-caused watcher events are ignored briefly (they would
    /// otherwise reload stale disk text over keystrokes typed right after
    /// the save). Failures surface in the error banner.
    fn save_document(&mut self) {
        if let Some(doc) = &self.doc
            && doc.lossy
        {
            self.error = Some(
                "This file is not valid UTF-8 — saving is disabled to avoid data loss.".to_string(),
            );
            return;
        }
        let outcome = self
            .doc
            .as_ref()
            .map(|doc| (doc.save(), doc.file_name(), doc.raw.clone()));
        match outcome {
            Some((Ok(()), _, raw)) => {
                self.saved_raw = Some(raw);
                self.last_save_at = Some(Instant::now());
                self.conflict = false;
                self.refresh_confirm = false;
                self.error = None;
            }
            Some((Err(e), name, _)) => {
                self.error = Some(format!("Failed to save {name}: {e}"));
            }
            None => {}
        }
    }

    /// macOS-style segmented control for the three view modes.
    ///
    /// A soft `faint_bg_color` container holds rounded segments; the active
    /// mode gets an accent chip instead of a hard-edged block. Returns the
    /// mode whose segment was clicked, if any. Segments keep the accessible
    /// labels "Rendered"/"Split"/"Source" that the kittest tests click on.
    fn mode_switcher(ui: &mut egui::Ui, current: ViewMode) -> Option<ViewMode> {
        const SEGMENT_PADDING: egui::Vec2 = egui::vec2(10.0, 4.0);
        const SEGMENT_RADIUS: u8 = 5;
        const CONTAINER_RADIUS: u8 = 7;
        let mut clicked = None;
        let modes = [
            (ViewMode::Rendered, "Rendered"),
            (ViewMode::Split, "Split"),
            (ViewMode::Source, "Source"),
        ];
        let font_id = egui::TextStyle::Button.resolve(ui.style());
        let sizes: Vec<egui::Vec2> = modes
            .iter()
            .map(|(_, label)| {
                let galley = ui.painter().layout_no_wrap(
                    label.to_string(),
                    font_id.clone(),
                    egui::Color32::WHITE,
                );
                galley.size() + 2.0 * SEGMENT_PADDING
            })
            .collect();
        // A Frame allocates its rect anchored at the cursor with no
        // cross-axis alignment, which left the tabs below the toolbar
        // center. Reserving the exact container size first centers it like
        // every other toolbar widget (icons allocate the same way).
        let inner_w: f32 = sizes.iter().map(|s| s.x).sum::<f32>() + 2.0 * (sizes.len() - 1) as f32;
        let inner_h = sizes.iter().map(|s| s.y).fold(0.0f32, f32::max);
        let total = egui::vec2(inner_w + 4.0, inner_h + 4.0);
        let (container_rect, _) = ui.allocate_exact_size(total, egui::Sense::hover());
        ui.painter().rect_filled(
            container_rect,
            CONTAINER_RADIUS,
            ui.visuals().faint_bg_color,
        );
        // Lay the segments out in a child pinned to the container rect with
        // an explicit centered layout — the same pattern as the toolbar's
        // icon group, which centers correctly. `Frame`/`ui.horizontal`
        // re-anchor their content at the cursor and sag below the center.
        let mut inner = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(container_rect.shrink2(egui::vec2(2.0, 2.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center))
                .id_salt("mode_switcher_inner"),
        );
        inner.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
        for ((mode, label), size) in modes.iter().zip(&sizes) {
            let (rect, response) = inner.allocate_exact_size(*size, egui::Sense::click());
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
            let response = response
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text("Cycle view mode (Ctrl+E)");
            if response.clicked() {
                clicked = Some(*mode);
            }
            let selected = *mode == current;
            let pressed = response.hovered() || response.is_pointer_button_down_on();
            let bg = if selected {
                inner.visuals().selection.bg_fill
            } else if pressed {
                inner.style().interact(&response).weak_bg_fill
            } else {
                egui::Color32::TRANSPARENT
            };
            if bg != egui::Color32::TRANSPARENT {
                inner.painter().rect_filled(rect, SEGMENT_RADIUS, bg);
            }
            let text_color = if selected {
                inner.visuals().selection.stroke.color
            } else if pressed {
                inner.visuals().widgets.hovered.fg_stroke.color
            } else {
                inner.visuals().weak_text_color()
            };
            inner.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                label,
                font_id.clone(),
                text_color,
            );
        }
        clicked
    }

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
                    self.queue_jumps();
                }
                if ui.button(">").clicked() {
                    self.search.step(1);
                    self.queue_jumps();
                }
                if ui.button("Close").clicked() {
                    self.close_search();
                }
            });
        });
    }

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

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = self.window_title();
        if self.last_title.as_deref() != Some(title.as_str()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = Some(title);
        }
    }

    /// The collapsible table-of-contents sidebar (only while open and a
    /// document is loaded). Entries indent by heading level.
    fn show_toc_panel(&mut self, ui: &mut egui::Ui) {
        if !self.toc_open || self.doc.is_none() {
            return;
        }
        egui::Panel::left("rumd_toc")
            .resizable(true)
            .default_size(230.0)
            .min_size(160.0)
            .max_size(400.0)
            .show(ui, |ui| {
                let intents = toc::show_panel(ui, &self.toc_entries, &self.toc_collapsed);
                if let Some(path) = intents.toggled
                    && !self.toc_collapsed.remove(&path)
                {
                    self.toc_collapsed.insert(path);
                }
                if intents.jump_to.is_some() {
                    self.pending_toc_jump = intents.jump_to;
                }
            });
    }

    /// The window title: file name, with an unsaved-changes marker while
    /// the text differs from disk.
    fn window_title(&self) -> String {
        match &self.doc {
            Some(doc) if self.is_dirty() => format!("rumd - {} •", doc.file_name()),
            Some(doc) => format!("rumd - {}", doc.file_name()),
            None => "rumd".to_string(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Mutex;

    use egui_kittest::{Harness, kittest::Queryable};

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rumd_app_{}_{}", std::process::id(), name))
    }

    /// Stable path for snapshot tests: the rendered file name must not vary
    /// between runs, so no pid in the name. One directory per test keeps
    /// parallel tests from colliding.
    fn stable_path(test: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rumd_snap_{}", test));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("snapshot.md")
    }

    /// open_path changes the process working directory (for relative image
    /// resolution), so harness tests that open files serialize on this lock.
    static CWD_LOCK: Mutex<()> = Mutex::new(());

    fn min_repaint_delay(harness: &Harness) -> std::time::Duration {
        harness
            .output()
            .viewport_output
            .values()
            .map(|v| v.repaint_delay)
            .min()
            .unwrap_or(std::time::Duration::MAX)
    }

    // Each UI test shares its App with the harness closure through an Rc so
    // the test can mutate the app between frames. Build the harness inline:
    //
    //     let app = Rc::new(RefCell::new(App::new(None)));
    //     let app_for_ui = app.clone();
    //     let mut harness = Harness::builder()
    //         .with_size(egui::vec2(800.0, 600.0))
    //         .build_ui(move |ui| {
    //             app_for_ui.borrow_mut().show(ui);
    //         });

    #[test]
    fn mode_segments_are_vertically_centered_with_toolbar_icons() {
        use egui_kittest::kittest::Queryable;
        let doc = temp_path("tabalign.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Tabs").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 200.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.run();
        let icon_center = harness.get_by_label("Refresh").rect().center().y;
        for label in ["Rendered", "Split", "Source"] {
            let segment_center = harness.get_by_label(label).rect().center().y;
            assert!(
                (segment_center - icon_center).abs() < 0.5,
                "{label} center {segment_center:.1} must match icon center {icon_center:.1}"
            );
        }
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn editing_source_scrolls_the_preview_to_the_edited_section() {
        let doc = temp_path("editjump.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        let mut md = String::new();
        for i in 0..40 {
            md.push_str(&format!("# Section {i}\n\nparagraph {i} text\n\n"));
        }
        std::fs::write(&doc, &md).unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().mode = ViewMode::Split;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1100.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(4);
        assert!(
            harness.query_by_label("paragraph 39 text").is_none(),
            "precondition: the last section must start out of view"
        );
        let full_text = app.borrow().doc.as_ref().unwrap().raw.clone();
        harness.get_by_value(&full_text).focus();
        harness.run();
        harness.get_by_value(&full_text).type_text("X");
        let mut jumped = false;
        for _ in 0..60 {
            harness.run();
            if harness.query_by_label("paragraph 39 text").is_some() {
                jumped = true;
                break;
            }
        }
        assert!(
            jumped,
            "editing at the end must scroll the preview to the edited section"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn refresh_with_unsaved_edits_asks_for_confirmation() {
        let path = temp_path("refreshask.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "# Refresh Ask\n\noriginal").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_value("# Refresh Ask\n\noriginal").focus();
        harness.run();
        harness
            .get_by_value("# Refresh Ask\n\noriginal")
            .type_text("X");
        harness.run();
        assert!(app.borrow().is_dirty());

        // Meanwhile the file changed on disk.
        std::fs::write(&path, "# Changed on disk\n\nv2").unwrap();

        // Refresh must ask, not reload.
        app.borrow_mut().refresh();
        harness.run();
        assert!(
            harness
                .query_by_label_contains("Discard unsaved changes")
                .is_some(),
            "refresh with unsaved edits must ask for confirmation"
        );
        let raw = app.borrow().doc.as_ref().unwrap().raw.clone();
        assert!(raw.contains('X'), "nothing may reload before confirmation");
        assert!(
            app.borrow().is_dirty(),
            "edits must be kept pending the answer"
        );

        // Confirming discards the edits and loads the disk version.
        harness.get_by_label("Discard & reload").click();
        let mut reloaded = false;
        for _ in 0..40 {
            harness.run();
            if app
                .borrow()
                .doc
                .as_ref()
                .unwrap()
                .raw
                .contains("Changed on disk")
            {
                reloaded = true;
                break;
            }
        }
        assert!(reloaded, "confirmation must reload from disk");
        assert!(!app.borrow().is_dirty(), "reload must clear the dirty flag");
        assert!(
            harness
                .query_by_label_contains("Discard unsaved changes")
                .is_none(),
            "the banner must disappear once answered"
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn refresh_confirm_can_be_cancelled() {
        let path = temp_path("refreshcancel.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "# Refresh Cancel\n\noriginal").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_value("# Refresh Cancel\n\noriginal").focus();
        harness.run();
        harness
            .get_by_value("# Refresh Cancel\n\noriginal")
            .type_text("X");
        harness.run();

        std::fs::write(&path, "# Changed on disk\n\nv2").unwrap();
        app.borrow_mut().refresh();
        harness.run();
        assert!(
            harness
                .query_by_label_contains("Discard unsaved changes")
                .is_some()
        );

        harness.get_by_label("Cancel").click();
        harness.run_steps(2);
        assert!(
            harness
                .query_by_label_contains("Discard unsaved changes")
                .is_none(),
            "Cancel must dismiss the banner"
        );
        assert!(
            app.borrow().is_dirty(),
            "edits must survive a cancelled refresh"
        );
        let raw = app.borrow().doc.as_ref().unwrap().raw.clone();
        assert!(raw.contains('X'), "edits must survive a cancelled refresh");
        assert!(
            !raw.contains("Changed on disk"),
            "a cancelled refresh must not load the disk version"
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn refresh_when_clean_reloads_immediately() {
        let path = temp_path("refreshclean.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "# Refresh Clean\n\noriginal").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);

        std::fs::write(&path, "# Fresh from disk\n\nv2").unwrap();
        app.borrow_mut().refresh();
        let mut reloaded = false;
        for _ in 0..40 {
            harness.run();
            if app
                .borrow()
                .doc
                .as_ref()
                .unwrap()
                .raw
                .contains("Fresh from disk")
            {
                reloaded = true;
                break;
            }
        }
        assert!(reloaded, "a clean refresh must reload without asking");
        assert!(
            harness
                .query_by_label_contains("Discard unsaved changes")
                .is_none(),
            "a clean refresh must not show the confirmation banner"
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn shortcut_actions_map_correctly() {
        let ctrl = egui::Modifiers::CTRL;
        let cmd = egui::Modifiers::COMMAND;
        assert_eq!(shortcut_action(ctrl, egui::Key::O), Some(Action::Open));
        assert_eq!(shortcut_action(cmd, egui::Key::O), Some(Action::Open));
        assert_eq!(
            shortcut_action(ctrl, egui::Key::E),
            Some(Action::ToggleMode)
        );
        assert_eq!(shortcut_action(cmd, egui::Key::E), Some(Action::ToggleMode));
        assert_eq!(
            shortcut_action(ctrl, egui::Key::D),
            Some(Action::ToggleTheme)
        );
        assert_eq!(
            shortcut_action(cmd, egui::Key::D),
            Some(Action::ToggleTheme)
        );
        assert_eq!(
            shortcut_action(egui::Modifiers::NONE, egui::Key::F5),
            Some(Action::Refresh)
        );
        assert_eq!(shortcut_action(cmd, egui::Key::F5), None);
        assert_eq!(shortcut_action(egui::Modifiers::NONE, egui::Key::O), None);
        assert_eq!(shortcut_action(ctrl, egui::Key::X), None);
        assert_eq!(shortcut_action(ctrl, egui::Key::F), Some(Action::Search));
        assert_eq!(shortcut_action(cmd, egui::Key::F), Some(Action::Search));
        assert_eq!(shortcut_action(ctrl, egui::Key::T), Some(Action::ToggleToc));
        assert_eq!(shortcut_action(cmd, egui::Key::T), Some(Action::ToggleToc));
    }

    #[test]
    fn event_action_only_fires_on_key_press() {
        let key = egui::Event::Key {
            key: egui::Key::E,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        };
        assert_eq!(event_action(&key), Some(Action::ToggleMode));
        let released = egui::Event::Key {
            key: egui::Key::E,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        };
        assert_eq!(event_action(&released), None);
        let pointer = egui::Event::PointerMoved(egui::pos2(1.0, 2.0));
        assert_eq!(event_action(&pointer), None);
    }

    #[test]
    fn empty_state_shows_hint() {
        let app = Rc::new(RefCell::new(App::new(None)));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.get_by_label("Drop a .md file here or press Ctrl+O");
    }

    #[test]
    fn open_failure_shows_error_banner_keeps_previous_doc() {
        let good = temp_path("good.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&good, "# Keep Me").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&good);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.get_by_label("Keep Me");

        app.borrow_mut()
            .open_path(Path::new("/nonexistent/rumd/missing.md"));
        harness.run();
        assert!(harness.query_by_label_contains("Failed to open").is_some());
        // Previous document must still be visible.
        harness.get_by_label("Keep Me");
        std::fs::remove_file(&good).unwrap();
    }

    #[test]
    fn lossy_badge_appears_for_invalid_utf8() {
        let bad = temp_path("bad.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&bad, [b'#', b' ', 0xFF]).unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&bad);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.get_by_label("not valid UTF-8");
        std::fs::remove_file(&bad).unwrap();
    }

    #[test]
    fn mode_toggle_button_switches_views() {
        let doc = temp_path("toggle.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# T5 Heading\n\nbody text").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.get_by_label("T5 Heading");

        harness.get_by_label("Source").click();
        harness.run();
        assert!(harness.query_by_label("T5 Heading").is_none());

        harness.get_by_label("Rendered").click();
        harness.run();
        harness.get_by_label("T5 Heading");
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn theme_button_toggles_theme_and_keeps_view() {
        let doc = temp_path("theme.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Themed Doc").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.get_by_label("Themed Doc");

        let before = app.borrow().theme_pref;
        harness.get_by_label("Toggle theme").click();
        harness.run();
        let mid = app.borrow().theme_pref;
        assert_ne!(mid, before, "one click must flip the theme preference");
        harness.get_by_label("Themed Doc");
        harness.get_by_label("Toggle theme").click();
        harness.run();
        let after = app.borrow().theme_pref;
        assert_ne!(after, mid, "second click must flip the theme back");
        // The toggle cycles Dark/Light absolutely (never back to System).
        let mid_is_dark = mid == egui::ThemePreference::Dark;
        assert_eq!(
            after,
            if mid_is_dark {
                egui::ThemePreference::Light
            } else {
                egui::ThemePreference::Dark
            }
        );
        harness.get_by_label("Themed Doc");
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn search_icon_opens_search_bar() {
        let doc = temp_path("searchicon.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "alpha and alpha").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        assert!(!app.borrow().search.open, "search starts closed");
        harness.get_by_label("Search").click();
        harness.run();
        assert!(app.borrow().search.open, "Search button must open the bar");
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn refresh_disabled_without_document() {
        let app = Rc::new(RefCell::new(App::new(None)));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        assert!(
            harness
                .query_by(|n| n.label().as_deref() == Some("Refresh") && n.is_disabled())
                .is_some(),
            "Refresh must be disabled when no document is open"
        );
    }

    #[test]
    fn top_bar_snapshot() {
        let doc = stable_path("top_bar");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Snapshot\n\nsome body text").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 200.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.run();
        // open_path changed the process CWD; pin the snapshot location.
        let mut opts = egui_kittest::SnapshotOptions::default();
        opts.output_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
        harness.snapshot_options("top_bar", &opts);
        std::fs::remove_file(&doc).unwrap();
    }

    /// Capture the website screenshots into `website/assets/`. Gated behind
    /// RUMD_CAPTURE=1 so ordinary `cargo test` runs stay hermetic; the
    /// release workflow runs it with:
    ///
    ///     RUMD_CAPTURE=1 cargo test capture_website_screenshots -- --nocapture
    ///
    /// Override the destination with RUMD_CAPTURE_DIR. The subject is the
    /// dummy document in tests/fixtures/sample.md, never a real project
    /// file.
    #[test]
    fn capture_website_screenshots() {
        if std::env::var("RUMD_CAPTURE").map_or(true, |v| v != "1") {
            return;
        }
        let out_dir = std::env::var("RUMD_CAPTURE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("website/assets")
            });
        std::fs::create_dir_all(&out_dir).unwrap();

        // A friendly dummy name for the website title bar; kept out of
        // stable_path so the snapshot tests keep their own fixture name.
        let dir = std::env::temp_dir().join(format!("rumd_capture_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let doc = dir.join("aurora-field-notes.md");
        let _cwd = CWD_LOCK.lock().unwrap();
        std::fs::write(&doc, include_str!("../tests/fixtures/sample.md")).unwrap();

        let capture = |name: &str,
                       theme: egui::ThemePreference,
                       mode: ViewMode,
                       toc: bool,
                       search_query: Option<&str>| {
            let app = Rc::new(RefCell::new(App::new(None)));
            {
                let mut a = app.borrow_mut();
                a.theme_pref = theme;
                a.mode = mode;
                a.open_path(&doc);
                if let Some(query) = search_query {
                    a.search.open = true;
                    a.search.query = query.to_owned();
                    a.recompute_matches();
                }
            }
            let app_for_ui = app.clone();
            let mut harness = Harness::builder()
                .with_size(egui::vec2(1280.0, 800.0))
                .build_ui(move |ui| {
                    app_for_ui.borrow_mut().show(ui);
                });
            if toc {
                harness.get_by_label("Table of contents").click();
            }
            harness.run();
            harness.run();
            let image = harness.render().expect("screenshot render must succeed");
            image.save(out_dir.join(name)).expect("screenshot save must succeed");
        };

        capture(
            "dark-rendered.png",
            egui::ThemePreference::Dark,
            ViewMode::Rendered,
            true,
            None,
        );
        capture(
            "dark-split.png",
            egui::ThemePreference::Dark,
            ViewMode::Split,
            false,
            None,
        );
        capture(
            "light-rendered.png",
            egui::ThemePreference::Light,
            ViewMode::Rendered,
            false,
            None,
        );
        capture(
            "dark-search.png",
            egui::ThemePreference::Dark,
            ViewMode::Rendered,
            false,
            Some("sensor"),
        );

        std::fs::remove_file(&doc).unwrap();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn top_bar_light_theme_snapshot() {
        let doc = stable_path("top_bar_light");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Snapshot\n\nsome body text").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().theme_pref = egui::ThemePreference::Light;
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 200.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.run();
        let mut opts = egui_kittest::SnapshotOptions::default();
        opts.output_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
        harness.snapshot_options("top_bar_light", &opts);
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn tiny_window_does_not_panic() {
        let app = Rc::new(RefCell::new(App::new(None)));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(300.0, 200.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.run();
    }

    #[test]
    fn editing_source_marks_document_dirty() {
        let doc = temp_path("dirty.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Dirty Doc\n\noriginal text").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        assert!(
            !app.borrow().is_dirty(),
            "a freshly opened document must not be dirty"
        );
        harness.get_by_value("# Dirty Doc\n\noriginal text").focus();
        harness.run();
        harness
            .get_by_value("# Dirty Doc\n\noriginal text")
            .type_text("X");
        harness.run();
        assert!(
            app.borrow().is_dirty(),
            "typing in the source pane must mark the document dirty"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn editing_source_recomputes_search_matches() {
        let doc = temp_path("editsearch.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Search Doc\n\nplain text").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().mode = ViewMode::Source;
        {
            let mut a = app.borrow_mut();
            a.search.open = true;
            a.search.query = "needle".into();
            a.recompute_matches();
        }
        assert_eq!(app.borrow().search.matches.len(), 0);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_value("# Search Doc\n\nplain text").focus();
        harness.run();
        harness
            .get_by_value("# Search Doc\n\nplain text")
            .type_text("needle");
        harness.run();
        assert_eq!(
            app.borrow().search.matches.len(),
            1,
            "matches must be recomputed after edits"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn ctrl_s_saves_dirty_document_to_disk() {
        let doc = temp_path("saveto.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Save Doc\n\noriginal").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_value("# Save Doc\n\noriginal").focus();
        harness.run();
        harness
            .get_by_value("# Save Doc\n\noriginal")
            .type_text("X");
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::S);
        // The save fires the watcher; the app keeps requesting repaints
        // until the debounced reload settles.
        harness.run_steps(4);
        let on_disk = std::fs::read_to_string(&doc).unwrap();
        assert!(
            on_disk.contains('X'),
            "Ctrl+S must write the edited text, disk has {on_disk:?}"
        );
        assert!(!app.borrow().is_dirty(), "saving must clear the dirty flag");
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn save_button_saves_and_is_disabled_when_clean() {
        let doc = temp_path("savebtn.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Save Button\n\noriginal").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        assert!(
            harness
                .query_by(|n| n.label().as_deref() == Some("Save") && n.is_disabled())
                .is_some(),
            "Save must be disabled while the document is clean"
        );
        harness.get_by_value("# Save Button\n\noriginal").focus();
        harness.run();
        harness
            .get_by_value("# Save Button\n\noriginal")
            .type_text("X");
        harness.run();
        harness.get_by_label("Save").click();
        harness.run_steps(4);
        let on_disk = std::fs::read_to_string(&doc).unwrap();
        assert!(
            on_disk.contains('X'),
            "Save button must write the edited text, disk has {on_disk:?}"
        );
        assert!(!app.borrow().is_dirty(), "saving must clear the dirty flag");
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn window_title_marks_unsaved_changes() {
        let doc = stable_path("title");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Titled").unwrap();
        let mut app = App::new(None);
        app.open_path(&doc);
        assert_eq!(app.window_title(), "rumd - snapshot.md");
        app.doc.as_mut().unwrap().raw.push_str(" edited");
        assert_eq!(app.window_title(), "rumd - snapshot.md •");
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn reload_after_save_keeps_post_save_edits() {
        let doc = temp_path("postsavesup.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# Sup Doc\n\noriginal").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_value("# Sup Doc\n\noriginal").focus();
        harness.run();
        harness.get_by_value("# Sup Doc\n\noriginal").type_text("X");
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::S);
        harness.run_steps(2);
        // Keep typing after the save. The save's own watcher event must
        // never reload disk text over these edits.
        harness
            .get_by_value("# Sup Doc\n\noriginalX")
            .type_text("Y");
        let mut survived = true;
        for _ in 0..40 {
            harness.run_steps(1);
            std::thread::sleep(std::time::Duration::from_millis(25));
            let raw = app.borrow().doc.as_ref().unwrap().raw.clone();
            if !raw.contains('Y') || !raw.contains('X') {
                survived = false;
                break;
            }
        }
        assert!(
            survived,
            "post-save edits must survive the self-caused reload, text: {:?}",
            app.borrow().doc.as_ref().unwrap().raw
        );
        assert!(app.borrow().is_dirty(), "post-save edits must be dirty");
        let on_disk = std::fs::read_to_string(&doc).unwrap();
        assert!(on_disk.contains('X') && !on_disk.contains('Y'));
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn external_change_while_dirty_shows_conflict_and_keeps_edits() {
        let path = temp_path("conflict.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "# Conflict Doc\n\noriginal").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_value("# Conflict Doc\n\noriginal").focus();
        harness.run();
        harness
            .get_by_value("# Conflict Doc\n\noriginal")
            .type_text("X");
        harness.run();

        std::fs::write(&path, "# Changed Externally\n\nfrom disk").unwrap();
        let mut showed = false;
        for _ in 0..200 {
            harness.run_steps(1);
            if harness.query_by_label_contains("changed on disk").is_some() {
                showed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(
            showed,
            "conflict banner must appear when the file changes on disk during edits"
        );
        let raw = app.borrow().doc.as_ref().unwrap().raw.clone();
        assert!(raw.contains('X'), "unsaved edits must survive, got {raw:?}");
        assert!(
            !raw.contains("Externally"),
            "disk version must not replace unsaved edits"
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn conflict_banner_buttons_resolve_the_conflict() {
        let path = temp_path("conflictbtn.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "# Conflict Btn\n\noriginal").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_value("# Conflict Btn\n\noriginal").focus();
        harness.run();
        harness
            .get_by_value("# Conflict Btn\n\noriginal")
            .type_text("X");
        harness.run();

        // Trigger the conflict.
        std::fs::write(&path, "# Externally Changed\n\nv2").unwrap();
        for _ in 0..200 {
            harness.run_steps(1);
            if harness.query_by_label_contains("changed on disk").is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        // "Keep editing" dismisses the banner and keeps both the edits and
        // the dirty flag.
        harness.get_by_label("Keep editing").click();
        harness.run_steps(2);
        assert!(
            harness.query_by_label_contains("changed on disk").is_none(),
            "Keep editing must dismiss the banner"
        );
        assert!(app.borrow().is_dirty(), "edits must still be unsaved");
        let raw = app.borrow().doc.as_ref().unwrap().raw.clone();
        assert!(raw.contains('X'), "edits must survive, got {raw:?}");

        // A fresh external change raises the banner again; "Reload from
        // disk" then discards the edits and adopts the disk version.
        std::fs::write(&path, "# Externally Changed\n\nv3").unwrap();
        let mut showed = false;
        for _ in 0..200 {
            harness.run_steps(1);
            if harness.query_by_label_contains("changed on disk").is_some() {
                showed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(showed, "second external change must raise the banner again");
        harness.get_by_label("Reload from disk").click();
        let mut reloaded = false;
        for _ in 0..40 {
            harness.run_steps(1);
            if app.borrow().doc.as_ref().unwrap().raw.contains("v3") {
                reloaded = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(reloaded, "Reload from disk must adopt the disk version");
        assert!(!app.borrow().is_dirty(), "reload must clear the dirty flag");
        // Settle past the click frame, which still paints the banner.
        harness.run_steps(2);
        assert!(
            harness.query_by_label_contains("changed on disk").is_none(),
            "reload must dismiss the banner"
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn ctrl_s_does_not_rewrite_lossy_files() {
        let path = temp_path("lossysave.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        let original: &[u8] = &[b'#', b' ', 0xFF];
        std::fs::write(&path, original).unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::S);
        harness.run_steps(2);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            original,
            "saving must never rewrite a non-UTF-8 file"
        );
        assert!(
            app.borrow()
                .error
                .as_deref()
                .is_some_and(|e| e.contains("not valid UTF-8")),
            "the user must be told why saving is unavailable, got {:?}",
            app.borrow().error
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn app_keeps_lossy_files_read_only() {
        use egui_kittest::kittest::Queryable;
        let path = temp_path("lossyapp.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, [b'#', b' ', 0xFF]).unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        app.borrow_mut().mode = ViewMode::Source;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        let editor = harness.query_by(|n| {
            n.role() == egui::accesskit::Role::MultilineTextInput
                && n.value().is_some_and(|v| v.contains('\u{FFFD}'))
        });
        assert!(
            editor.is_some(),
            "the lossy text must be shown in the source pane"
        );
        editor.unwrap().focus();
        harness.run();
        harness
            .get_by(|n| {
                n.role() == egui::accesskit::Role::MultilineTextInput
                    && n.value().is_some_and(|v| v.contains('\u{FFFD}'))
            })
            .type_text("X");
        harness.run();
        assert!(
            !app.borrow().is_dirty(),
            "a lossy document must not become editable"
        );
        assert_eq!(
            app.borrow().doc.as_ref().unwrap().raw.matches('X').count(),
            0,
            "typing must not reach the text of a lossy document"
        );
        std::fs::remove_file(&path).unwrap();
    }

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
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::E);
        harness.run();
        assert_eq!(app.borrow().mode, ViewMode::Split);
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::E);
        harness.run();
        assert_eq!(app.borrow().mode, ViewMode::Source);
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::E);
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
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::F);
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
        assert!(
            harness.query_by_label("1/2").is_none(),
            "esc closes the bar"
        );
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

        std::fs::write(&path, "one two").unwrap();
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
            a.queue_jumps();
        }
        assert!(app.borrow().source_highlight.is_some());
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(3);
        assert!(
            app.borrow().source_highlight.is_some(),
            "highlight persists while the find bar stays open"
        );
        std::fs::remove_file(&doc).unwrap();
    }

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
        assert!(
            app.borrow().pending_render_jump.is_none(),
            "jump consumed by rendered view"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn prefs_roundtrip_and_defaults() {
        let mut app = App::new(None);
        app.mode = ViewMode::Source;
        app.theme_pref = egui::ThemePreference::Dark;
        app.zoom = 1.3;
        app.toc_open = true;
        let s = app.prefs_to_string();
        assert!(s.contains("mode=source"));
        assert!(s.contains("theme=dark"));
        assert!(s.contains("zoom=1.3"));
        assert!(s.contains("toc=open"));

        let mut other = App::new(None);
        other.apply_prefs_string(&s);
        assert_eq!(other.mode, ViewMode::Source);
        assert_eq!(other.theme_pref, egui::ThemePreference::Dark);
        assert!((other.zoom - 1.3).abs() < 1e-6);
        assert!(other.toc_open);

        // Garbage and missing values fall back without panicking.
        other.apply_prefs_string("theme=bogus\nzoom=zzz\nmode=huh\ntoc=huh\n");
        assert_eq!(other.theme_pref, egui::ThemePreference::System);
        assert_eq!(other.mode, ViewMode::Rendered);
        assert!((other.zoom - 1.0).abs() < 1e-6);
        assert!(!other.toc_open, "a bogus toc value closes the ToC");

        // A missing toc line (older prefs file) leaves the default closed.
        other.apply_prefs_string("theme=dark\n");
        assert!(!other.toc_open);
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
        // egui's native zoom shortcuts are bound to Modifiers::COMMAND
        // (the winit backend maps ctrl to command on non-Mac; kittest must
        // inject the command bit explicitly).
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
        harness.run();
        assert!(
            app.borrow().zoom > 1.0,
            "ctrl+= must zoom in natively, zoom={}",
            app.borrow().zoom
        );
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num0);
        harness.run();
        assert!(
            (app.borrow().zoom - 1.0).abs() < 1e-6,
            "ctrl+0 must reset zoom, zoom={}",
            app.borrow().zoom
        );
    }

    #[test]
    fn zoom_delta_events_update_mirrored_zoom() {
        let app = Rc::new(RefCell::new(App::new(None)));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        // egui routes ctrl/cmd+wheel and trackpad pinch into a per-frame
        // zoom delta; `Event::Zoom` is the headless way to inject it.
        harness.event(egui::Event::Zoom(1.25));
        harness.run();
        assert!(
            (app.borrow().zoom - 1.25).abs() < 1e-4,
            "a zoom event must scale the ui, zoom={}",
            app.borrow().zoom
        );
        assert!(
            app.borrow().prefs_to_string().contains("zoom=1.25"),
            "wheel zoom must persist like shortcut zoom, prefs={}",
            app.borrow().prefs_to_string()
        );
        // Overshooting the range clamps to the configured limits.
        for _ in 0..5 {
            harness.event(egui::Event::Zoom(2.0));
            harness.run();
        }
        assert!(
            (app.borrow().zoom - ZOOM_MAX).abs() < 1e-4,
            "zoom must clamp at the maximum, zoom={}",
            app.borrow().zoom
        );
        for _ in 0..5 {
            harness.event(egui::Event::Zoom(0.1));
            harness.run();
        }
        assert!(
            (app.borrow().zoom - ZOOM_MIN).abs() < 1e-4,
            "zoom must clamp at the minimum, zoom={}",
            app.borrow().zoom
        );
        // The keyboard reset still recovers from a wheel-zoomed state.
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num0);
        harness.run();
        assert!(
            (app.borrow().zoom - 1.0).abs() < 1e-6,
            "ctrl+0 must reset wheel zoom too, zoom={}",
            app.borrow().zoom
        );
    }

    fn notch(lines: f32) -> egui::Event {
        egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, lines),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::COMMAND,
        }
    }

    #[test]
    fn wheel_notch_zooms_in_discrete_steps() {
        let app = Rc::new(RefCell::new(App::new(None)));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        // One notch = one discrete step, like Ctrl+= (and browsers).
        harness.event(notch(1.0));
        harness.run_steps(1);
        assert!(
            (app.borrow().zoom - 1.1).abs() < 1e-4,
            "one notch must zoom one step, zoom={}",
            app.borrow().zoom
        );
        harness.event(notch(1.0));
        harness.run_steps(1);
        assert!(
            (app.borrow().zoom - 1.2).abs() < 1e-4,
            "second notch must zoom a further step, zoom={}",
            app.borrow().zoom
        );
        // The notch must not leak into egui's smoothed wheel zoom: the
        // value must stay put on the following frames.
        harness.run_steps(5);
        assert!(
            (app.borrow().zoom - 1.2).abs() < 1e-4,
            "zoom must not drift after the notch, zoom={}",
            app.borrow().zoom
        );
        // Scrolling down zooms out.
        harness.event(notch(-1.0));
        harness.run_steps(1);
        assert!(
            (app.borrow().zoom - 1.1).abs() < 1e-4,
            "reverse notch must zoom out, zoom={}",
            app.borrow().zoom
        );
        // A burst of notches between frames applies all of them at once…
        for _ in 0..20 {
            harness.event(notch(-1.0));
        }
        harness.run_steps(1);
        // …and clamping keeps the value inside the configured range.
        assert!(
            (app.borrow().zoom - ZOOM_MIN).abs() < 1e-4,
            "zoom must clamp at the minimum, zoom={}",
            app.borrow().zoom
        );
        // The keyboard reset still recovers from a wheel-zoomed state.
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num0);
        harness.run();
        assert!(
            (app.borrow().zoom - 1.0).abs() < 1e-6,
            "ctrl+0 must reset wheel zoom too, zoom={}",
            app.borrow().zoom
        );
    }

    #[test]
    fn empty_state_offers_open_button() {
        let app = Rc::new(RefCell::new(App::new(None)));
        // Stub the dialog: the real rfd call would block headless.
        let dialog_used = Rc::new(std::cell::Cell::new(false));
        {
            let dialog_used = dialog_used.clone();
            app.borrow_mut().open_dialog = Box::new(move || {
                dialog_used.set(true);
                None
            });
        }
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.get_by_label("rumd");
        harness.get_by_label("Open…").click();
        harness.run();
        assert!(dialog_used.get(), "Open… button must reach the dialog hook");
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

    #[test]
    fn zoom_prefs_restore_into_the_context() {
        let mut app = App::new(None);
        app.apply_prefs_string("zoom=1.5\n");
        let ctx = egui::Context::default();
        app.apply_zoom_to_ctx(&ctx);
        assert!(
            (ctx.zoom_factor() - 1.5).abs() < 1e-6,
            "loaded zoom must be applied to the egui context, got {}",
            ctx.zoom_factor()
        );
    }

    #[test]
    fn recompute_matches_drops_stale_pending_jumps() {
        let doc = temp_path("stalejump.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&doc, "# One\n\nalpha\n\n# Two\n\nalpha\n").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        {
            let mut a = app.borrow_mut();
            a.search.query = "alpha".into();
            a.recompute_matches();
            a.search.step(1);
            a.queue_jumps();
        }
        assert!(app.borrow().pending_render_jump.is_some());
        // A reload recomputes matches; pending jumps queued against the old
        // text must be dropped, not scrolled to.
        app.borrow_mut().open_path(&doc);
        assert!(
            app.borrow().pending_render_jump.is_none(),
            "stale rendered jump dropped on reload"
        );
        assert!(
            app.borrow().source_highlight.is_none(),
            "stale source highlight dropped on reload"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn first_drop_picks_first_path() {
        let a = PathBuf::from("/tmp/a.md");
        let b = PathBuf::from("/tmp/b.md");
        assert_eq!(first_drop(&[]), None);
        assert_eq!(first_drop(&[a.clone(), b]), Some(a));
    }

    #[test]
    fn dropping_directory_shows_error_keeps_doc() {
        let good = temp_path("drop_keep.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&good, "# Still Here").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&good);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();

        app.borrow_mut().open_path(Path::new("/tmp"));
        harness.run();
        assert!(harness.query_by_label_contains("Failed to open").is_some());
        harness.get_by_label("Still Here");
        std::fs::remove_file(&good).unwrap();
    }

    #[test]
    fn auto_reload_updates_rendered_view() {
        let path = temp_path("reload.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "# First Heading").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_label("First Heading");

        std::fs::write(&path, "# Reloaded Heading").unwrap();
        let mut updated = false;
        for _ in 0..100 {
            harness.run_steps(1);
            if harness.query_by_label("Reloaded Heading").is_some() {
                updated = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(updated, "auto-reload never updated the view");
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn refresh_keeps_error_when_file_missing() {
        let path = temp_path("refresh.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "# Gone Soon").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        std::fs::remove_file(&path).unwrap();

        app.borrow_mut().refresh();
        let error = app.borrow().error.clone();
        assert!(
            error
                .as_deref()
                .is_some_and(|e| e.contains("Failed to open")),
            "expected refresh failure to be reported, got {error:?}"
        );
        // The previously loaded document stays visible.
        assert!(app.borrow().doc.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn watcher_failure_shows_banner_once() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // Root bypasses directory permissions; the test cannot simulate
        // watch failure there.
        if std::fs::metadata("/").unwrap().uid() == 0 {
            return;
        }
        let _cwd = CWD_LOCK.lock().unwrap();
        let dir = temp_path("watchfail_dir");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("w.md");
        std::fs::write(&file, "# Watch Fail").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&file);
        {
            let app = app.borrow();
            assert!(app.watcher_failed);
            assert!(app.doc.is_some(), "document must still load");
            let error = app.error.clone().unwrap_or_default();
            assert!(
                error.contains("Auto-reload unavailable"),
                "expected one-time info banner, got {error:?}"
            );
        }

        // A later refresh must not repeat the message.
        app.borrow_mut().refresh();
        assert!(app.borrow().error.is_none());
        assert!(app.borrow().watcher_failed);

        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn auto_reload_fires_for_relative_path_open() {
        let _cwd = CWD_LOCK.lock().unwrap();
        let dir = temp_path("relreload_dir");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rel.md");
        std::fs::write(&file, "# First Heading").unwrap();

        std::env::set_current_dir(&dir).unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(Path::new("rel.md"));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.get_by_label("First Heading");

        std::fs::write(&file, "# Reloaded Heading").unwrap();
        let mut updated = false;
        for _ in 0..100 {
            harness.run_steps(1);
            if harness.query_by_label("Reloaded Heading").is_some() {
                updated = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(
            updated,
            "auto-reload never updated the view for a relative open path"
        );
        std::fs::remove_file(&file).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn watcher_event_schedules_repaint_wakeup() {
        let path = temp_path("wake.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "# Wake").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);

        std::fs::write(&path, "# Wake 2").unwrap();
        // Frame until the watcher event has been drained into changed_at,
        // then the very next frame must schedule a wakeup within the
        // debounce window while it waits to reload.
        let mut drained = false;
        for _ in 0..100 {
            harness.run_steps(1);
            if app.borrow().changed_at.is_some() {
                drained = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(drained, "watcher event never reached the app");
        harness.run_steps(1);
        let delay = min_repaint_delay(&harness);
        assert!(
            delay <= std::time::Duration::from_millis(200),
            "pending reload did not schedule a repaint wakeup, delay {delay:?}"
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn watcher_debounce_settles_for_harness_run() {
        // Regression: egui subtracts the predicted frame time from repaint
        // delays, and kittest's default step_dt (250ms) exceeds the whole
        // debounce window — the request collapsed to an immediate repaint
        // and `Harness::run` looped into max_steps. Drain a watcher event,
        // then `run()` must settle instead of spinning.
        let path = temp_path("debouncesettle.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        std::fs::write(&path, "# Debounce Settle").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&path);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);

        std::fs::write(&path, "# Debounce Settle 2").unwrap();
        let mut drained = false;
        for _ in 0..100 {
            harness.run_steps(1);
            if app.borrow().changed_at.is_some() {
                drained = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(drained, "watcher event never reached the app");
        harness.run();
        harness.run();
        harness.run();
        std::fs::remove_file(&path).unwrap();
    }

    // --- perf probe: Rendered <-> Split switch frame cost -----------------
    // Temporary measurement harness. Run with:
    //   cargo test --release perf_probe_mode_switch -- --ignored --nocapture

    fn large_markdown(sections: usize) -> String {
        let mut md = String::new();
        md.push_str("# Perf Probe Document\n\n");
        for i in 0..sections {
            md.push_str(&format!("## Section {i}\n\n"));
            md.push_str(
                "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod \
                 tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim \
                 veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea \
                 commodo consequat. Duis aute irure dolor in reprehenderit in voluptate.\n\n",
            );
            for j in 0..6 {
                md.push_str(&format!(
                    "- item {i}.{j} with some longer wrapping text to fill the line width out\n"
                ));
            }
            md.push_str("\n```rust\n");
            for j in 0..8 {
                md.push_str(&format!(
                    "fn step_{j}(x: usize) -> usize {{ x.wrapping_mul({i}).wrapping_add({j}) }}\n"
                ));
            }
            md.push_str("```\n\n");
            md.push_str("| col a | col b | col c | col d |\n|---|---|---|---|\n");
            for j in 0..3 {
                md.push_str(&format!("| {i}{j} | cell | cell | cell |\n"));
            }
            md.push('\n');
        }
        md
    }

    fn timed_step(harness: &mut Harness<'_, ()>) -> f64 {
        let t = std::time::Instant::now();
        harness.run();
        t.elapsed().as_secs_f64() * 1000.0
    }

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v[v.len() / 2]
    }

    fn stats(label: &str, v: &[f64]) {
        println!(
            "{label}: median {:.1} ms, max {:.1} ms, first {:.1} ms",
            median(v.to_vec()),
            v.iter().cloned().fold(0.0, f64::max),
            v[0]
        );
    }

    #[test]
    #[ignore = "perf probe: cargo test --release perf_probe_mode_switch -- --ignored --nocapture"]
    fn perf_probe_mode_switch_timing() {
        let doc = temp_path("perf.md");
        let _cwd = CWD_LOCK.lock().unwrap();

        let md = large_markdown(40);
        std::fs::write(&doc, &md).unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });

        for _ in 0..3 {
            timed_step(&mut harness); // warmup fonts, caches, allocator
        }
        let rendered: Vec<f64> = (0..10).map(|_| timed_step(&mut harness)).collect();

        // Differential: is the switch spike one-time warm-up (H2) or paid on
        // every toggle because the new wrap width re-shapes everything (H1)?
        let mut toggles: Vec<f64> = Vec::new();
        for phase in 0..6 {
            let next = if phase % 2 == 0 {
                ViewMode::Split
            } else {
                ViewMode::Rendered
            };
            app.borrow_mut().mode = next;
            toggles.push(timed_step(&mut harness));
            timed_step(&mut harness); // let caches settle at this width
        }
        println!("toggle first-frames: {:?}", toggles);

        app.borrow_mut().mode = ViewMode::Rendered;
        for _ in 0..5 {
            timed_step(&mut harness);
        }
        let back: Vec<f64> = (0..5).map(|_| timed_step(&mut harness)).collect();

        stats("rendered      ", &rendered);
        stats("back rendered ", &back);
        println!(
            "toggle first-frames median {:.1} ms, max {:.1} ms",
            median(toggles.clone()),
            toggles.iter().cloned().fold(0.0, f64::max)
        );

        // "Snappy" budget: every toggle frame must stay under ~2 refreshes
        // steady-state and ~6 for the switch frame itself.
        assert!(
            toggles.iter().cloned().fold(0.0, f64::max) < 100.0,
            "switch frames peaked at {:.1} ms (budget 100 ms)",
            toggles.iter().cloned().fold(0.0, f64::max)
        );
        assert!(
            median(back.clone()) < 33.0,
            "steady rendered frame median {:.1} ms (budget 33 ms)",
            median(back)
        );
        std::fs::remove_file(&doc).unwrap();
    }

    // Perf probe: one ctrl+wheel notch must cost one rebuild (one zoom
    // frame), not a smoothed crawl of intermediate zoom levels.
    #[test]
    #[ignore = "perf probe: cargo test --release perf_probe_zoom_notch -- --ignored --nocapture"]
    fn perf_probe_zoom_notch_timing() {
        let doc = temp_path("zoom-perf.md");
        let _cwd = CWD_LOCK.lock().unwrap();
        std::fs::write(&doc, large_markdown(400)).unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(5);

        // One real ctrl+wheel notch: count frames with a zoom change.
        harness.event(notch(1.0));
        let mut zoom_frames = 0;
        let mut prev = app.borrow().zoom;
        let t_notch = std::time::Instant::now();
        for _ in 0..60 {
            harness.run_steps(1);
            let z = app.borrow().zoom;
            if (z - prev).abs() > f32::EPSILON {
                zoom_frames += 1;
                prev = z;
            }
        }
        let t_notch = t_notch.elapsed();

        // Steady-state baseline: same frame count, no events.
        let t_base = std::time::Instant::now();
        harness.run_steps(60);
        let t_base = t_base.elapsed();

        println!("zoom_frames={zoom_frames} notch60={t_notch:?} baseline60={t_base:?}");
        assert_eq!(zoom_frames, 1, "a notch must zoom in a single frame");
        std::fs::remove_file(&doc).unwrap();
    }

    fn toc_app(
        name: &str,
    ) -> (
        std::path::PathBuf,
        Rc<RefCell<App>>,
        Harness<'_>,
        std::sync::MutexGuard<'static, ()>,
    ) {
        let doc = temp_path(name);
        let cwd = CWD_LOCK.lock().unwrap();
        std::fs::write(
            &doc,
            "# Top\n\ntop text\n\n## Sub\n\nsub text\n\n### Deep\n\ndeep text\n",
        )
        .unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        (doc, app, harness, cwd)
    }

    /// Nodes labeled `label`: 1 from the rendered heading, +1 per ToC
    /// entry (same text) while the sidebar is open.
    fn toc_entry_count(harness: &Harness, label: &str) -> usize {
        harness.query_all_by_label(label).count() - 1
    }

    #[test]
    fn ctrl_t_toggles_the_toc_sidebar() {
        let (doc, _app, mut harness, _cwd) = toc_app("tockey.md");
        harness.run_steps(2);
        assert_eq!(
            toc_entry_count(&harness, "Sub"),
            0,
            "the ToC must start closed"
        );
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::T);
        harness.run_steps(2);
        assert_eq!(toc_entry_count(&harness, "Top"), 1);
        assert_eq!(toc_entry_count(&harness, "Sub"), 1);
        assert_eq!(
            toc_entry_count(&harness, "Deep"),
            1,
            "every heading is listed"
        );
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::T);
        harness.run_steps(2);
        assert_eq!(
            toc_entry_count(&harness, "Sub"),
            0,
            "Ctrl+T again must close the ToC"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn toc_toolbar_button_toggles_the_sidebar() {
        let (doc, _app, mut harness, _cwd) = toc_app("tocbtn.md");
        harness.run_steps(2);
        assert_eq!(toc_entry_count(&harness, "Sub"), 0, "start closed");
        harness.get_by_label("Table of contents").click();
        harness.run_steps(2);
        assert_eq!(toc_entry_count(&harness, "Top"), 1, "button opens the ToC");
        harness.get_by_label("Table of contents").click();
        harness.run_steps(2);
        assert_eq!(toc_entry_count(&harness, "Top"), 0, "button closes the ToC");
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn toc_shortcut_is_a_noop_without_a_document() {
        let app = Rc::new(RefCell::new(App::new(None)));
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(800.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::T);
        harness.run_steps(2);
        assert!(
            !app.borrow().toc_open,
            "no document: the shortcut must not open the ToC"
        );
    }

    #[test]
    fn toc_chevron_collapses_a_group() {
        let (doc, _app, mut harness, _cwd) = toc_app("tocchev.md");
        harness.run_steps(2);
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::T);
        harness.run_steps(2);
        assert_eq!(toc_entry_count(&harness, "Sub"), 1);
        assert_eq!(toc_entry_count(&harness, "Deep"), 1);
        harness.get_by_label("Collapse Top").click();
        harness.run_steps(2);
        assert_eq!(
            toc_entry_count(&harness, "Sub"),
            0,
            "collapsing Top must hide Sub"
        );
        assert_eq!(
            toc_entry_count(&harness, "Deep"),
            0,
            "collapsing Top must hide nested Deep"
        );
        assert_eq!(
            toc_entry_count(&harness, "Top"),
            1,
            "Top itself stays visible"
        );
        harness.get_by_label("Expand Top").click();
        harness.run_steps(2);
        assert_eq!(
            toc_entry_count(&harness, "Sub"),
            1,
            "expanding restores Sub"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    /// The ToC sidebar sits on the left, so among all widgets sharing an
    /// entry's label (entry button + rendered heading) the leftmost is the
    /// ToC entry.
    fn toc_entry<'h>(harness: &'h Harness, title: &'h str) -> egui_kittest::Node<'h> {
        harness
            .query_all_by_label(title)
            .min_by(|a, b| a.rect().left().total_cmp(&b.rect().left()))
            .unwrap_or_else(|| panic!("no ToC entry labeled {title}"))
    }

    #[test]
    fn toc_click_jumps_the_rendered_view() {
        let doc = temp_path("tocjump.md");
        let _cwd = CWD_LOCK.lock().unwrap();
        // Ten ToC rows fit the sidebar without scrolling, while eight
        // paragraphs per section push the last section past the viewer's
        // 3x-viewport paint margin (where culling kicks in).
        let mut md = String::new();
        for i in 0..10 {
            md.push_str(&format!("# Section {i}\n\n"));
            for p in ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] {
                md.push_str(&format!("paragraph {i} {p}\n\n"));
            }
        }
        std::fs::write(&doc, &md).unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().toc_open = true;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        assert!(
            harness.query_by_label("paragraph 9 h").is_none(),
            "precondition: the last section must start out of view"
        );
        toc_entry(&harness, "Section 9").click();
        let mut jumped = false;
        for _ in 0..60 {
            harness.run();
            if harness.query_by_label("paragraph 9 h").is_some() {
                jumped = true;
                break;
            }
        }
        assert!(
            jumped,
            "clicking a ToC entry must scroll the rendered view to it"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    /// Screen-space top of the *rendered* heading (rightmost node with
    /// that label; a ToC entry button would be leftmost).
    fn rendered_heading_top(harness: &Harness, title: &str) -> f32 {
        harness
            .query_all_by_label(title)
            .max_by(|a, b| a.rect().right().total_cmp(&b.rect().right()))
            .unwrap_or_else(|| panic!("no rendered heading labeled {title}"))
            .rect()
            .top()
    }

    /// Run frames until the heading stops moving (scroll animation and
    /// layout materialization settle).
    fn settle_heading(harness: &mut Harness, title: &str) -> f32 {
        let mut prev = f32::NAN;
        for _ in 0..90 {
            harness.run();
            let top = rendered_heading_top(harness, title);
            if (top - prev).abs() < 0.5 {
                return top;
            }
            prev = top;
        }
        panic!("heading {title} never settled (top={prev})");
    }

    #[test]
    fn toc_jumps_land_the_heading_at_a_deterministic_position() {
        let doc = temp_path("tocland.md");
        let _cwd = CWD_LOCK.lock().unwrap();
        let mut md = String::new();
        for i in 0..10 {
            md.push_str(&format!("# Section {i}\n\n"));
            for p in ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h'] {
                md.push_str(&format!("paragraph {i} {p}\n\n"));
            }
        }
        // Content below the last target section so the scroll is not
        // clamped by the end of the document.
        md.push_str("# Appendix\n\n");
        for i in 0..14 {
            md.push_str(&format!("appendix note {i}\n\n"));
        }
        std::fs::write(&doc, &md).unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().toc_open = true;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);

        // Jump down to Section 9 from a fresh (top) scroll.
        toc_entry(&harness, "Section 9").click();
        let top_far = settle_heading(&mut harness, "Section 9");

        // Jump up to Section 2, then back down to Section 9: the landing
        // must not depend on the direction of approach.
        toc_entry(&harness, "Section 2").click();
        let _ = settle_heading(&mut harness, "Section 2");
        toc_entry(&harness, "Section 9").click();
        let top_again = settle_heading(&mut harness, "Section 9");

        assert!(
            (top_far - top_again).abs() < 1.0,
            "Section 9 must land at the same position from any starting point: \
             {top_far} vs {top_again}"
        );
        assert!(
            top_far < 120.0,
            "the jumped-to heading must sit at the top of the view, got {top_far}"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn toc_click_moves_the_source_caret() {
        let doc = temp_path("toccaret.md");
        let _cwd = CWD_LOCK.lock().unwrap();
        std::fs::write(&doc, "# Top\n\nbody\n\n## Target\n\ntail\n").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().open_path(&doc);
        app.borrow_mut().mode = ViewMode::Source;
        app.borrow_mut().toc_open = true;
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run_steps(2);
        toc_entry(&harness, "Target").click();
        harness.run();
        // The jump parks the caret on the heading's line start and focuses
        // the editor; typed text must land exactly there.
        harness.get_by_label("Source").type_text("X");
        harness.run();
        let raw = app.borrow().doc.as_ref().unwrap().raw.clone();
        assert_eq!(
            raw, "# Top\n\nbody\n\nX## Target\n\ntail\n",
            "the caret must sit at the Target heading line"
        );
        std::fs::remove_file(&doc).unwrap();
    }

    #[test]
    fn toc_toggle_persists_to_the_prefs_file() {
        let doc = temp_path("tocprefs.md");
        let prefs = temp_path("tocprefs.txt");
        let _cwd = CWD_LOCK.lock().unwrap();
        std::fs::write(&doc, "# Top\n").unwrap();
        let app = Rc::new(RefCell::new(App::new(None)));
        app.borrow_mut().set_prefs_file(Some(prefs.clone()));
        app.borrow_mut().open_path(&doc);
        let app_for_ui = app.clone();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1000.0, 600.0))
            .build_ui(move |ui| {
                app_for_ui.borrow_mut().show(ui);
            });
        harness.run();
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::T);
        harness.run();
        assert!(
            std::fs::read_to_string(&prefs)
                .unwrap()
                .contains("toc=open"),
            "opening the ToC must persist to the prefs file"
        );
        harness.key_press_modifiers(egui::Modifiers::CTRL, egui::Key::T);
        harness.run();
        assert!(
            std::fs::read_to_string(&prefs)
                .unwrap()
                .contains("toc=closed"),
            "closing the ToC must persist to the prefs file"
        );
        std::fs::remove_file(&doc).unwrap();
        std::fs::remove_file(&prefs).unwrap();
    }
}
