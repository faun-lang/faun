//! Target-specific limits; conformance cases and size baselines are shared.
use faun_core::targets::Target;

use crate::X86_64Linux;
use shared::{narrow, wide};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "float_tests.rs"]
mod float_tests;

#[path = "abi_tests.rs"]
mod abi_tests;

mod layout;
mod runtime_lifetimes;
mod runtime_naturals;

use faun_core as backend_core;
#[path = "../../../core/tests/support/faun.rs"]
mod faun;

#[path = "../../../core/tests/support/backend.rs"]
mod shared;

const TARGET: X86_64Linux = X86_64Linux;
const RUNNER: Option<&str> = None;

#[test]
fn ladder_code_size_ratchets() {
    let mut code = Vec::new();
    for (name, text) in shared::LADDER {
        let machine = shared::read(text).unwrap();
        let prepared = TARGET.prepare(&machine).unwrap();
        let allocated = crate::selected::select_and_allocate(&prepared).unwrap();
        code.push((name, crate::encode::encode(&allocated).bytes));
    }

    let sizes: Vec<_> = code
        .iter()
        .map(|(name, bytes)| (*name, bytes.len()))
        .collect();
    assert_eq!(
        sizes,
        [
            ("a constant", 7),
            ("the count", 9),
            ("the count and a constant", 12),
            ("the low bits of a sum", 11),
            ("the count under a guard", 17),
            ("the count and more", 22),
            ("a shared intermediate", 31),
            ("a greeting", 45),
        ]
    );
    assert_eq!(code[0].1, [0x31, 0xFF, 0x6A, 0x3C, 0x58, 0x0F, 0x05]);
    // The count's low byte, read straight from where the kernel left it.
    assert_eq!(
        code[1].1,
        [0x0F, 0xB6, 0x3C, 0x24, 0x6A, 0x3C, 0x58, 0x0F, 0x05]
    );
}

#[test]
fn it_holds_what_a_program_holds() {
    assert!(TARGET.accepts(&wide(70_000)).is_ok());
    assert!(TARGET.accepts(&wide(4)).is_ok());
    assert!(TARGET.accepts(&narrow(32)).is_ok());
}

fn division_case(
    dividend_bits: u16,
    divisor_bits: u16,
    numerator: u64,
    denominator: u64,
    constant_divisor: bool,
) -> faun_core::ir::Program {
    use faun_core::ir::{Binary, Class, Program, Relation};
    let (mut program, main) = Program::new("main");
    let left = Class::fixed(dividend_bits);
    let right = Class::fixed(divisor_bits);
    let divide = program.declare("divide", &[left, right], &[Class::Word, Class::Word]);
    program.define(divide, |builder, args| {
        let divisor = if constant_divisor {
            builder.constant(right, denominator)
        } else {
            args[1]
        };
        let quotient = builder.binary_at(Class::Word, Binary::Div, args[0], divisor);
        let remainder = builder.binary_at(Class::Word, Binary::Mod, args[0], divisor);
        builder.ret(&[quotient, remainder]);
    });
    program.define(main, |builder, _| {
        let dividend = builder.constant(left, numerator);
        let divisor = builder.constant(right, denominator);
        let pair = builder.call(divide, &[dividend, divisor]);
        let quotient =
            builder.constant(Class::Word, numerator.checked_div(denominator).unwrap_or(0));
        let remainder = builder.constant(
            Class::Word,
            numerator.checked_rem(denominator).unwrap_or(numerator),
        );
        let first = builder.compare(Relation::Equal, pair[0], quotient);
        let second = builder.compare(Relation::Equal, pair[1], remainder);
        let both = builder.binary(Binary::And, first, second);
        builder.ret(&[both]);
    });
    program
}

#[test]
fn unsigned_division_chooses_width_from_both_inputs_and_handles_zero() {
    use crate::machine::{Inst, RegisterWidth};

    for (dividend_bits, divisor_bits, numerator, denominator) in [
        (32, 32, u64::from(u32::MAX), 7_u64),
        (32, 32, u64::from(u32::MAX), 0),
        (64, 32, (1 << 42) + 3, 7),
        (32, 64, 9, 1 << 32),
        (64, 64, u64::MAX, 0),
    ] {
        for constant_divisor in [false, true] {
            let program = division_case(
                dividend_bits,
                divisor_bits,
                numerator,
                denominator,
                constant_divisor,
            );
            let prepared = TARGET.prepare(&program).unwrap();
            if !constant_divisor {
                let expected = if dividend_bits <= 32 && divisor_bits <= 32 {
                    RegisterWidth::Low32
                } else {
                    RegisterWidth::Word
                };
                let code = crate::selected::select_and_allocate(&prepared).unwrap();
                assert!(code.instructions().contains(&Inst::Divide(expected)));
            }
            assert_eq!(shared::status(&prepared), 1);
        }
    }
}

