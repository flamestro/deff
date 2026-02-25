use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fmt::{self, Display},
    fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
use crossterm::{
    cursor::{Hide, Show},
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use once_cell::sync::{Lazy, OnceCell};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Clear, Paragraph},
    Terminal,
};
use regex::Regex;
use syntect::{
    easy::HighlightLines,
    highlighting::{FontStyle, Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};

const DEFAULT_HEAD_REF: &str = "HEAD";
const MOUSE_WHEEL_SCROLL_LINES: usize = 3;
const MOUSE_WHEEL_HORIZONTAL_COLUMNS: usize = 8;
const HEADER_LINE_COUNT: usize = 4;
const FOOTER_LINE_COUNT: usize = 2;
const FRAME_DIVIDER_LINE_COUNT: usize = 2;
const MIN_BODY_LINE_COUNT: usize = 3;
const MISSING_LEFT: &str = "<file does not exist in base revision>";
const MISSING_RIGHT: &str = "<file does not exist in target revision>";
const BINARY_PLACEHOLDER: &str = "<binary file preview not available>";
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
static HUNK_HEADER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")
        .expect("hunk header regex should be valid")
});

fn parse_terminal_palette_index(value: &str) -> Option<usize> {
    value.trim().parse::<usize>().ok()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum ThemeMode {
    #[value(name = "auto")]
    Auto,
    #[value(name = "dark")]
    Dark,
    #[value(name = "light")]
    Light,
}

fn set_theme_mode_override(mode: ThemeMode) {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum StrategyArg {
    #[value(name = "upstream-ahead")]
    UpstreamAhead,
    #[value(name = "range")]
    Range,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StrategyId {
    UpstreamAhead,
    Range,
}

impl Display for StrategyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StrategyId::UpstreamAhead => write!(f, "upstream-ahead"),
            StrategyId::Range => write!(f, "range"),
        }
    }
}

