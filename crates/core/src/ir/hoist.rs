//! Building something every round of a loop that depends on nothing the
//! loop does, done once before it instead.
//!
//! A value known before the program runs and not held in the data, such as
//! a list of strings, is built where it is read: a buffer made empty and
//! filled from known bytes. Read inside a loop it is built again every
//! round, and let go every round, though every round builds the same one.
//!
//! What is worked out from constants alone is the same wherever it is
//! worked out, and doing it reads and writes nothing anything else sees: a
//! buffer or cell built from it is a fresh value, and so is a loop over
//! such values. So a construction of that kind inside a loop -- in its
//! body, an arm, or a loop within it -- is written before the loop, with
//! everything it is worked out from, and the loop reads the one built
//! there. Who holds a claim on it, and for how long, is legalization's to
//! work out from where it is read, as for any value read across a loop.
//!
//! Only constructions are moved: a number worked out from constants is
//! cheaper to work out again than to hold in a register across a loop.

use super::model::{Exit, Terminator};
use super::ownership::managed;
use super::rewrite::{self, Pass, Rebuild};
use super::{Builder, Class, Op, Program, RegionId, ValueId};

/// `source` with every construction from constants alone inside a loop
/// built once before the outermost loop holding it.
pub(super) fn hoist(source: &Program) -> Program {
    let constant = Constants::of(source);
    if !constant.any_construction {
        return source.clone();
    }
    rewrite::rebuild(
        source,
        &mut Hoist {
            constant: constant.values,
            writers: writers(source),
            moved: vec![false; source.values_count()],
            carried: Vec::new(),
        },
    )
}

/// Which values are worked out from constants alone.
struct Constants {
    values: Vec<bool>,
    any_construction: bool,
}

impl Constants {
    fn of(source: &Program) -> Self {
        let mut found = Self {
            values: vec![false; source.values_count()],
            any_construction: false,
        };
        for function in source.functions() {
            if let Some(body) = function.body() {
                found.region(source, body);
            }
        }
        found
    }

    /// Mark what `region`, and every region within it, works out from
    /// constants alone.
    fn region(&mut self, source: &Program, region: RegionId) {
        for (op, results) in source.walk(region) {
            let constant = match *op {
                Op::If {
                    condition,
                    then_region,
                    else_region,
                } => {
                    self.region(source, then_region);
                    self.region(source, else_region);
                    self.values[condition.index()]
                        && closed(source, then_region, &self.values)
                        && closed(source, else_region, &self.values)
                }
                Op::Loop { initial, body } => {
                    self.region(source, body);
                    source
                        .values(initial)
                        .iter()
                        .all(|value| self.values[value.index()])
                        && closed(source, body, &self.values)
                }
                _ => pure(op) && source.operands(op).all(|value| self.values[value.index()]),
            };
            if constant {
                for &value in results {
                    self.values[value.index()] = true;
                }
                self.any_construction |= constructs(source, op, results);
            }
        }
    }
}

/// Whether an instruction neither reads nor writes anything outside the
/// values it is handed, and cannot fail: what may be worked out anywhere
/// its operands are. Room in a buffer is not: it is made for something
/// outside to write into, so each is its own, wherever it is asked for.
const fn pure(op: &Op) -> bool {
    matches!(
        op,
        Op::Constant { .. }
            | Op::NaturalConstant(_)
            | Op::AddressOf(_)
            | Op::Cell { .. }
            | Op::CellTag { .. }
            | Op::CellField { .. }
            | Op::BufferEmpty { .. }
            | Op::BufferKnown { .. }
            | Op::BufferPush { .. }
            | Op::BufferAppend { .. }
            | Op::BufferReplace { .. }
            | Op::BufferPrepend { .. }
            | Op::BufferExtend { .. }
            | Op::BufferField { .. }
            | Op::Binary { .. }
            | Op::Compare { .. }
            | Op::Shift { .. }
            | Op::Convert { .. }
            | Op::Float { .. }
    )
}

/// Whether an instruction builds something worth building once: a buffer
/// or a cell it fills, or a branch or loop answering one. An empty buffer,
/// known bytes and a cell of no fields cost nothing to make again, and held
/// across a loop would only take a register.
fn constructs(source: &Program, op: &Op, results: &[ValueId]) -> bool {
    let fills = match *op {
        Op::BufferPush { .. }
        | Op::BufferAppend { .. }
        | Op::BufferReplace { .. }
        | Op::BufferPrepend { .. }
        | Op::BufferExtend { .. }
        | Op::If { .. }
        | Op::Loop { .. } => true,
        Op::Cell { fields, .. } => !source.values(fields).is_empty(),
        _ => false,
    };
    fills
        && results
            .iter()
            .any(|&value| matches!(source.class(value), Class::Buffer | Class::Cell))
}

/// Whether `region` reads nothing but constants and what it works out
/// itself, does nothing but pure work, and leaves only to the instruction
/// holding it.
fn closed(source: &Program, region: RegionId, constant: &[bool]) -> bool {
    let mut own = vec![false; source.values_count()];
    within(source, region, constant, &mut own)
}

