use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use once_cell::sync::Lazy;
use syntect::parsing::{SyntaxDefinition, SyntaxSet, SyntaxSetBuilder};

const DEFAULT_RELATIVE_SYNTAX_DIRS: &[&str] = &["assets/syntaxes", ".deff/syntaxes"];

include!(concat!(env!("OUT_DIR"), "/bundled_syntaxes.rs"));

static SYNTAX_SET: Lazy<SyntaxSet> = Lazy::new(load_syntax_set);

pub(crate) fn syntax_set() -> &'static SyntaxSet {
    &SYNTAX_SET
}

fn load_syntax_set() -> SyntaxSet {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    add_bundled_syntaxes(&mut builder);

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

fn add_bundled_syntaxes(builder: &mut SyntaxSetBuilder) {
    for (file_name, source) in BUNDLED_SYNTAXES {
        let fallback_name = Path::new(file_name)
            .file_stem()
            .and_then(|stem| stem.to_str());

        match SyntaxDefinition::load_from_str(source, true, fallback_name) {
            Ok(definition) => builder.add(definition),
            Err(error) => {
                eprintln!("deff: failed to load bundled syntax {}: {error}", file_name);
            }
        }
    }
}

fn syntax_directories() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    candidates.extend(DEFAULT_RELATIVE_SYNTAX_DIRS.iter().map(PathBuf::from));

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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use syntect::parsing::SyntaxDefinition;

    use super::{BUNDLED_SYNTAXES, load_syntax_set};

    #[test]
    fn every_bundled_syntax_file_is_loaded() {
        let syntaxes = load_syntax_set();

        for (file_name, source) in BUNDLED_SYNTAXES {
            let fallback_name = Path::new(file_name)
                .file_stem()
                .and_then(|stem| stem.to_str());
            let definition = SyntaxDefinition::load_from_str(source, true, fallback_name)
                .unwrap_or_else(|error| {
                    panic!("failed to parse bundled syntax {file_name}: {error}")
                });

            assert!(
                syntaxes.find_syntax_by_name(&definition.name).is_some(),
                "expected bundled syntax {} from {file_name}",
                definition.name
            );
        }
    }
}