impl From<StrategyArg> for StrategyId {
    fn from(value: StrategyArg) -> Self {
        match value {
            StrategyArg::UpstreamAhead => StrategyId::UpstreamAhead,
            StrategyArg::Range => StrategyId::Range,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FileContentSource {
    Commit,
    WorkingTree,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LineHighlightKind {
    None,
    Deleted,
    Added,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaneSide {
    Left,
    Right,
}

#[derive(Parser, Debug)]
#[command(
    name = "deff",
    about = "Shows side-by-side file content for a git diff in an interactive terminal UI.",
    after_help = r#"Examples:
  deff
  deff --strategy upstream-ahead
  deff --include-uncommitted
  deff --strategy range --base <git-ref> [--head <git-ref>]
  deff --strategy range --base <git-ref> --include-uncommitted
  deff --theme dark

Key bindings:
  h / left-arrow   previous file
  l / right-arrow  next file
  j / down-arrow   scroll down
  k / up-arrow     scroll up
  ctrl-d           page down
  ctrl-u           page up
  g / home         top of file
  G / end          bottom of file
  mouse wheel      vertical scroll
  shift+wheel      horizontal scroll (hovered pane)
  h-wheel          horizontal scroll (hovered pane)
  q                quit"#
)]
struct Cli {
    #[arg(long, value_enum)]
    strategy: Option<StrategyArg>,
    #[arg(long)]
    base: Option<String>,
    #[arg(long, default_value = DEFAULT_HEAD_REF)]
    head: String,
    #[arg(long)]
    include_uncommitted: bool,
    #[arg(long, value_enum, default_value_t = ThemeMode::Auto)]
    theme: ThemeMode,
}

#[derive(Clone, Debug)]
struct CliOptions {
    strategy_id: StrategyId,
    base_ref: Option<String>,
    head_ref: String,
    include_uncommitted: bool,
    theme_mode: ThemeMode,
}

impl TryFrom<Cli> for CliOptions {
    type Error = anyhow::Error;

    fn try_from(value: Cli) -> Result<Self> {
        let strategy_explicitly_set = value.strategy.is_some();
        let strategy_id = match value.strategy {
            Some(strategy) => StrategyId::from(strategy),
            None => {
                if value.base.is_some() {
                    StrategyId::Range
                } else {
                    StrategyId::UpstreamAhead
                }
            }
        };

        if strategy_id == StrategyId::Range && value.base.is_none() {
            bail!("--strategy range requires --base <git-ref>");
        }

        if strategy_explicitly_set
            && strategy_id == StrategyId::UpstreamAhead
            && value.base.is_some()
        {
            bail!("--base can only be used with --strategy range");
        }

        if value.include_uncommitted && value.head != DEFAULT_HEAD_REF {
            bail!("--include-uncommitted currently requires --head HEAD");
        }

        Ok(Self {
            strategy_id,
            base_ref: value.base,
            head_ref: value.head,
            include_uncommitted: value.include_uncommitted,
            theme_mode: value.theme,
        })
    }
}

#[derive(Clone, Debug)]
struct ResolvedComparison {
    strategy_id: StrategyId,
    base_ref: String,
    head_ref: String,
    base_commit: String,
    head_commit: String,
    summary: String,
    details: Vec<String>,
    ahead_count: Option<usize>,
    includes_uncommitted: bool,
}

#[derive(Clone, Debug)]
struct DiffFileDescriptor {
    raw_status: String,
    display_path: String,
    base_path: Option<String>,
    head_path: Option<String>,
    base_source: FileContentSource,
    head_source: FileContentSource,
}

#[derive(Clone, Debug)]
struct DiffFileView {
    descriptor: DiffFileDescriptor,
    left_lines: Vec<String>,
    right_lines: Vec<String>,
    left_language: Option<String>,
    right_language: Option<String>,
    left_deleted_line_indexes: HashSet<usize>,
    right_added_line_indexes: HashSet<usize>,
    left_max_content_length: usize,
    right_max_content_length: usize,
}

#[derive(Clone, Copy, Debug, Default)]
struct PaneOffsets {
    left: usize,
    right: usize,
}

#[derive(Clone, Copy, Debug)]
struct FrameLayout {
    columns: usize,
    body_line_count: usize,
    separator: &'static str,
    left_pane_width: usize,
    right_pane_width: usize,
    left_content_width: usize,
    right_content_width: usize,
    line_number_width: usize,
    body_start_row: usize,
    body_end_row: usize,
    left_pane_start_column: usize,
    left_pane_end_column: usize,
    right_pane_start_column: usize,
    right_pane_end_column: usize,
}

#[derive(Clone, Debug)]
struct RenderFrameOutput {
    lines: Vec<Line<'static>>,
    max_scroll: usize,
    clamped_pane_offsets: PaneOffsets,
}

#[derive(Clone, Debug)]
struct FileLineHighlights {
    left_deleted_line_indexes: HashSet<usize>,
    right_added_line_indexes: HashSet<usize>,
}

#[derive(Clone, Debug)]
struct AppState {
    file_index: usize,
    scroll_offset: usize,
    pane_offsets_by_file: Vec<PaneOffsets>,
}

impl AppState {
    fn new(file_count: usize) -> Self {
        Self {
            file_index: 0,
            scroll_offset: 0,
            pane_offsets_by_file: vec![PaneOffsets::default(); file_count],
        }
    }

    fn current_offsets(&self) -> PaneOffsets {
        self.pane_offsets_by_file[self.file_index]
    }

    fn set_current_offsets(&mut self, pane_offsets: PaneOffsets) {
        self.pane_offsets_by_file[self.file_index] = pane_offsets;
    }
}

#[derive(Clone, Debug)]
struct GitOutput {
    stdout: Vec<u8>,
}

fn get_body_line_count(rows: usize) -> usize {
    rows.saturating_sub(HEADER_LINE_COUNT + FOOTER_LINE_COUNT + FRAME_DIVIDER_LINE_COUNT)
        .max(MIN_BODY_LINE_COUNT)
}

fn run_git<I, S>(args: I, cwd: &Path) -> Result<GitOutput>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args_vec: Vec<OsString> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect();

    let output = Command::new("git")
        .args(&args_vec)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("failed to run git in {}", cwd.display()))?;

    let stderr_text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        let command = format!(
            "git {}",
            args_vec
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        );

        let details = if stderr_text.is_empty() {
            format!("exit status {}", output.status)
        } else {
            stderr_text
        };

        bail!("{command} failed: {details}");
    }

    Ok(GitOutput {
        stdout: output.stdout,
    })
}

fn run_git_text<I, S>(args: I, cwd: &Path) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = run_git(args, cwd)?;
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_usize_value(raw: &str, context: &str) -> Result<usize> {
    raw.trim()
        .parse::<usize>()
        .with_context(|| format!("unable to parse {context}: {}", raw.trim()))
}

fn get_repository_root(cwd: &Path) -> Result<PathBuf> {
    let output = run_git_text(["rev-parse", "--show-toplevel"], cwd)?;
    Ok(PathBuf::from(output.trim()))
}