#[test]
fn low_word_multiplication_preserves_unsigned_high_bits() {
    use faun_core::ir::{Binary, Class, Program, Relation};
    for (left, right) in [
        (u64::MAX, u64::MAX),
        (1 << 63, 3),
        (u64::from(u32::MAX), u64::from(u32::MAX)),
        (0, u64::MAX),
    ] {
        let (mut p, main) = Program::new("main");
        p.define(main, |b, _| {
            let a = b.constant(Class::Word, left);
            let c = b.constant(Class::Word, right);
            let product = b.binary(Binary::Mul, a, c);
            let expected = b.constant(Class::Word, left.wrapping_mul(right));
            let equal = b.compare(Relation::Equal, product, expected);
            b.ret(&[equal]);
        });
        assert_eq!(shared::status(&p), 1, "{left} * {right}");
    }
}

/// A comparison or a monus of numbers held at different widths reads both
/// at the wider one, for every pair of widths: each pair is handed, through
/// a call so that nothing folds them, a number at the top of the narrower
/// width and one at the wider whose low thirty-two bits are the same, and
/// answers, a bit each, whether they are equal, which is below the other,
/// and whether the narrower less the wider is what it is. Read at the
/// left's width only the wide one's low bits were compared: nought, held in
/// no bits, was equal to `2^52 + 2^51`, which miscompiled the software float
/// routines' `0 == mantissa` and so mandelbrot without hardware floats.
#[test]
fn numbers_of_different_widths_are_compared_at_the_wider() {
    let top = |bits: u16| {
        if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        }
    };
    let widths = [0u16, 1, 8, 31, 32, 33, 52, 53, 64];
    for &narrow in &widths {
        for &wide in widths.iter().filter(|&&wide| wide >= narrow) {
            let small = top(narrow);
            // The wide number's low thirty-two bits are the small one's.
            let big = (top(wide) & !u64::from(u32::MAX)) | (small & u64::from(u32::MAX));
            for (left, right) in [
                ((small, narrow), (big, wide)),
                ((big, wide), (small, narrow)),
            ] {
                let answer = i32::from(left.0 == right.0)
                    | i32::from(left.0 < right.0) << 1
                    | i32::from(left.0 > right.0) << 2
                    | 1 << 3;
                assert_eq!(
                    compared(left, right),
                    answer,
                    "{} at {} bits and {} at {}",
                    left.0,
                    left.1,
                    right.0,
                    right.1
                );
            }
        }
    }
}

/// What a program answers comparing `left` and `right`, each a number and
/// the bits it is held in, handed through a call so that nothing folds
/// them: a bit each for whether they are equal, whether the left is below,
/// whether it is above, and whether the left less the right is what it is.
fn compared((left, left_bits): (u64, u16), (right, right_bits): (u64, u16)) -> i32 {
    use faun_core::ir::{Binary, Class, Direction, Program, Relation};
    let class = |bits: u16| {
        if bits == 64 {
            Class::Word
        } else {
            Class::fixed(bits)
        }
    };
    let (mut p, main) = Program::new("main");
    let probe = p.declare(
        "probe",
        &[class(left_bits), class(right_bits)],
        &[Class::Word],
    );
    p.define(probe, |b, params| {
        let (x, y) = (params[0], params[1]);
        let equal = b.compare(Relation::Equal, x, y);
        let below = b.compare(Relation::Less, x, y);
        let above = b.compare(Relation::Less, y, x);
        let less = b.binary_at(class(left_bits), Binary::Monus, x, y);
        let expected = b.constant(class(left_bits), left.saturating_sub(right));
        let monus = b.compare(Relation::Equal, less, expected);
        let below = b.shift(Class::Word, Direction::Up, below, 1);
        let above = b.shift(Class::Word, Direction::Up, above, 2);
        let monus = b.shift(Class::Word, Direction::Up, monus, 3);
        let answer = b.binary(Binary::Or, equal, below);
        let answer = b.binary(Binary::Or, answer, above);
        let answer = b.binary(Binary::Or, answer, monus);
        b.ret(&[answer]);
    });
    p.define(main, |b, _| {
        let x = b.constant(class(left_bits), left);
        let y = b.constant(class(right_bits), right);
        let answered = b.call(probe, &[x, y]);
        b.ret(&answered);
    });
    shared::status(&p)
}

