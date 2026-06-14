//! The small, self-contained argument parser for `@m1:` annotations.
//!
//! Pure string helpers — no CST, no diagnostics — that turn the interior of an
//! annotation's parenthesised arg list into [`AnnotationArg`]s, plus the comment
//! / paren / quote scanners they build on. Kept apart from the attachment and
//! registry logic in the parent module so the parsing is easy to test in
//! isolation.

use super::AnnotationArg;

/// Strip `//` or `/* … */` markers from a comment's raw text.
pub(super) fn strip_comment_markers(text: &str) -> &str {
    if let Some(rest) = text.strip_prefix("//") {
        rest
    } else if let Some(rest) = text.strip_prefix("/*") {
        rest.strip_suffix("*/").unwrap_or(rest)
    } else {
        text
    }
}

/// Walk `s` once, tracking double-quoted-string and nested-parenthesis state,
/// and invoke `on_break` at every character seen *outside* a string and at the
/// top level (paren depth 0). `'"'` toggles string state, `'('`/`')'` adjust
/// depth (saturating, so a stray `)` cannot underflow). The walk stops and
/// returns the byte index of the first character for which `on_break` returns
/// `true`; if none does, returns `None`.
///
/// This is the single quote-aware, depth-tracking state machine that
/// [`find_close_paren`], [`split_top_level`], and [`split_named`] all share —
/// they differ only in which top-level character they treat as a delimiter.
fn scan_top_level(s: &str, mut on_break: impl FnMut(usize, char) -> bool) -> Option<usize> {
    let mut in_str = false;
    let mut depth: usize = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str && depth > 0 => depth = depth.saturating_sub(1),
            _ if !in_str && depth == 0 && on_break(i, c) => return Some(i),
            _ => {}
        }
    }
    None
}

/// Index of the top-level `)` in `s` (the arg list interior, after the `(`),
/// respecting double-quoted strings and nested parentheses. A `(` inside the
/// interior (e.g. an argument that is itself a call like `scale(x, 2)`) opens a
/// nested group whose `)` does not close the arg list. `None` if unmatched.
pub(super) fn find_close_paren(s: &str) -> Option<usize> {
    scan_top_level(s, |_, c| c == ')')
}

/// Parse the interior of an arg list (between the parens) into arguments.
pub(super) fn parse_args(s: &str) -> Vec<AnnotationArg> {
    split_top_level(s)
        .into_iter()
        .filter_map(|tok| {
            let tok = tok.trim();
            if tok.is_empty() {
                return None;
            }
            match split_named(tok) {
                Some((k, v)) => Some(AnnotationArg::Named {
                    key: k.trim().to_string(),
                    value: strip_quotes(v.trim()).to_string(),
                }),
                None => Some(AnnotationArg::Positional(strip_quotes(tok).to_string())),
            }
        })
        .collect()
}

/// Split on top-level commas, respecting double-quoted strings and nested
/// parentheses (a comma inside a `( … )` group belongs to that argument).
fn split_top_level(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    // Collect every top-level comma; the callback never breaks (always `false`)
    // so the scan runs to the end of `s`.
    scan_top_level(s, |i, c| {
        if c == ',' {
            out.push(&s[start..i]);
            start = i + 1;
        }
        false
    });
    out.push(&s[start..]);
    out
}

/// Split `key=value` on the first top-level `=` (not inside quotes or a nested
/// `( … )` group), if present.
fn split_named(tok: &str) -> Option<(&str, &str)> {
    scan_top_level(tok, |_, c| c == '=').map(|i| (&tok[..i], &tok[i + 1..]))
}

