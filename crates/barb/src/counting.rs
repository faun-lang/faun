//! Recognize counts of a word's bits from the arithmetic that works them
//! out, before choosing a word size.
//!
//! A definition taking one number, or one number wrapped, and answering a
//! natural is read into terms: what calls it makes are followed into their
//! bodies, a call whose every input is known is worked out, and a wrapper is
//! looked through. The terms are then read as arithmetic modulo two to the
//! power of a width, and recognized where they are the count Hacker's
//! Delight gives, without a loop:
//!
//! - **ones:** a pair, then a nibble, then a byte at a time, and the bytes
//!   summed by one multiplication, `(bytes * 0x0101..) >> (width - 8)`, or
//!   for a byte the nibbles summed;
//! - **trailing zeros:** the count of ones of `!w & (w - 1)`, the bits below
//!   the lowest set one, all of them for nought;
//! - **leading zeros:** the width less the count of ones of the word with
//!   its highest set bit copied into every place below it, by or-ing it
//!   with itself moved down by each power of two below the width.
//!
//! Each holds at nought: nought has no ones, and the width for either count
//! of zeros. Nothing is recognized by its name, and anything spelled
//! another way -- a loop clearing the lowest set bit until none is left, a
//! table of bytes -- is not recognized, and runs its body as written.
use num_bigint::BigUint;
use num_traits::ToPrimitive;
use std::rc::Rc;

use super::budget::{Budget, FoldLimits};
use super::constant::Held;
use super::evaluate::Evaluator;
use super::{
    Atom, Body, ConstantView, Expr, ExprId, FnId, Operation, Program, TypeShape, type_shape,
};

/// Which count of a word's bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Count {
    /// How many bits are set.
    Ones,
    /// How many bits are clear below the lowest set one: the width for
    /// nought.
    TrailingZeros,
    /// How many bits are clear above the highest set one: the width for
    /// nought.
    LeadingZeros,
}

impl Count {
    const ALL: [Self; 3] = [Self::Ones, Self::TrailingZeros, Self::LeadingZeros];

    const fn bit(self) -> u8 {
        match self {
            Self::Ones => 1,
            Self::TrailingZeros => 2,
            Self::LeadingZeros => 4,
        }
    }
}

/// A count of the bits of a number below two to the power of `bits`.
///
/// The width is one of 8, 16, 32 and 64, and the answer a natural of at
/// most `bits`. The input must fit in `bits`; Facts checks that range
/// before exposing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BitCount {
    pub count: Count,
    pub bits: u16,
}

impl BitCount {
    /// What it answers for `value`, read at its width.
    #[must_use]
    pub fn calculate(self, value: u64) -> u64 {
        let unused = 64 - u32::from(self.bits);
        let value = value & (u64::MAX >> unused);
        let answer = match self.count {
            Count::Ones => value.count_ones(),
            Count::TrailingZeros if value == 0 => u32::from(self.bits),
            Count::TrailingZeros => value.trailing_zeros(),
            Count::LeadingZeros => value.leading_zeros() - unused,
        };
        u64::from(answer)
    }
}

impl std::fmt::Display for BitCount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = match self.count {
            Count::Ones => "count ones",
            Count::TrailingZeros => "trailing zeros",
            Count::LeadingZeros => "leading zeros",
        };
        write!(f, "{count} {}", self.bits)
    }
}

/// Which counts a target answers with an instruction of its own, rather
/// than by walking the definition: see [`Program::answer_counts`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Counts(u8);

impl Counts {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(1 | 2 | 4);

    /// These and `count` as well.
    #[must_use]
    pub const fn with(self, count: Count) -> Self {
        Self(self.0 | count.bit())
    }

    /// These but not those.
    #[must_use]
    pub const fn without(self, those: Self) -> Self {
        Self(self.0 & !those.0)
    }

    #[must_use]
    pub const fn has(self, count: Count) -> bool {
        self.0 & count.bit() != 0
    }

    /// Each count it holds.
    pub fn iter(self) -> impl Iterator<Item = Count> {
        Count::ALL.into_iter().filter(move |&count| self.has(count))
    }
}

