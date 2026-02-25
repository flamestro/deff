mod app;
mod cli;
mod diff;
mod git;
mod model;
mod render;
mod review;
mod terminal;
mod text;

use anyhow::{Context, Result};

use crate::{
    cli::parse_cli_options,
    diff::{build_file_views, get_diff_file_descriptors},
    git::{get_repository_root, resolve_comparison},
    model::{ResolvedComparison, StrategyId},
    render::set_theme_mode_override,
    review::ReviewStore,
    terminal::start_interactive_review,
};

pub fn run() -> Result<()> {
    let options = parse_cli_options()?;
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
    let review_store = ReviewStore::load(&repository_root, &comparison)?;
    start_interactive_review(&file_views, &comparison, review_store)
}