/// A recursion's way out, which touches nothing it keeps for its caller,
/// returns without saving it, favouring speed; favouring size, the frame
/// opens where the function starts.
#[test]
fn an_early_way_out_saves_nothing() {
    use crate::machine::Inst;
    use faun_core::targets::Favour;
    let program = TARGET.prepare(&faun_core::programs::recurses()).unwrap();
    let first = |favour| {
        let allocated =
            crate::selected::select_and_allocate_favouring(&program, favour, false).unwrap();
        let (_, start) = allocated
            .functions()
            .find(|&(function, _)| function != program.entry())
            .unwrap();
        allocated.instructions()[start]
    };
    assert!(!matches!(first(Favour::Speed), Inst::OpenFrame { .. }));
    assert!(matches!(first(Favour::Size), Inst::OpenFrame { .. }));
}

/// Written with its symbols, a program is one to profile: every function
/// but the entry pushes `rbp` and points it at its frame where it starts,
/// and gives it back before each way out. The entry pushes nothing, Linux
/// starting it with `rbp` nought, which ends the chain.
#[test]
fn a_program_written_with_symbols_chains_its_frames() {
    use crate::machine::{Inst, Reg, RegisterWidth};
    use faun_core::targets::Favour;
    let program = TARGET.prepare(&faun_core::programs::recurses()).unwrap();
    for favour in [Favour::Speed, Favour::Size] {
        let allocated =
            crate::selected::select_and_allocate_favouring(&program, favour, true).unwrap();
        let code = allocated.instructions();
        for (function, start) in allocated.functions() {
            if function == program.entry() {
                assert_ne!(code[start], Inst::PushRegister { src: Reg::Rbp });
            } else {
                assert_eq!(
                    code[start..start + 2],
                    [
                        Inst::PushRegister { src: Reg::Rbp },
                        Inst::Move {
                            dst: Reg::Rbp,
                            src: Reg::Rsp,
                            width: RegisterWidth::Word,
                        },
                    ]
                );
            }
        }
        for (at, inst) in code.iter().enumerate() {
            if *inst == Inst::Return {
                assert_eq!(code[at - 1], Inst::Pop { dst: Reg::Rbp }, "{favour:?}");
            }
        }
    }
}

