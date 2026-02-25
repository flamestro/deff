use anyhow::{Result, bail};
use clap::Parser;

use crate::model::{StrategyArg, StrategyId, ThemeMode};

const DEFAULT_HEAD_REF: &str = "HEAD";

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
  /                start in-diff search
  n / N            next / previous search match
  r                toggle reviewed for current file
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
pub(crate) struct CliOptions {
    pub(crate) strategy_id: StrategyId,
    pub(crate) base_ref: Option<String>,
    pub(crate) head_ref: String,
    pub(crate) include_uncommitted: bool,
    pub(crate) theme_mode: ThemeMode,
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

pub(crate) fn parse_cli_options() -> Result<CliOptions> {
    let cli = Cli::parse();
    CliOptions::try_from(cli)
}
