//! Where a value waiting in memory is written there.
//!
//! A value never changes, so once its slot holds it, it holds it for good,
//! and only the first move of it into memory along any path needs writing.
//! Writing it where it is made covers every path at once, but costs a
//! store each time it is made, though it may wait in memory only on a cold
//! path, around a call taken rarely. Writing it where it goes into memory
//! costs a store only where that happens, but each time round a loop it
//! happens in that the value was made outside.
//!
//! So, as LLVM's spill placement and regalloc2 weigh where a store goes, a
//! value is written where it goes into memory where that is one place, in
//! no deeper a loop than where it is made: it runs no more often than the
//! value is made, and the store is still one instruction. Otherwise it is
//! written where it is made, as a loop's invariant store is hoisted out of
//! the loop. Then a move into memory is written only where no path to it
//! has written the value already.

use barb::index::Range;

use super::{Allocator, BlockId, Location, Register, Transition, VReg, edges};

/// Each block's predecessors, and the arguments each hands it.
type Predecessors = [Vec<(BlockId, Range)>];

/// Where each value waiting in memory is written there.
pub(super) struct Spills {
    /// Whether each value is written where it is made.
    pub(super) at_definition: Vec<bool>,
    /// Each value written where it goes into memory, numbered from nought.
    moved: Vec<Option<usize>>,
    /// Of those, what each block's end has written along every path to it.
    written: Vec<Words>,
}

impl Spills {
    /// Whether a move of `value` into its slot at the end of `from` writes
    /// nothing new.
    pub(super) fn holds(&self, from: BlockId, value: VReg) -> bool {
        self.at_definition[value.index()]
            || self.moved[value.index()].is_some_and(|at| self.written[from.index()].has(at))
    }
}

/// A set of the values written where they go into memory, by number.
#[derive(Clone, PartialEq, Eq)]
struct Words(Vec<u64>);

impl Words {
    fn empty(count: usize) -> Self {
        Self(vec![0; count.div_ceil(64)])
    }

    fn full(count: usize) -> Self {
        Self(vec![u64::MAX; count.div_ceil(64)])
    }

    fn has(&self, at: usize) -> bool {
        self.0[at / 64] & (1 << (at % 64)) != 0
    }

    fn add(&mut self, at: usize) {
        self.0[at / 64] |= 1 << (at % 64);
    }

    fn meet(&mut self, other: &Self) {
        for (word, other) in self.0.iter_mut().zip(&other.0) {
            *word &= other;
        }
    }
}

/// Each block's transitions writing a value into its own slot: where each
/// is in the list, and the value's number where it is written there.
type Writes = [Vec<(usize, Option<usize>)>];

