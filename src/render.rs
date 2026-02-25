use once_cell::sync::{Lazy, OnceCell};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};

use crate::{
    model::{
        DiffFileView, LineHighlightKind, PaneOffsets, PaneSide, ResolvedComparison, ThemeMode,
    },
    text::{fit_line, normalize_content, normalized_char_count, pad_to_width, slice_chars},
};

const HEADER_LINE_COUNT: usize = 4;
const FOOTER_LINE_COUNT: usize = 2;
const FRAME_DIVIDER_LINE_COUNT: usize = 2;
const MIN_BODY_LINE_COUNT: usize = 3;
const PANE_SEPARATOR: &str = " | ";

const COLOR_BG_DELETED: Color = Color::Rgb(48, 24, 24);
const COLOR_BG_ADDED: Color = Color::Rgb(22, 34, 24);
const DARK_THEME_CANDIDATES: &[&str] = &[
    "base16-ocean.dark",
    "base16-eighties.dark",
    "base16-mocha.dark",
    "Solarized (dark)",
];
const LIGHT_THEME_CANDIDATES: &[&str] =
    &["InspiredGitHub", "Solarized (light)", "base16-ocean.light"];

static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(SyntaxSet::load_defaults_newlines);
static THEME_SET: Lazy<ThemeSet> = Lazy::new(ThemeSet::load_defaults);
static THEME_MODE_OVERRIDE: OnceCell<ThemeMode> = OnceCell::new();
static THEME: Lazy<Theme> = Lazy::new(|| {
    let prefer_dark_theme = should_prefer_dark_theme();
    let candidates = if prefer_dark_theme {
        DARK_THEME_CANDIDATES
    } else {
        LIGHT_THEME_CANDIDATES
    };

    candidates
        .iter()
        .find_map(|name| THEME_SET.themes.get(*name).cloned())
        .or_else(|| {
            if prefer_dark_theme {
                LIGHT_THEME_CANDIDATES
                    .iter()
                    .find_map(|name| THEME_SET.themes.get(*name).cloned())
            } else {
                DARK_THEME_CANDIDATES
                    .iter()
                    .find_map(|name| THEME_SET.themes.get(*name).cloned())
            }
        })
        .or_else(|| THEME_SET.themes.values().next().cloned())
        .expect("syntect should always provide at least one default theme")
});

#[derive(Clone, Copy, Debug)]
pub(crate) struct FrameLayout {
    pub(crate) columns: usize,
    pub(crate) body_line_count: usize,
    pub(crate) separator: &'static str,
    pub(crate) left_pane_width: usize,
    pub(crate) right_pane_width: usize,
    pub(crate) left_content_width: usize,
    pub(crate) right_content_width: usize,
    pub(crate) line_number_width: usize,
    pub(crate) body_start_row: usize,
    pub(crate) body_end_row: usize,
    pub(crate) left_pane_start_column: usize,
    pub(crate) left_pane_end_column: usize,
    pub(crate) right_pane_start_column: usize,
    pub(crate) right_pane_end_column: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct RenderFrameOutput {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) max_scroll: usize,
    pub(crate) clamped_pane_offsets: PaneOffsets,
}

fn parse_terminal_palette_index(value: &str) -> Option<usize> {
    value.trim().parse::<usize>().ok()
}

pub(crate) fn set_theme_mode_override(mode: ThemeMode) {
    let _ = THEME_MODE_OVERRIDE.set(mode);
}

fn should_prefer_dark_theme() -> bool {
    if let Some(mode) = THEME_MODE_OVERRIDE.get() {
        match mode {
            ThemeMode::Dark => return true,
            ThemeMode::Light => return false,
            ThemeMode::Auto => {}
        }
    }

    if let Ok(value) = std::env::var("DEFF_THEME") {
        match value.trim().to_ascii_lowercase().as_str() {
            "dark" => return true,
            "light" => return false,
            _ => {}
        }
    }

    if let Ok(value) = std::env::var("COLORFGBG") {
        let background_index = value
            .split(|ch| ch == ';' || ch == ':')
            .next_back()
            .and_then(parse_terminal_palette_index);

        if let Some(index) = background_index {
            return index <= 6;
        }
    }

    true
}

fn syntax_for_language(language: &str) -> Option<&'static SyntaxReference> {
    SYNTAX_SET
        .find_syntax_by_token(language)
        .or_else(|| SYNTAX_SET.find_syntax_by_extension(language))
}

fn base_style(tint_background: Option<Color>) -> Style {
    let mut style = Style::default();
    if let Some(color) = tint_background {
        style = style.bg(color);
    }
    style
}

fn syntect_style_to_ratatui(
    style: syntect::highlighting::Style,
    tint_background: Option<Color>,
) -> Style {
    let mut mapped = Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ));

    if let Some(color) = tint_background {
        mapped = mapped.bg(color);
    }

    if style.font_style.contains(FontStyle::BOLD) {
        mapped = mapped.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        mapped = mapped.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        mapped = mapped.add_modifier(Modifier::UNDERLINED);
    }

    mapped
}

