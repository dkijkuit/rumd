use crate::alerts::AlertBundle;
use egui::{RichText, TextBuffer, TextStyle, Ui, text::LayoutJob};
use std::collections::HashMap;

use crate::pulldown::ScrollableCache;

#[cfg(feature = "better_syntax_highlighting")]
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::{SyntaxDefinition, SyntaxSet},
    util::LinesWithEndings,
};

#[cfg(feature = "better_syntax_highlighting")]
const DEFAULT_THEME_LIGHT: &str = "base16-ocean.light";
#[cfg(feature = "better_syntax_highlighting")]
const DEFAULT_THEME_DARK: &str = "base16-ocean.dark";

pub struct CommonMarkOptions<'f> {
    pub indentation_spaces: usize,
    pub max_image_width: Option<usize>,
    pub show_alt_text_on_hover: bool,
    pub default_width: Option<usize>,
    #[cfg(feature = "better_syntax_highlighting")]
    pub theme_light: String,
    #[cfg(feature = "better_syntax_highlighting")]
    pub theme_dark: String,
    pub use_explicit_uri_scheme: bool,
    pub default_implicit_uri_scheme: String,
    pub alerts: AlertBundle,
    /// Whether to present a mutable ui for things like checkboxes
    pub mutable: bool,
    pub math_fn: Option<&'f crate::RenderMathFn>,
    pub html_fn: Option<&'f crate::RenderHtmlFn>,
    /// Whether to enable scrolling to headings by their ID.
    /// To give a heading an ID, use the syntax `# Heading {#myheadingid}`. Then links to `#myheadingid` e.g. `[click me!](#myheadingid)` will scroll to that heading.
    pub enable_scroll_to_heading: bool,
}

impl std::fmt::Debug for CommonMarkOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("CommonMarkOptions");

        s.field("indentation_spaces", &self.indentation_spaces)
            .field("max_image_width", &self.max_image_width)
            .field("show_alt_text_on_hover", &self.show_alt_text_on_hover)
            .field("default_width", &self.default_width);

        #[cfg(feature = "better_syntax_highlighting")]
        s.field("theme_light", &self.theme_light)
            .field("theme_dark", &self.theme_dark);

        s.field("use_explicit_uri_scheme", &self.use_explicit_uri_scheme)
            .field(
                "default_implicit_uri_scheme",
                &self.default_implicit_uri_scheme,
            )
            .field("alerts", &self.alerts)
            .field("mutable", &self.mutable)
            .finish()
    }
}

impl Default for CommonMarkOptions<'_> {
    fn default() -> Self {
        Self {
            indentation_spaces: 4,
            max_image_width: None,
            show_alt_text_on_hover: true,
            default_width: None,
            #[cfg(feature = "better_syntax_highlighting")]
            theme_light: DEFAULT_THEME_LIGHT.to_owned(),
            #[cfg(feature = "better_syntax_highlighting")]
            theme_dark: DEFAULT_THEME_DARK.to_owned(),
            use_explicit_uri_scheme: false,
            default_implicit_uri_scheme: "file://".to_owned(),
            alerts: AlertBundle::gfm(),
            mutable: false,
            math_fn: None,
            html_fn: None,
            enable_scroll_to_heading: false,
        }
    }
}

impl CommonMarkOptions<'_> {
    #[cfg(feature = "better_syntax_highlighting")]
    pub fn curr_theme(&self, ui: &Ui) -> &str {
        if ui.style().visuals.dark_mode {
            &self.theme_dark
        } else {
            &self.theme_light
        }
    }

    pub fn max_width(&self, ui: &Ui) -> f32 {
        let max_image_width = self.max_image_width.unwrap_or(0) as f32;
        let available_width = ui.available_width();

        let max_width = max_image_width.max(available_width);
        if let Some(default_width) = self.default_width {
            if default_width as f32 > max_width {
                default_width as f32
            } else {
                max_width
            }
        } else {
            max_width
        }
    }
}

