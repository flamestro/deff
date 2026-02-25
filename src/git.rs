use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

use crate::{
    cli::CliOptions,
    model::{ResolvedComparison, StrategyId},
};

pub(crate) fn run_git<I, S>(args: I, cwd: &Path) -> Result<Vec<u8>>
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

    Ok(output.stdout)
}

pub(crate) fn run_git_text<I, S>(args: I, cwd: &Path) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = run_git(args, cwd)?;
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn parse_usize_value(raw: &str, context: &str) -> Result<usize> {
    raw.trim()
        .parse::<usize>()
        .with_context(|| format!("unable to parse {context}: {}", raw.trim()))
}

pub(crate) fn get_repository_root(cwd: &Path) -> Result<PathBuf> {
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

pub(crate) fn resolve_comparison(
    repo_root: &Path,
    options: &CliOptions,
) -> Result<ResolvedComparison> {
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