fn highlight_visible_content(
    value: &str,
    language: Option<&str>,
    tint_background: Option<Color>,
) -> Vec<Span<'static>> {
    let default_span = || vec![Span::styled(value.to_string(), base_style(tint_background))];

    let Some(language_name) = language else {
        return default_span();
    };

    if value.trim().is_empty() {
        return default_span();
    }

    let Some(syntax) = syntax_for_language(language_name) else {
        return default_span();
    };

    let mut highlighter = HighlightLines::new(syntax, &THEME);
    let highlighted = match highlighter.highlight_line(value, &SYNTAX_SET) {
        Ok(ranges) => ranges,
        Err(_) => return default_span(),
    };

    if highlighted.is_empty() {
        return default_span();
    }

    highlighted
        .into_iter()
        .map(|(style, text)| {
            Span::styled(
                text.to_string(),
                syntect_style_to_ratatui(style, tint_background),
            )
        })
        .collect()
}

fn format_pane_line(
    line_value: Option<&str>,
    line_index: usize,
    pane_width: usize,
    line_number_width: usize,
    line_highlight_kind: LineHighlightKind,
    horizontal_offset: usize,
    language: Option<&str>,
) -> Vec<Span<'static>> {
    let line_number_text = match line_value {
        Some(_) => format!("{:>width$}", line_index + 1, width = line_number_width),
        None => " ".repeat(line_number_width),
    };
    let prefix = format!("{line_number_text} ");
    let prefix_width = normalized_char_count(&prefix);
    let tint_background = match line_highlight_kind {
        LineHighlightKind::None => None,
        LineHighlightKind::Deleted => Some(COLOR_BG_DELETED),
        LineHighlightKind::Added => Some(COLOR_BG_ADDED),
    };

    if pane_width <= prefix_width {
        return vec![Span::styled(
            fit_line(&prefix, pane_width),
            base_style(tint_background),
        )];
    }

    let content_width = pane_width - prefix_width;
    let content_text = line_value.map(normalize_content).unwrap_or_default();
    let visible_content = slice_chars(&content_text, horizontal_offset, content_width);
    let padded_visible_content = pad_to_width(visible_content, content_width);

    let mut spans = vec![Span::styled(prefix, base_style(tint_background))];
    spans.extend(highlight_visible_content(
        &padded_visible_content,
        language,
        tint_background,
    ));
    spans
}

fn short_commit(commit: &str) -> String {
    commit.chars().take(8).collect()
}

pub(crate) fn get_body_line_count(rows: usize) -> usize {
    rows.saturating_sub(HEADER_LINE_COUNT + FOOTER_LINE_COUNT + FRAME_DIVIDER_LINE_COUNT)
        .max(MIN_BODY_LINE_COUNT)
}

pub(crate) fn create_frame_layout(columns: u16, rows: u16, max_lines: usize) -> FrameLayout {
    let columns = columns as usize;
    let rows = rows as usize;
    let body_line_count = get_body_line_count(rows);
    let available_pane_width = columns.saturating_sub(PANE_SEPARATOR.len()).max(2);
    let left_pane_width = (available_pane_width / 2).max(1);
    let right_pane_width = available_pane_width.saturating_sub(left_pane_width).max(1);
    let line_number_width = max_lines.to_string().len().max(3);
    let left_content_width = left_pane_width.saturating_sub(line_number_width + 1);
    let right_content_width = right_pane_width.saturating_sub(line_number_width + 1);
    let body_start_row = HEADER_LINE_COUNT + 1;
    let body_end_row = body_start_row + body_line_count.saturating_sub(1);
    let left_pane_start_column = 0;
    let left_pane_end_column = left_pane_width.saturating_sub(1);
    let right_pane_start_column = left_pane_width + PANE_SEPARATOR.len();
    let right_pane_end_column = right_pane_start_column + right_pane_width.saturating_sub(1);

    FrameLayout {
        columns,
        body_line_count,
        separator: PANE_SEPARATOR,
        left_pane_width,
        right_pane_width,
        left_content_width,
        right_content_width,
        line_number_width,
        body_start_row,
        body_end_row,
        left_pane_start_column,
        left_pane_end_column,
        right_pane_start_column,
        right_pane_end_column,
    }
}

fn get_max_pane_offset(max_content_length: usize, content_width: usize) -> usize {
    if content_width == 0 {
        0
    } else {
        max_content_length.saturating_sub(content_width)
    }
}

pub(crate) fn get_max_pane_offsets(file: &DiffFileView, layout: &FrameLayout) -> PaneOffsets {
    PaneOffsets {
        left: get_max_pane_offset(file.left_max_content_length, layout.left_content_width),
        right: get_max_pane_offset(file.right_max_content_length, layout.right_content_width),
    }
}

pub(crate) fn get_pane_for_column(column: usize, layout: &FrameLayout) -> Option<PaneSide> {
    if column >= layout.left_pane_start_column && column <= layout.left_pane_end_column {
        return Some(PaneSide::Left);
    }

    if column >= layout.right_pane_start_column && column <= layout.right_pane_end_column {
        return Some(PaneSide::Right);
    }

    None
}