/// A definition's answer, read as terms of its one parameter.
enum Term {
    Parameter,
    Known(Held),
    Call(Operation, Rc<Self>, Rc<Self>),
}

/// How many expressions one definition's reading may visit, through every
/// body it follows: the definitions recognized take a few hundred.
const READ: usize = 2048;
/// How deep a reading follows calls into bodies.
const DEPTH: usize = 8;

struct Reader<'a> {
    program: &'a Program,
    operations: &'a [Option<Operation>],
    remaining: usize,
    depth: usize,
}

impl Reader<'_> {
    fn body(&mut self, body: Body, mut scope: Vec<Rc<Term>>) -> Option<Rc<Term>> {
        for &id in self.program.bindings(body.bindings) {
            scope.push(self.expr(id, &scope)?);
        }
        self.expr(body.tail, &scope)
    }

    const fn evaluator(&self) -> Evaluator<'_> {
        Evaluator {
            program: self.program,
            operations: self.operations,
        }
    }

    const fn budget() -> Budget {
        Budget::new(FoldLimits {
            work: 100_000,
            work_per_function: 100_000,
            max_bytes: 1024,
            max_depth: 64,
        })
    }

    fn expr(&mut self, id: ExprId, scope: &[Rc<Term>]) -> Option<Rc<Term>> {
        self.remaining = self.remaining.checked_sub(1)?;
        let atom = |a: Atom| scope[a.0.index()].clone();
        match self.program.expr(id) {
            Expr::Atom(a) => Some(atom(a)),
            Expr::Static(value) => Some(Rc::new(Term::Known(
                self.program
                    .constant(value)
                    .to_owned(&mut Self::budget())
                    .ok()?,
            ))),
            Expr::Call(callee, args) => {
                let args: Vec<_> = self.program.atoms(args).iter().map(|&a| atom(a)).collect();
                self.call(callee, args)
            }
            Expr::Con(ctor, fields) => {
                let fields: Vec<_> = self
                    .program
                    .atoms(fields)
                    .iter()
                    .map(|&a| atom(a))
                    .collect();
                self.construct(ctor, &fields)
            }
            Expr::Match(on, arms) => self.matched(atom(on), arms, scope),
        }
    }

    /// A value built where its fields are known, and otherwise what a
    /// wrapper holds, which it adds nothing to.
    fn construct(&self, ctor: super::CtorId, fields: &[Rc<Term>]) -> Option<Rc<Term>> {
        if let Some(known) = known_all(fields) {
            let held = self
                .evaluator()
                .construct(ctor, &known, &mut Self::budget())
                .ok()?;
            return Some(Rc::new(Term::Known(held)));
        }
        matches!(
            type_shape(self.program, self.program.ctor(ctor).owner),
            TypeShape::Wrapper { .. }
        )
        .then(|| fields[0].clone())
    }

    /// The arm a known value takes, or what a wrapper holds; a match on
    /// anything else is not followed.
    fn matched(
        &mut self,
        on: Rc<Term>,
        arms: super::Range,
        scope: &[Rc<Term>],
    ) -> Option<Rc<Term>> {
        let arms = self.program.arms(arms);
        let mut inner = scope.to_vec();
        if let Term::Known(held) = &*on {
            let (ctor, fields) = held
                .as_constant(self.program.type_info())
                .parts(&mut Self::budget())
                .ok()?;
            let arm = arms.iter().find(|arm| arm.ctor == ctor)?;
            inner.extend(fields.into_iter().map(|held| Rc::new(Term::Known(held))));
            return self.body(arm.body, inner);
        }
        let [arm] = arms else {
            return None;
        };
        if !matches!(
            type_shape(self.program, self.program.ctor(arm.ctor).owner),
            TypeShape::Wrapper { .. }
        ) {
            return None;
        }
        inner.push(on);
        self.body(arm.body, inner)
    }

    /// A call worked out where everything it is handed is known, an
    /// operation where it is one, and otherwise its body, followed.
    fn call(&mut self, callee: FnId, args: Vec<Rc<Term>>) -> Option<Rc<Term>> {
        if let Some(known) = known_all(&args) {
            let held = self
                .evaluator()
                .call(callee, known, &mut Self::budget())
                .ok()?;
            return Some(Rc::new(Term::Known(held)));
        }
        if let Some(operation) = self.operations[callee.index()] {
            let [a, b] = <[_; 2]>::try_from(args).ok()?;
            return Some(Rc::new(Term::Call(operation, a, b)));
        }
        let body = self.program.function(callee).body()?;
        self.depth = self.depth.checked_sub(1)?;
        let answer = self.body(body, args);
        self.depth += 1;
        answer
    }
}

