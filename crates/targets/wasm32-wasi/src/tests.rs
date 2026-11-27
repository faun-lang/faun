//! Target-specific limits; conformance cases and size baselines are shared.
use faun_core::programs;
use faun_core::targets::Target;

use crate::Wasm32Wasi;
use shared::{EXIT_ZERO, bytes, execute, narrow, read, status, wide};

use faun_core as backend_core;
#[path = "../../../core/tests/support/faun.rs"]
mod faun;

#[path = "../../../core/tests/support/backend.rs"]
mod shared;

const TARGET: Wasm32Wasi = Wasm32Wasi;
const RUNNER: Option<&str> = Some("wasmtime");

#[test]
fn it_holds_what_a_program_holds() {
    assert!(TARGET.accepts(&wide(70_000)).is_ok());
    assert!(TARGET.accepts(&wide(4)).is_ok());
    assert!(TARGET.accepts(&narrow(32)).is_ok());
    let refused = TARGET.accepts(&narrow(33)).unwrap_err();
    assert!(
        refused.contains("holds 32 bits, and a value here needs 33"),
        "{refused}"
    );
}

/// `proc_exit(0)` at the end of the entry needs no import, memory or data.
#[test]
fn the_smallest_module_imports_nothing() {
    assert_eq!(execute(&programs::empty()).1, "");
    let written = read(EXIT_ZERO).expect("a program");
    assert_eq!(bytes(&written), bytes(&programs::empty()));
}

/// WASM uses its operand stack for multiple returns and region transfers.
#[test]
fn multivalue_function_and_region_abis_are_supported() {
    assert_eq!(status(&programs::abi::function(7, 3)), 0);
    for kind in [
        programs::abi::Transfer::If,
        programs::abi::Transfer::LoopState,
        programs::abi::Transfer::LoopResults,
    ] {
        assert_eq!(status(&programs::abi::region(kind, 3)), 0);
    }
}

/// Each count of a word's bits is one instruction at a width up to the
/// word's, and a word of 64 bits is counted a half at a time.
#[test]
fn a_count_of_bits_is_counted_a_word_at_a_time() {
    use faun_core::ir::{Function, Op};
    for (word, bits) in [("U8", 8), ("U16", 16), ("U32", 32), ("U64", 32)] {
        for method in ["count_ones", "trailing_zeros", "leading_zeros"] {
            let source = format!(
                "import std.host {{Host}}

                 main! : Host -> []
                 main!(host) -> host.exit!(U8.of_nat({word}.of_nat(host.args.len()).{method}()))"
            );
            let prepared = TARGET
                .prepare(&faun::lower(&source, &TARGET).unwrap())
                .unwrap();
            let widths: Vec<u16> = prepared
                .functions()
                .iter()
                .filter_map(Function::body)
                .flat_map(|body| prepared.every(body))
                .filter_map(|op| match op {
                    Op::Count { count, .. } => Some(count.bits),
                    _ => None,
                })
                .collect();
            let halves = if word == "U64" { 2 } else { 1 };
            assert_eq!(widths, vec![bits; halves], "{word}.{method}");
        }
    }
}
