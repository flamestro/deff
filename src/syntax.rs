use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use once_cell::sync::Lazy;
use syntect::parsing::SyntaxSet;

const DEFAULT_RELATIVE_SYNTAX_DIRS: &[&str] = &["assets/syntaxes", ".deff/syntaxes"];
const ENV_SYNTAX_DIR: &str = "DEFF_SYNTAX_DIR";
const ENV_SYNTAX_PATHS: &str = "DEFF_SYNTAX_PATHS";

static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(load_syntax_set);

pub(crate) fn syntax_set() -> &'static SyntaxSet {
    &SYNTAX_SET
}

fn load_syntax_set() -> SyntaxSet {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();

    for directory in syntax_directories() {
        if let Err(error) = builder.add_from_folder(&directory, true) {
            eprintln!(
                "deff: ignoring syntax directory {}: {error}",
                directory.display()
            );
        }
    }

    builder.build()
}

fn syntax_directories() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/syntaxes"));
    candidates.extend(DEFAULT_RELATIVE_SYNTAX_DIRS.iter().map(PathBuf::from));

    if let Some(value) = std::env::var_os(ENV_SYNTAX_PATHS) {
        candidates.extend(std::env::split_paths(&value));
    }

    if let Some(value) = std::env::var_os(ENV_SYNTAX_DIR) {
        candidates.push(PathBuf::from(value));
    }

    let cwd = std::env::current_dir().ok();
    let mut unique = HashSet::new();
    let mut resolved = Vec::new();
    for candidate in candidates {
        let absolute = if candidate.is_relative() {
            match cwd.as_ref() {
                Some(directory) => directory.join(candidate),
                None => candidate,
            }
        } else {
            candidate
        };

        if !absolute.is_dir() {
            continue;
        }

        if unique.insert(absolute.clone()) {
            resolved.push(absolute);
        }
    }

    resolved
}
