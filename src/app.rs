use std::path::{Path, PathBuf};

use eframe::egui;

use crate::document::Document;
use crate::source;
use crate::viewer::RenderedView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Rendered,
    Source,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    ToggleMode,
    ToggleTheme,
    Refresh,
}

/// Map a modifier+key combination to a global action.
pub fn shortcut_action(mods: egui::Modifiers, key: egui::Key) -> Option<Action> {
    let primary = mods.ctrl || mods.command;
    match (primary, key) {
        (true, egui::Key::O) => Some(Action::Open),
        (true, egui::Key::E) => Some(Action::ToggleMode),
        (true, egui::Key::D) => Some(Action::ToggleTheme),
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

const OPEN_FILTER_NAME: &str = "Markdown";
const OPEN_FILTER_EXTS: &[&str] = &["md", "markdown", "mdown", "txt"];
const EMPTY_HINT: &str = "Drop a .md file here or press Ctrl+O";
const LOSSY_BADGE: &str = "not valid UTF-8";

pub struct App {
    pub doc: Option<Document>,
    pub mode: ViewMode,
    pub theme_pref: egui::ThemePreference,
    pub error: Option<String>,
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
        match Document::load(path) {
            Ok(doc) => {
                // egui resolves relative image paths against the process
                // working directory, so follow the document.
                if let Some(dir) = doc.dir() {
                    let _ = std::env::set_current_dir(dir);
                }
                self.doc = Some(doc);
                self.error = None;
            }
            Err(e) => {
                self.error = Some(format!("Failed to open {}: {}", path.display(), e));
            }
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.handle_events(&ctx);
        self.apply_theme(&ctx);
        self.show_error_banner(ui);
        self.show_top_bar(ui);
        self.update_window_title(&ctx);

        egui::CentralPanel::default().show(ui, |ui| match &self.doc {
            None => self.show_empty_state(ui),
            Some(doc) => match self.mode {
                ViewMode::Rendered => self.rendered.show(ui, &doc.raw),
                ViewMode::Source => {
                    let theme = source::code_theme(self.theme_pref, &ctx);
                    source::show(ui, doc, theme);
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
                        ViewMode::Rendered => ViewMode::Source,
                        ViewMode::Source => ViewMode::Rendered,
                    };
                    self.error = None;
                }
                Some(Action::ToggleTheme) => {
                    self.toggle_theme(ctx);
                    self.error = None;
                }
                Some(Action::Refresh) => self.refresh(),
                None => {}
            }
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

    fn refresh(&mut self) {
        if let Some(doc) = &self.doc {
            let path = doc.path.clone();
            self.open_path(&path);
        }
        self.error = None;
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
                    if ui
                        .selectable_label(self.mode == ViewMode::Rendered, "Rendered")
                        .clicked()
                    {
                        if self.mode != ViewMode::Rendered {
                            action = Some(Action::ToggleMode);
                        }
                    }
                    if ui
                        .selectable_label(self.mode == ViewMode::Source, "Source")
                        .clicked()
                    {
                        if self.mode != ViewMode::Source {
                            action = Some(Action::ToggleMode);
                        }
                    }
                });
            });
        });
        if action.is_some() {
            self.error = None;
        }
        match action {
            Some(Action::Open) => self.open_requested = true,
            Some(Action::ToggleMode) => {
                self.mode = match self.mode {
                    ViewMode::Rendered => ViewMode::Source,
                    ViewMode::Source => ViewMode::Rendered,
                };
            }
            Some(Action::ToggleTheme) => self.toggle_theme(&ctx),
            Some(Action::Refresh) => self.refresh(),
            None => {}
        }
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

    use egui_kittest::{Harness, kittest::Queryable};

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("rumd_app_{}_{}", std::process::id(), name))
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
        assert_eq!(shortcut_action(ctrl, egui::Key::E), Some(Action::ToggleMode));
        assert_eq!(shortcut_action(cmd, egui::Key::E), Some(Action::ToggleMode));
        assert_eq!(shortcut_action(ctrl, egui::Key::D), Some(Action::ToggleTheme));
        assert_eq!(shortcut_action(cmd, egui::Key::D), Some(Action::ToggleTheme));
        assert_eq!(
            shortcut_action(egui::Modifiers::NONE, egui::Key::F5),
            Some(Action::Refresh)
        );
        assert_eq!(shortcut_action(cmd, egui::Key::F5), None);
        assert_eq!(shortcut_action(egui::Modifiers::NONE, egui::Key::O), None);
        assert_eq!(shortcut_action(ctrl, egui::Key::X), None);
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
}