#[derive(Default, Clone)]
pub struct Style {
    pub heading: Option<u8>,
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    pub quote: bool,
    pub code: bool,
    /// rumd patch: table header cells render in the bold family,
    /// like GitHub's `th`.
    pub table_header: bool,
}

impl Style {
    /// rumd patch: the line height to use for text rendered in the
    /// current state, derived from the same sizes `to_richtext` picks.
    pub fn line_height(&self, ui: &Ui) -> f32 {
        match self.heading {
            Some(level) => {
                let size = heading_size(ui, level);
                size * crate::typography::LINE_HEIGHT_HEADING
            }
            // Quotes render at body size; code matches its line's body
            // height so inline code never stretches the surrounding row.
            _ => body_size(ui) * crate::typography::LINE_HEIGHT_BODY,
        }
    }

    pub fn to_richtext(&self, ui: &Ui, text: &str) -> RichText {
        use crate::typography::{FAMILY_BOLD, FAMILY_BOLD_ITALIC, FAMILY_HEADING, FAMILY_ITALIC};

        let mut text = RichText::new(text);

        // rumd patch: Typora's heading ramp with real weights. egui has
        // no bold synthesis (`RichText::strong` only brightens color), so
        // headings and bold runs select real font families registered by
        // rumd's font setup.
        if let Some(level) = self.heading {
            let size = heading_size(ui, level);
            match level {
                // h1..=h5: semibold heading family.
                0..=4 => {
                    text = text
                        .size(size)
                        .family(egui::FontFamily::Name(FAMILY_HEADING.into()));
                }
                // h6: body weight, muted — GitHub styles it as small gray.
                _ => {
                    text = text
                        .size(size)
                        .color(crate::typography::palette(ui).muted);
                }
            }
        }

        if self.quote {
            // rumd patch: GitHub quote text color (upstream used egui's
            // weak color, which is near-invisible on some themes).
            text = text.color(crate::typography::palette(ui).muted);
        }

        if self.strong || self.table_header {
            // Real weight via family; `.strong` alone is just a color.
            let family = if self.emphasis {
                FAMILY_BOLD_ITALIC
            } else {
                FAMILY_BOLD
            };
            text = text
                .family(egui::FontFamily::Name(family.into()))
                .strong();
        }

        if self.emphasis {
            // Real italics via family instead of a synthetic shear.
            let family = if self.strong {
                FAMILY_BOLD_ITALIC
            } else {
                FAMILY_ITALIC
            };
            text = text.family(egui::FontFamily::Name(family.into()));
        }

        if self.strikethrough {
            text = text.strikethrough();
        }

        if self.code {
            // rumd patch: inline code as a GitHub-style pill — slightly
            // smaller monospace on a subtle background.
            let mono = ui
                .style()
                .text_styles
                .get(&TextStyle::Monospace)
                .map_or(14.0, |d| d.size);
            text = text
                .monospace()
                .size(mono)
                .background_color(crate::typography::palette(ui).inline_code_bg);
        }

        text
    }
}

/// rumd patch: body font size from the ui's text styles.
fn body_size(ui: &Ui) -> f32 {
    ui.style()
        .text_styles
        .get(&TextStyle::Body)
        .map_or(16.0, |d| d.size)
}

/// rumd patch: font size for a heading level (0 = h1).
fn heading_size(ui: &Ui, level: u8) -> f32 {
    let base = body_size(ui);
    // Ramp is authored against a 16px body; scale it if the body size
    // ever changes (e.g. a future density setting).
    let scale = base / 16.0;
    crate::typography::HEADING_SIZES[(level as usize).min(5)] * scale
}

#[derive(Default)]
pub struct Link {
    pub destination: String,
    pub text: Vec<RichText>,
}

