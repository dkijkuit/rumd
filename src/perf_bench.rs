//! Frame-time benchmarks for large documents. Not part of the normal test
//! suite; run explicitly with:
//!
//!     cargo test --release perf_bench -- --ignored --nocapture
//!
//! The frame timings go through the real [`crate::app::App`] path in a
//! headless wgpu harness, so they include egui layout, shaping, and the
//! paint/tessellation work a real frame does. The micro benches at the
//! bottom attribute the cost: markdown parsing and syntect highlighting
//! are measured in isolation, and whatever the frame time exceeds them by
//! is egui widget/layout/shaping work.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use eframe::egui;
use egui_kittest::{Harness, kittest::Queryable};

use crate::app::{App, ViewMode};

/// One realistic section: heading, prose paragraph, list, table, code block.
/// The fence uses `rs` so the syntect path (extension lookup) is exercised,
/// like real-world ```sh / ```c / ```js fences.
const CODE_BLOCK: &str = "```rs\nfn main() {\n    let values: Vec<u32> = (0..32).collect();\n    let doubled: Vec<u32> = values.iter().map(|v| v * 2).collect();\n    for (index, value) in doubled.iter().enumerate() {\n        println!(\"item {index} is {value}\");\n    }\n}\n\nstruct Point {\n    x: f32,\n    y: f32,\n}\n\nimpl Point {\n    fn distance(&self, other: &Point) -> f32 {\n        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()\n    }\n}\n```\n\n";

fn section(i: usize) -> String {
    format!(
        "# Section {i}: performance of larger documents\n\n\
        This paragraph exists to give the section realistic body weight. It wraps \
        across the reading column and measures the shaping cost of ordinary prose \
        in the rendered view, with a [link](https://example.org), `inline code`, \
        **bold** and *italic* runs mixed into the text.\n\n\
        - first list item with some text\n\
        - second list item with `code` and **bold**\n\
        - [ ] a task list item\n\
        - [x] a done task\n\n\
        | Column A | Column B | Column C |\n\
        |----------|----------|----------|\n\
        | alpha    | 12       | extra    |\n\
        | beta     | 34       | cells    |\n\
        | gamma    | 56       | here     |\n\n\
        {CODE_BLOCK}"
    )
}

fn big_doc(sections: usize) -> String {
    let mut md = String::new();
    for i in 0..sections {
        md.push_str(&section(i));
    }
    md
}

/// Section count override: `RUMD_BENCH_SECTIONS=240 cargo test ...` scales
/// the document (240 sections ≈ 250KB).
fn bench_sections() -> usize {
    std::env::var("RUMD_BENCH_SECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60)
}

/// Serialize bench tests that call `open_path` (it changes the process CWD).
static CWD_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn temp_doc(name: &str, markdown: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rumd_perf_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, markdown).unwrap();
    path
}

fn app_harness(
    doc: &std::path::Path,
    size: egui::Vec2,
) -> (Rc<RefCell<App>>, Harness<'static, ()>) {
    let app = Rc::new(RefCell::new(App::new(None)));
    app.borrow_mut().open_path(doc);
    let app_for_ui = app.clone();
    let harness = Harness::builder()
        .with_size(size)
        .build_ui(move |ui| {
            app_for_ui.borrow_mut().show(ui);
        });
    (app, harness)
}

fn report_frame(label: &str, frame: usize, ms: f64) {
    println!("  {label} frame {frame}: {ms:.2} ms");
}

/// Frame times for a ~60KB document: cold open (the first frame lays out
/// every section because no heights are known yet), the settled steady
/// state, and scrolling.
#[test]
#[ignore]
fn bench_large_document_frame_times() {
    let _cwd = CWD_LOCK.lock().unwrap();
    let sections = bench_sections();
    let markdown = big_doc(sections);
    println!("document: {} bytes, {sections} sections", markdown.len());
    let doc = temp_doc("bench_large.md", &markdown);

    let (_app, mut harness) = app_harness(&doc, egui::vec2(900.0, 700.0));

    println!("rendered view (900x700):");
    for frame in 1..=6 {
        let t = Instant::now();
        harness.run();
        report_frame(if frame == 1 { "cold" } else { "steady" }, frame, elapsed_ms(t));
    }
    let painted = harness
        .output()
        .shapes
        .iter()
        .filter(|c| matches!(&c.shape, egui::Shape::Text(_)))
        .count();
    let last_visible = harness.query_by_label("Section 59: performance of larger documents");
    println!(
        "  probe: {painted} text shapes, last section visible: {}",
        last_visible.is_some()
    );

    // Scroll down through the document like a wheel user: repeated notches,
    // one timed frame each.
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(450.0, 350.0)));
    let mut scroll_times = Vec::new();
    for _ in 0..40 {
        harness.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -600.0),
            modifiers: egui::Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        });
        let t = Instant::now();
        harness.run();
        scroll_times.push(elapsed_ms(t));
    }
    let mid = &scroll_times[scroll_times.len() / 2..];
    let avg = mid.iter().sum::<f64>() / mid.len() as f64;
    let worst = scroll_times.iter().cloned().fold(0.0, f64::max);
    println!(
        "  scrolling: avg(last 20) {avg:.2} ms, worst {worst:.2} ms over {} frames",
        scroll_times.len()
    );

    // Source view of the same document.
    let app = Rc::new(RefCell::new(App::new(None)));
    {
        let mut a = app.borrow_mut();
        a.mode = ViewMode::Source;
        a.open_path(&doc);
    }
    let app_for_ui = app.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui(move |ui| {
            app_for_ui.borrow_mut().show(ui);
        });
    println!("source view (900x700):");
    for frame in 1..=4 {
        let t = Instant::now();
        harness.run();
        report_frame(if frame == 1 { "cold" } else { "steady" }, frame, elapsed_ms(t));
    }

    // Split view: both panes live.
    let app = Rc::new(RefCell::new(App::new(None)));
    {
        let mut a = app.borrow_mut();
        a.mode = ViewMode::Split;
        a.open_path(&doc);
    }
    let app_for_ui = app.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui(move |ui| {
            app_for_ui.borrow_mut().show(ui);
        });
    println!("split view (900x700):");
    for frame in 1..=4 {
        let t = Instant::now();
        harness.run();
        report_frame(if frame == 1 { "cold" } else { "steady" }, frame, elapsed_ms(t));
    }

    std::fs::remove_file(&doc).ok();
}

