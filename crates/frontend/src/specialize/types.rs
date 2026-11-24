//! Concrete types, each held once.
//!
//! A [`Type`] is a handle to the one copy of its structure, the way a
//! [`Symbol`] is to its name: copying one copies a pointer, two are equal
//! when they are the same copy, and hashing one hashes the number it was
//! given when first made. Specializing writes a type at every expression it
//! makes and every function it keys, so these cost nothing to pass around.
//!
//! Each structure is made once, by [`Type::new`], and kept for the rest of
//! the process: a compilation's types are few, and many compilations in one
//! process share most of them.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{LazyLock, Mutex};

use rustc_hash::FxHashMap as HashMap;

use super::Signature;
use crate::Symbol;

/// A concrete type: a handle to the structure [`Type::kind`] reads.
#[derive(Clone, Copy)]
pub(crate) struct Type(&'static Node);

struct Node {
    /// Which this is of the types made so far, in the order they were.
    number: u32,
    kind: TypeKind,
}

/// What a concrete type is made of.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum TypeKind {
    Named(Symbol, Vec<Type>),
    Function(Signature),
    /// A closed tag row, its tags in the order of their names
    /// ([`Symbols::order`](crate::Symbols)), so that two spellings of one
    /// row are one type and a tag's number is fixed by the row alone. The
    /// empty row is the type with no values.
    Variants(Vec<(Symbol, Vec<Type>)>),
    /// A closed record, its fields in the order of their names, as it is
    /// built, compared, hashed and shown.
    Record(Vec<(Symbol, Type)>),
    /// A positional product of two or more.
    Tuple(Vec<Type>),
}

/// Every type made, by its structure.
static TYPES: LazyLock<Mutex<HashMap<&'static TypeKind, Type>>> =
    LazyLock::new(|| Mutex::new(HashMap::default()));

impl Type {
    /// The type made of `kind`: the one already made, or a new one.
    pub(crate) fn new(kind: TypeKind) -> Self {
        let mut types = TYPES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&ty) = types.get(&kind) {
            return ty;
        }
        let number = u32::try_from(types.len()).expect("fewer than 4G types");
        let node: &'static Node = Box::leak(Box::new(Node { number, kind }));
        let ty = Self(node);
        types.insert(&node.kind, ty);
        ty
    }

    /// What this type is made of.
    pub(crate) const fn kind(self) -> &'static TypeKind {
        &self.0.kind
    }

    pub(crate) fn named(name: Symbol, arguments: Vec<Self>) -> Self {
        Self::new(TypeKind::Named(name, arguments))
    }

    pub(crate) fn function(signature: Signature) -> Self {
        Self::new(TypeKind::Function(signature))
    }

    pub(crate) fn tuple(members: Vec<Self>) -> Self {
        Self::new(TypeKind::Tuple(members))
    }
}

impl PartialEq for Type {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.0, other.0)
    }
}

impl Eq for Type {}

impl Hash for Type {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.number.hash(state);
    }
}

/// Types are ordered by their structure, not by when they were made, so
/// what is kept in their order comes out the same in every compilation.
impl Ord for Type {
    fn cmp(&self, other: &Self) -> Ordering {
        if self == other {
            Ordering::Equal
        } else {
            self.kind().cmp(other.kind())
        }
    }
}

impl PartialOrd for Type {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}