fn resolve_upstream_ahead_comparison(
    repo_root: &Path,
    head_ref: &str,
) -> Result<ResolvedComparison> {
    let upstream_ref = match run_git_text(
        [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        repo_root,
    ) {
        Ok(value) => value.trim().to_string(),
        Err(_) => {
            bail!(
                "No upstream branch configured for the current branch. Use --strategy range --base <git-ref> instead."
            )
        }
    };

    let current_branch = run_git_text(["rev-parse", "--abbrev-ref", "HEAD"], repo_root)?
        .trim()
        .to_string();
    let base_commit = run_git_text(
        ["rev-parse", &format!("{upstream_ref}^{{commit}}")],
        repo_root,
    )?
    .trim()
    .to_string();
    let head_commit = run_git_text(["rev-parse", &format!("{head_ref}^{{commit}}")], repo_root)?
        .trim()
        .to_string();
    let ahead_count_raw = run_git_text(
        [
            "rev-list",
            "--count",
            &format!("{upstream_ref}..{head_ref}"),
        ],
        repo_root,
    )?;
    let behind_count_raw = run_git_text(
        [
            "rev-list",
            "--count",
            &format!("{head_ref}..{upstream_ref}"),
        ],
        repo_root,
    )?;

    let ahead_count = parse_usize_value(&ahead_count_raw, "ahead count")?;
    let behind_count = parse_usize_value(&behind_count_raw, "behind count")?;

    Ok(ResolvedComparison {
        strategy_id: StrategyId::UpstreamAhead,
        base_ref: upstream_ref.clone(),
        head_ref: head_ref.to_string(),
        base_commit,
        head_commit,
        summary: format!("{upstream_ref}..{head_ref}"),
        details: vec![
            format!("branch: {current_branch}"),
            format!("upstream: {upstream_ref}"),
            format!("ahead: {ahead_count}"),
            format!("behind: {behind_count}"),
        ],
        ahead_count: Some(ahead_count),
        includes_uncommitted: false,
    })
}

fn resolve_range_comparison(
    repo_root: &Path,
    base_ref: &str,
    head_ref: &str,
) -> Result<ResolvedComparison> {
    let base_commit = run_git_text(["rev-parse", &format!("{base_ref}^{{commit}}")], repo_root)?
        .trim()
        .to_string();
    let head_commit = run_git_text(["rev-parse", &format!("{head_ref}^{{commit}}")], repo_root)?
        .trim()
        .to_string();
    let commit_count_raw = run_git_text(
        ["rev-list", "--count", &format!("{base_ref}..{head_ref}")],
        repo_root,
    )?;
    let commit_count = parse_usize_value(&commit_count_raw, "commit count")?;

    Ok(ResolvedComparison {
        strategy_id: StrategyId::Range,
        base_ref: base_ref.to_string(),
        head_ref: head_ref.to_string(),
        base_commit,
        head_commit,
        summary: format!("{base_ref}..{head_ref}"),
        details: vec![format!("commits in range: {commit_count}")],
        ahead_count: None,
        includes_uncommitted: false,
    })
}

fn resolve_comparison(repo_root: &Path, options: &CliOptions) -> Result<ResolvedComparison> {
    match options.strategy_id {
        StrategyId::Range => {
            let base_ref = options
                .base_ref
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("missing base reference for range strategy"))?;
            resolve_range_comparison(repo_root, base_ref, &options.head_ref)
        }
        StrategyId::UpstreamAhead => {
            resolve_upstream_ahead_comparison(repo_root, &options.head_ref)
        }
    }
}

fn split_null_terminated(raw_output: &[u8]) -> Vec<String> {
    raw_output
        .split(|byte| *byte == b'\0')
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
        .collect()
}

fn parse_diff_name_status_output(
    raw_output: &[u8],
    base_source: FileContentSource,
    head_source: FileContentSource,
) -> Vec<DiffFileDescriptor> {
    if raw_output.is_empty() {
        return Vec::new();
    }

    let tokens = split_null_terminated(raw_output);
    let mut files = Vec::new();
    let mut index = 0;

    while index < tokens.len() {
        let status_token = match tokens.get(index) {
            Some(value) => value,
            None => break,
        };
        index += 1;

        let status_code = status_token.chars().next().unwrap_or_default();
        if status_code == 'R' || status_code == 'C' {
            let old_path = match tokens.get(index) {
                Some(value) => value,
                None => break,
            };
            let new_path = match tokens.get(index + 1) {
                Some(value) => value,
                None => break,
            };
            index += 2;

            if old_path.is_empty() || new_path.is_empty() {
                continue;
            }

            files.push(DiffFileDescriptor {
                raw_status: status_token.clone(),
                display_path: format!("{old_path} -> {new_path}"),
                base_path: Some(old_path.clone()),
                head_path: Some(new_path.clone()),
                base_source,
                head_source,
            });
            continue;
        }

        let path_value = match tokens.get(index) {
            Some(value) => value,
            None => break,
        };
        index += 1;

        if path_value.is_empty() {
            continue;
        }

        match status_code {
            'A' => files.push(DiffFileDescriptor {
                raw_status: status_token.clone(),
                display_path: path_value.clone(),
                base_path: None,
                head_path: Some(path_value.clone()),
                base_source: FileContentSource::Missing,
                head_source,
            }),
            'D' => files.push(DiffFileDescriptor {
                raw_status: status_token.clone(),
                display_path: path_value.clone(),
                base_path: Some(path_value.clone()),
                head_path: None,
                base_source,
                head_source: FileContentSource::Missing,
            }),
            _ => files.push(DiffFileDescriptor {
                raw_status: status_token.clone(),
                display_path: path_value.clone(),
                base_path: Some(path_value.clone()),
                head_path: Some(path_value.clone()),
                base_source,
                head_source,
            }),
        }
    }

    files
}