pub(crate) fn render_frame(
    files: &[DiffFileView],
    comparison: &ResolvedComparison,
    file_index: usize,
    scroll_offset: usize,
    pane_offsets: PaneOffsets,
    reviewed_count: usize,
    current_file_reviewed: bool,
    search_status_text: String,
    columns: u16,
    rows: u16,
) -> RenderFrameOutput {
    let current_file = &files[file_index];
    let max_lines = current_file
        .left_lines
        .len()
        .max(current_file.right_lines.len());
    let layout = create_frame_layout(columns, rows, max_lines);
    let max_scroll = max_lines.saturating_sub(layout.body_line_count);
    let clamped_scroll_offset = scroll_offset.min(max_scroll);
    let max_pane_offsets = get_max_pane_offsets(current_file, &layout);
    let clamped_pane_offsets = PaneOffsets {
        left: pane_offsets.left.min(max_pane_offsets.left),
        right: pane_offsets.right.min(max_pane_offsets.right),
    };

    let mut body_lines: Vec<Line<'static>> = Vec::with_capacity(layout.body_line_count);
    for row in 0..layout.body_line_count {
        let line_number = clamped_scroll_offset + row;
        let left_line = current_file.left_lines.get(line_number).map(String::as_str);
        let right_line = current_file
            .right_lines
            .get(line_number)
            .map(String::as_str);
        let left_highlight_kind = if current_file
            .left_deleted_line_indexes
            .contains(&line_number)
        {
            LineHighlightKind::Deleted
        } else {
            LineHighlightKind::None
        };
        let right_highlight_kind = if current_file.right_added_line_indexes.contains(&line_number) {
            LineHighlightKind::Added
        } else {
            LineHighlightKind::None
        };

        let left_rendered = format_pane_line(
            left_line,
            line_number,
            layout.left_pane_width,
            layout.line_number_width,
            left_highlight_kind,
            clamped_pane_offsets.left,
            current_file.left_language.as_deref(),
        );
        let right_rendered = format_pane_line(
            right_line,
            line_number,
            layout.right_pane_width,
            layout.line_number_width,
            right_highlight_kind,
            clamped_pane_offsets.right,
            current_file.right_language.as_deref(),
        );

        let mut spans = Vec::with_capacity(left_rendered.len() + right_rendered.len() + 1);
        spans.extend(left_rendered);
        spans.push(Span::raw(layout.separator));
        spans.extend(right_rendered);
        body_lines.push(Line::from(spans));
    }

    let first_visible_line = if max_lines == 0 {
        0
    } else {
        clamped_scroll_offset + 1
    };
    let last_visible_line = if max_lines == 0 {
        0
    } else {
        max_lines.min(clamped_scroll_offset + layout.body_line_count)
    };

    let mut lines = Vec::new();
    let side_summary = if comparison.includes_uncommitted {
        format!(
            "left: {} ({})  right: working tree ({} + local changes)",
            comparison.base_ref,
            short_commit(&comparison.base_commit),
            comparison.head_ref
        )
    } else {
        format!(
            "left: {} ({})  right: {} ({})",
            comparison.base_ref,
            short_commit(&comparison.base_commit),
            comparison.head_ref,
            short_commit(&comparison.head_commit)
        )
    };

    let filename_line = format!("filename: {}", current_file.descriptor.display_path);
    let file_meta_line = format!(
        "file {}/{} [{}] [{}] reviewed: {}/{}  {}",
        file_index + 1,
        files.len(),
        current_file.descriptor.raw_status,
        if current_file_reviewed {
            "reviewed"
        } else {
            "unreviewed"
        },
        reviewed_count,
        files.len(),
        side_summary
    );

    lines.push(Line::from(fit_line(
        &format!(
            "deff review ({})  {}",
            comparison.strategy_id, comparison.summary
        ),
        layout.columns,
    )));
    lines.push(Line::styled(
        fit_line(&filename_line, layout.columns),
        Style::default()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::UNDERLINED),
    ));
    lines.push(Line::from(fit_line(&file_meta_line, layout.columns)));
    lines.push(Line::from(fit_line(
        &comparison.details.join(" | "),
        layout.columns,
    )));

    lines.push(Line::from(fit_line(
        &"-".repeat(layout.columns.max(1)),
        layout.columns,
    )));
    lines.extend(body_lines);
    lines.push(Line::from(fit_line(
        &"-".repeat(layout.columns.max(1)),
        layout.columns,
    )));
    lines.push(Line::from(fit_line(
        "h/l: file  j/k: scroll  ctrl-u/d: page  g/G: top/bottom  /: search  n/N: next/prev match  r: reviewed  q: quit",
        layout.columns,
    )));
    lines.push(Line::from(fit_line(
        &format!(
            "lines {first_visible_line}-{last_visible_line}/{max_lines}  v {clamped_scroll_offset}/{max_scroll}  xL {}/{}  xR {}/{}  {}",
            clamped_pane_offsets.left,
            max_pane_offsets.left,
            clamped_pane_offsets.right,
            max_pane_offsets.right,
            search_status_text,
        ),
        layout.columns,
    )));

    RenderFrameOutput {
        lines,
        max_scroll,
        clamped_pane_offsets,
    }
}
