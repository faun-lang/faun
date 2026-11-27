//! Where each claim to a managed value is taken and given up, written into
//! the program as [`Op::Claim`] and [`Op::Release`].
//!
//! Every local managed result owns a claim, and so do loop parameters and
//! the parameters [`super::ownership`] finds owned; the rest borrow. A claim
//! is given up right after its value's last use in its region. A use that
//! takes a claim -- an exit, a cell field, a buffer an instruction builds
//! from, a loop's initial state, a same-class conversion, an owned
//! parameter -- moves the value's own claim where that use is its last, and
//! takes a new one otherwise. A value whose last use is an `if` hands its
//! claim to both arms, and each gives it up at its own last use. An exit
//! gives up whatever the scopes it leaves still hold.
//!
//! A managed value read out of a field takes a claim of its own there. An
//! immortal buffer, a cell borrowed from a caller-owned root, and a cell a
//! region constructor builds hold no claim of their own.
//!
//! Written here, every claim is an instruction a pass may read, count, and
//! take out where it can show the claim does nothing; legalization only
//! writes the counts. Nothing here changes what a program does or how many
//! claims it takes.

use std::cell::Cell;

use super::Builder;
use super::legalize::{immortal, packed};
use super::ownership::{self, Liveness, managed};
use super::region::Plan;
use super::rewrite::{Declarations, Rebuild, skeleton};
use super::{Class, Exit, FunctionId, Op, Program, RegionId, Terminator, ValueId};

/// `source` with every claim it takes and gives up written where it is,
/// for a target of `word_bits` whose heap grows where `grows` says: what
/// legalization reads, which `faun emit claims` prints.
#[must_use]
pub fn place(source: &Program, word_bits: u16, grows: bool) -> Program {
    let packed = packed(source, word_bits);
    let plan = if grows && word_bits == 64 {
        Plan::of(source, &packed)
    } else {
        Plan::default()
    };
    let owned = ownership::owned_parameters(source);
    let borrows = ownership::Borrows::of(source, &owned);
    let placing = Placing {
        borrows,
        rebuild: Rebuild::new(source),
        borrowed_cells: ownership::borrowed_cells(source, &owned),
        immortal: immortal(source),
        constructing: Cell::new(false),
        plan,
        owned,
    };
    let mut output = skeleton(source, Declarations::of(source));
    for (id, function) in source.functions.iter_enumerated() {
        let Some(body) = function.body() else {
            continue;
        };
        placing
            .constructing
            .set(placing.plan.constructors.contains(&id));
        let held = placing.owned_parameters(id, body);
        output.define(id, |b, params| {
            placing.region(b, body, &mut Vec::new(), ScopeKind::Ordinary, params, held);
        });
    }
    output
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeKind {
    Ordinary,
    /// Parameters own the claims carried by this iteration.
    Loop,
}

/// The claims a region holds at one point of its walk, by source value.
#[derive(Clone)]
struct Scope {
    held: Vec<ValueId>,
    kind: ScopeKind,
    /// A loop's parameters it borrows, by place, which go round holding no
    /// claim.
    borrowed: Vec<(usize, ValueId)>,
}

const fn top(scopes: &mut [Scope]) -> &mut Scope {
    scopes.last_mut().expect("a region being walked")
}

struct Placing<'a> {
    rebuild: Rebuild<'a>,
    /// Which parameters each loop borrows.
    borrows: ownership::Borrows,
    plan: Plan,
    owned: Vec<Vec<bool>>,
    borrowed_cells: Vec<bool>,
    immortal: Vec<bool>,
    /// Whether the function being walked builds region cells.
    constructing: Cell<bool>,
}

