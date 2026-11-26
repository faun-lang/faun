//! Shifts by a distance worked out while the program runs, written as a
//! loop of shifts by one place where a target has no instruction for them,
//! where the value is wider than its word and splitting will hold it as
//! several -- a shift by one is what splitting knows how to carry across --
//! or where a shift down may be by the word or more, which an instruction
//! takes modulo the word. A target that shifts by a register is handed
//! only shifts it can do in one instruction.

use super::rewrite::{self, Pass, Rebuild};
use super::{Binary, Builder, Class, Direction, Op, Program, Relation, ValueId};

/// `program` with every shift by a worked-out distance a target on words of
/// `word_bits` cannot do in one instruction -- all of them, where `native`
/// says it has none -- written as a loop.
#[must_use]
pub fn expand_shifts(program: &Program, word_bits: u16, native: bool) -> Program {
    rewrite::rebuild(program, &mut Expand { word_bits, native })
}

struct Expand {
    word_bits: u16,
    native: bool,
}

impl Pass for Expand {
    fn op(&mut self, rebuild: &Rebuild<'_>, b: &mut Builder, op: Op, results: &[ValueId]) -> bool {
        let Op::Binary {
            op: op @ (Binary::ShiftUp | Binary::ShiftDown),
            left,
            right,
        } = op
        else {
            return false;
        };
        let class = rebuild.source().class(results[0]);
        if class == Class::Natural {
            return self.natural(rebuild, b, op, (left, right), results[0]);
        }
        let bits = class.width().unwrap_or(self.word_bits);
        let wide = bits > self.word_bits;
        let most = rebuild
            .source()
            .class(right)
            .facts()
            .and_then(|facts| facts.high);
        // An instruction moves by the distance modulo the word, which is the
        // distance itself where it is below the word. Up, a value moved a
        // word or more fits only where it is nought, which that answers too,
        // once the distance is read at a word.
        let below = most.is_some_and(|most| most < u64::from(self.word_bits));
        if self.native && !wide && (op == Binary::ShiftUp || below) {
            return self.in_one(rebuild, b, (op, class), (left, right), results[0]);
        }
        let direction = direction(op);
        let value = at_class(b, rebuild.value(left), class);
        let count = rebuild.value(right);
        let answer = if most.is_some_and(|most| most < u64::from(bits)) {
            self.moved(b, direction, (op, class), value, count)
        } else {
            self.clamped(b, direction, (op, class), value, count)
        };
        rebuild.stands_for(results[0], answer);
        true
    }
}

impl Expand {
    /// A natural's shift, which is the natural runtime's and legalization
    /// writes, handed a distance a word holds or a natural: one wider than
    /// the target's word is read as a natural, which it is exactly.
    /// Whether it was rewritten.
    fn natural(
        &self,
        rebuild: &Rebuild<'_>,
        b: &mut Builder,
        op: Binary,
        (left, right): (ValueId, ValueId),
        result: ValueId,
    ) -> bool {
        let by = rebuild.value(right);
        let wider = b
            .class(by)
            .width()
            .is_some_and(|width| width > self.word_bits);
        if !wider {
            return false;
        }
        let by = b.convert(Class::Natural, by);
        let moved = b.binary_at(Class::Natural, op, rebuild.value(left), by);
        rebuild.stands_for(result, moved);
        true
    }

    /// A shift a target makes in one instruction, its distance read at a
    /// word where it is wider than the target's: whether it was rewritten.
    fn in_one(
        &self,
        rebuild: &Rebuild<'_>,
        b: &mut Builder,
        (op, class): (Binary, Class),
        (left, right): (ValueId, ValueId),
        result: ValueId,
    ) -> bool {
        let held = b.class(rebuild.value(right));
        let in_word =
            held == Class::Word || held.width().is_some_and(|width| width <= self.word_bits);
        if in_word {
            return false;
        }
        let by = b.convert(Class::Word, rebuild.value(right));
        let moved = b.binary_at(class, op, rebuild.value(left), by);
        rebuild.stands_for(result, moved);
        true
    }

    /// `value` moved `count` places, which may be its width or more: that
    /// moves every bit out, down; up, it fits only a nought. Below it, the
    /// count is read in as few bits as hold it, so a loop of places goes
    /// round fewer times than the width.
    fn clamped(
        &self,
        b: &mut Builder,
        direction: Direction,
        (op, class): (Binary, Class),
        value: ValueId,
        count: ValueId,
    ) -> ValueId {
        let bits = class.width().unwrap_or(self.word_bits);
        let width = b.constant(Class::fixed(16), u64::from(bits));
        let near = super::arithmetic::compare(b, Relation::Less, count, width);
        b.if_(
            near,
            &[class],
            |b| {
                let needed = (u16::BITS - (bits - 1).leading_zeros()).max(1);
                let narrow = Class::fixed(u16::try_from(needed).expect("a width's bits"));
                let count = b.convert(narrow, count);
                let moved = self.moved(b, direction, (op, class), value, count);
                b.yield_(&[moved]);
            },
            |b| {
                let nought = b.constant(class, 0);
                b.yield_(&[nought]);
            },
        )[0]
    }

    /// `value` moved `count` places, which is below its width: one
    /// instruction where the target has it, and a loop of places otherwise.
    fn moved(
        &self,
        b: &mut Builder,
        direction: Direction,
        (op, class): (Binary, Class),
        value: ValueId,
        count: ValueId,
    ) -> ValueId {
        let wide = class.width().unwrap_or(self.word_bits) > self.word_bits;
        if self.native
            && !wide
            && b.class(count)
                .facts()
                .and_then(|facts| facts.high)
                .is_some_and(|most| most < u64::from(self.word_bits))
        {
            b.binary_at(class, op, value, count)
        } else {
            looped(b, direction, value, count)
        }
    }
}

/// Which way `op` moves bits.
fn direction(op: Binary) -> Direction {
    if op == Binary::ShiftUp {
        Direction::Up
    } else {
        Direction::Down
    }
}

/// `value` read at `class`.
fn at_class(b: &mut Builder, value: ValueId, class: Class) -> ValueId {
    if b.class(value) == class {
        value
    } else {
        b.convert(class, value)
    }
}

/// `value` moved `count` places, a place at a time.
fn looped(b: &mut Builder, direction: Direction, value: ValueId, count: ValueId) -> ValueId {
    let (class, counted) = (b.class(value), b.class(count));
    b.loop_(&[value, count], &[class], |b, state| {
        let (value, count) = (state[0], state[1]);
        let zero = b.constant(counted, 0);
        let done = b.compare(Relation::Equal, count, zero);
        b.leave_if(done, |b| b.break_(&[value]));
        let moved = b.shift(class, direction, value, 1);
        let one = b.constant(counted, 1);
        let rest = b.binary_at(counted, Binary::Sub, count, one);
        b.continue_(&[moved, rest]);
    })[0]
}