fn parse_null_separated_list(raw_output: &[u8]) -> Vec<String> {
    split_null_terminated(raw_output)
}

fn get_diff_file_descriptors(
    repo_root: &Path,
    comparison: &ResolvedComparison,
) -> Result<Vec<DiffFileDescriptor>> {
    if comparison.includes_uncommitted {
        let tracked_output = run_git(
            [
                "diff",
                "--name-status",
                "--find-renames",
                "-z",
                comparison.base_commit.as_str(),
            ],
            repo_root,
        )?;

        let mut descriptors = parse_diff_name_status_output(
            &tracked_output.stdout,
            FileContentSource::Commit,
            FileContentSource::WorkingTree,
        );

        let mut seen_paths: HashSet<String> = descriptors
            .iter()
            .filter_map(|descriptor| {
                descriptor
                    .head_path
                    .clone()
                    .or_else(|| descriptor.base_path.clone())
            })
            .collect();

        let untracked_output = run_git(
            ["ls-files", "--others", "--exclude-standard", "-z"],
            repo_root,
        )?;
        let untracked_paths = parse_null_separated_list(&untracked_output.stdout);

        for untracked_path in untracked_paths {
            if seen_paths.contains(&untracked_path) {
                continue;
            }

            descriptors.push(DiffFileDescriptor {
                raw_status: "??".to_string(),
                display_path: untracked_path.clone(),
                base_path: None,
                head_path: Some(untracked_path.clone()),
                base_source: FileContentSource::Missing,
                head_source: FileContentSource::WorkingTree,
            });
            seen_paths.insert(untracked_path);
        }

        return Ok(descriptors);
    }

    let committed_output = run_git(
        [
            "diff",
            "--name-status",
            "--find-renames",
            "-z",
            &format!("{}..{}", comparison.base_commit, comparison.head_commit),
        ],
        repo_root,
    )?;

    Ok(parse_diff_name_status_output(
        &committed_output.stdout,
        FileContentSource::Commit,
        FileContentSource::Commit,
    ))
}

fn create_empty_line_highlights() -> FileLineHighlights {
    FileLineHighlights {
        left_deleted_line_indexes: HashSet::new(),
        right_added_line_indexes: HashSet::new(),
    }
}

fn create_range_line_indexes(line_count: usize) -> HashSet<usize> {
    (0..line_count).collect()
}

fn parse_hunk_count(value: Option<&str>) -> usize {
    match value {
        None => 1,
        Some(raw) => raw.parse::<usize>().unwrap_or(0),
    }
}

fn parse_line_highlights_from_patch(diff_output: &str) -> FileLineHighlights {
    let mut highlights = create_empty_line_highlights();

    for line in diff_output.lines() {
        let Some(captures) = HUNK_HEADER_RE.captures(line) else {
            continue;
        };

        let old_start = captures
            .get(1)
            .and_then(|value| value.as_str().parse::<usize>().ok());
        let old_count = parse_hunk_count(captures.get(2).map(|value| value.as_str()));
        let new_start = captures
            .get(3)
            .and_then(|value| value.as_str().parse::<usize>().ok());
        let new_count = parse_hunk_count(captures.get(4).map(|value| value.as_str()));

        if let Some(start) = old_start {
            let start_index = start.saturating_sub(1);
            for offset in 0..old_count {
                highlights
                    .left_deleted_line_indexes
                    .insert(start_index.saturating_add(offset));
            }
        }

        if let Some(start) = new_start {
            let start_index = start.saturating_sub(1);
            for offset in 0..new_count {
                highlights
                    .right_added_line_indexes
                    .insert(start_index.saturating_add(offset));
            }
        }
    }

    highlights
}

fn get_line_highlights_for_descriptor(
    repo_root: &Path,
    comparison: &ResolvedComparison,
    descriptor: &DiffFileDescriptor,
    left_line_count: usize,
    right_line_count: usize,
) -> FileLineHighlights {
    if descriptor.base_source == FileContentSource::Missing {
        return FileLineHighlights {
            left_deleted_line_indexes: HashSet::new(),
            right_added_line_indexes: create_range_line_indexes(right_line_count),
        };
    }

    if descriptor.head_source == FileContentSource::Missing {
        return FileLineHighlights {
            left_deleted_line_indexes: create_range_line_indexes(left_line_count),
            right_added_line_indexes: HashSet::new(),
        };
    }

    let Some(base_path) = descriptor.base_path.as_deref() else {
        return create_empty_line_highlights();
    };
    let Some(head_path) = descriptor.head_path.as_deref() else {
        return create_empty_line_highlights();
    };

    let path_specs = if base_path == head_path {
        vec![base_path.to_string()]
    } else {
        vec![base_path.to_string(), head_path.to_string()]
    };

    let mut diff_args: Vec<OsString> = Vec::new();
    diff_args.push(OsString::from("diff"));
    diff_args.push(OsString::from("--no-color"));
    diff_args.push(OsString::from("--unified=0"));

    if comparison.includes_uncommitted {
        diff_args.push(OsString::from(comparison.base_commit.as_str()));
    } else {
        diff_args.push(OsString::from("--find-renames"));
        diff_args.push(OsString::from(format!(
            "{}..{}",
            comparison.base_commit, comparison.head_commit
        )));
    }

    diff_args.push(OsString::from("--"));
    for path_spec in path_specs {
        diff_args.push(OsString::from(path_spec));
    }

    let diff_output = match run_git_text(diff_args, repo_root) {
        Ok(value) => value,
        Err(_) => return create_empty_line_highlights(),
    };

    parse_line_highlights_from_patch(&diff_output)
}

