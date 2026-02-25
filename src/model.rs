use std::{
    collections::HashSet,
    fmt::{self, Display},
};

use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum ThemeMode {
    #[value(name = "auto")]
    Auto,
    #[value(name = "dark")]
    Dark,
    #[value(name = "light")]
    Light,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum StrategyArg {
    #[value(name = "upstream-ahead")]
    UpstreamAhead,
    #[value(name = "range")]
    Range,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StrategyId {
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
pub(crate) enum FileContentSource {
    Commit,
    WorkingTree,
    Missing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LineHighlightKind {
    None,
    Deleted,
    Added,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaneSide {
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedComparison {
    pub(crate) strategy_id: StrategyId,
    pub(crate) base_ref: String,
    pub(crate) head_ref: String,
    pub(crate) base_commit: String,
    pub(crate) head_commit: String,
    pub(crate) summary: String,
    pub(crate) details: Vec<String>,
    pub(crate) ahead_count: Option<usize>,
    pub(crate) includes_uncommitted: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct DiffFileDescriptor {
    pub(crate) raw_status: String,
    pub(crate) display_path: String,
    pub(crate) base_path: Option<String>,
    pub(crate) head_path: Option<String>,
    pub(crate) base_source: FileContentSource,
    pub(crate) head_source: FileContentSource,
}

#[derive(Clone, Debug)]
pub(crate) struct DiffFileView {
    pub(crate) descriptor: DiffFileDescriptor,
    pub(crate) review_key: String,
    pub(crate) left_lines: Vec<String>,
    pub(crate) right_lines: Vec<String>,
    pub(crate) left_language: Option<String>,
    pub(crate) right_language: Option<String>,
    pub(crate) left_deleted_line_indexes: HashSet<usize>,
    pub(crate) right_added_line_indexes: HashSet<usize>,
    pub(crate) left_max_content_length: usize,
    pub(crate) right_max_content_length: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PaneOffsets {
    pub(crate) left: usize,
    pub(crate) right: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct FileLineHighlights {
    pub(crate) left_deleted_line_indexes: HashSet<usize>,
    pub(crate) right_added_line_indexes: HashSet<usize>,
}