/// What each term is, where every one is known.
fn known_all(terms: &[Rc<Term>]) -> Option<Vec<Held>> {
    terms
        .iter()
        .map(|term| match &**term {
            Term::Known(held) => Some(held.clone()),
            _ => None,
        })
        .collect()
}

/// A word's arithmetic, modulo two to the power of its width.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Word {
    /// The parameter.
    Input,
    Constant(u64),
    Not(Box<Self>),
    And(Box<Self>, Box<Self>),
    Or(Box<Self>, Box<Self>),
    Add(Box<Self>, Box<Self>),
    Sub(Box<Self>, Box<Self>),
    Mul(Box<Self>, Box<Self>),
    /// Moved down this many places.
    Down(Box<Self>, u32),
}

/// How many word terms reading one definition may write: sharing is lost
/// in a word's tree, so a definition reusing what it reuses doubles it.
const WORDS: usize = 4096;

/// Terms read as words at one width.
struct Words<'a> {
    program: &'a Program,
    bits: u16,
    remaining: usize,
}

impl Words<'_> {
    fn number(&self, term: &Term) -> Option<BigUint> {
        let Term::Known(held) = term else {
            return None;
        };
        let value =
            super::single_field_constant(self.program, held.as_constant(self.program.type_info()));
        let ConstantView::Natural(number) = value.view() else {
            return None;
        };
        Some(number.to_biguint())
    }

    fn modulus(&self) -> BigUint {
        BigUint::from(1u8) << self.bits
    }

    fn spend(&mut self) -> Option<()> {
        self.remaining = self.remaining.checked_sub(1)?;
        Some(())
    }

    /// The word `term` is, where its value is that word's and below the
    /// modulus.
    fn exact(&mut self, term: &Term) -> Option<Word> {
        self.spend()?;
        match term {
            Term::Parameter => Some(Word::Input),
            Term::Known(_) => {
                let number = self.number(term)?;
                (number < self.modulus()).then(|| Word::Constant(number.to_u64().unwrap_or(0)))
            }
            Term::Call(Operation::Mod, value, modulus) => {
                (self.number(modulus)? == self.modulus()).then_some(())?;
                self.modular(value)
            }
            Term::Call(Operation::Monus, all, value)
                if self.number(all) == Some(self.modulus() - 1u8) =>
            {
                Some(Word::Not(Box::new(self.exact(value)?)))
            }
            Term::Call(Operation::Div, value, divisor) => {
                let divisor = self.number(divisor)?;
                if divisor.count_ones() != 1 {
                    return None;
                }
                let by = u32::try_from(divisor.trailing_zeros()?).ok()?;
                Some(Word::Down(Box::new(self.exact(value)?), by))
            }
            // Only the bits one side has can be set in both, so the other
            // need only agree below the modulus.
            Term::Call(Operation::And, a, b) => {
                let (exact, other) = match self.exact(a) {
                    Some(word) => (word, b),
                    None => (self.exact(b)?, a),
                };
                Some(Word::And(Box::new(exact), Box::new(self.modular(other)?)))
            }
            Term::Call(Operation::Or, a, b) => {
                Some(Word::Or(Box::new(self.exact(a)?), Box::new(self.exact(b)?)))
            }
            Term::Call(..) => None,
        }
    }

    /// A word whose value agrees with `term`'s below the modulus.
    fn modular(&mut self, term: &Term) -> Option<Word> {
        if let Some(word) = self.exact(term) {
            return Some(word);
        }
        self.spend()?;
        let modulus = self.modulus();
        match term {
            Term::Known(_) => {
                let number = self.number(term)? % &modulus;
                Some(Word::Constant(number.to_u64().unwrap_or(0)))
            }
            Term::Call(Operation::Add, a, b) => {
                let (a, b) = (self.modular(a)?, self.modular(b)?);
                Some(match (a, b) {
                    (Word::Constant(0), word) | (word, Word::Constant(0)) => word,
                    (a, b) => Word::Add(Box::new(a), Box::new(b)),
                })
            }
            Term::Call(Operation::Mul, a, b) => Some(Word::Mul(
                Box::new(self.modular(a)?),
                Box::new(self.modular(b)?),
            )),
            Term::Call(Operation::And, a, b) => Some(Word::And(
                Box::new(self.modular(a)?),
                Box::new(self.modular(b)?),
            )),
            Term::Call(Operation::Or, a, b) => Some(Word::Or(
                Box::new(self.modular(a)?),
                Box::new(self.modular(b)?),
            )),
            Term::Call(Operation::Monus, from, taken) => self.difference(from, taken),
            _ => None,
        }
    }

    /// `from` less `taken` as a word's difference, where nothing is
    /// truncated: where what is taken is below the modulus, and what it is
    /// taken from is at least the modulus, or the modulus more than
    /// something.
    fn difference(&mut self, from: &Term, taken: &Term) -> Option<Word> {
        let modulus = self.modulus();
        let taken = self.exact(taken)?;
        let from = match from {
            Term::Call(Operation::Add, a, b) => {
                let (rest, more) = if self.number(b).is_some_and(|n| n >= modulus) {
                    (a, b)
                } else if self.number(a).is_some_and(|n| n >= modulus) {
                    (b, a)
                } else {
                    return None;
                };
                let more = self.modular(more)?;
                let rest = self.modular(rest)?;
                match more {
                    Word::Constant(0) => rest,
                    more => Word::Add(Box::new(rest), Box::new(more)),
                }
            }
            Term::Known(_) if self.number(from)? >= modulus => self.modular(from)?,
            _ => return None,
        };
        Some(Word::Sub(Box::new(from), Box::new(taken)))
    }
}

