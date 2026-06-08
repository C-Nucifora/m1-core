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

/// Index of the top-level `)` in `s` (the arg list interior, after the `(`),
/// respecting double-quoted strings and nested parentheses. A `(` inside the
/// interior (e.g. an argument that is itself a call like `scale(x, 2)`) opens a
/// nested group whose `)` does not close the arg list. `None` if unmatched.
pub(super) fn find_close_paren(s: &str) -> Option<usize> {
    let mut in_str = false;
    let mut depth: usize = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
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
    let mut in_str = false;
    let mut depth: usize = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => depth = depth.saturating_sub(1),
            ',' if !in_str && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// Split `key=value` on the first top-level `=` (not inside quotes or a nested
/// `( … )` group), if present.
fn split_named(tok: &str) -> Option<(&str, &str)> {
    let mut in_str = false;
    let mut depth: usize = 0;
    for (i, c) in tok.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '(' if !in_str => depth += 1,
            ')' if !in_str => depth = depth.saturating_sub(1),
            '=' if !in_str && depth == 0 => return Some((&tok[..i], &tok[i + 1..])),
            _ => {}
        }
    }
    None
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