impl Placing<'_> {
    const fn source(&self) -> &Program {
        self.rebuild.source()
    }

    /// The parameters of a function's body that arrive holding a claim.
    fn owned_parameters(&self, id: FunctionId, body: RegionId) -> Vec<ValueId> {
        self.source()
            .values(self.source().region(body).params)
            .iter()
            .zip(ownership::taken(&self.owned, id))
            .filter(|&(_, &owned)| owned)
            .map(|(&param, _)| param)
            .collect()
    }

    fn region_cell(&self, old: ValueId) -> bool {
        self.constructing.get() && self.source().class(old) == Class::Cell
    }

    /// Whether a result holds a claim of its own once written.
    fn holds(&self, old: ValueId) -> bool {
        managed(self.source().class(old))
            && !self.borrowed_cells[old.index()]
            && !self.immortal[old.index()]
            && !self.region_cell(old)
    }

    /// Take a new claim to `old`, where it counts one.
    fn claim(&self, b: &mut Builder, old: ValueId) {
        if managed(self.source().class(old))
            && !self.region_cell(old)
            && !self.immortal[old.index()]
        {
            b.claim(self.rebuild.value(old));
        }
    }

    fn release(&self, b: &mut Builder, old: ValueId) {
        b.release(self.rebuild.value(old));
    }

    /// Walk one region holding `held` on entry: claims handed down by an
    /// enclosing `if`, owned parameters, or a loop's carried state.
    fn region(
        &self,
        b: &mut Builder,
        region: RegionId,
        scopes: &mut Vec<Scope>,
        kind: ScopeKind,
        params: &[ValueId],
        held: Vec<ValueId>,
    ) {
        let source = self.source();
        let old = source.region(region);
        for (&was, &now) in source.values(old.params).iter().zip(params) {
            self.rebuild.stands_for(was, now);
        }
        let live = Liveness::of(source, region);
        let borrowed = match kind {
            ScopeKind::Loop => source
                .values(old.params)
                .iter()
                .zip(self.borrows.of_loop(region))
                .enumerate()
                .filter(|&(_, (_, borrowed))| borrowed)
                .map(|(at, (&param, _))| (at, param))
                .collect(),
            ScopeKind::Ordinary => Vec::new(),
        };
        scopes.push(Scope {
            held,
            kind,
            borrowed,
        });
        self.release_dead(b, scopes, &live, None);
        let len = source.ops(region).len();
        for (at, (op, results)) in source.walk(region).enumerate() {
            self.op(b, *op, results, scopes, (&live, at));
            for &result in results {
                if self.holds(result) {
                    top(scopes).held.push(result);
                }
            }
            // A terminal loop, if or call has no continuation. Returns
            // from its child scopes have already given up their claims.
            if old.terminator.is_some() || at + 1 < len {
                self.release_dead(b, scopes, &live, Some(at));
            }
        }
        if let Some(terminator) = old.terminator {
            self.exit(b, scopes, terminator);
        }
        scopes.pop();
    }

    /// Give up what the innermost scope holds and nothing reads from `at`
    /// on.
    fn release_dead(
        &self,
        b: &mut Builder,
        scopes: &mut [Scope],
        live: &Liveness,
        at: Option<usize>,
    ) {
        let scope = top(scopes);
        let (dead, kept): (Vec<_>, Vec<_>) =
            scope.held.iter().partition(|&&value| live.dead(value, at));
        scope.held = kept;
        for &value in dead.iter().rev() {
            self.release(b, value);
        }
    }

    /// A claim to `old` for a use at `at` that takes one: its own, where
    /// this region holds it and this is its only and last reading, and a
    /// new one otherwise.
    fn take(
        &self,
        b: &mut Builder,
        scopes: &mut [Scope],
        live: &Liveness,
        at: usize,
        old: ValueId,
    ) {
        self.take_times(b, scopes, live, (at, 1), old);
    }

    /// Claims to each of `olds` for a use at `at` that takes them all: a
    /// value read nowhere else there and last there gives its own claim to
    /// its first place and a new one to each other.
    fn take_all(
        &self,
        b: &mut Builder,
        scopes: &mut [Scope],
        live: &Liveness,
        at: usize,
        olds: &[ValueId],
    ) {
        for (place, &old) in olds.iter().enumerate() {
            let times = olds.iter().filter(|&&other| other == old).count();
            if olds.iter().position(|&other| other == old) == Some(place) {
                self.take_times(b, scopes, live, (at, times), old);
            } else {
                self.claim(b, old);
            }
        }
    }

    fn take_times(
        &self,
        b: &mut Builder,
        scopes: &mut [Scope],
        live: &Liveness,
        (at, times): (usize, usize),
        old: ValueId,
    ) {
        let scope = top(scopes);
        if managed(self.source().class(old))
            && live.last_times(old, at, times)
            && let Some(held) = scope.held.iter().position(|&value| value == old)
        {
            scope.held.remove(held);
            return;
        }
        self.claim(b, old);
    }

    fn exit(&self, b: &mut Builder, scopes: &mut [Scope], terminator: Terminator) {
        let start = match terminator.exit {
            Exit::Return => 0,
            Exit::Yield => scopes.len() - 1,
            Exit::Break | Exit::Continue => scopes
                .iter()
                .rposition(|s| s.kind == ScopeKind::Loop)
                .expect("validated loop exit"),
        };
        let leaving = self.source().values(terminator.values);
        let looped = scopes[start].borrowed.clone();
        for (at, &old) in leaving.iter().enumerate() {
            // A borrowed parameter handed on in its own place holds no claim.
            if terminator.exit == Exit::Continue && looped.contains(&(at, old)) {
                continue;
            }
            // Everything the exited scopes hold dies here, so a claim one
            // of them holds moves out, unless it leaves twice.
            let once = leaving.iter().filter(|&&value| value == old).count() == 1;
            let held = scopes[start..].iter_mut().find_map(|scope| {
                let at = scope.held.iter().position(|&value| value == old)?;
                Some((scope, at))
            });
            match held {
                Some((scope, at)) if once => {
                    scope.held.remove(at);
                }
                _ => self.claim(b, old),
            }
        }
        for scope in scopes[start..].iter().rev() {
            for &value in scope.held.iter().rev() {
                self.release(b, value);
            }
        }
        let outgoing = self.rebuild.args(terminator.values);
        match terminator.exit {
            Exit::Yield => b.yield_(&outgoing),
            Exit::Return => b.ret(&outgoing),
            Exit::Continue => b.continue_(&outgoing),
            Exit::Break => b.break_(&outgoing),
        }
    }

    /// The claims an instruction takes, then the instruction.
    fn op(
        &self,
        b: &mut Builder,
        op: Op,
        results: &[ValueId],
        scopes: &mut [Scope],
        (live, at): (&Liveness, usize),
    ) {
        let source = self.source();
        let fresh = match op {
            Op::If {
                condition,
                then_region,
                else_region,
            } => self.branch(
                b,
                (condition, then_region, else_region),
                results,
                scopes,
                (live, at),
            ),
            Op::Loop { initial, body } => {
                self.looped(b, (initial, body), results, scopes, (live, at))
            }
            _ => {
                self.takes(b, op, results, scopes, (live, at));
                let fresh = self.rebuild.plain(b, op, results);
                // A managed value read out of a field is a holder of its own,
                // unless it is a cell borrowed from a caller-owned root.
                if matches!(op, Op::BufferField { .. } | Op::CellField { .. })
                    && managed(source.class(results[0]))
                    && !self.borrowed_cells[results[0].index()]
                {
                    b.claim(fresh[0]);
                }
                fresh
            }
        };
        self.stand(results, &fresh);
    }

    /// The claims an instruction holding no region takes from its operands.
    fn takes(
        &self,
        b: &mut Builder,
        op: Op,
        results: &[ValueId],
        scopes: &mut [Scope],
        (live, at): (&Liveness, usize),
    ) {
        let source = self.source();
        let taken: Vec<ValueId> = match op {
            Op::Cell { fields, .. } => source.values(fields).to_vec(),
            Op::BufferPush { operands, .. } | Op::BufferAppend { operands, .. } => {
                let operands = source.values(operands);
                std::iter::once(operands[0])
                    .chain(operands[3..].iter().copied())
                    .collect()
            }
            Op::BufferReplace { operands, .. } => {
                let operands = source.values(operands);
                std::iter::once(operands[0])
                    .chain(operands[4..].iter().copied())
                    .collect()
            }
            Op::BufferPrepend { operands, .. } | Op::BufferExtend { operands, .. } => {
                let operands = source.values(operands);
                self.take_all(b, scopes, live, at, &[operands[0], operands[3]]);
                return;
            }
            Op::Convert { value }
                if matches!(
                    (source.class(value), source.class(results[0])),
                    (Class::Natural, Class::Natural) | (Class::Cell, Class::Cell)
                ) =>
            {
                vec![value]
            }
            Op::Call { function, args } => source
                .values(args)
                .iter()
                .zip(ownership::taken(&self.owned, function))
                .filter(|&(_, &owns)| owns)
                .map(|(&old, _)| old)
                .collect(),
            _ => Vec::new(),
        };
        for old in taken {
            self.take(b, scopes, live, at, old);
        }
    }

    /// An `if`: what this region holds and reads last here is each arm's to
    /// give up, where that arm last reads it.
    fn branch(
        &self,
        b: &mut Builder,
        (condition, then_region, else_region): (ValueId, RegionId, RegionId),
        results: &[ValueId],
        scopes: &mut [Scope],
        (live, at): (&Liveness, usize),
    ) -> Vec<ValueId> {
        let scope = top(scopes);
        let (handed, kept): (Vec<_>, Vec<_>) = scope
            .held
            .iter()
            .partition(|&&value| live.dead(value, Some(at)));
        scope.held = kept;
        let arms: &[Scope] = scopes;
        let arm = |b: &mut Builder, region| {
            let mut scopes = arms.to_vec();
            self.region(
                b,
                region,
                &mut scopes,
                ScopeKind::Ordinary,
                &[],
                handed.clone(),
            );
        };
        b.if_(
            self.rebuild.value(condition),
            &self.rebuild.classes(results),
            |b| arm(b, then_region),
            |b| arm(b, else_region),
        )
    }

    /// A loop: its initial state taken, each round holding what it carries.
    fn looped(
        &self,
        b: &mut Builder,
        (initial, body): (super::Range, RegionId),
        results: &[ValueId],
        scopes: &mut [Scope],
        (live, at): (&Liveness, usize),
    ) -> Vec<ValueId> {
        let source = self.source();
        let params = source.values(source.region(body).params);
        let borrowed: Vec<bool> = self.borrows.of_loop(body).take(params.len()).collect();
        let carried: Vec<_> = params
            .iter()
            .zip(&borrowed)
            .filter(|&(&param, &borrowed)| managed(source.class(param)) && !borrowed)
            .map(|(&param, _)| param)
            .collect();
        let mut start = Vec::new();
        for ((&old, &param), &borrowed) in source.values(initial).iter().zip(params).zip(&borrowed)
        {
            if !borrowed {
                self.take(b, scopes, live, at, old);
            }
            let value = self.rebuild.value(old);
            let class = source.class(param);
            start.push(if b.class(value) == class {
                value
            } else {
                b.convert(class, value)
            });
        }
        let outer: &[Scope] = scopes;
        b.loop_(&start, &self.rebuild.classes(results), |b, now| {
            let mut scopes = outer.to_vec();
            self.region(b, body, &mut scopes, ScopeKind::Loop, now, carried.clone());
        })
    }

    fn stand(&self, results: &[ValueId], fresh: &[ValueId]) {
        for (&was, &now) in results.iter().zip(fresh) {
            self.rebuild.stands_for(was, now);
        }
    }
}

