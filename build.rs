use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

fn main() {
    let syntax_directory = Path::new("assets/syntaxes");
    println!("cargo:rerun-if-changed={}", syntax_directory.display());

    let syntax_files = collect_syntax_files(syntax_directory)
        .unwrap_or_else(|error| panic!("failed to scan {}: {error}", syntax_directory.display()));

    for syntax_file in &syntax_files {
        println!("cargo:rerun-if-changed={}", syntax_file.display());
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR is set by cargo"));
    let output_path = out_dir.join("bundled_syntaxes.rs");
    write_bundled_syntaxes(&output_path, &syntax_files)
        .unwrap_or_else(|error| panic!("failed to write {}: {error}", output_path.display()));
}

fn collect_syntax_files(directory: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![directory.to_path_buf()];

    while let Some(current_directory) = pending.pop() {
        for entry in fs::read_dir(&current_directory)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;

            if file_type.is_dir() {
                pending.push(path);
                continue;
            }

            if file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "sublime-syntax")
            {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn write_bundled_syntaxes(output_path: &Path, syntax_files: &[PathBuf]) -> io::Result<()> {
    let mut contents = String::from("const BUNDLED_SYNTAXES: &[(&str, &str)] = &[\n");

    for syntax_file in syntax_files {
        let file_name = syntax_file
            .file_name()
            .and_then(|name| name.to_str())
            .expect("syntax file name should be valid UTF-8");
        let relative_path = syntax_file.to_string_lossy().replace('\\', "/");

        contents.push_str(&format!(
            "    ({file_name:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{relative_path}\"))),\n"
        ));
    }

    contents.push_str("];\n");
    fs::write(output_path, contents)
}
