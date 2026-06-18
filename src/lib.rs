//! `m1-core` — shared foundation for the MoTeC M1 (.m1scr) tooling.
//!
//! v1 provides the syntactic layer: [`parse`] returns a [`Cst`] that wraps the
//! tree-sitter tree behind m1-core's own [`Node`]/[`Kind`] types, plus a shared
//! [`Diagnostic`] type and syntax-error reporting. tree-sitter is an
//! implementation detail and is never exposed to consumers.

mod annotation;
mod cst;
mod diagnostic;
mod field;
mod kind;
mod kind_pred;
mod syntax;

pub use annotation::{Annotation, AnnotationArg, Annotations, MARKER, Registry, annotations};
pub use cst::{Children, Cst, Descendants, Edit, MAX_RECURSION_DEPTH, Node, parse};
pub use diagnostic::{Code, Diagnostic, Position, Range, Severity, byte_to_position};
pub use field::Field;
pub use kind::Kind;
pub use kind_pred::{is_assignment_op, is_binary_op, is_comment, is_compound_assign, is_unary_op};