fn within(source: &Program, region: RegionId, constant: &[bool], own: &mut [bool]) -> bool {
    let read = |own: &[bool], value: ValueId| constant[value.index()] || own[value.index()];
    let held = source.region(region);
    for &param in source.values(held.params) {
        own[param.index()] = true;
    }
    for (op, results) in source.walk(region) {
        let fine = match *op {
            Op::If {
                condition,
                then_region,
                else_region,
            } => {
                read(own, condition)
                    && within(source, then_region, constant, own)
                    && within(source, else_region, constant, own)
            }
            Op::Loop { initial, body } => {
                source.values(initial).iter().all(|&value| read(own, value))
                    && within(source, body, constant, own)
            }
            _ => pure(op) && source.operands(op).all(|value| read(own, value)),
        };
        if !fine {
            return false;
        }
        for &value in results {
            own[value.index()] = true;
        }
    }
    held.terminator.is_none_or(|terminator| {
        terminator.exit != Exit::Return
            && source
                .values(terminator.values)
                .iter()
                .all(|&value| read(own, value))
    })
}

struct Hoist {
    constant: Vec<bool>,
    /// What wrote each value, for following what a construction is worked
    /// out from.
    writers: Vec<Option<Op>>,
    /// The values written before a loop, which the loop reads there.
    moved: Vec<bool>,
    /// For each loop being rebuilt, innermost last, the values written
    /// before it that it carries round, handed on as they came wherever it
    /// goes round again.
    carried: Vec<Vec<ValueId>>,
}

impl Pass for Hoist {
    fn op(&mut self, rebuild: &Rebuild<'_>, b: &mut Builder, op: Op, results: &[ValueId]) -> bool {
        if !results.is_empty() && results.iter().all(|value| self.moved[value.index()]) {
            return true;
        }
        let Op::Loop { initial, body } = op else {
            return false;
        };
        let source = rebuild.source();
        self.write_before(rebuild, b, body);
        let carried = self.carried_by(source, body);
        let before: Vec<ValueId> = carried.iter().map(|&value| rebuild.value(value)).collect();
        let params = source.values(source.region(body).params).to_vec();
        let mut start = started(rebuild, b, initial, &params);
        start.extend(&before);
        self.carried.push(carried.clone());
        let fresh = b.loop_(&start, &rebuild.classes(results), |b, now| {
            let (own, round) = now.split_at(params.len());
            for (&was, &held) in carried.iter().zip(round) {
                rebuild.stands_for(was, held);
            }
            rebuild.region(b, body, own, self);
        });
        self.carried.pop();
        for (&was, &held) in carried.iter().zip(&before) {
            rebuild.stands_for(was, held);
        }
        for (&was, now) in results.iter().zip(fresh) {
            rebuild.stands_for(was, now);
        }
        true
    }

    fn exit(&mut self, rebuild: &Rebuild<'_>, b: &mut Builder, terminator: Terminator) -> bool {
        let Some(carried) = self.carried.last() else {
            return false;
        };
        if terminator.exit != Exit::Continue || carried.is_empty() {
            return false;
        }
        let mut values = rebuild.args(terminator.values);
        values.extend(carried.iter().map(|&value| rebuild.value(value)));
        b.continue_(&values);
        true
    }
}

impl Hoist {
    /// Write before a loop of body `body` every construction in it from
    /// constants alone, with what each is worked out from.
    fn write_before(&mut self, rebuild: &Rebuild<'_>, b: &mut Builder, body: RegionId) {
        let source = rebuild.source();
        let mut wanted = vec![false; source.values_count()];
        let mut order = Vec::new();
        self.roots(source, body, &mut wanted, &mut order);
        for (op, results) in order {
            if results.iter().all(|value| wanted[value.index()]) {
                // Written as it stands: it is closed, and goes round with
                // nothing of the loop it leaves.
                rebuild.copy(b, op, &results, &mut AsItStands);
                for &value in &results {
                    self.moved[value.index()] = true;
                }
            }
        }
    }

    /// What a loop of body `body` carries round: every value written before
    /// it, or before a loop holding it, that carries a count and that it
    /// reads. A claim is held by a loop's state, so such a value is read
    /// through the state, never from outside the loop.
    fn carried_by(&self, source: &Program, body: RegionId) -> Vec<ValueId> {
        let read = self.read_within(source, body);
        (0..source.values_count())
            .map(ValueId::at)
            .filter(|&value| {
                self.moved[value.index()] && read[value.index()] && managed(source.class(value))
            })
            .collect()
    }