/// `source` without the claims that count nothing.
///
/// Those are claims to a natural held in its word, which no count is kept
/// for, and to a natural laid out before the program runs, whose count
/// never changes. Every claim to such a value goes, taken and given up
/// alike, so what is left balances as what was there did.
#[must_use]
pub fn elide(source: &Program, word_bits: u16) -> Program {
    let bits = natural_bits(source);
    let mut laid_out = vec![false; source.values_count()];
    for (id, _) in source.regions.iter_enumerated() {
        for (op, results) in source.walk(id) {
            if matches!(op, Op::NaturalConstant(_)) {
                for &value in results {
                    laid_out[value.index()] = true;
                }
            }
        }
    }
    let inert = |value: ValueId| {
        bits[value.index()].is_some_and(|bits| bits < u32::from(word_bits))
            || laid_out[value.index()]
    };
    let mut pass = Elide {
        drop: (0..source.values_count())
            .map(|at| source.class(ValueId::at(at)) == Class::Natural && inert(ValueId::at(at)))
            .collect(),
    };
    super::rewrite::rebuild(source, &mut pass)
}

struct Elide {
    drop: Vec<bool>,
}

impl super::rewrite::Pass for Elide {
    fn op(&mut self, _: &Rebuild<'_>, _: &mut Builder, op: Op, _: &[ValueId]) -> bool {
        match op {
            Op::Claim { value } | Op::Release { value } => self.drop[value.index()],
            _ => false,
        }
    }
}

