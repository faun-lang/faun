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