impl Link {
    pub fn end(
        self,
        ui: &mut Ui,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        scroll_to_heading: &mut Option<String>,
    ) {
        let Self { destination, text } = self;

        // When a link wraps an image (`[![alt](img)](url)`), all text events are captured
        // by the image widget and link.text is never populated. Rendering an empty Label in
        // a wrapping layout resets cursor.min.x to 0, superimposing subsequent elements on
        // the image that was just drawn. Nothing to render, so return early.
        if text.is_empty() {
            return;
        }

        let mut layout_job = LayoutJob::default();
        for t in text {
            t.append_to(
                &mut layout_job,
                ui.style(),
                egui::FontSelection::Default,
                egui::Align::LEFT,
            );
        }
        // rumd patch: links get the same explicit leading as body text.
        crate::typography::apply_line_height(
            &mut layout_job,
            ui.style()
                .text_styles
                .get(&TextStyle::Body)
                .map_or(24.0, |d| d.size)
                * crate::typography::LINE_HEIGHT_BODY,
        );
        if cache.link_hooks().contains_key(&destination) {
            let ui_link = ui.link(layout_job);
            if ui_link.clicked() || ui_link.middle_clicked() {
                cache.link_hooks_mut().insert(destination, true);
            }
        } else if options.enable_scroll_to_heading
            && let Some(stripped) = destination.strip_prefix("#")
        {
            if ui.link(layout_job).clicked() {
                scroll_to_heading.replace(stripped.to_string());
            };
        } else {
            ui.hyperlink_to(layout_job, destination);
        }
    }
}

pub struct Image {
    pub uri: String,
    pub alt_text: Vec<RichText>,
}

impl Image {
    // FIXME: string conversion
    pub fn new(uri: &str, options: &CommonMarkOptions) -> Self {
        let has_scheme = uri.contains("://") || uri.starts_with("data:");
        let uri = if options.use_explicit_uri_scheme || has_scheme {
            uri.to_string()
        } else {
            // Assume file scheme
            format!("{}{uri}", options.default_implicit_uri_scheme)
        };

        Self {
            uri,
            alt_text: Vec::new(),
        }
    }

    pub fn end(self, ui: &mut Ui, options: &CommonMarkOptions) {
        let response = ui.add(
            egui::Image::from_uri(&self.uri)
                .fit_to_original_size(1.0)
                .max_width(options.max_width(ui)),
        );

        if !self.alt_text.is_empty() && options.show_alt_text_on_hover {
            response.on_hover_ui_at_pointer(|ui| {
                for alt in self.alt_text {
                    ui.label(alt);
                }
            });
        }
    }
}

pub struct CodeBlock {
    pub lang: Option<String>,
    pub content: String,
}

impl CodeBlock {
    pub fn end(
        &self,
        ui: &mut Ui,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        ui.scope(|ui| {
            Self::pre_syntax_highlighting(cache, options, ui);

            let mut layout = |ui: &Ui, string: &dyn TextBuffer, wrap_width: f32| {
                let mut job = if let Some(lang) = &self.lang {
                    self.syntax_highlighting(cache, options, lang, ui, string.as_str())
                } else {
                    plain_highlighting(ui, string.as_str())
                };

                job.wrap.max_width = wrap_width;
                // rumd patch: explicit code leading (GitHub code blocks
                // are ~1.45em).
                let mono = ui
                    .style()
                    .text_styles
                    .get(&TextStyle::Monospace)
                    .map_or(14.0, |d| d.size);
                crate::typography::apply_line_height(
                    &mut job,
                    mono * crate::typography::LINE_HEIGHT_CODE,
                );
                ui.fonts_mut(|f| f.layout_job(job))
            };

            crate::elements::code_block(ui, max_width, &self.content, &mut layout);
        });
    }
}

#[cfg(not(feature = "better_syntax_highlighting"))]
impl CodeBlock {
    fn pre_syntax_highlighting(
        _cache: &mut CommonMarkCache,
        _options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        ui.style_mut().visuals.extreme_bg_color = ui.visuals().extreme_bg_color;
    }

    fn syntax_highlighting(
        &self,
        _cache: &mut CommonMarkCache,
        _options: &CommonMarkOptions,
        extension: &str,
        ui: &Ui,
        text: &str,
    ) -> egui::text::LayoutJob {
        simple_highlighting(ui, text, extension)
    }
}