const fn and(word: &Word) -> Option<(&Word, &Word)> {
    match word {
        Word::And(a, b) => Some((a, b)),
        _ => None,
    }
}

const fn add(word: &Word) -> Option<(&Word, &Word)> {
    match word {
        Word::Add(a, b) => Some((a, b)),
        _ => None,
    }
}

const fn or(word: &Word) -> Option<(&Word, &Word)> {
    match word {
        Word::Or(a, b) => Some((a, b)),
        _ => None,
    }
}

const fn mul(word: &Word) -> Option<(&Word, &Word)> {
    match word {
        Word::Mul(a, b) => Some((a, b)),
        _ => None,
    }
}

/// What `word` keeps of where it is that and'ed with `mask`, either way
/// round.
fn masked(word: &Word, mask: u64) -> Option<&Word> {
    let (a, b) = and(word)?;
    match (a, b) {
        (value, Word::Constant(m)) | (Word::Constant(m), value) if *m == mask => Some(value),
        _ => None,
    }
}

/// `value` where `word` is it moved down `by` places.
fn down(word: &Word, by: u32) -> Option<&Word> {
    match word {
        Word::Down(value, moved) if *moved == by => Some(value),
        _ => None,
    }
}

/// A byte repeated across `bits`.
fn repeated(byte: u8, bits: u16) -> u64 {
    (0..bits / 8).fold(0, |word, at| word | u64::from(byte) << (at * 8))
}