impl<R: Register> Allocator<'_, R> {
    /// Decide where each value waiting in memory is written, and drop each
    /// transition into memory that writes nothing new.
    pub(super) fn place_spills(&mut self, predecessors: &Predecessors) -> Spills {
        let at_definition = self.written_where_made();
        let mut moved = vec![None; self.code.values];
        let mut count = 0;
        for (value, slot) in self.value_spills.iter().enumerate() {
            if slot.is_some() && !at_definition[value] {
                moved[value] = Some(count);
                count += 1;
            }
        }
        let starts = self.starting_in_memory(&moved, count);
        let writes = self.writes_by_block(&moved);
        let written = self.written_at_ends(predecessors, count, &starts, &writes);
        let quiet = self.quiet(predecessors, count, &starts, &writes, &written);
        let mut at = 0;
        self.transitions.retain(|_| {
            at += 1;
            !quiet[at - 1]
        });
        Spills {
            at_definition,
            moved,
            written,
        }
    }

    /// Each block's transitions writing a value into its own slot.
    fn writes_by_block(&self, moved: &[Option<usize>]) -> Vec<Vec<(usize, Option<usize>)>> {
        let mut writes = vec![Vec::new(); self.code.blocks.len()];
        for (at, &transition) in self.transitions.iter().enumerate() {
            if let Some(value) = self.written_into_slot(transition) {
                let block = self.liveness.block_at(transition.at).index();
                writes[block].push((at, moved[value.index()]));
            }
        }
        writes
    }

    /// Whether each transition writes nothing new: its value is written
    /// where it is made, or every path to it has written it already.
    fn quiet(
        &self,
        predecessors: &Predecessors,
        count: usize,
        starts: &[Vec<usize>],
        writes: &Writes,
        written: &[Words],
    ) -> Vec<bool> {
        let mut quiet = vec![false; self.transitions.len()];
        for layout in &self.liveness.blocks {
            let block = layout.block.index();
            let mut held = written_at_entry(predecessors, layout.block, count, written);
            for &at in &starts[block] {
                held.add(at);
            }
            for &(at, number) in &writes[block] {
                quiet[at] = number.is_none_or(|number| {
                    let already = held.has(number);
                    held.add(number);
                    already
                });
            }
        }
        quiet
    }

    /// The numbers of what each block starts with in memory, of the values
    /// written where they go there: it is in memory only because it was
    /// written.
    fn starting_in_memory(&self, moved: &[Option<usize>], count: usize) -> Vec<Vec<usize>> {
        let mut starts = vec![Vec::new(); self.code.blocks.len()];
        if count == 0 {
            return starts;
        }
        for layout in &self.liveness.blocks {
            if let Some(state) = self.state(layout.entry) {
                starts[layout.block.index()] = self
                    .liveness
                    .live(layout.entry)
                    .iter()
                    .filter(|&value| matches!(state.location(value), Some(Location::Spill(_))))
                    .filter_map(|value| moved[value.index()])
                    .collect();
            }
        }
        starts
    }

    /// The value `transition` writes into its own slot from a register.
    fn written_into_slot(&self, transition: Transition<R>) -> Option<VReg> {
        match (transition.from, transition.to) {
            (Location::Register(_), Location::Spill(slot))
                if self.value_spills[transition.value.index()] == Some(slot) =>
            {
                Some(transition.value)
            }
            _ => None,
        }
    }

    /// Whether each value is written where it is made: where it goes into
    /// memory in more than one place, or in a deeper loop than where it is
    /// made.
    fn written_where_made(&self) -> Vec<bool> {
        let depth = self.loop_depths();
        let made = self.made_in();
        // Where each value goes into memory: how many places, and the
        // deepest of them.
        let mut places = vec![(0u32, 0u32); self.code.values];
        let mut goes = |value: VReg, block: BlockId| {
            let (count, deepest) = &mut places[value.index()];
            *count = count.saturating_add(1);
            *deepest = (*deepest).max(depth[block.index()]);
        };
        for &transition in &self.transitions {
            if let Some(value) = self.written_into_slot(transition) {
                goes(value, self.liveness.block_at(transition.at));
            }
        }
        for layout in &self.liveness.blocks {
            for (target, _) in edges(self.code.blocks[layout.block].terminator) {
                for value in self.stored_on_edge(layout.block, target) {
                    goes(value, layout.block);
                }
            }
        }
        (0..self.code.values)
            .map(|at| {
                let (count, deepest) = places[at];
                count > 1 || made[at].is_none_or(|block| deepest > depth[block.index()])
            })
            .collect()
    }

    /// The block each value is made in.
    fn made_in(&self) -> Vec<Option<BlockId>> {
        let mut made = vec![None; self.code.values];
        for layout in &self.liveness.blocks {
            let block = self.code.blocks[layout.block];
            for &param in &self.code.block_params[block.params.range()] {
                made[param.index()] = Some(layout.block);
            }
            for points in &layout.instructions {
                if let Some(result) = self.code.instructions[points.instruction].result {
                    made[result.index()] = Some(layout.block);
                }
            }
        }
        made
    }

    /// How many loops hold each block, a loop being the run of blocks from
    /// its head to the last block going back to it.
    fn loop_depths(&self) -> Vec<u32> {
        let mut depth = vec![0; self.code.blocks.len()];
        for layout in &self.liveness.blocks {
            let from = layout.block.index();
            for (target, _) in edges(self.code.blocks[layout.block].terminator) {
                if target.index() <= from {
                    for held in &mut depth[target.index()..=from] {
                        *held += 1;
                    }
                }
            }
        }
        depth
    }

    /// What the edge from `from` to `target` moves into its own slot from a
    /// register, other than a parameter the edge hands a value to.
    fn stored_on_edge(&self, from: BlockId, target: BlockId) -> Vec<VReg> {
        let (Some(end), Some(start)) = (
            self.state(self.liveness.blocks[from.index()].terminator),
            self.state(self.liveness.blocks[target.index()].entry),
        ) else {
            return Vec::new();
        };
        let block = self.code.blocks[target];
        let params = &self.code.block_params[block.params.range()];
        self.liveness
            .live(self.liveness.blocks[target.index()].entry)
            .iter()
            .filter(|&value| {
                let Some(slot) = self.value_spills[value.index()] else {
                    return false;
                };
                start.location(value) == Some(Location::Spill(slot))
                    && matches!(end.location(value), Some(Location::Register(_)))
                    && !params.contains(&value)
            })
            .collect()
    }

    /// What each block's end has written to memory along every path, worked
    /// out from every block holding everything, down until no block's
    /// changes: a loop's head holds what both its entry and the way round
    /// have written.
    fn written_at_ends(
        &self,
        predecessors: &Predecessors,
        count: usize,
        starts: &[Vec<usize>],
        writes: &Writes,
    ) -> Vec<Words> {
        let blocks = self.code.blocks.len();
        if count == 0 {
            return vec![Words::empty(0); blocks];
        }
        let mut written = vec![Words::full(count); blocks];
        let mut changed = true;
        while changed {
            changed = false;
            for layout in &self.liveness.blocks {
                let block = layout.block.index();
                let mut held = written_at_entry(predecessors, layout.block, count, &written);
                for &at in &starts[block] {
                    held.add(at);
                }
                for number in writes[block].iter().filter_map(|&(_, number)| number) {
                    held.add(number);
                }
                if held != written[block] {
                    written[block] = held;
                    changed = true;
                }
            }
        }
        written
    }
}

/// What every path into `block` has written to memory: what every
/// predecessor's end holds.
fn written_at_entry(
    predecessors: &Predecessors,
    block: BlockId,
    count: usize,
    written: &[Words],
) -> Words {
    let mut held: Option<Words> = None;
    for &(from, _) in &predecessors[block.index()] {
        let from = &written[from.index()];
        match &mut held {
            None => held = Some(from.clone()),
            Some(held) => held.meet(from),
        }
    }
    held.unwrap_or_else(|| Words::empty(count))
}