#[cfg(feature = "better_syntax_highlighting")]
impl CodeBlock {
    fn pre_syntax_highlighting(
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        let curr_theme = cache.curr_theme(ui, options);
        let style = ui.style_mut();

        style.visuals.extreme_bg_color = curr_theme
            .settings
            .background
            .map(syntect_color_to_egui)
            .unwrap_or(style.visuals.extreme_bg_color);

        if let Some(color) = curr_theme.settings.selection_foreground {
            style.visuals.selection.bg_fill = syntect_color_to_egui(color);
        }
    }

    fn syntax_highlighting(
        &self,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        extension: &str,
        ui: &Ui,
        text: &str,
    ) -> egui::text::LayoutJob {
        // rumd patch: resolve the fence language by extension first
        // ("rs", "py", "sh"), then by language name/token ("rust",
        // "python") so the common ```rust fences get real highlighting
        // instead of falling back to the plain theme.
        let syntax = cache
            .ps
            .find_syntax_by_extension(extension)
            .or_else(|| cache.ps.find_syntax_by_token(extension));
        let Some(syntax) = syntax else {
            return simple_highlighting(ui, text, extension);
        };

        // rumd patch: cache the highlighted job. Syntect parses the code
        // with regexes per line, which costs milliseconds per block —
        // doing that for every visible code block on every frame is what
        // made large, code-heavy documents sluggish. The job only changes
        // when the code, the resolved syntax, the theme, or the mono font
        // size changes, so those fully key the cache.
        let theme_name = options.curr_theme(ui).to_string();
        let mono_size = ui
            .style()
            .text_styles
            .get(&TextStyle::Monospace)
            .map_or(14.0, |d| d.size);
        let key = egui::util::hash((
            text,
            syntax.name.as_str(),
            theme_name.as_str(),
            mono_size.to_bits(),
        ));
        if let Some(job) = cache.highlight_cache.get(&key) {
            return job.clone();
        }

        let mut job = egui::text::LayoutJob::default();
        let mut h = HighlightLines::new(syntax, cache.curr_theme(ui, options));

        for line in LinesWithEndings::from(text) {
            let ranges = h.highlight_line(line, &cache.ps).unwrap();
            for v in ranges {
                let front = v.0.foreground;
                job.append(
                    v.1,
                    0.0,
                    egui::TextFormat::simple(
                        TextStyle::Monospace.resolve(ui.style()),
                        syntect_color_to_egui(front),
                    ),
                );
            }
        }

        // Bound the cache: documents are viewed one at a time, so the
        // working set is tiny; a cap just guards pathological sessions.
        if cache.highlight_cache.len() >= 256 {
            cache.highlight_cache.clear();
        }
        cache.highlight_cache.insert(key, job.clone());

        job
    }
}

fn simple_highlighting(ui: &Ui, text: &str, extension: &str) -> egui::text::LayoutJob {
    egui_extras::syntax_highlighting::highlight(
        ui.ctx(),
        ui.style(),
        &egui_extras::syntax_highlighting::CodeTheme::from_style(ui.style()),
        text,
        extension,
    )
}

fn plain_highlighting(ui: &Ui, text: &str) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat::simple(
            TextStyle::Monospace.resolve(ui.style()),
            ui.style().visuals.text_color(),
        ),
    );
    job
}

#[cfg(feature = "better_syntax_highlighting")]
fn syntect_color_to_egui(color: syntect::highlighting::Color) -> egui::Color32 {
    egui::Color32::from_rgb(color.r, color.g, color.b)
}

#[cfg(feature = "better_syntax_highlighting")]
fn default_theme(ui: &Ui) -> &str {
    if ui.style().visuals.dark_mode {
        DEFAULT_THEME_DARK
    } else {
        DEFAULT_THEME_LIGHT
    }
}

/// A cache used for storing content such as images.
#[derive(Debug)]
pub struct CommonMarkCache {
    // Everything stored in `CommonMarkCache` must take into account that
    // the cache is for multiple `CommonMarkviewer`s with different source_ids.
    #[cfg(feature = "better_syntax_highlighting")]
    ps: SyntaxSet,