/// The most bits an operation's answer can have, from its operands'.
fn binary_bits(op: super::Binary, l: Option<u32>, r: Option<u32>) -> Option<u32> {
    use super::Binary;
    match op {
        Binary::Add => l.zip(r).map(|(l, r)| l.max(r) + 1),
        Binary::Mul => l.zip(r).map(|(l, r)| l + r),
        Binary::Sub | Binary::Monus | Binary::Div | Binary::ShiftDown => l,
        Binary::Mod => l.zip(r).map(|(l, r)| l.min(r)).or(l),
        Binary::And => match (l, r) {
            (Some(l), Some(r)) => Some(l.min(r)),
            (l, r) => l.or(r),
        },
        Binary::Or | Binary::Xor => l.zip(r).map(|(l, r)| l.max(r)),
        Binary::ShiftUp => None,
    }
}

/// The most bits what an arm answers in place `at` can have, or nought
/// where it leaves some other way and answers nothing.
fn yielded_bits(
    source: &Program,
    region: RegionId,
    bits: &[Option<u32>],
    at: usize,
) -> Option<u32> {
    let terminator = source.region(region).terminator?;
    if terminator.exit != Exit::Yield {
        return Some(0);
    }
    let value = source.values(terminator.values)[at];
    match source.class(value) {
        Class::Natural => bits[value.index()],
        class => class.width().map(u32::from),
    }
}

