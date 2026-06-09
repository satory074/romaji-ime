//! Developer automation tasks. Run as `cargo run -p xtask -- <task>`.
//!
//! Tasks:
//!   gen-header   Regenerate crates/ime-ffi/include/romaji_ime.h with cbindgen.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR for xtask is <root>/xtask; its parent is the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent dir")
        .to_path_buf()
}

fn gen_header() -> Result<(), String> {
    let root = workspace_root();
    let crate_dir = root.join("crates/ime-ffi");
    let out_dir = crate_dir.join("include");
    let out_file = out_dir.join("romaji_ime.h");

    std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {out_dir:?}: {e}"))?;

    let config = cbindgen::Config::from_file(crate_dir.join("cbindgen.toml"))
        .map_err(|e| format!("read cbindgen.toml: {e}"))?;

    let bindings = cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .map_err(|e| format!("cbindgen generate: {e}"))?;

    bindings.write_to_file(&out_file);
    println!("wrote {}", out_file.display());
    Ok(())
}

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "gen-header" => gen_header(),
        other => {
            eprintln!("unknown task {other:?}");
            eprintln!("usage: cargo run -p xtask -- <gen-header>");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask error: {e}");
            ExitCode::FAILURE
        }
    }
}
