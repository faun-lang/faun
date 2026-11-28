//! `selected` flattens machine-IR regions into a verified target-local SSA
//! CFG. Its allocator computes per-point liveness, splits around constraints
//! and clobbers, plans transfers through `parallel_copy`, and lowers a fully
//! verified symbolic `machine::Allocated` sequence. Allowed-register sets
//! separate integer and scalar XMM operands; copies and spills preserve their
//! low-word bit patterns across banks. `encode` alone lays that sequence out
//! as bytes. Data addresses remain `Reloc`s until `elf` chooses
//! the image layout and fills them in.
//!
//! It reads the machine IR every target shares, as `faun-core` lowers and
//! legalizes it, and keeps its own instruction selection, register
//! allocator and ELF emitter.

mod elf;
mod encode;
mod jumps;
mod machine;
mod parallel_copy;
mod selected;
#[cfg(test)]
mod tests;

use std::io;
use std::path::{Path, PathBuf};

use faun_core::ir::{DataId, FunctionId, Program};
use faun_core::targets::{Favour, SymbolTable, Target, startup_parameters, write_image};

/// How wide this machine's own word is, which is the widest value it holds
/// and what any class but a fixed one means here.
pub(crate) const WORD_BITS: u16 = 64;

/// Which instructions past the x86-64 baseline an image may use. None, so
/// an image runs on every x86-64 processor, as rustc's do by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Extensions {
    /// `popcnt`, which counts a word's set bits in one instruction where
    /// the baseline takes a dozen. A count of ones is answered by it where
    /// this is set, and by the definition's own arithmetic, Hacker's
    /// Delight's count, where it is not, as rustc answers `count_ones` at
    /// the baseline. `lzcnt` and `tzcnt`, from the same generation, would
    /// answer the counts of zeros a jump shorter than `bsr` and `bsf` do.
    pub(crate) popcnt: bool,
}

/// The extensions images are built for: the baseline alone.
pub(crate) const EXTENSIONS: Extensions = Extensions { popcnt: false };

/// Parameters a call passes, one per System V integer argument register.
const ARG_COUNT: usize = 6;

/// This target, for whoever is choosing between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct X86_64Linux;

impl Target for X86_64Linux {
    fn name(&self) -> &'static str {
        "x86_64-linux"
    }

    fn word(&self) -> u16 {
        WORD_BITS
    }

    /// The streams', and the kernel's cryptographically secure generator.
    fn provides(&self) -> &'static [&'static str] {
        faun_core::targets::STREAMS_AND_ENTROPY
    }

    /// A process is given the low 47 bits of address.
    fn address(&self) -> u16 {
        47
    }

    /// SSE2 is part of x86-64: every processor it runs on has scalar
    /// binary32 and binary64 arithmetic, correctly rounded to nearest-even
    /// with exceptions masked, as the control word is when a process starts.
    fn floats(&self) -> bool {
        true
    }

    fn shifts_by_register(&self) -> bool {
        true
    }

    /// The counts of zeros are `bsf` and `bsr` at the baseline, with a jump
    /// for nought; a count of ones is `popcnt`, past it, so only where
    /// the target's extensions say, none by default.
    fn counts(&self) -> barb::Counts {
        let zeros = barb::Counts::NONE
            .with(barb::Count::TrailingZeros)
            .with(barb::Count::LeadingZeros);
        if EXTENSIONS.popcnt {
            zeros.with(barb::Count::Ones)
        } else {
            zeros
        }
    }

    fn limits(&self, program: &Program) -> Result<(), String> {
        startup_parameters(program, "x86-64-linux")
    }

    fn emit(&self, program: &Program) -> Vec<u8> {
        self.emit_with(program, Favour::Speed, SymbolTable::Omitted)
    }

    fn emit_with(&self, program: &Program, favour: Favour, symbols: SymbolTable) -> Vec<u8> {
        // A program written with its symbols is one to profile, so each
        // function keeps a frame pointer a profiler can walk the stack by.
        let frame_pointers = symbols == SymbolTable::Written;
        let allocated = selected::select_and_allocate_favouring(program, favour, frame_pointers)
            .expect("accepted machine IR has a valid x86 selected allocation");
        let code = encode::encode(&allocated);
        elf::image(&code, program, symbols)
    }

    /// An executable named `output`.
    fn write(&self, image: &[u8], output: &Path) -> io::Result<PathBuf> {
        write_image(output, "", image, true)
    }
}

/// Machine code whose data addresses are not resolved yet.
struct Code {
    bytes: Vec<u8>,
    relocs: Vec<Reloc>,
    /// Each function and where in `bytes` its code starts, in order.
    functions: Vec<(FunctionId, usize)>,
}

struct Reloc {
    offset: usize,
    data: DataId,
}
