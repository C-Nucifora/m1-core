//! Every script in the m1-example corpus must parse with zero syntax diagnostics.

use std::path::{Path, PathBuf};

/// The corpus directory. Override with the `M1_CORPUS_PATH` env var (e.g. in CI
/// or to point at a private corpus); otherwise defaults to the sibling m1-example
/// example project.
fn corpus_dir() -> PathBuf {
    match std::env::var_os("M1_CORPUS_PATH") {
        Some(p) => PathBuf::from(p),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../m1-example/UQR-EV/01.00/Scripts"),
    }
}

#[test]
fn ev_m1_corpus_parses_clean() {
    let dir = corpus_dir();
    assert!(dir.is_dir(), "corpus dir not found: {}", dir.display());

    let mut checked = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for entry in std::fs::read_dir(&dir).expect("read corpus dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("m1scr") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read script");
        let cst = m1_core::parse(&src);
        let diags = cst.syntax_diagnostics();
        checked += 1;
        if !diags.is_empty() {
            failures.push(format!(
                "{}: {} diagnostic(s), first at line {}",
                path.file_name().unwrap().to_string_lossy(),
                diags.len(),
                diags[0].range.start.line + 1
            ));
        }
    }

    assert!(checked >= 80, "expected >= 80 corpus scripts, found {checked}");
    assert!(failures.is_empty(), "scripts with syntax diagnostics:\n{}", failures.join("\n"));
}
