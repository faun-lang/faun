//! Where managed values are used last, and which parameters a function owns.
//!
//! Legalization places reference-count operations from these facts. A claim
//! is released right after its value's last use in the region holding it,
//! and a use that takes a claim -- an exit, a cell field, a loop's initial
//! state, an owned parameter -- takes the value's own claim when that use
//! is its last. That is what lets a count of one mean that nothing else can
//! see a value, which is what updating one in place will ask.
//!
//! A parameter is owned when its function takes a claim to it somewhere:
//! the caller then hands over a claim, moving its own where the call is the
//! argument's last use. A parameter nothing consumes is borrowed, so reading
//! one costs its caller nothing. Owning is decided to a fixed point, since
//! passing a parameter on to an owned one consumes it.

use rustc_hash::FxHashMap as HashMap;

use super::{Class, FunctionId, Op, Program, RegionId, ValueId};

/// Whether a value's class carries a reference count.
pub(super) const fn managed(class: Class) -> bool {
    matches!(class, Class::Cell | Class::Buffer | Class::Natural)
}

/// Every value an instruction reads, including everything read inside the
/// regions it holds and what they leave with, once per reading.
pub(super) fn deep_uses(source: &Program, op: &Op, out: &mut Vec<ValueId>) {
    out.extend(source.operands(op));
    match *op {
        Op::If {
            then_region,
            else_region,
            ..
        } => {
            region_uses(source, then_region, out);
            region_uses(source, else_region, out);
        }
        Op::Loop { body, .. } => region_uses(source, body, out),
        _ => {}
    }
}

fn region_uses(source: &Program, region: RegionId, out: &mut Vec<ValueId>) {
    for (op, _) in source.walk(region) {
        deep_uses(source, op, out);
    }
    out.extend_from_slice(source.outgoing(region));
}

/// Where each value is read in one region: by which instruction last, with
/// the region's exit counted as the position after its last instruction,
/// and what each instruction reads.
pub(super) struct Liveness {
    last: HashMap<ValueId, usize>,
    uses: Vec<Vec<ValueId>>,
}

impl Liveness {
    pub(super) fn of(source: &Program, region: RegionId) -> Self {
        let mut last = HashMap::default();
        let mut uses = Vec::new();
        for (at, (op, _)) in source.walk(region).enumerate() {
            let mut read = Vec::new();
            deep_uses(source, op, &mut read);
            for &value in &read {
                last.insert(value, at);
            }
            uses.push(read);
        }
        let exit = uses.len();
        for &value in source.outgoing(region) {
            last.insert(value, exit);
        }
        Self { last, uses }
    }

    /// Whether nothing at or after position `at` reads `value`; `None` asks
    /// about the region's entry, before its first instruction.
    pub(super) fn dead(&self, value: ValueId, at: Option<usize>) -> bool {
        match (self.last.get(&value), at) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(&last), Some(at)) => last <= at,
        }
    }

    /// Whether instruction `at` is the last use of `value` and reads it
    /// exactly `times` times, each a use that takes a claim: one of them may
    /// then take its own, and the rest new ones, leaving nothing else in
    /// that instruction reading a released value.
    pub(super) fn last_times(&self, value: ValueId, at: usize, times: usize) -> bool {
        self.last.get(&value) == Some(&at)
            && self.uses[at].iter().filter(|&&read| read == value).count() == times
    }
}

/// Which parameters of each function are owned, by function and position.
/// External declarations own nothing.
pub(super) fn owned_parameters(source: &Program) -> Vec<Vec<bool>> {
    let mut owned: Vec<Vec<bool>> = source
        .functions()
        .iter()
        .map(|function| vec![false; function.params.len()])
        .collect();
    loop {
        let mut changed = false;
        let borrows = Borrows::of(source, &owned);
        for (id, function) in source.functions.iter_enumerated() {
            let Some(body) = function.body() else {
                continue;
            };
            let mut consumed = Vec::new();
            consumed_in(source, body, (&owned, &borrows), &mut consumed);
            for (at, &param) in source.values(source.region(body).params).iter().enumerate() {
                if managed(source.class(param))
                    && !owned[id.index()][at]
                    && consumed.contains(&param)
                {
                    owned[id.index()][at] = true;
                    changed = true;
                }
            }
        }
        if !changed {
            return owned;
        }
    }
}

