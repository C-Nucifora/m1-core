//! Every script in the EV-M1 corpus must parse with zero syntax diagnostics.

use std::path::{Path, PathBuf};

/// The corpus directory. Override with the `M1_CORPUS_PATH` env var (e.g. in CI
/// or to point at a private corpus); otherwise defaults to the sibling EV-M1
/// example project.
fn corpus_dir() -> PathBuf {
    match std::env::var_os("M1_CORPUS_PATH") {
        Some(p) => PathBuf::from(p),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../EV-M1/UQR-EV/01.00/Scripts"),
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

#[test]
fn corpus_traversal_is_faithful() {
    let dir = corpus_dir();
    if !dir.is_dir() {
        eprintln!("corpus dir not found ({}); skipping", dir.display());
        return;
    }

    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("read corpus dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("m1scr") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read script");
        let cst = m1_core::parse(&src);

        // descendants() must equal a recursive children() pre-order walk.
        fn rec(n: m1_core::Node, out: &mut Vec<std::ops::Range<usize>>) {
            out.push(n.byte_range());
            for c in n.children() {
                rec(c, out);
            }
        }
        let mut reference = Vec::new();
        rec(cst.root(), &mut reference);
        let via_iter: Vec<_> = cst.root().descendants().map(|n| n.byte_range()).collect();
        assert_eq!(
            via_iter, reference,
            "descendants() diverged from recursive walk in {}",
            path.file_name().unwrap().to_string_lossy()
        );

        // node_at_offset must be total: sample offsets across the file.
        let step = (src.len() / 50).max(1);
        let mut off = 0;
        while off <= src.len() {
            let _ = cst.node_at_offset(off).kind();
            let _ = cst.named_node_at_offset(off).kind();
            off += step;
        }

        checked += 1;
    }
    assert!(checked >= 80, "expected >= 80 corpus scripts, found {checked}");
}
