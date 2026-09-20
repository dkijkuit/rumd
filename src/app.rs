use std::path::{Path, PathBuf};
use std::time::Instant;
use eframe::egui;

use crate::document::{Document, FileWatcher, RELOAD_DEBOUNCE, debounce_ready};
use crate::search;
use crate::source;
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
}

/// Map a modifier+key combination to a global action.
pub fn shortcut_action(mods: egui::Modifiers, key: egui::Key) -> Option<Action> {
    let primary = mods.ctrl || mods.command;
    match (primary, key) {
        (true, egui::Key::O) => Some(Action::Open),
        (true, egui::Key::E) => Some(Action::ToggleMode),
        (true, egui::Key::D) => Some(Action::ToggleTheme),
        (true, egui::Key::F) => Some(Action::Search),
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
const EMPTY_HINT: &str = "Drop a .md file here or press Ctrl+O";
const LOSSY_BADGE: &str = "not valid UTF-8";
const SEARCH_FIELD: &str = "rumd_search_field";

const KEY_THEME: &str = "theme";
const KEY_ZOOM: &str = "zoom";
const KEY_MODE: &str = "mode";
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

pub struct App {
    pub doc: Option<Document>,
    pub mode: ViewMode,
    pub theme_pref: egui::ThemePreference,
    pub error: Option<String>,
    pub watcher: Option<FileWatcher>,
    pub watcher_failed: bool,
    pub changed_at: Option<Instant>,
    pub search: search::SearchState,
    /// Mirrored from `ctx.zoom_factor()` every frame (egui's native zoom).
    pub zoom: f32,
    pending_source_match: Option<std::ops::Range<usize>>,
    pub sections: Vec<std::ops::Range<usize>>,
    pending_render_jump: Option<usize>,
    /// Where prefs are persisted; `None` (tests) disables writing.
    prefs_file: Option<PathBuf>,
    saved_theme: Option<egui::ThemePreference>,
    saved_mode: Option<ViewMode>,
    saved_zoom: f32,
    rendered: RenderedView,
    open_requested: bool,
    applied_theme: Option<egui::ThemePreference>,
    last_title: Option<String>,
}

impl App {
    pub fn new(initial: Option<PathBuf>) -> Self {
        let mut app = App {
            doc: None,
            mode: ViewMode::Rendered,
            theme_pref: egui::ThemePreference::System,
            error: None,
            watcher: None,
            watcher_failed: false,
            changed_at: None,
            search: search::SearchState::default(),
            zoom: 1.0,
            pending_source_match: None,
            sections: Vec::new(),
            pending_render_jump: None,
            prefs_file: None,
            saved_theme: None,
            saved_mode: None,
            saved_zoom: 1.0,
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
    fn poll_watcher(&mut self, ctx: &egui::Context) {
        let mut changed = false;
        if let Some(w) = &self.watcher {
            while let Ok(()) = w.events.try_recv() {
                changed = true;
            }
        }
        if changed {
            self.changed_at = Some(Instant::now());
        }
        if let Some(t) = self.changed_at {
            if debounce_ready(Some(t), Instant::now(), RELOAD_DEBOUNCE) {
                self.changed_at = None;
                if let Some(path) = self.doc.as_ref().map(|d| d.path.clone()) {
                    self.open_path(&path);
                }
            } else {
                ctx.request_repaint_after(RELOAD_DEBOUNCE - t.elapsed());
            }
        }
    }

    /// Recompute search matches against the current document and clamp
    /// the current index (used on query edits, open, and reloads).
    pub fn recompute_matches(&mut self) {
        let text = self.doc.as_ref().map(|d| d.raw.as_str()).unwrap_or("");
        self.search.matches = search::find_matches(text, &self.search.query);
        if self.search.current >= self.search.matches.len() {
            self.search.current = self.search.matches.len().saturating_sub(1);
        }
    }

    fn queue_jumps(&mut self) {
        self.pending_source_match = self.search.current_match().cloned();
        self.pending_render_jump = self
            .search
            .current_match()
            .and_then(|m| search::section_containing(&self.sections, m.start));
    }

    /// Opt in to preference persistence (main only; tests leave `None`).
    pub fn set_prefs_file(&mut self, path: Option<PathBuf>) {
        self.prefs_file = path;
    }

    /// Serialize the current preferences as `key=value` lines.
    pub fn prefs_to_string(&self) -> String {
        format!(
            "{}={}\n{}={}\n{}={}\n",
            KEY_THEME,
            encode_theme(self.theme_pref),
            KEY_ZOOM,
            self.zoom,
            KEY_MODE,
            encode_mode(self.mode)
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
        }
    }

    /// Write preferences when they changed since the last write. Called
    /// every frame; zoom mirroring makes zoom changes visible here.
    fn persist_prefs_if_changed(&mut self) {
        let changed = self.saved_theme != Some(self.theme_pref)
            || self.saved_mode != Some(self.mode)
            || (self.saved_zoom - self.zoom).abs() > f32::EPSILON;
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
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.handle_events(&ctx);
        self.poll_watcher(&ctx);
        self.apply_theme(&ctx);
        Self::apply_typography(&ctx);
        self.zoom = ctx.zoom_factor();
        self.persist_prefs_if_changed();
        self.show_error_banner(ui);
        self.show_top_bar(ui);
        self.show_search_bar(ui);
        self.update_window_title(&ctx);

        egui::CentralPanel::default().show(ui, |ui| match &self.doc {
            None => self.show_empty_state(ui),
            Some(doc) => match self.mode {
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
            },
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
                self.search.open = false;
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
            if let Some(path) = rfd::FileDialog::new()
                .add_filter(OPEN_FILTER_NAME, OPEN_FILTER_EXTS)
                .pick_file()
            {
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
        match &self.doc {
            Some(doc) => {
                let path = doc.path.clone();
                self.open_path(&path);
            }
            None => self.error = None,
        }
    }

    fn show_error_banner(&mut self, ui: &mut egui::Ui) {
        if let Some(message) = self.error.clone() {
            egui::Panel::top("error_banner").show(ui, |ui| {
                egui::Frame::default()
                    .fill(ui.visuals().warn_fg_color.gamma_multiply(0.15))
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.colored_label(ui.visuals().warn_fg_color, message);
                    });
            });
        }
    }

    fn show_top_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut action: Option<Action> = None;
        let mut mode_clicked = false;
        egui::Panel::top("top_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open").clicked() {
                    action = Some(Action::Open);
                }
                if let Some(doc) = &self.doc {
                    ui.strong(doc.file_name());
                    if doc.lossy {
                        ui.label(
                            egui::RichText::new(LOSSY_BADGE)
                                .small()
                                .color(ui.visuals().warn_fg_color),
                        );
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Refresh").clicked() {
                        action = Some(Action::Refresh);
                    }
                    let theme_label = match ctx.theme() {
                        egui::Theme::Dark => "Light theme",
                        egui::Theme::Light => "Dark theme",
                    };
                    if ui.button(theme_label).clicked() {
                        action = Some(Action::ToggleTheme);
                    }
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        // Right-to-left layout: add in reverse to read
                        // Rendered | Split | Source left-to-right. Clicking
                        // a segment selects that mode directly (Ctrl+E cycles).
                        if ui
                            .selectable_label(self.mode == ViewMode::Source, "Source")
                            .clicked()
                            && self.mode != ViewMode::Source
                        {
                            self.mode = ViewMode::Source;
                            mode_clicked = true;
                        }
                        if ui
                            .selectable_label(self.mode == ViewMode::Split, "Split")
                            .clicked()
                            && self.mode != ViewMode::Split
                        {
                            self.mode = ViewMode::Split;
                            mode_clicked = true;
                        }
                        if ui
                            .selectable_label(self.mode == ViewMode::Rendered, "Rendered")
                            .clicked()
                            && self.mode != ViewMode::Rendered
                        {
                            self.mode = ViewMode::Rendered;
                            mode_clicked = true;
                        }
                    });
                });
            });
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
            Some(Action::Search) => {
                self.search.open = true;
                self.recompute_matches();
                ctx.memory_mut(|m| m.request_focus(egui::Id::new(SEARCH_FIELD)));
            }
            None => {}
        }
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
                    self.search.open = false;
                }
            });
        });
    }

    fn show_empty_state(&self, ui: &mut egui::Ui) {
        ui.centered_and_justified(|ui| {
            ui.label(EMPTY_HINT);
        });
    }

    fn update_window_title(&mut self, ctx: &egui::Context) {
        let title = match &self.doc {
            Some(doc) => format!("rumd - {}", doc.file_name()),
            None => "rumd".to_string(),
        };
        if self.last_title.as_deref() != Some(title.as_str()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = Some(title);
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
    fn theme_toggle_flips_button_and_keeps_view() {
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

        // Which label the button shows first depends on the effective theme
        // in the headless harness; clicking must flip it and keep the view.
        let (first, second) = if harness.query_by_label("Light theme").is_some() {
            ("Light theme", "Dark theme")
        } else {
            ("Dark theme", "Light theme")
        };
        harness.get_by_label(first).click();
        harness.run();
        harness.get_by_label("Themed Doc");
        harness.get_by_label(second).click();
        harness.run();
        harness.get_by_label("Themed Doc");
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
        assert!(
            harness.query_by_label("0/0").is_none(),
            "bar starts closed"
        );
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
        let s = app.prefs_to_string();
        assert!(s.contains("mode=source"));
        assert!(s.contains("theme=dark"));
        assert!(s.contains("zoom=1.3"));

        let mut other = App::new(None);
        other.apply_prefs_string(&s);
        assert_eq!(other.mode, ViewMode::Source);
        assert_eq!(other.theme_pref, egui::ThemePreference::Dark);
        assert!((other.zoom - 1.3).abs() < 1e-6);

        // Garbage and missing values fall back without panicking.
        other.apply_prefs_string("theme=bogus\nzoom=zzz\nmode=huh\n");
        assert_eq!(other.theme_pref, egui::ThemePreference::System);
        assert_eq!(other.mode, ViewMode::Rendered);
        assert!((other.zoom - 1.0).abs() < 1e-6);
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
}
