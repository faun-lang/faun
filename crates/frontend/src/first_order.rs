//! The first-order program resource checking and pattern compilation read.
//!
//! [`crate::callbacks`] leaves no function value: every callback is made
//! into the definition it is handed to, and every call of one is direct.
//! What is left is the specialized program with nothing but data flowing
//! between definitions, which these types say by being specialization's at
//! [`FirstOrder`], a phase with no values, so that no function value can be
//! built. Function signatures and the distinction between external
//! providers and defined bodies survive callback lowering.

use crate::specialize::{self as s, FirstOrder};

/// A specialized program whose values are all data.
pub(crate) type Program = s::Program<FirstOrder>;
pub(crate) type Function = s::Function<FirstOrder>;
pub(crate) type Definition = s::Definition<FirstOrder>;
pub(crate) type Clause = s::Clause<FirstOrder>;
pub(crate) type Body = s::Body<FirstOrder>;
pub(crate) type Binding = s::Binding<FirstOrder>;
pub(crate) type Expr = s::Expr<FirstOrder>;
pub(crate) type ExprKind = s::ExprKind<FirstOrder>;
pub(crate) type Arm = s::Arm<FirstOrder>;
