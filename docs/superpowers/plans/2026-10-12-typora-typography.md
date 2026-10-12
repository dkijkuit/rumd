# rumd Typora Typography Plan

**Goal:** Make the rendered markdown view read like Typora's default (GitHub) theme — 16px body at 1.5 line height, real bold/italic weights, generous paragraph/list spacing, Typora-style headings, code blocks, quotes, tables — replacing the current cramped egui defaults.

**Approved in chat:** reference app = Typora; bundle a regular-weight font (Inter picked: crisp Helvetica-ish look, OFL); bigger body applies app-wide.

## Measured baseline (why it looks cluttered)

- egui default Ubuntu-Light line metrics = **1.149em** → 18.4px pitch at 16px font (Typora: 24px)
- Intra-paragraph rows come from one galley: font metrics only, no item_spacing — **1.15 line spacing**
- egui 0.36 `RichText::strong()` changes **color only** — rumd has **no real bold anywhere** (headings and `**bold**` render in the same thin weight)
- Headings: `body + (heading−body)·factor` ramp — h2..h6 all large-ish, h4+ not even strong
- Current assets: `apply_typography` already sets Body 16px, item_spacing (8,10), Heading 28

## Key egui 0.36 facts (verified in registry sources)

- `row_height = ascent − descent + line_gap` from font hhea metrics; `FontTweak.scale` scales glyphs+metrics uniformly (no leading trick)
- **`TextFormat::line_height: Option<f32>`** — explicit line height honored by the text layouter → the way to get 1.5 leading per label
- `RichText::family(FontFamily::Name(...))`, `background_color()` exist; custom families need explicit fallback chains (NotoEmoji, emoji-icon-font)
- Default chains: Proportional=[Ubuntu-Light, NotoEmoji, emoji-icon-font], Monospace=[Hack, Ubuntu-Light, …]

## Work

### 1. Assets: Inter statics (`assets/fonts/`)
Bundle Inter-Regular/SemiBold/Bold/Italic/BoldItalic.ttf (from rsms/inter v4.1, OFL — include license). New `src/fonts.rs::install(ctx)`:
- Proportional family → [Inter-Regular, NotoEmoji, emoji-icon-font]
- Custom families: `rumd-bold` [Inter-Bold,…], `rumd-italic` [Inter-Italic,…], `rumd-bold-italic` [Inter-BoldItalic,…], `rumd-heading` [Inter-SemiBold,…]
- Keep Monospace = Hack

### 2. Vendor `egui_commonmark_backend` (`vendor/egui_commonmark_backend`)
Copy from registry 0.25.0, patch crates.io. Patches (all marked `rumd patch`):
- `Style::to_richtext`: Typora heading ramp (32/24/20/16/14/13.5, semibold h1–h4 via `rumd-heading`/`rumd-bold`, h6 muted), strong→`rumd-bold`, emphasis→`rumd-italic` real italics
- `Link::end`: apply line_height to job sections
- `elements.rs`: `blockquote` Typora style (4px bar, muted fg, padding); `code_block` (GitHub bg light `#f6f8fa`/dark `#161b22`, border, inner margin, mono line-height); bullets (bigger dot, wider slot); `rule` quiet 1px
- `misc.rs` code-block layouters: explicit line_height

### 3. Frontend vendor patches (`vendor/egui_commonmark`)
- `event_text`: build LayoutJob via helper — sets body line_height (1.5em), inline-code monospace + bg pill color
- Block gaps: paragraph/heading newlines become spacer labels with explicit small line_height (tune to 16px paragraph gap, ~24px heading margins); HardBreak keeps full row
- `List::start_item`: pixel indent (24px/level), 24px marker slot, 4–6px item gap instead of full blank line
- Table: header row tint + bold, 1px row separators, cell padding via `Grid::spacing`, no zebra

### 4. App (`src/app.rs`, `src/main.rs`, `src/viewer.rs`)
- main.rs: `fonts::install(&cc.egui_ctx)` once at startup
- apply_theme: GitHub text/link colors (light `#1f2328`/`#0969da`, dark `#e6edf3`/`#4493f8`)
- viewer.rs: `COLUMN_WIDTH` 660 → 850
- item_spacing.y = 0 inside the markdown Ui (gaps become explicit)

### 5. Tests
- Geometry probe first (measure current pitch: paragraph line pitch, gaps, list pitch)
- Update `column_width_clamps` (850)
- New assertions: body line pitch ≈ 24px, paragraph gap ≥ 20, h1 taller than h2 > body, list item gap ∈ [4,10], inline code bg painted, table header tint painted
- Existing golden/table/wrap/search tests keep passing; regenerate `website/assets/dark-rendered.png` via capture test
- `cargo clippy --all-targets -- -D warnings` clean

## Non-goals
- Source editor styling beyond the app-wide font/size it already inherits
- New preferences UI (sizes are constants; zoom already exists)
- Touching the uncommitted ToC-highlight work in the tree