fn is_binary_content(content: &[u8]) -> bool {
    let sample_size = content.len().min(8192);
    content[..sample_size].contains(&0)
}

fn split_into_lines(content: &str) -> Vec<String> {
    let normalized = content.replace("\r\n", "\n");

    if normalized.is_empty() {
        return vec![String::new()];
    }

    let mut lines: Vec<String> = normalized.split('\n').map(ToOwned::to_owned).collect();
    if lines.len() > 1 && lines.last().is_some_and(|last| last.is_empty()) {
        let _ = lines.pop();
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

fn read_lines_at_revision(repo_root: &Path, revision: &str, file_path: &str) -> Vec<String> {
    let revision_spec = format!("{revision}:{file_path}");
    match run_git(["show", revision_spec.as_str()], repo_root) {
        Ok(output) => {
            if is_binary_content(&output.stdout) {
                return vec![BINARY_PLACEHOLDER.to_string()];
            }

            split_into_lines(&String::from_utf8_lossy(&output.stdout))
        }
        Err(error) => vec![format!("<unable to load file: {error}>")],
    }
}

fn read_lines_at_working_tree(repo_root: &Path, file_path: &str) -> Vec<String> {
    let absolute_path = repo_root.join(file_path);
    match fs::read(&absolute_path) {
        Ok(buffer) => {
            if is_binary_content(&buffer) {
                return vec![BINARY_PLACEHOLDER.to_string()];
            }

            split_into_lines(&String::from_utf8_lossy(&buffer))
        }
        Err(error) => vec![format!("<unable to load file: {error}>")],
    }
}

fn build_file_views(
    repo_root: &Path,
    comparison: &ResolvedComparison,
    descriptors: &[DiffFileDescriptor],
) -> Vec<DiffFileView> {
    let mut views = Vec::with_capacity(descriptors.len());

    for descriptor in descriptors {
        let left_lines = match descriptor.base_source {
            FileContentSource::Missing => vec![MISSING_LEFT.to_string()],
            FileContentSource::WorkingTree => descriptor
                .base_path
                .as_deref()
                .map(|path| read_lines_at_working_tree(repo_root, path))
                .unwrap_or_else(|| vec![MISSING_LEFT.to_string()]),
            FileContentSource::Commit => descriptor
                .base_path
                .as_deref()
                .map(|path| read_lines_at_revision(repo_root, &comparison.base_commit, path))
                .unwrap_or_else(|| vec![MISSING_LEFT.to_string()]),
        };

        let right_lines = match descriptor.head_source {
            FileContentSource::Missing => vec![MISSING_RIGHT.to_string()],
            FileContentSource::WorkingTree => descriptor
                .head_path
                .as_deref()
                .map(|path| read_lines_at_working_tree(repo_root, path))
                .unwrap_or_else(|| vec![MISSING_RIGHT.to_string()]),
            FileContentSource::Commit => descriptor
                .head_path
                .as_deref()
                .map(|path| read_lines_at_revision(repo_root, &comparison.head_commit, path))
                .unwrap_or_else(|| vec![MISSING_RIGHT.to_string()]),
        };

        let line_highlights = get_line_highlights_for_descriptor(
            repo_root,
            comparison,
            descriptor,
            left_lines.len(),
            right_lines.len(),
        );

        views.push(DiffFileView {
            descriptor: descriptor.clone(),
            left_language: get_language_for_path(descriptor.base_path.as_deref()),
            right_language: get_language_for_path(descriptor.head_path.as_deref()),
            left_deleted_line_indexes: line_highlights.left_deleted_line_indexes,
            right_added_line_indexes: line_highlights.right_added_line_indexes,
            left_max_content_length: get_max_normalized_line_length(&left_lines),
            right_max_content_length: get_max_normalized_line_length(&right_lines),
            left_lines,
            right_lines,
        });
    }

    views
}

fn normalized_char_count(value: &str) -> usize {
    value.chars().count()
}

fn slice_chars(value: &str, start: usize, len: usize) -> String {
    if len == 0 {
        return String::new();
    }

    value.chars().skip(start).take(len).collect()
}

fn truncate_to_width(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    if normalized_char_count(value) <= width {
        return value.to_string();
    }

    if width <= 3 {
        return value.chars().take(width).collect();
    }

    let mut truncated: String = value.chars().take(width - 3).collect();
    truncated.push_str("...");
    truncated
}

fn pad_to_width(value: String, width: usize) -> String {
    let len = normalized_char_count(&value);
    if len >= width {
        value.chars().take(width).collect()
    } else {
        format!("{value}{}", " ".repeat(width - len))
    }
}

fn fit_line(value: &str, width: usize) -> String {
    let truncated = truncate_to_width(value, width);
    pad_to_width(truncated, width)
}

fn normalize_content(value: &str) -> String {
    value.replace('\t', "  ").replace('\r', "")
}

fn get_max_normalized_line_length(lines: &[String]) -> usize {
    lines
        .iter()
        .map(|line| normalized_char_count(&normalize_content(line)))
        .max()
        .unwrap_or(0)
}

fn extension_to_language(extension: &str) -> Option<&'static str> {
    match extension {
        "c" => Some("c"),
        "cc" => Some("cpp"),
        "cjs" => Some("javascript"),
        "cpp" => Some("cpp"),
        "css" => Some("css"),
        "go" => Some("go"),
        "h" => Some("c"),
        "hpp" => Some("cpp"),
        "htm" => Some("html"),
        "html" => Some("html"),
        "java" => Some("java"),
        "js" => Some("javascript"),
        "json" => Some("json"),
        "jsx" => Some("jsx"),
        "md" => Some("markdown"),
        "mjs" => Some("javascript"),
        "py" => Some("python"),
        "rb" => Some("ruby"),
        "rs" => Some("rust"),
        "scss" => Some("scss"),
        "sh" => Some("bash"),
        "sql" => Some("sql"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "xml" => Some("xml"),
        "yaml" => Some("yaml"),
        "yml" => Some("yaml"),
        "zsh" => Some("bash"),
        _ => None,
    }
}

fn get_language_for_path(file_path: Option<&str>) -> Option<String> {
    let file_path = file_path?;

    let path = Path::new(file_path);
    let file_name = path.file_name()?.to_string_lossy().to_lowercase();
    if file_name == "dockerfile" {
        return Some("dockerfile".to_string());
    }

    let extension = path.extension()?.to_string_lossy().to_lowercase();
    extension_to_language(&extension).map(ToOwned::to_owned)
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

fn create_frame_layout(columns: u16, rows: u16, max_lines: usize) -> FrameLayout {
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

fn get_max_pane_offsets(file: &DiffFileView, layout: &FrameLayout) -> PaneOffsets {
    PaneOffsets {
        left: get_max_pane_offset(file.left_max_content_length, layout.left_content_width),
        right: get_max_pane_offset(file.right_max_content_length, layout.right_content_width),
    }
}

fn get_pane_for_column(column: usize, layout: &FrameLayout) -> Option<PaneSide> {
    if column >= layout.left_pane_start_column && column <= layout.left_pane_end_column {
        return Some(PaneSide::Left);
    }

    if column >= layout.right_pane_start_column && column <= layout.right_pane_end_column {
        return Some(PaneSide::Right);
    }

    None
}

fn render_frame(
    files: &[DiffFileView],
    comparison: &ResolvedComparison,
    file_index: usize,
    scroll_offset: usize,
    pane_offsets: PaneOffsets,
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

    lines.push(Line::from(fit_line(
        &format!(
            "deff review ({})  {}",
            comparison.strategy_id, comparison.summary
        ),
        layout.columns,
    )));
    lines.push(Line::from(fit_line(
        &format!(
            "file {}/{} [{}] {}",
            file_index + 1,
            files.len(),
            current_file.descriptor.raw_status,
            current_file.descriptor.display_path
        ),
        layout.columns,
    )));
    lines.push(Line::from(fit_line(&side_summary, layout.columns)));
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
        "h/l: file  j/k: scroll  ctrl-u/d: page  g/G: top/bottom  wheel: v-scroll  shift+wheel/h-wheel: x-scroll  q: quit",
        layout.columns,
    )));
    lines.push(Line::from(fit_line(
        &format!(
            "lines {first_visible_line}-{last_visible_line}/{max_lines}  v {clamped_scroll_offset}/{max_scroll}  xL {}/{}  xR {}/{}  tint: deleted(left)/added(right)",
            clamped_pane_offsets.left,
            max_pane_offsets.left,
            clamped_pane_offsets.right,
            max_pane_offsets.right,
        ),
        layout.columns,
    )));

    RenderFrameOutput {
        lines,
        max_scroll,
        clamped_pane_offsets,
    }
}

fn max_scroll_for_current_file(files: &[DiffFileView], app: &AppState, rows: u16) -> usize {
    let current_file = &files[app.file_index];
    let max_lines = current_file
        .left_lines
        .len()
        .max(current_file.right_lines.len());
    let body_line_count = get_body_line_count(rows as usize);
    max_lines.saturating_sub(body_line_count)
}

fn move_file(delta: isize, files: &[DiffFileView], app: &mut AppState) {
    let max_index = files.len().saturating_sub(1) as isize;
    let next_index = (app.file_index as isize + delta).clamp(0, max_index) as usize;
    if next_index != app.file_index {
        app.file_index = next_index;
        app.scroll_offset = 0;
    }
}

fn move_scroll(delta: isize, files: &[DiffFileView], app: &mut AppState, rows: u16) {
    let max_scroll = max_scroll_for_current_file(files, app, rows);
    let next_offset = (app.scroll_offset as isize + delta).clamp(0, max_scroll as isize) as usize;
    app.scroll_offset = next_offset;
}

fn scroll_to_top(app: &mut AppState) {
    app.scroll_offset = 0;
}

fn scroll_to_bottom(files: &[DiffFileView], app: &mut AppState, rows: u16) {
    app.scroll_offset = max_scroll_for_current_file(files, app, rows);
}

fn move_horizontal(
    pane: PaneSide,
    delta: isize,
    files: &[DiffFileView],
    app: &mut AppState,
    columns: u16,
    rows: u16,
) {
    let current_file = &files[app.file_index];
    let max_lines = current_file
        .left_lines
        .len()
        .max(current_file.right_lines.len());
    let layout = create_frame_layout(columns, rows, max_lines);
    let max_offsets = get_max_pane_offsets(current_file, &layout);
    let current_offsets = &mut app.pane_offsets_by_file[app.file_index];

    match pane {
        PaneSide::Left => {
            current_offsets.left = (current_offsets.left as isize + delta)
                .clamp(0, max_offsets.left as isize) as usize;
        }
        PaneSide::Right => {
            current_offsets.right = (current_offsets.right as isize + delta)
                .clamp(0, max_offsets.right as isize) as usize;
        }
    }
}

fn handle_keypress(key: KeyEvent, files: &[DiffFileView], app: &mut AppState, rows: u16) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return true;
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => true,
        KeyCode::Left => {
            move_file(-1, files, app);
            false
        }
        KeyCode::Right => {
            move_file(1, files, app);
            false
        }
        KeyCode::Up => {
            move_scroll(-1, files, app, rows);
            false
        }
        KeyCode::Down => {
            move_scroll(1, files, app, rows);
            false
        }
        KeyCode::Char('h') => {
            move_file(-1, files, app);
            false
        }
        KeyCode::Char('l') => {
            move_file(1, files, app);
            false
        }
        KeyCode::Char('k') => {
            move_scroll(-1, files, app, rows);
            false
        }
        KeyCode::Char('j') => {
            move_scroll(1, files, app, rows);
            false
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let page_size = get_body_line_count(rows as usize).max(1) as isize;
            move_scroll(-page_size, files, app, rows);
            false
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let page_size = get_body_line_count(rows as usize).max(1) as isize;
            move_scroll(page_size, files, app, rows);
            false
        }
        KeyCode::PageUp => {
            let page_size = get_body_line_count(rows as usize).max(1) as isize;
            move_scroll(-page_size, files, app, rows);
            false
        }
        KeyCode::PageDown => {
            let page_size = get_body_line_count(rows as usize).max(1) as isize;
            move_scroll(page_size, files, app, rows);
            false
        }
        KeyCode::Home => {
            scroll_to_top(app);
            false
        }
        KeyCode::End => {
            scroll_to_bottom(files, app, rows);
            false
        }
        KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::SHIFT) => {
            scroll_to_bottom(files, app, rows);
            false
        }
        KeyCode::Char('G') => {
            scroll_to_bottom(files, app, rows);
            false
        }
        KeyCode::Char('g') => {
            scroll_to_top(app);
            false
        }
        _ => false,
    }
}