/// The most bits each natural can have, where its instructions say: what
/// a narrower number converted holds, a constant's own, and what sums,
/// products, differences, quotients, remainders, bitwise ands and shifts
/// down of such numbers can come to; a branch answers the more of its arms'.
fn natural_bits(source: &Program) -> Vec<Option<u32>> {
    let mut bits: Vec<Option<u32>> = vec![None; source.values_count()];
    for function in source.functions() {
        if let Some(body) = function.body() {
            region_bits(source, body, &mut bits);
        }
    }
    bits
}

fn region_bits(source: &Program, region: RegionId, bits: &mut Vec<Option<u32>>) {
    for (op, results) in source.walk(region) {
        let of = |bits: &Vec<Option<u32>>, value: ValueId| -> Option<u32> {
            match source.class(value) {
                Class::Natural => bits[value.index()],
                class => class.width().map(u32::from),
            }
        };
        let answer = match *op {
            Op::Constant { value } => Some(u64::BITS - value.leading_zeros()),
            Op::Convert { value } => of(bits, value),
            Op::Binary { op, left, right } => binary_bits(op, of(bits, left), of(bits, right)),
            Op::If {
                then_region,
                else_region,
                ..
            } => {
                region_bits(source, then_region, bits);
                region_bits(source, else_region, bits);
                for (at, &result) in results.iter().enumerate() {
                    bits[result.index()] = yielded_bits(source, then_region, bits, at)
                        .zip(yielded_bits(source, else_region, bits, at))
                        .map(|(a, b)| a.max(b));
                }
                continue;
            }
            Op::Loop { body, .. } => {
                region_bits(source, body, bits);
                None
            }
            _ => None,
        };
        if let [result] = results {
            bits[result.index()] = answer;
        }
    }
}
