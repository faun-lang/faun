//! A run of branches on one condition, taken as one branch.
//!
//! Lowering writes a branch for each value an arm answers, so a lookup that
//! can miss -- a read past an end answering a default -- asks its condition
//! twice, or more, in a row: once for the element, once for the answer
//! built from it. Each branch is a test and a jump, and a condition read by
//! more than one is worked out into a register a target then tests, rather
//! than branched on where it is asked.
//!
//! Branches on one condition, one straight after another, each arm of
//! which answers rather than leaving, are one branch: its first arm does
//! what each first arm did, in order, and its second what each second arm
//! did, each reading what those before it answered there. What the
//! program does is the same, with one test and one jump.

use rustc_hash::FxHashMap as HashMap;

use super::model::Exit;
use super::rewrite::{self, Pass, Rebuild};
use super::{Builder, Op, Program, RegionId, ValueId};

/// `source` with each run of branches on one condition merged into one,
/// answering no more than `transfers` values: a run answering more is
/// merged a stretch at a time, each within it.
#[must_use]
pub fn merge_ifs(source: &Program, transfers: usize) -> Program {
    let runs = runs(source, transfers);
    if runs.is_empty() {
        return source.clone();
    }
    rewrite::rebuild(
        source,
        &mut Merge {
            runs,
            merged: vec![false; source.values_count()],
        },
    )
}

/// The branches of a run: each one's arms and results, in order.
type Run = Vec<(RegionId, RegionId, Vec<ValueId>)>;

/// Every run of two or more branches on one condition, each arm of which
/// answers and which together answer at most `transfers` values, by the
/// first branch's first result: a run whose first branch answers nothing is
/// left as it is.
fn runs(source: &Program, transfers: usize) -> HashMap<ValueId, (ValueId, Run)> {
    let mut found = HashMap::default();
    for (region, _) in source.regions.iter_enumerated() {
        let ops: Vec<_> = source.walk(region).collect();
        let mut at = 0;
        while at < ops.len() {
            let Some(condition) = branch_on(source, ops[at].0) else {
                at += 1;
                continue;
            };
            let mut end = at + 1;
            let mut answered = ops[at].1.len();
            while end < ops.len()
                && branch_on(source, ops[end].0) == Some(condition)
                && answered + ops[end].1.len() <= transfers
            {
                answered += ops[end].1.len();
                end += 1;
            }
            if end - at > 1
                && let Some(&first) = ops[at].1.first()
            {
                let run = ops[at..end]
                    .iter()
                    .map(|&(op, results)| arms(op, results))
                    .collect();
                found.insert(first, (condition, run));
            }
            at = end;
        }
    }
    found
}

/// A branch's arms and results.
fn arms(op: &Op, results: &[ValueId]) -> (RegionId, RegionId, Vec<ValueId>) {
    let Op::If {
        then_region,
        else_region,
        ..
    } = *op
    else {
        unreachable!("a run is of branches");
    };
    (then_region, else_region, results.to_vec())
}

/// The condition a branch both of whose arms answer is on.
fn branch_on(source: &Program, op: &Op) -> Option<ValueId> {
    let Op::If {
        condition,
        then_region,
        else_region,
    } = *op
    else {
        return None;
    };
    let answers = |region: RegionId| {
        let region = source.region(region);
        region.params.is_empty() && region.terminator.is_some_and(|t| t.exit == Exit::Yield)
    };
    (answers(then_region) && answers(else_region)).then_some(condition)
}

struct Merge {
    /// Each run, by its first branch's first result.
    runs: HashMap<ValueId, (ValueId, Run)>,
    /// The results of branches already written as part of a run.
    merged: Vec<bool>,
}

impl Merge {
    /// The first arm of the merged branch, or its second: each branch's arm
    /// on that side in order, then everything they answered.
    fn arm(&mut self, rebuild: &Rebuild<'_>, b: &mut Builder, run: &Run, first: bool) {
        let source = rebuild.source();
        let mut answered = Vec::new();
        for (then_region, else_region, results) in run {
            let region = if first { *then_region } else { *else_region };
            for (op, held) in source.walk(region) {
                if !self.op(rebuild, b, *op, held) {
                    rebuild.copy(b, *op, held, self);
                }
            }
            let terminator = source
                .region(region)
                .terminator
                .expect("an arm that answers");
            let values = rebuild.args(terminator.values);
            // What a later arm reads of this branch is what this arm answered.
            for (&result, &value) in results.iter().zip(&values) {
                rebuild.stands_for(result, value);
            }
            answered.extend(values);
        }
        b.yield_(&answered);
    }
}

impl Pass for Merge {
    fn op(&mut self, rebuild: &Rebuild<'_>, b: &mut Builder, op: Op, results: &[ValueId]) -> bool {
        if !results.is_empty() && results.iter().all(|value| self.merged[value.index()]) {
            return true;
        }
        if !matches!(op, Op::If { .. }) {
            return false;
        }
        let Some((condition, run)) = results.first().and_then(|first| self.runs.remove(first))
        else {
            return false;
        };
        let all: Vec<ValueId> = run
            .iter()
            .flat_map(|(_, _, results)| results.iter().copied())
            .collect();
        let classes = rebuild.classes(&all);
        let condition = rebuild.value(condition);
        let fresh = {
            let pass = std::cell::RefCell::new(&mut *self);
            b.if_(
                condition,
                &classes,
                |b| pass.borrow_mut().arm(rebuild, b, &run, true),
                |b| pass.borrow_mut().arm(rebuild, b, &run, false),
            )
        };
        for (&was, now) in all.iter().zip(fresh) {
            rebuild.stands_for(was, now);
            self.merged[was.index()] = true;
        }
        true
    }
}