    #[cfg(feature = "better_syntax_highlighting")]
    ts: ThemeSet,

    /// The ID of the heading to scroll to. This is set when a link whose destination is a fragment (e.g. `#my-heading`) has been clicked.
    scroll_to_id_target: Option<String>,
    link_hooks: HashMap<String, bool>,

    /// rumd patch: highlighted code-block layout jobs, keyed by
    /// (content, syntax name, theme name, mono size). Syntect highlighting
    /// runs per line regex parsing; caching the finished job keeps it off
    /// the per-frame path (see `CodeBlock::syntax_highlighting`). Keys
    /// fully determine the value, so sharing one cache across viewers is
    /// safe.
    #[cfg(feature = "better_syntax_highlighting")]
    highlight_cache: HashMap<u64, egui::text::LayoutJob>,

    scroll: HashMap<egui::Id, ScrollableCache>,
    pub(self) has_installed_loaders: bool,
}

#[allow(clippy::derivable_impls)]
impl Default for CommonMarkCache {
    fn default() -> Self {
        Self {
            #[cfg(feature = "better_syntax_highlighting")]
            ps: SyntaxSet::load_defaults_newlines(),
            #[cfg(feature = "better_syntax_highlighting")]
            ts: ThemeSet::load_defaults(),
            #[cfg(feature = "better_syntax_highlighting")]
            highlight_cache: HashMap::new(),
            link_hooks: HashMap::new(),
            scroll: Default::default(),
            scroll_to_id_target: None,
            has_installed_loaders: false,
        }
    }
}

impl CommonMarkCache {
    #[cfg(feature = "better_syntax_highlighting")]
    pub fn add_syntax_from_folder(&mut self, path: &str) {
        let mut builder = self.ps.clone().into_builder();
        let _ = builder.add_from_folder(path, true);
        self.ps = builder.build();
    }

    #[cfg(feature = "better_syntax_highlighting")]
    pub fn add_syntax_from_str(
        &mut self,
        s: &str,
        fallback_name: Option<&str>,
    ) -> Result<(), syntect::parsing::ParseSyntaxError> {
        let mut builder = self.ps.clone().into_builder();
        SyntaxDefinition::load_from_str(s, true, fallback_name).map(|d| builder.add(d))?;
        self.ps = builder.build();
        Ok(())
    }

    #[cfg(feature = "better_syntax_highlighting")]
    /// Add more color themes for code blocks(.tmTheme files). Set the color theme with
    /// [`syntax_theme_dark`](CommonMarkViewer::syntax_theme_dark) and
    /// [`syntax_theme_light`](CommonMarkViewer::syntax_theme_light)
    pub fn add_syntax_themes_from_folder(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<(), syntect::LoadingError> {
        self.ts.add_from_folder(path)
    }

    #[cfg(feature = "better_syntax_highlighting")]
    /// Add color theme for code blocks(.tmTheme files). Set the color theme with
    /// [`syntax_theme_dark`](CommonMarkViewer::syntax_theme_dark) and
    /// [`syntax_theme_light`](CommonMarkViewer::syntax_theme_light)
    pub fn add_syntax_theme_from_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: &[u8],
    ) -> Result<(), syntect::LoadingError> {
        let mut cursor = std::io::Cursor::new(bytes);
        self.ts
            .themes
            .insert(name.into(), ThemeSet::load_from_reader(&mut cursor)?);
        Ok(())
    }

    /// Get the desired fragment. This is the id which will be scrolled to if it is found in the markdown.
    pub fn scroll_to_id_target(&self) -> Option<&str> {
        self.scroll_to_id_target.as_deref()
    }

    /// Get mutable access to the desired fragment. Setting this will cause the viewer to scroll to the heading with this id if it exists. Setting it to None will prevent scrolling.
    pub fn scroll_to_id_target_mut(&mut self) -> &mut Option<String> {
        &mut self.scroll_to_id_target
    }

