//! Operator classification predicates over [`Kind`].
//!
//! Token-kind groupings the downstream tools (m1-fmt, m1-lint) previously each
//! duplicated. Centralising them here keeps the operator sets in lock-step with
//! the generated [`Kind`] enum and with one another, so a grammar change that
//! adds an operator is picked up everywhere.
//!
//! `Kind` itself is `@generated` from the grammar; these classifications are the
//! hand-maintained layer on top of it and so live in their own module.

use crate::kind::Kind;

/// Whether `k` is a binary (infix) operator token.
///
/// Covers the arithmetic, bitwise, shift, comparison and logical operators,
/// including the keyword spellings (`and`, `or`, `eq`, `neq`). The assignment
/// operators (`=`, `+=`, …) are *not* binary operators here; use
/// [`is_compound_assign`] for the compound forms.
pub fn is_binary_op(k: Kind) -> bool {
    matches!(
        k,
        // arithmetic
        Kind::Plus
            | Kind::Minus
            | Kind::Star
            | Kind::Slash
            | Kind::Percent
            // bitwise / shift
            | Kind::Amp
            | Kind::Pipe
            | Kind::Caret
            | Kind::LtLt
            | Kind::GtGt
            // comparison
            | Kind::Lt
            | Kind::Gt
            | Kind::LtEq
            | Kind::GtEq
            | Kind::EqEq
            | Kind::BangEq
            | Kind::Eq
            | Kind::Neq
            // logical
            | Kind::AmpAmp
            | Kind::PipePipe
            | Kind::And
            | Kind::Or
    )
}

/// Whether `k` is a compound-assignment operator token
/// (`+= -= *= /= %= &= |= ^= <<= >>=`).
pub fn is_compound_assign(k: Kind) -> bool {
    matches!(
        k,
        Kind::PlusEq
            | Kind::MinusEq
            | Kind::StarEq
            | Kind::SlashEq
            | Kind::PercentEq
            | Kind::AmpEq
            | Kind::PipeEq
            | Kind::CaretEq
            | Kind::LtLtEq
            | Kind::GtGtEq
    )
}

/// Whether `k` is any assignment operator token.
///
/// Covers the grammar's full `_assignment_operator` family: plain assignment
/// (`=`, [`Kind::Assign`]) plus the ten compound forms
/// (`+= -= *= /= %= &= |= ^= <<= >>=`). The manual likewise lists plain
/// assignment and the compound-assignment forms as one operator family, so this
/// is the predicate consumers want when asking "is this *any* assignment
/// operator?" rather than spelling out `Kind::Assign | is_compound_assign(k)`
/// by hand. For the compound forms alone, use [`is_compound_assign`].
pub fn is_assignment_op(k: Kind) -> bool {
    k == Kind::Assign || is_compound_assign(k)
}

/// Whether `k` is a unary (prefix) operator token (`! - ~`, plus the keyword
/// `not`).
///
/// `Kind::Minus` is both binary and unary (negation vs. subtraction); callers
/// distinguish the two by grammar context, not by the token kind alone.
pub fn is_unary_op(k: Kind) -> bool {
    matches!(k, Kind::Bang | Kind::Minus | Kind::Tilde | Kind::Not)
}

/// Whether `k` is a comment token (`Kind::LineComment` or `Kind::BlockComment`).
///
/// Comments are the M1 trivia category every tool must skip when walking
/// statements and expressions (the formatter, linter and LSP each previously
/// hand-rolled this `matches!`). Centralising it here keeps the comment set in
/// lock-step with the generated [`Kind`] enum, the same way [`is_binary_op`]
/// and friends centralise the operator sets.
pub fn is_comment(k: Kind) -> bool {
    matches!(k, Kind::LineComment | Kind::BlockComment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_ops_are_classified() {
        for k in [
            Kind::Plus,
            Kind::Minus,
            Kind::Star,
            Kind::Slash,
            Kind::Percent,
            Kind::Amp,
            Kind::Pipe,
            Kind::Caret,
            Kind::LtLt,
            Kind::GtGt,
            Kind::Lt,
            Kind::Gt,
            Kind::LtEq,
            Kind::GtEq,
            Kind::EqEq,
            Kind::BangEq,
            Kind::Eq,
            Kind::Neq,
            Kind::AmpAmp,
            Kind::PipePipe,
            Kind::And,
            Kind::Or,
        ] {
            assert!(is_binary_op(k), "{k:?} should be a binary op");
        }
    }

    #[test]
    fn non_binary_ops_are_rejected() {
        for k in [
            Kind::Assign,
            Kind::PlusEq,
            Kind::Bang,
            Kind::Tilde,
            Kind::Identifier,
            Kind::Number,
            Kind::LParen,
            Kind::Other,
        ] {
            assert!(!is_binary_op(k), "{k:?} should not be a binary op");
        }
    }

    #[test]
    fn compound_assigns_are_classified() {
        for k in [
            Kind::PlusEq,
            Kind::MinusEq,
            Kind::StarEq,
            Kind::SlashEq,
            Kind::PercentEq,
            Kind::AmpEq,
            Kind::PipeEq,
            Kind::CaretEq,
            Kind::LtLtEq,
            Kind::GtGtEq,
        ] {
            assert!(is_compound_assign(k), "{k:?} should be a compound assign");
        }
    }

    #[test]
    fn plain_assign_is_not_compound() {
        assert!(!is_compound_assign(Kind::Assign));
        assert!(!is_compound_assign(Kind::Plus));
        assert!(!is_compound_assign(Kind::EqEq));
    }

    #[test]
    fn assignment_ops_are_classified() {
        for k in [
            // plain assignment
            Kind::Assign,
            // the ten compound forms
            Kind::PlusEq,
            Kind::MinusEq,
            Kind::StarEq,
            Kind::SlashEq,
            Kind::PercentEq,
            Kind::AmpEq,
            Kind::PipeEq,
            Kind::CaretEq,
            Kind::LtLtEq,
            Kind::GtGtEq,
        ] {
            assert!(is_assignment_op(k), "{k:?} should be an assignment op");
        }
    }

    #[test]
    fn non_assignment_ops_are_rejected() {
        for k in [
            Kind::Plus,
            Kind::EqEq,
            Kind::BangEq,
            Kind::Lt,
            Kind::Identifier,
            Kind::Number,
        ] {
            assert!(!is_assignment_op(k), "{k:?} should not be an assignment op");
        }
    }

    #[test]
    fn unary_ops_are_classified() {
        for k in [Kind::Bang, Kind::Minus, Kind::Tilde, Kind::Not] {
            assert!(is_unary_op(k), "{k:?} should be a unary op");
        }
    }

    #[test]
    fn non_unary_ops_are_rejected() {
        for k in [Kind::Plus, Kind::Star, Kind::And, Kind::Identifier] {
            assert!(!is_unary_op(k), "{k:?} should not be a unary op");
        }
    }

    #[test]
    fn comments_are_classified() {
        for k in [Kind::LineComment, Kind::BlockComment] {
            assert!(is_comment(k), "{k:?} should be a comment");
        }
    }

    #[test]
    fn non_comments_are_rejected() {
        for k in [
            Kind::Identifier,
            Kind::Number,
            Kind::Plus,
            Kind::Other,
            Kind::LParen,
        ] {
            assert!(!is_comment(k), "{k:?} should not be a comment");
        }
    }
}