fn handle_mouse(
    mouse: MouseEvent,
    files: &[DiffFileView],
    app: &mut AppState,
    columns: u16,
    rows: u16,
) {
    let current_file = &files[app.file_index];
    let max_lines = current_file
        .left_lines
        .len()
        .max(current_file.right_lines.len());
    let layout = create_frame_layout(columns, rows, max_lines);

    let row = mouse.row as usize;
    if row < layout.body_start_row || row > layout.body_end_row {
        return;
    }

    let column = mouse.column as usize;
    let hovered_pane = get_pane_for_column(column, &layout);

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                if let Some(pane) = hovered_pane {
                    move_horizontal(
                        pane,
                        -(MOUSE_WHEEL_HORIZONTAL_COLUMNS as isize),
                        files,
                        app,
                        columns,
                        rows,
                    );
                }
            } else {
                move_scroll(-(MOUSE_WHEEL_SCROLL_LINES as isize), files, app, rows);
            }
        }
        MouseEventKind::ScrollDown => {
            if mouse.modifiers.contains(KeyModifiers::SHIFT) {
                if let Some(pane) = hovered_pane {
                    move_horizontal(
                        pane,
                        MOUSE_WHEEL_HORIZONTAL_COLUMNS as isize,
                        files,
                        app,
                        columns,
                        rows,
                    );
                }
            } else {
                move_scroll(MOUSE_WHEEL_SCROLL_LINES as isize, files, app, rows);
            }
        }
        MouseEventKind::ScrollLeft => {
            if let Some(pane) = hovered_pane {
                move_horizontal(
                    pane,
                    -(MOUSE_WHEEL_HORIZONTAL_COLUMNS as isize),
                    files,
                    app,
                    columns,
                    rows,
                );
            }
        }
        MouseEventKind::ScrollRight => {
            if let Some(pane) = hovered_pane {
                move_horizontal(
                    pane,
                    MOUSE_WHEEL_HORIZONTAL_COLUMNS as isize,
                    files,
                    app,
                    columns,
                    rows,
                );
            }
        }
        _ => {}
    }
}