    /// Clear the cache for all scrollable elements
    pub fn clear_scrollable(&mut self) {
        self.scroll.clear();
    }

    /// Clear the cache for a specific scrollable viewer. Returns false if the
    /// id was not in the cache.
    pub fn clear_scrollable_with_id(&mut self, source_id: impl egui::AsId) -> bool {
        self.scroll.remove(&egui::Id::new(source_id)).is_some()
    }

    /// If the user clicks on a link in the markdown render that has `name` as a link. The hook
    /// specified with this method will be set to true. It's status can be acquired
    /// with [`get_link_hook`](Self::get_link_hook). Be aware that all hook state is reset once
    /// [`CommonMarkViewer::show`] gets called
    ///
    /// # Why use link hooks
    ///
    /// egui provides a method for checking links afterwards so why use this instead?
    ///
    /// ```rust
    /// # use egui::__run_test_ctx;
    /// # __run_test_ctx(|ctx| {
    /// ctx.output_mut(|o| for command in &o.commands {
    ///     matches!(command, egui::OutputCommand::OpenUrl(_));
    /// });
    /// # });
    /// ```
    ///
    /// The main difference is that link hooks allows egui_commonmark to check for link hooks
    /// while rendering. Normally when hovering over a link, egui_commonmark will display the full
    /// url. With link hooks this feature is disabled, but to do that all hooks must be known.
    // Works when displayed through egui_commonmark
    #[allow(rustdoc::broken_intra_doc_links)]
    pub fn add_link_hook<S: Into<String>>(&mut self, name: S) {
        self.link_hooks.insert(name.into(), false);
    }

    /// Returns None if the link hook could not be found. Returns the last known status of the
    /// hook otherwise.
    pub fn remove_link_hook(&mut self, name: &str) -> Option<bool> {
        self.link_hooks.remove(name)
    }

    /// Get status of link. Returns true if it was clicked
    pub fn get_link_hook(&self, name: &str) -> Option<bool> {
        self.link_hooks.get(name).copied()
    }

    /// Remove all link hooks
    pub fn link_hooks_clear(&mut self) {
        self.link_hooks.clear();
    }

    /// All link hooks
    pub fn link_hooks(&self) -> &HashMap<String, bool> {
        &self.link_hooks
    }

    /// Raw access to link hooks
    pub fn link_hooks_mut(&mut self) -> &mut HashMap<String, bool> {
        &mut self.link_hooks
    }

    /// Set all link hooks to false
    fn deactivate_link_hooks(&mut self) {
        for v in self.link_hooks.values_mut() {
            *v = false;
        }
    }

    #[cfg(feature = "better_syntax_highlighting")]
    fn curr_theme(&self, ui: &Ui, options: &CommonMarkOptions) -> &Theme {
        self.ts
            .themes
            .get(options.curr_theme(ui))
            // Since we have called load_defaults, the default theme *should* always be available..
            .unwrap_or_else(|| &self.ts.themes[default_theme(ui)])
    }
}

pub fn scroll_cache<'a>(cache: &'a mut CommonMarkCache, id: &egui::Id) -> &'a mut ScrollableCache {
    if !cache.scroll.contains_key(id) {
        cache.scroll.insert(*id, Default::default());
    }
    cache.scroll.get_mut(id).unwrap()
}

/// Should be called before any rendering
pub fn prepare_show(cache: &mut CommonMarkCache, ctx: &egui::Context) {
    if !cache.has_installed_loaders {
        // Even though the install function can be called multiple times, its not the cheapest
        // so we ensure that we only call it once.
        // This could be done at the creation of the cache, however it is better to keep the
        // cache free from egui's Ui and Context types as this allows it to be created before
        // any egui instances. It also keeps the API similar to before the introduction of the
        // image loaders.
        #[cfg(feature = "embedded_image")]
        crate::data_url_loader::install_loader(ctx);

        egui_extras::install_image_loaders(ctx);
        cache.has_installed_loaders = true;
    }

    cache.deactivate_link_hooks();
}