    /// Every instruction in `region` and the regions within it worked out
    /// from constants alone, in the order they are written, not looking
    /// inside one that holds regions of its own; and, in `wanted`, the
    /// constructions among them and what each is worked out from.
    fn roots(
        &self,
        source: &Program,
        region: RegionId,
        wanted: &mut Vec<bool>,
        order: &mut Vec<(Op, Vec<ValueId>)>,
    ) {
        for (op, results) in source.walk(region) {
            // Already written before a loop holding this one.
            if !results.is_empty() && results.iter().all(|value| self.moved[value.index()]) {
                continue;
            }
            if !results.is_empty() && results.iter().all(|value| self.constant[value.index()]) {
                if constructs(source, op, results) {
                    for &value in results {
                        wanted[value.index()] = true;
                    }
                    self.want(source, op, wanted);
                }
                order.push((*op, results.to_vec()));
                continue;
            }
            match *op {
                Op::If {
                    then_region,
                    else_region,
                    ..
                } => {
                    self.roots(source, then_region, wanted, order);
                    self.roots(source, else_region, wanted, order);
                }
                Op::Loop { body, .. } => self.roots(source, body, wanted, order),
                _ => {}
            }
        }
    }

    /// Mark as wanted everything `op` is worked out from, and what that is
    /// worked out from in turn.
    fn want(&self, source: &Program, op: &Op, wanted: &mut [bool]) {
        let reads: Vec<ValueId> = match *op {
            Op::If {
                condition,
                then_region,
                else_region,
            } => {
                let mut reads = vec![condition];
                reads.extend(outside(source, then_region));
                reads.extend(outside(source, else_region));
                reads
            }
            Op::Loop { initial, body } => {
                let mut reads = source.values(initial).to_vec();
                reads.extend(outside(source, body));
                reads
            }
            _ => source.operands(op).collect(),
        };
        for value in reads {
            if !wanted[value.index()] {
                wanted[value.index()] = true;
                if let Some(writer) = self.writers[value.index()] {
                    self.want(source, &writer, wanted);
                }
            }
        }
    }
}

impl Hoist {
    /// Which values `region`, and the regions within it, read other than
    /// in what is written before the loop.
    fn read_within(&self, source: &Program, region: RegionId) -> Vec<bool> {
        let mut read = vec![false; source.values_count()];
        self.mark_reads(source, region, &mut read);
        read
    }

    fn mark_reads(&self, source: &Program, region: RegionId, read: &mut [bool]) {
        for (op, results) in source.walk(region) {
            if !results.is_empty() && results.iter().all(|value| self.moved[value.index()]) {
                continue;
            }
            for value in source.operands(op) {
                read[value.index()] = true;
            }
            match *op {
                Op::If {
                    condition,
                    then_region,
                    else_region,
                } => {
                    read[condition.index()] = true;
                    self.mark_reads(source, then_region, read);
                    self.mark_reads(source, else_region, read);
                }
                Op::Loop { initial, body } => {
                    for &value in source.values(initial) {
                        read[value.index()] = true;
                    }
                    self.mark_reads(source, body, read);
                }
                _ => {}
            }
        }
        if let Some(terminator) = source.region(region).terminator {
            for &value in source.values(terminator.values) {
                read[value.index()] = true;
            }
        }
    }
}

/// What `region`, and the regions within it, read that it does not work
/// out itself.
fn outside(source: &Program, region: RegionId) -> Vec<ValueId> {
    let mut own = vec![false; source.values_count()];
    let mut reads = Vec::new();
    gather(source, region, &mut own, &mut reads);
    reads
}

fn gather(source: &Program, region: RegionId, own: &mut [bool], reads: &mut Vec<ValueId>) {
    let held = source.region(region);
    for &param in source.values(held.params) {
        own[param.index()] = true;
    }
    for (op, results) in source.walk(region) {
        let read: Vec<ValueId> = match *op {
            Op::If {
                condition,
                then_region,
                else_region,
            } => {
                gather(source, then_region, own, reads);
                gather(source, else_region, own, reads);
                vec![condition]
            }
            Op::Loop { initial, body } => {
                gather(source, body, own, reads);
                source.values(initial).to_vec()
            }
            _ => source.operands(op).collect(),
        };
        reads.extend(read.into_iter().filter(|value| !own[value.index()]));
        for &value in results {
            own[value.index()] = true;
        }
    }
    if let Some(terminator) = held.terminator {
        let left: Vec<ValueId> = source.values(terminator.values).to_vec();
        reads.extend(left.into_iter().filter(|value| !own[value.index()]));
    }
}

/// What wrote each value, where an instruction did.
fn writers(source: &Program) -> Vec<Option<Op>> {
    let mut writers = vec![None; source.values_count()];
    for (id, _) in source.regions.iter_enumerated() {
        for (op, results) in source.walk(id) {
            for &value in results {
                writers[value.index()] = Some(*op);
            }
        }
    }
    writers
}

/// What a loop starts from, each read at the class its parameter has.
fn started(
    rebuild: &Rebuild<'_>,
    b: &mut Builder,
    initial: super::Range,
    params: &[ValueId],
) -> Vec<ValueId> {
    let source = rebuild.source();
    rebuild
        .args(initial)
        .into_iter()
        .zip(params)
        .map(|(value, &param)| {
            let class = source.class(param);
            if b.class(value) == class {
                value
            } else {
                b.convert(class, value)
            }
        })
        .collect()
}

/// Copies every instruction as it stands.
struct AsItStands;

impl Pass for AsItStands {
    fn op(&mut self, _: &Rebuild<'_>, _: &mut Builder, _: Op, _: &[ValueId]) -> bool {
        false
    }
}