/// Strip a single pair of surrounding double quotes, if present.
fn strip_quotes(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_and_block_markers() {
        assert_eq!(strip_comment_markers("// hi"), " hi");
        assert_eq!(strip_comment_markers("/* hi */"), " hi ");
        assert_eq!(strip_comment_markers("/* hi"), " hi");
        assert_eq!(strip_comment_markers("plain"), "plain");
    }

    #[test]
    fn finds_top_level_close_paren_past_quoted_paren() {
        // The `)` inside the string must be ignored.
        let s = "a, \"b)c\") tail";
        let i = find_close_paren(s).unwrap();
        assert_eq!(&s[..i], "a, \"b)c\"");
        assert_eq!(find_close_paren("no close here"), None);
    }

    #[test]
    fn parses_positional_named_and_quoted() {
        let args = parse_args("L010, min=-100, \"a, message\"");
        assert_eq!(
            args,
            vec![
                AnnotationArg::Positional("L010".into()),
                AnnotationArg::Named {
                    key: "min".into(),
                    value: "-100".into(),
                },
                AnnotationArg::Positional("a, message".into()),
            ]
        );
    }

    #[test]
    fn close_paren_respects_nested_parens() {
        // `s` is the interior after the arg list's opening `(`. The first `)`
        // here closes the inner `foo(x)`, not the arg list; the top-level close
        // is the `)` that follows `bar`.
        let s = "foo(x), bar) tail";
        let i = find_close_paren(s).unwrap();
        assert_eq!(&s[..i], "foo(x), bar");
        // Quoted parens still do not count toward depth.
        let q = "\"(\" ) end";
        let j = find_close_paren(q).unwrap();
        assert_eq!(&q[..j], "\"(\" ");
    }

    #[test]
    fn nested_parens_are_one_argument() {
        // A comma inside a nested-paren group must not split the argument, and
        // the whole call expression must survive intact.
        let args = parse_args("foo(a, b), bar");
        assert_eq!(
            args,
            vec![
                AnnotationArg::Positional("foo(a, b)".into()),
                AnnotationArg::Positional("bar".into()),
            ]
        );
    }

    #[test]
    fn scan_top_level_is_quote_and_depth_aware() {
        // The shared scanner skips delimiters inside strings and nested parens,
        // breaking only at a top-level match. All three public scanners are
        // thin wrappers over it, so this exercises the one state machine.
        let s = "a(b=1, \"c=d\"), e=f";
        // First top-level '=' is the one after `e`, not the ones inside the
        // nested call or the quoted string.
        let eq = scan_top_level(s, |_, c| c == '=').unwrap();
        assert_eq!(&s[..eq], "a(b=1, \"c=d\"), e");
        // First top-level ',' is the one after the closing paren of `a(...)`.
        let comma = scan_top_level(s, |_, c| c == ',').unwrap();
        assert_eq!(&s[..comma], "a(b=1, \"c=d\")");
        // No top-level ')' here (the only ')' closes the nested group).
        assert_eq!(scan_top_level(s, |_, c| c == ')'), None);
    }

    #[test]
    fn split_named_finds_first_top_level_equals() {
        assert_eq!(split_named("min=-100"), Some(("min", "-100")));
        // `=` inside a nested group is not a top-level key/value separator.
        assert_eq!(split_named("scale(x=1)"), None);
        // Quoted `=` is ignored; the real separator is the bare one.
        assert_eq!(split_named("k=\"a=b\""), Some(("k", "\"a=b\"")));
    }

    #[test]
    fn stray_top_level_close_paren_does_not_panic() {
        // A `)` with no matching `(` stays at depth 0 (saturating) and, for the
        // comma/equals scanners, is simply not a delimiter — it must not
        // underflow the depth counter.
        assert_eq!(split_top_level(")a,b"), vec![")a", "b"]);
        assert_eq!(split_named(")k=v"), Some((")k", "v")));
        // For find_close_paren, that leading `)` IS the top-level close.
        assert_eq!(find_close_paren(")a,b"), Some(0));
    }

    #[test]
    fn empty_and_whitespace_tokens_are_dropped() {
        assert!(parse_args("").is_empty());
        assert!(parse_args("  ").is_empty());
        assert_eq!(
            parse_args("a, , b"),
            vec![
                AnnotationArg::Positional("a".into()),
                AnnotationArg::Positional("b".into()),
            ]
        );
    }
}