fn elapsed_ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Attribution: how expensive are the two per-frame workhorses on their own?
#[test]
#[ignore]
fn bench_parse_and_highlight() {
    let markdown = big_doc(60);
    println!("document: {} bytes", markdown.len());

    // Full-document parse (the cold path parses every section = the whole
    // document; the steady path parses only the visible sections).
    let mut options = pulldown_cmark::Options::empty();
    options |= pulldown_cmark::Options::ENABLE_TABLES;
    options |= pulldown_cmark::Options::ENABLE_TASKLISTS;
    options |= pulldown_cmark::Options::ENABLE_STRIKETHROUGH;
    let t = Instant::now();
    let events = pulldown_cmark::Parser::new_ext(&markdown, options).count();
    println!("pulldown parse full doc: {:.2} ms ({events} events)", elapsed_ms(t));

    let one_section = section(0);
    let t = Instant::now();
    let iterations = 100;
    for _ in 0..iterations {
        let _ = pulldown_cmark::Parser::new_ext(&one_section, options).count();
    }
    println!(
        "pulldown parse one section ({} bytes): {:.3} ms",
        one_section.len(),
        elapsed_ms(t) / iterations as f64
    );

    // Syntect highlighting of the bench code block, as the vendored
    // backend does it per code block per frame.
    use egui_commonmark::syntect::{
        easy::HighlightLines,
        highlighting::ThemeSet,
        parsing::SyntaxSet,
        util::LinesWithEndings,
    };
    let ps = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();
    // What the vendored backend does with a ```rust fence: look up by
    // extension. "rust" is not an extension (it's "rs"), so check both and
    // report which path the app actually takes.
    let syntax_rust_ext = ps.find_syntax_by_extension("rust");
    println!(
        "find_syntax_by_extension(\"rust\"): {:?} (None means fences fall back to simple_highlighting)",
        syntax_rust_ext.map(|s| s.name.clone())
    );
    let syntax = ps
        .find_syntax_by_extension("rust")
        .or_else(|| ps.find_syntax_by_extension("rs"))
        .or_else(|| ps.find_syntax_by_name("Rust"))
        .expect("rust syntax must exist in the default set");
    let theme = &ts.themes["base16-ocean.dark"];
    let code = CODE_BLOCK.trim().trim_start_matches("```rust\n").trim_end_matches("```");
    let t = Instant::now();
    for _ in 0..iterations {
        let mut h = HighlightLines::new(syntax, theme);
        for line in LinesWithEndings::from(code) {
            let _ = h.highlight_line(line, &ps).unwrap();
        }
    }
    println!(
        "syntect highlight code block ({} bytes): {:.3} ms",
        code.len(),
        elapsed_ms(t) / iterations as f64
    );
}

/// How much of the source view's per-frame cost is the egui_code_editor
/// tokenizer (tokenize + LayoutJob build) versus plain text shaping? A
/// plain multiline TextEdit of the same text measures the shaping floor.
#[test]
#[ignore]
fn bench_source_text_edit_floor() {
    let markdown = big_doc(240);
    let owned = markdown.clone();
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui(move |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::TextEdit::multiline(&mut owned.as_str().to_owned())
                    .desired_width(ui.available_width())
                    .show(ui);
            });
        });
    println!("plain TextEdit, {} bytes:", markdown.len());
    for frame in 1..=4 {
        let t = Instant::now();
        harness.run();
        report_frame(if frame == 1 { "cold" } else { "steady" }, frame, elapsed_ms(t));
    }

    // egui_code_editor with the app's exact configuration.
    let owned = markdown.clone();
    let ctx = egui::Context::default();
    let theme = crate::source::code_theme(egui::ThemePreference::Dark, &ctx);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui(move |ui| {
            crate::source::show(ui, &mut owned.as_str().to_owned(), false, theme, None, None);
        });
    println!("egui_code_editor via source::show, {} bytes:", markdown.len());
    for frame in 1..=4 {
        let t = Instant::now();
        harness.run();
        report_frame(if frame == 1 { "cold" } else { "steady" }, frame, elapsed_ms(t));
    }

    // Same, but without line numbers: isolates numlines_show's cost.
    let owned = markdown.clone();
    let ctx = egui::Context::default();
    let theme = crate::source::code_theme(egui::ThemePreference::Dark, &ctx);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(900.0, 700.0))
        .build_ui(move |ui| {
            let mut editor = egui_code_editor::CodeEditor::default()
                .id_source("bench_no_numlines")
                .with_rows(24)
                .with_fontsize(14.0)
                .with_theme(theme)
                .with_numlines(false)
                .with_clickable_links(true)
                .vscroll(true);
            editor.show(ui, &mut owned.as_str().to_owned(), &crate::source::markdown_syntax());
        });
    println!("egui_code_editor without numlines, {} bytes:", markdown.len());
    for frame in 1..=4 {
        let t = Instant::now();
        harness.run();
        report_frame(if frame == 1 { "cold" } else { "steady" }, frame, elapsed_ms(t));
    }
}