/// And it answers what it answers without them, its frames chained or not:
/// a function reading arguments its caller left on the stack finds them a
/// word further up.
#[test]
fn chaining_frames_changes_no_answer() {
    use faun_core::targets::{Favour, SymbolTable};
    let mut programs = vec![
        faun_core::programs::recurses(),
        faun_core::programs::values_survive_a_call(),
        faun_core::programs::calls_a_function(),
    ];
    programs.extend(
        faun_core::programs::printing()
            .into_iter()
            .map(|(_, program, _)| program),
    );
    for program in programs {
        let program = TARGET.prepare(&program).unwrap();
        let run = |symbols| {
            let dir = std::env::temp_dir().join(format!(
                "faun-frames-{}-{symbols:?}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let image = TARGET.emit_with(&program, Favour::Speed, symbols);
            let path = TARGET.write(&image, &dir.join("program")).unwrap();
            let output = std::process::Command::new(&path).output().unwrap();
            std::fs::remove_dir_all(&dir).unwrap();
            (output.status.code(), output.stdout)
        };
        assert_eq!(run(SymbolTable::Omitted), run(SymbolTable::Written));
    }
}

/// A full-width sum whose first operand is read again after is worked out
/// as an address into a register of its own, `lea d, [a + b]`, rather than
/// the operand copied and the sum written over the copy.
#[test]
fn a_sum_beside_what_it_was_worked_out_from_is_one_instruction() {
    use faun_core::ir::Class;
    // A sum bounded below 2^47 is worked out across the register all the
    // same, so it answers anywhere as a word's does.
    let held = |high| {
        Class::Held(barb::Bounds {
            low: 0,
            high: Some(high),
        })
    };
    for (operand, sum) in [(Class::Word, Class::Word), (held(1 << 46), held(1 << 47))] {
        sum_is_one_instruction(operand, sum);
    }
}

fn sum_is_one_instruction(operand: faun_core::ir::Class, sum: faun_core::ir::Class) {
    use crate::machine::{Alu, Inst};
    use faun_core::targets::Favour;
    let program = TARGET.prepare(&sum_then_its_operand(operand, sum)).unwrap();
    let allocated =
        crate::selected::select_and_allocate_favouring(&program, Favour::Speed, false).unwrap();
    let code = allocated.instructions();
    let (_, start) = allocated
        .functions()
        .find(|&(function, _)| function != program.entry())
        .unwrap();
    let body = &code[start..];
    let end = body
        .iter()
        .position(|inst| matches!(inst, Inst::Return))
        .unwrap();
    let body = &body[..=end];
    assert!(
        body.iter()
            .any(|inst| matches!(inst, Inst::Add3 { index: Some(_), .. })),
        "{body:?}"
    );
    assert!(
        !body
            .windows(2)
            .any(|pair| matches!(pair, [Inst::Move { .. }, Inst::Alu { op: Alu::Add, .. }])),
        "{body:?}"
    );
}

/// `(a + b) ^ a`, worked out by a function of its own from three and four,
/// each operand held at `operand` and the sum at `sum`.
fn sum_then_its_operand(
    operand: faun_core::ir::Class,
    sum: faun_core::ir::Class,
) -> faun_core::ir::Program {
    use faun_core::ir::{Binary, Class, Program};
    let (mut program, main) = Program::new("main");
    let mixed = program.declare("mixed", &[operand, operand], &[Class::Word]);
    program.define(mixed, |b, params| {
        let added = b.binary_at(sum, Binary::Add, params[0], params[1]);
        let first = b.convert(Class::Word, params[0]);
        let added = b.convert(Class::Word, added);
        let answer = b.binary(Binary::Xor, added, first);
        b.ret(&[answer]);
    });
    program.define(main, |b, _| {
        let three = b.constant(operand, 3);
        let four = b.constant(operand, 4);
        let answer = b.call(mixed, &[three, four]);
        b.ret(&answer);
    });
    program
}

/// A condition kept as a number and branched on as well is branched on by
/// the flags that set it, not tested again.
#[test]
fn a_condition_kept_and_branched_on_is_not_tested_again() {
    use crate::machine::{Alu, Inst};
    use faun_core::ir::{Binary, Class, Program, Relation};
    use faun_core::targets::Favour;
    let (mut source, main) = Program::new("main");
    let chosen = source.declare("chosen", &[Class::Word, Class::Word], &[Class::Word]);
    source.define(chosen, |b, params| {
        let below = b.compare(Relation::Less, params[0], params[1]);
        let picked = b.if_(
            below,
            &[Class::Word],
            |b| b.yield_(&[params[0]]),
            |b| b.yield_(&[params[1]]),
        );
        let answer = b.binary(Binary::Xor, picked[0], below);
        b.ret(&[answer]);
    });
    source.define(main, |b, _| {
        let three = b.constant(Class::Word, 3);
        let four = b.constant(Class::Word, 4);
        let answer = b.call(chosen, &[three, four]);
        b.ret(&answer);
    });
    let program = TARGET.prepare(&source).unwrap();
    let allocated =
        crate::selected::select_and_allocate_favouring(&program, Favour::Speed, false).unwrap();
    let code = allocated.instructions();
    assert!(
        code.iter().any(|inst| matches!(inst, Inst::SetIf { .. })),
        "{code:?}"
    );
    assert!(
        !code.windows(2).any(|pair| matches!(
            pair,
            [Inst::SetIf { dst, .. }, Inst::Alu { op: Alu::Test, dst: tested, .. }] if dst == tested
        )),
        "{code:?}"
    );
}

/// A branch on a value's low bit is the `and` that keeps the bit and a
/// jump on the flags it set, not tested again.
#[test]
fn a_branch_on_a_low_bit_jumps_on_the_mask() {
    use crate::machine::{Alu, Inst};
    use faun_core::ir::{Binary, Class, Program};
    use faun_core::targets::Favour;
    let (mut source, main) = Program::new("main");
    let odd = source.declare("odd", &[Class::Word, Class::Word], &[Class::Word]);
    source.define(odd, |b, params| {
        let low = b.convert(Class::fixed(1), params[0]);
        let picked = b.if_(
            low,
            &[Class::Word],
            |b| b.yield_(&[params[1]]),
            |b| {
                let sum = b.binary(Binary::Add, params[0], params[1]);
                b.yield_(&[sum]);
            },
        );
        b.ret(&picked);
    });
    source.define(main, |b, _| {
        let three = b.constant(Class::Word, 3);
        let four = b.constant(Class::Word, 4);
        let answer = b.call(odd, &[three, four]);
        b.ret(&answer);
    });
    let program = TARGET.prepare(&source).unwrap();
    let allocated =
        crate::selected::select_and_allocate_favouring(&program, Favour::Speed, false).unwrap();
    let code = allocated.instructions();
    assert!(
        code.iter()
            .any(|inst| matches!(inst, Inst::Narrow { bits: 1, .. })),
        "{code:?}"
    );
    assert!(
        !code.windows(2).any(|pair| matches!(
            pair,
            [Inst::Narrow { dst, .. }, Inst::Alu { op: Alu::Test, dst: tested, .. }] if dst == tested
        )),
        "{code:?}"
    );
}