/// Cells kept alive by the caller for the entire activation. An immutable
/// cell owns its child, so projecting another cell from a borrowed one
/// extends the same borrow, including inside a branch or loop. Joins,
/// carried state and call results already receive claims on transfer.
pub(super) fn borrowed_cells(source: &Program, owned: &[Vec<bool>]) -> Vec<bool> {
    let mut borrowed = vec![false; source.values_count()];
    for (id, function) in source.functions.iter_enumerated() {
        let Some(body) = function.body() else {
            continue;
        };
        for (&param, &owns) in source
            .values(source.region(body).params)
            .iter()
            .zip(taken(owned, id))
        {
            borrowed[param.index()] = source.class(param) == Class::Cell && !owns;
        }
        for (op, results) in source.walk_within(body) {
            if let Op::CellField { cell, .. } = *op
                && source.class(results[0]) == Class::Cell
            {
                borrowed[results[0].index()] = borrowed[cell.index()];
            }
        }
    }
    borrowed
}

/// What an instruction building a sequence from `operands` takes a claim
/// to. A push takes its buffer and its fields, past the start and length,
/// and a replacement past its index too; and a prepend or a join both
/// buffers, either of which may become the result: a prepend onto nothing
/// of a buffer nothing else holds turns it round where it lies.
fn sequence_taken(op: &Op, operands: &[ValueId], out: &mut Vec<ValueId>) {
    match op {
        Op::BufferPush { .. } | Op::BufferAppend { .. } => {
            out.push(operands[0]);
            out.extend_from_slice(&operands[3..]);
        }
        Op::BufferReplace { .. } => {
            out.push(operands[0]);
            out.extend_from_slice(&operands[4..]);
        }
        Op::BufferPrepend { .. } | Op::BufferExtend { .. } => {
            out.extend([operands[0], operands[3]]);
        }
        _ => unreachable!("only a sequence's instructions build one from operands"),
    }
}

/// The values some instruction in a region, or its exit, takes a claim to.
fn consumed_in(
    source: &Program,
    region: RegionId,
    (owned, borrows): (&[Vec<bool>], &Borrows),
    out: &mut Vec<ValueId>,
) {
    for (op, results) in source.walk(region) {
        taken_by(source, op, results, (owned, borrows), out);
        match *op {
            Op::If {
                then_region,
                else_region,
                ..
            } => {
                consumed_in(source, then_region, (owned, borrows), out);
                consumed_in(source, else_region, (owned, borrows), out);
            }
            Op::Loop { body, .. } => consumed_in(source, body, (owned, borrows), out),
            _ => {}
        }
    }
    out.extend_from_slice(source.outgoing(region));
}

/// The values one instruction takes a claim to itself, not counting what
/// the regions it holds take: a cell its fields, a sequence's instruction
/// what [`sequence_taken`] says, a conversion within one class its value,
/// a call what it hands an owned parameter, and a loop its initial state.
pub(super) fn taken_by(
    source: &Program,
    op: &Op,
    results: &[ValueId],
    (owned, borrows): (&[Vec<bool>], &Borrows),
    out: &mut Vec<ValueId>,
) {
    match *op {
        Op::Cell { fields, .. } => out.extend_from_slice(source.values(fields)),
        Op::BufferPush { operands, .. }
        | Op::BufferAppend { operands, .. }
        | Op::BufferReplace { operands, .. }
        | Op::BufferPrepend { operands, .. }
        | Op::BufferExtend { operands, .. } => {
            sequence_taken(op, source.values(operands), out);
        }
        Op::Convert { value } if source.class(value) == source.class(results[0]) => {
            out.push(value);
        }
        Op::Call { function, args } => out.extend(
            source
                .values(args)
                .iter()
                .zip(taken(owned, function))
                .filter(|&(_, &owned)| owned)
                .map(|(&arg, _)| arg),
        ),
        Op::Loop { initial, body } => out.extend(
            source
                .values(initial)
                .iter()
                .zip(borrows.of_loop(body))
                .filter(|&(_, borrowed)| !borrowed)
                .map(|(&value, _)| value),
        ),
        _ => {}
    }
}

