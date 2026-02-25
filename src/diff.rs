use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;

use crate::{
    git::{run_git, run_git_text},
    model::{
        DiffFileDescriptor, DiffFileView, FileContentSource, FileLineHighlights, ResolvedComparison,
    },
    review::compute_review_key,
    text::get_max_normalized_line_length,
};

const MISSING_LEFT: &str = "<file does not exist in base revision>";
const MISSING_RIGHT: &str = "<file does not exist in target revision>";
const BINARY_PLACEHOLDER: &str = "<binary file preview not available>";

static HUNK_HEADER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")
        .expect("hunk header regex should be valid")
});

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

pub(crate) fn get_diff_file_descriptors(
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
            &tracked_output,
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
        let untracked_paths = parse_null_separated_list(&untracked_output);

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
        &committed_output,
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

    let mut diff_args: Vec<OsString> = vec![
        OsString::from("diff"),
        OsString::from("--no-color"),
        OsString::from("--unified=0"),
    ];

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
            if is_binary_content(&output) {
                return vec![BINARY_PLACEHOLDER.to_string()];
            }

            split_into_lines(&String::from_utf8_lossy(&output))
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

    let path = PathBuf::from(file_path);
    let file_name = path.file_name()?.to_string_lossy().to_lowercase();
    if file_name == "dockerfile" {
        return Some("dockerfile".to_string());
    }

    let extension = path.extension()?.to_string_lossy().to_lowercase();
    extension_to_language(&extension).map(ToOwned::to_owned)
}

pub(crate) fn build_file_views(
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
            review_key: compute_review_key(descriptor, &left_lines, &right_lines),
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

#[cfg(test)]
mod tests {
    use crate::model::FileContentSource;

    use super::{
        parse_diff_name_status_output, parse_line_highlights_from_patch, split_into_lines,
    };

    #[test]
    fn parse_name_status_rename_entry() {
        let raw = b"R100\0old.txt\0new.txt\0";
        let descriptors = parse_diff_name_status_output(
            raw,
            FileContentSource::Commit,
            FileContentSource::Commit,
        );

        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].display_path, "old.txt -> new.txt");
    }

    #[test]
    fn parse_line_highlights_tracks_deleted_and_added_ranges() {
        let patch = "@@ -2,2 +5,3 @@";
        let highlights = parse_line_highlights_from_patch(patch);

        assert!(highlights.left_deleted_line_indexes.contains(&1));
        assert!(highlights.left_deleted_line_indexes.contains(&2));
        assert!(highlights.right_added_line_indexes.contains(&4));
        assert!(highlights.right_added_line_indexes.contains(&6));
    }

    #[test]
    fn split_into_lines_trims_trailing_newline() {
        let lines = split_into_lines("a\nb\n");
        assert_eq!(lines, vec!["a".to_string(), "b".to_string()]);
    }
}