fn draw_app<B: Backend>(
    terminal: &mut Terminal<B>,
    files: &[DiffFileView],
    comparison: &ResolvedComparison,
    app: &mut AppState,
) -> Result<()> {
    let size = terminal.size()?;
    let render_output = render_frame(
        files,
        comparison,
        app.file_index,
        app.scroll_offset,
        app.current_offsets(),
        size.width,
        size.height,
    );

    app.scroll_offset = app.scroll_offset.min(render_output.max_scroll);
    app.set_current_offsets(render_output.clamped_pane_offsets);

    let text = Text::from(render_output.lines);
    terminal.draw(move |frame| {
        let area = frame.area();
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(text), area);
    })?;

    Ok(())
}

fn run_event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    files: &[DiffFileView],
    comparison: &ResolvedComparison,
) -> Result<()> {
    let mut app = AppState::new(files.len());
    draw_app(terminal, files, comparison, &mut app)?;

    loop {
        match event::read().context("failed to read terminal event")? {
            Event::Key(key) => {
                if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    continue;
                }

                let (_, rows) =
                    crossterm::terminal::size().context("failed to read terminal size")?;
                if handle_keypress(key, files, &mut app, rows) {
                    break;
                }
            }
            Event::Mouse(mouse) => {
                let (columns, rows) =
                    crossterm::terminal::size().context("failed to read terminal size")?;
                handle_mouse(mouse, files, &mut app, columns, rows);
            }
            Event::Resize(_, _) => {}
            Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
        }

        draw_app(terminal, files, comparison, &mut app)?;
    }

    Ok(())
}

