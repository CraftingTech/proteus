//! Creates a placeholder UI bundle when `proteus-ui` has not been built with `dx` yet.

use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into()));
    let dist = manifest_dir.join("../proteus-ui/dist");
    let index = dist.join("index.html");

    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-changed={}", index.display());

    if index.is_file() {
        return;
    }

    if let Err(err) = fs::create_dir_all(&dist) {
        println!("cargo:warning=could not create proteus-ui/dist: {err}");
        return;
    }

    let placeholder = r#"<!doctype html>
<html lang="en"><head><meta charset="utf-8"/><title>Proteus</title></head>
<body>
  <h1>Proteus UI not built</h1>
  <p>Run <code>just build-ui</code>, or use the release Dockerfile.</p>
</body></html>
"#;

    if let Err(err) = fs::write(&index, placeholder) {
        println!("cargo:warning=could not write placeholder proteus-ui/dist/index.html: {err}");
    }
}