/// Which parameters of a loop of body `body` it borrows rather than owns:
/// each a managed value every way round hands on as it came, in its own
/// place, and nothing in the loop takes, hands elsewhere or leaves with.
/// The value it started from lives past the loop, so no claim of the
/// loop's own is needed to keep it: none is taken on the way in, none
/// carried round, and none given up on the way out.
fn borrowed_loop(
    source: &Program,
    body: RegionId,
    (owned, borrows): (&[Vec<bool>], &Borrows),
) -> Vec<bool> {
    let params = source.values(source.region(body).params);
    let mut passed = vec![true; params.len()];
    let mut consumed = Vec::new();
    loop_reads(
        source,
        body,
        (params, true),
        (owned, borrows),
        (&mut passed, &mut consumed),
    );
    params
        .iter()
        .zip(passed)
        .map(|(&param, passed)| {
            passed && managed(source.class(param)) && !consumed.contains(&param)
        })
        .collect()
}

/// What `region` takes or leaves with, but for the loop's own parameters
/// handed on in their own places, which `passed` records; `own` says
/// whether a `continue` here is the loop's, rather than one within it's.
fn loop_reads(
    source: &Program,
    region: RegionId,
    (params, own): (&[ValueId], bool),
    (owned, borrows): (&[Vec<bool>], &Borrows),
    (passed, consumed): (&mut Vec<bool>, &mut Vec<ValueId>),
) {
    for (op, results) in source.walk(region) {
        taken_by(source, op, results, (owned, borrows), consumed);
        match *op {
            Op::If {
                then_region,
                else_region,
                ..
            } => {
                loop_reads(
                    source,
                    then_region,
                    (params, own),
                    (owned, borrows),
                    (passed, consumed),
                );
                loop_reads(
                    source,
                    else_region,
                    (params, own),
                    (owned, borrows),
                    (passed, consumed),
                );
            }
            Op::Loop { body, .. } => {
                loop_reads(
                    source,
                    body,
                    (params, false),
                    (owned, borrows),
                    (passed, consumed),
                );
            }
            _ => {}
        }
    }
    if let Some(terminator) = source.region(region).terminator {
        leaving_reads(source, terminator, (params, own), (passed, consumed));
    }
}

/// What a region's exit leaves with: a parameter of the loop handed on in
/// its own place by the loop's `continue`, which `passed` keeps, or else
/// something the loop gives away.
fn leaving_reads(
    source: &Program,
    terminator: super::Terminator,
    (params, own): (&[ValueId], bool),
    (passed, consumed): (&mut [bool], &mut Vec<ValueId>),
) {
    let leaving = source.values(terminator.values);
    if !own || terminator.exit != super::Exit::Continue {
        consumed.extend_from_slice(leaving);
        return;
    }
    for (at, &value) in leaving.iter().enumerate() {
        if params.get(at) == Some(&value) {
            continue;
        }
        if let Some(passed) = passed.get_mut(at) {
            *passed = false;
        }
        consumed.push(value);
    }
}

/// Whether each parameter of `function` is owned.
pub(super) fn taken(owned: &[Vec<bool>], function: FunctionId) -> &[bool] {
    &owned[function.index()]
}

/// Which parameters each loop borrows, by its body, worked out once for
/// the whole program: a loop's after every loop within it, which reading
/// it asks of.
pub(super) struct Borrows(Vec<Option<Vec<bool>>>);

impl Borrows {
    pub(super) fn of(source: &Program, owned: &[Vec<bool>]) -> Self {
        let mut borrows = Self(vec![None; source.regions.len()]);
        for function in source.functions() {
            if let Some(body) = function.body() {
                borrows.visit(source, body, owned);
            }
        }
        borrows
    }

    fn visit(&mut self, source: &Program, region: RegionId, owned: &[Vec<bool>]) {
        for (op, _) in source.walk(region) {
            match *op {
                Op::If {
                    then_region,
                    else_region,
                    ..
                } => {
                    self.visit(source, then_region, owned);
                    self.visit(source, else_region, owned);
                }
                Op::Loop { body, .. } => {
                    self.visit(source, body, owned);
                    let borrowed = borrowed_loop(source, body, (owned, self));
                    self.0[body.index()] = Some(borrowed);
                }
                _ => {}
            }
        }
    }

    /// Which parameters the loop of body `body` borrows, in order.
    pub(super) fn of_loop(&self, body: RegionId) -> impl Iterator<Item = bool> + '_ {
        self.0[body.index()]
            .iter()
            .flatten()
            .copied()
            .chain(std::iter::repeat(false))
    }
}