/// What `word` counts the ones of, where it is Hacker's Delight's count at
/// `bits`.
fn ones(word: &Word, bits: u16) -> Option<&Word> {
    let bytes = if bits == 8 {
        word
    } else {
        let (a, b) = mul(down(word, u32::from(bits) - 8)?)?;
        match (a, b) {
            (bytes, Word::Constant(m)) | (Word::Constant(m), bytes)
                if *m == repeated(0x01, bits) =>
            {
                bytes
            }
            _ => return None,
        }
    };
    // bytes = (nibbles + nibbles >> 4) & 0x0f..
    let (a, b) = add(masked(bytes, repeated(0x0f, bits))?)?;
    let nibbles = if down(b, 4) == Some(a) {
        a
    } else if down(a, 4) == Some(b) {
        b
    } else {
        return None;
    };
    // nibbles = (pairs & 0x33..) + (pairs >> 2 & 0x33..)
    let (a, b) = add(nibbles)?;
    let (a, b) = (
        masked(a, repeated(0x33, bits))?,
        masked(b, repeated(0x33, bits))?,
    );
    let pairs = if down(b, 2) == Some(a) {
        a
    } else if down(a, 2) == Some(b) {
        b
    } else {
        return None;
    };
    // pairs = value - (value >> 1 & 0x55..)
    let Word::Sub(value, halved) = pairs else {
        return None;
    };
    (down(masked(halved, repeated(0x55, bits))?, 1) == Some(value)).then_some(value)
}

/// Whether `word` is the bits below the input's lowest set one,
/// `!w & (w - 1)`, either way round.
fn below_lowest(word: &Word) -> bool {
    let less = Word::Sub(Box::new(Word::Input), Box::new(Word::Constant(1)));
    let not = Word::Not(Box::new(Word::Input));
    and(word).is_some_and(|(a, b)| (*a == not && *b == less) || (*a == less && *b == not))
}

/// Whether `word` is the input with its highest set bit copied into every
/// place below it: or-ed with itself moved down by each power of two below
/// the width, once each, in any order.
fn smeared(mut word: &Word, bits: u16) -> bool {
    let mut moved = 0u32;
    while *word != Word::Input {
        let Some((a, b)) = or(word) else {
            return false;
        };
        let (rest, by) = match (a, b) {
            (rest, Word::Down(again, by)) | (Word::Down(again, by), rest) if **again == *rest => {
                (rest, *by)
            }
            _ => return false,
        };
        if !by.is_power_of_two() || by >= u32::from(bits) || moved & by != 0 {
            return false;
        }
        moved |= by;
        word = rest;
    }
    moved == u32::from(bits) - 1
}

/// What `term` counts at `bits`, if it is a count.
fn count(program: &Program, term: &Term, bits: u16) -> Option<Count> {
    let mut words = Words {
        program,
        bits,
        remaining: WORDS,
    };
    if let Term::Call(Operation::Monus, width, counted) = term
        && words.number(width) == Some(BigUint::from(bits))
    {
        let counted = words.exact(counted)?;
        return smeared(ones(&counted, bits)?, bits).then_some(Count::LeadingZeros);
    }
    let word = words.exact(term)?;
    let counted = ones(&word, bits)?;
    if *counted == Word::Input {
        Some(Count::Ones)
    } else {
        below_lowest(counted).then_some(Count::TrailingZeros)
    }
}

/// Whether `ty` is a natural, or one wrapped.
fn natural(program: &Program, ty: super::TypeId) -> bool {
    let mut ty = ty;
    for _ in 0..=program.types().len() {
        match type_shape(program, ty) {
            TypeShape::Wrapper { field, .. } => ty = field,
            TypeShape::Natural { .. } => return true,
            _ => return false,
        }
    }
    false
}

/// Only bounded structural reading runs here. Names and machine widths are
/// absent.
pub(super) fn recognize(
    program: &Program,
    operations: &[Option<Operation>],
) -> Vec<Option<BitCount>> {
    (0..program.functions().len())
        .map(|at| {
            let id = FnId::at(at);
            let function = program.function(id);
            let &[param] = program.params(id) else {
                return None;
            };
            if operations[at].is_some()
                || !natural(program, param)
                || !matches!(
                    type_shape(program, function.result),
                    TypeShape::Natural { .. }
                )
            {
                return None;
            }
            let mut reader = Reader {
                program,
                operations,
                remaining: READ,
                depth: DEPTH,
            };
            let answer = reader.body(function.body()?, vec![Rc::new(Term::Parameter)])?;
            [8, 16, 32, 64].into_iter().find_map(|bits| {
                count(program, &answer, bits).map(|count| BitCount { count, bits })
            })
        })
        .collect()
}
