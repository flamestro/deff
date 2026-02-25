use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    git::run_git_text,
    model::{DiffFileDescriptor, DiffFileView, ResolvedComparison},
};

const REVIEW_DIRECTORY: &str = "deff/reviewed";
const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

struct StableHasher {
    state: u64,
}

impl StableHasher {
    fn new() -> Self {
        Self {
            state: FNV_OFFSET_BASIS,
        }
    }

    fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
        self.write_bytes(&[0]);
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.state ^= u64::from(*byte);
            self.state = self.state.wrapping_mul(FNV_PRIME);
        }
    }

    fn finish_hex(&self) -> String {
        format!("{:016x}", self.state)
    }
}

fn get_git_dir(repo_root: &Path) -> Result<PathBuf> {
    let git_dir = run_git_text(["rev-parse", "--git-dir"], repo_root)?;
    let parsed = PathBuf::from(git_dir.trim());
    if parsed.is_absolute() {
        Ok(parsed)
    } else {
        Ok(repo_root.join(parsed))
    }
}

fn comparison_scope_key(comparison: &ResolvedComparison) -> String {
    let mut hasher = StableHasher::new();
    hasher.write_str(&comparison.strategy_id.to_string());
    hasher.write_str(&comparison.base_ref);
    hasher.write_str(&comparison.head_ref);
    hasher.write_str(if comparison.includes_uncommitted {
        "uncommitted"
    } else {
        "committed"
    });
    hasher.finish_hex()
}

fn parse_reviewed_hashes(raw: &str) -> HashSet<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn persist_reviewed_hashes(path: &Path, reviewed_hashes: &HashSet<String>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    let mut entries: Vec<&str> = reviewed_hashes.iter().map(String::as_str).collect();
    entries.sort_unstable();

    let mut output = entries.join("\n");
    if !output.is_empty() {
        output.push('\n');
    }

    fs::write(path, output)
        .with_context(|| format!("failed to write review state {}", path.display()))
}

pub(crate) fn compute_review_key(
    descriptor: &DiffFileDescriptor,
    left_lines: &[String],
    right_lines: &[String],
) -> String {
    let mut hasher = StableHasher::new();

    hasher.write_str(&descriptor.raw_status);
    hasher.write_str(&descriptor.display_path);
    hasher.write_str(descriptor.base_path.as_deref().unwrap_or(""));
    hasher.write_str(descriptor.head_path.as_deref().unwrap_or(""));

    for line in left_lines {
        hasher.write_str("L");
        hasher.write_str(line);
    }

    for line in right_lines {
        hasher.write_str("R");
        hasher.write_str(line);
    }

    hasher.finish_hex()
}

pub(crate) struct ReviewStore {
    path: PathBuf,
    reviewed_hashes: HashSet<String>,
}

impl ReviewStore {
    pub(crate) fn load(repo_root: &Path, comparison: &ResolvedComparison) -> Result<Self> {
        let git_dir = get_git_dir(repo_root)?;
        let scope_key = comparison_scope_key(comparison);
        let path = git_dir
            .join(REVIEW_DIRECTORY)
            .join(format!("{scope_key}.txt"));

        let reviewed_hashes = match fs::read_to_string(&path) {
            Ok(raw) => parse_reviewed_hashes(&raw),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashSet::new(),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read review state {}", path.display()));
            }
        };

        Ok(Self {
            path,
            reviewed_hashes,
        })
    }

    pub(crate) fn reviewed_flags_for_files(&self, files: &[DiffFileView]) -> Vec<bool> {
        files
            .iter()
            .map(|file| self.reviewed_hashes.contains(&file.review_key))
            .collect()
    }

    pub(crate) fn set_reviewed(&mut self, review_key: &str, reviewed: bool) {
        if reviewed {
            self.reviewed_hashes.insert(review_key.to_string());
        } else {
            self.reviewed_hashes.remove(review_key);
        }
    }

    pub(crate) fn persist(&self) -> Result<()> {
        persist_reviewed_hashes(&self.path, &self.reviewed_hashes)
    }
}

#[cfg(test)]
mod tests {
    use super::{compute_review_key, parse_reviewed_hashes, persist_reviewed_hashes};
    use crate::model::{DiffFileDescriptor, FileContentSource};
    use std::{
        collections::HashSet,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_temp_file_path() -> PathBuf {
        let now_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("deff-review-test-{now_nanos}.txt"))
    }

    #[test]
    fn parse_reviewed_hashes_ignores_empty_lines() {
        let parsed = parse_reviewed_hashes("abc\n\n  \ndef\n");
        assert!(parsed.contains("abc"));
        assert!(parsed.contains("def"));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn persist_round_trip_writes_sorted_lines() {
        let path = unique_temp_file_path();
        let mut hashes = HashSet::new();
        hashes.insert("bbb".to_string());
        hashes.insert("aaa".to_string());

        persist_reviewed_hashes(&path, &hashes).expect("persist should succeed");
        let raw = fs::read_to_string(&path).expect("saved file should be readable");
        assert_eq!(raw, "aaa\nbbb\n");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn review_key_changes_when_file_content_changes() {
        let descriptor = DiffFileDescriptor {
            raw_status: "M".to_string(),
            display_path: "src/main.rs".to_string(),
            base_path: Some("src/main.rs".to_string()),
            head_path: Some("src/main.rs".to_string()),
            base_source: FileContentSource::Commit,
            head_source: FileContentSource::Commit,
        };

        let first = compute_review_key(&descriptor, &["a".to_string()], &["b".to_string()]);
        let second = compute_review_key(&descriptor, &["a".to_string()], &["c".to_string()]);

        assert_ne!(first, second);
    }
}