fn start_interactive_review(files: &[DiffFileView], comparison: &ResolvedComparison) -> Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Interactive TTY is required to run deff");
    }

    enable_raw_mode().context("failed to enable raw mode")?;

    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide) {
        let _ = disable_raw_mode();
        return Err(error).context("failed to initialize terminal UI");
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            let _ = disable_raw_mode();
            let mut cleanup_stdout = io::stdout();
            let _ = execute!(
                cleanup_stdout,
                Show,
                DisableMouseCapture,
                LeaveAlternateScreen
            );
            return Err(error).context("failed to build terminal backend");
        }
    };

    let run_result = run_event_loop(&mut terminal, files, comparison);

    let mut restore_error: Option<anyhow::Error> = None;
    if let Err(error) = disable_raw_mode() {
        restore_error = Some(error.into());
    }
    if let Err(error) = execute!(
        terminal.backend_mut(),
        Show,
        DisableMouseCapture,
        LeaveAlternateScreen
    ) {
        if restore_error.is_none() {
            restore_error = Some(error.into());
        }
    }
    if let Err(error) = terminal.show_cursor() {
        if restore_error.is_none() {
            restore_error = Some(error.into());
        }
    }

    if let Some(error) = restore_error {
        return Err(error).context("failed to restore terminal state");
    }

    run_result
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let options = CliOptions::try_from(cli)?;
    set_theme_mode_override(options.theme_mode);

    let current_directory = std::env::current_dir().context("failed to read current directory")?;
    let repository_root = get_repository_root(&current_directory)?;
    let resolved_comparison = resolve_comparison(&repository_root, &options)?;

    let comparison = if options.include_uncommitted {
        let mut details = resolved_comparison.details.clone();
        details.push("uncommitted: included".to_string());
        ResolvedComparison {
            summary: format!("{}..WORKTREE", resolved_comparison.base_ref),
            details,
            includes_uncommitted: true,
            ..resolved_comparison
        }
    } else {
        resolved_comparison
    };

    if comparison.strategy_id == StrategyId::UpstreamAhead
        && !comparison.includes_uncommitted
        && comparison.ahead_count.is_some_and(|ahead| ahead == 0)
    {
        println!("No local commits ahead of {}.", comparison.base_ref);
        return Ok(());
    }

    let descriptors = get_diff_file_descriptors(&repository_root, &comparison)?;
    if descriptors.is_empty() {
        println!("No changed files found for {}.", comparison.summary);
        return Ok(());
    }

    let file_views = build_file_views(&repository_root, &comparison, &descriptors);
    start_interactive_review(&file_views, &comparison)
}

fn main() {
    if let Err(error) = run() {
        eprintln!("deff failed: {error}");
        std::process::exit(1);
    }
}
