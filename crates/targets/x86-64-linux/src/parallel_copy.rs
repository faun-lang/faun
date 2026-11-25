//! The shared planner of simultaneous moves, over this target's registers.

use super::machine::Reg;

pub(crate) use faun_regalloc::parallel_copy::{CopySource, plan, temporary};

pub(crate) type Place = faun_regalloc::parallel_copy::Place<Reg>;
pub(crate) type Copy = faun_regalloc::parallel_copy::Copy<Reg>;
pub(crate) type Move = faun_regalloc::parallel_copy::Move<Reg>;
