//! Verify recognition through the frontend and the public Barb facts API.
mod support;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use faun_core::targets::Target;
use faun_x86_64_linux::X86_64Linux;
use support::Package;

fn optimized(package: &Package) -> barb::Program {
    let mut program =
        faun_frontend::lower_to_barb(&package.entry(), &faun_driver::platform(&X86_64Linux))
            .unwrap();
    barb::optimize(&mut program);
    barb::validate(&program).unwrap();
    program
}

/// The facts a target with float instructions reads, which name the float
/// operations it answers.
fn with_floats(program: &barb::Program) -> barb::Facts {
    program.assuming(&barb::Assumed {
        floats: true,
        ..barb::Assumed::default()
    })
}

#[test]
fn rotations_are_recognized_by_equations_with_a_checked_input_range() {
    for (modulus, recognized) in [(256, true), (1024, false)] {
        let package = Package::new(
            &format!("rotation-{modulus}"),
            &format!(
                r"
            turn : (Nat, Nat) -> Nat
            turn(value, amount) -> (
              moved = value * 2.pow(amount % 8)
              moved % 256 + moved / 256
            )
            main!(host) -> host.exit!(U8.of_nat(turn(host.args.len() % {modulus}, host.args.len())))
        "
            ),
        );
        let program = optimized(&package);
        let facts = program.analyze();
        assert_eq!(
            program
                .function_ids()
                .any(|id| facts.rotation(id).is_some_and(|r| r.bits == 8)),
            recognized
        );
    }
}

/// What each definition `program` keeps is recognized as counting, by name,
/// built for `target` and optimized.
fn bit_counts(package: &Package, target: &dyn Target) -> BTreeMap<String, barb::BitCount> {
    let mut program =
        faun_frontend::lower_to_barb(&package.entry(), &faun_driver::platform(target)).unwrap();
    barb::optimize(&mut program);
    barb::validate(&program).unwrap();
    let facts = program.analyze();
    program
        .function_ids()
        .filter_map(|id| {
            let count = facts.bit_count(id)?;
            Some((program.name(program.function(id).name).to_owned(), count))
        })
        .collect()
}

const fn counting(count: barb::Count, bits: u16) -> barb::BitCount {
    barb::BitCount { count, bits }
}

/// A program calling each of the words' counts of their bits on a word of
/// every width.
fn every_bit_count() -> Package {
    let mut calls = String::new();
    for word in ["U8", "U16", "U32", "U64"] {
        for method in [
            "count_ones",
            "count_zeros",
            "trailing_zeros",
            "trailing_ones",
            "leading_zeros",
            "leading_ones",
        ] {
            write!(calls, " + {word}.of_nat(n).{method}()").unwrap();
        }
    }
    Package::new(
        "every-bit-count",
        &format!("main!(host) -> (\n  n = host.args.len()\n  host.exit!(U8.of_nat(0{calls}))\n)"),
    )
}

#[test]
fn the_words_counts_of_their_bits_are_recognized_at_their_widths() {
    use barb::Count::{LeadingZeros, Ones, TrailingZeros};
    let found = bit_counts(&every_bit_count(), &faun_wasm32_wasi::Wasm32Wasi);
    let mut expected = BTreeMap::new();
    for bits in [8, 16, 32, 64] {
        for (method, count) in [
            ("count_ones", Ones),
            ("trailing_zeros", TrailingZeros),
            ("leading_zeros", LeadingZeros),
        ] {
            expected.insert(format!("U{bits}.{method}"), counting(count, bits));
        }
    }
    // The complements' counts are each a count of the complement, and so a
    // call of what is recognized rather than a count of their own.
    assert_eq!(found, expected);
}

/// A target keeps whole only the counts it has instructions for: x86-64's
/// baseline has `bsf` and `bsr` but no `popcnt`, and the 6502 none.
#[test]
fn only_the_counts_a_target_answers_are_recognized() {
    use barb::Count::{LeadingZeros, TrailingZeros};
    let package = every_bit_count();
    let x86: Vec<_> = bit_counts(&package, &X86_64Linux).into_values().collect();
    assert!(
        x86.iter()
            .all(|count| matches!(count.count, TrailingZeros | LeadingZeros)),
        "{x86:?}"
    );
    assert_eq!(x86.len(), 8);
    assert!(bit_counts(&package, &faun_mos6502_sim65::Mos6502Sim65).is_empty());
}

/// Recognition is by shape: a program's own definitions written with the
/// standard's arithmetic are recognized, under any names.
#[test]
fn bit_counts_written_in_a_program_are_recognized_by_shape() {
    use barb::Count::{LeadingZeros, Ones, TrailingZeros};
    let package = Package::new(
        "own-bit-counts",
        r"
        weight : U32 -> Nat
        weight(x) -> (
          a = x - (x.shift_right(1) & 0x55555555)
          b = (a & 0x33333333) + (a.shift_right(2) & 0x33333333)
          c = (b + b.shift_right(4)) & 0x0f0f0f0f
          (c * 0x01010101).shift_right(24).to_nat()
        )

        lowest : U32 -> Nat
        lowest(x) -> weight((x - 1) & !x)

        clear_above : U16 -> Nat
        clear_above(x) -> (
          a = x | x.shift_right(8)
          b = a | a.shift_right(4)
          c = b | b.shift_right(2)
          d = c | c.shift_right(1)
          e = d - (d.shift_right(1) & 0x5555)
          f = (e.shift_right(2) & 0x3333) + (e & 0x3333)
          g = (f.shift_right(4) + f) & 0x0f0f
          16 - (0x0101 * g).shift_right(8).to_nat()
        )

        main!(host) -> (
          n = host.args.len()
          host.exit!(U8.of_nat(weight(U32.of_nat(n)) + lowest(U32.of_nat(n)) + clear_above(U16.of_nat(n))))
        )
    ",
    );
    let found = bit_counts(&package, &faun_wasm32_wasi::Wasm32Wasi);
    assert_eq!(found.get("weight"), Some(&counting(Ones, 32)));
    assert_eq!(found.get("lowest"), Some(&counting(TrailingZeros, 32)));
    assert_eq!(found.get("clear_above"), Some(&counting(LeadingZeros, 16)));
}

/// A definition one step from a count is not one, and runs its body: a
/// mask a bit short, a sum read from the wrong byte, the bits above the
/// lowest set one, a smear a step short, the width taken wrongly, and a
/// count written as a loop clearing the lowest set bit until none is left.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "each near miss written beside the arithmetic its body comes to"
)]
fn near_misses_and_loops_are_not_recognized_and_run_their_bodies() {
    let count = |mask: &str, by: u32| {
        format!(
            "(
              a = x - (x.shift_right(1) & {mask})
              b = (a & 0x33333333) + (a.shift_right(2) & 0x33333333)
              c = (b + b.shift_right(4)) & 0x0f0f0f0f
              (c * 0x01010101).shift_right({by}).to_nat()
            )"
        )
    };
    let misses = [
        ("short_mask", count("0x55555554", 24)),
        ("wrong_byte", count("0x55555555", 16)),
        ("above_lowest", "U32.count_ones(!x & (x + 1))".to_owned()),
        (
            "short_smear",
            "(
              a = x | x.shift_right(1)
              b = a | a.shift_right(2)
              c = b | b.shift_right(4)
              32 - U32.count_ones(c | c.shift_right(8))
            )"
            .to_owned(),
        ),
        (
            "wrong_width",
            "(
              a = x | x.shift_right(1)
              b = a | a.shift_right(2)
              c = b | b.shift_right(4)
              d = c | c.shift_right(8)
              31 - U32.count_ones(d | d.shift_right(16))
            )"
            .to_owned(),
        ),
    ];
    let mut source = String::new();
    let mut sum = String::from("0");
    for (name, body) in &misses {
        writeln!(source, "{name} : U32 -> Nat\n{name}(x) -> {body}\n").unwrap();
        write!(sum, " + {name}(w)").unwrap();
    }
    source.push_str(
        "clearing : (U32, Nat) -> Nat
        clearing(x, n) if x == 0 -> n
        clearing(x, n) -> clearing(x & (x - 1), n + 1)

        ",
    );
    write!(
        source,
        "main!(host) -> (\n  w = U32.of_nat(host.args.len() + 0x57fff)\n  host.exit!(U8.of_nat({sum} + clearing(w, 0)))\n)"
    )
    .unwrap();
    let package = Package::new("near-bit-counts", &source);
    let found = bit_counts(&package, &faun_wasm32_wasi::Wasm32Wasi);
    let names: Vec<_> = misses
        .iter()
        .map(|(name, _)| *name)
        .chain(["clearing"])
        .collect();
    for name in names {
        assert!(!found.contains_key(name), "{name}: {found:?}");
    }
    // Each runs its body: with an argc of one, w is 0x58000.
    let w: u32 = 0x5_8000;
    let swar = |mask: u32, by: u32| {
        let a = w.wrapping_sub((w >> 1) & mask);
        let b = (a & 0x3333_3333) + ((a >> 2) & 0x3333_3333);
        let c = (b + (b >> 4)) & 0x0f0f_0f0f;
        c.wrapping_mul(0x0101_0101) >> by
    };
    let smeared = |steps: &[u32]| steps.iter().fold(w, |x, by| x | x >> by).count_ones();
    let expected = swar(0x5555_5554, 24)
        + swar(0x5555_5555, 16)
        + (!w & (w + 1)).count_ones()
        + (32 - smeared(&[1, 2, 4, 8]))
        + 31u32.saturating_sub(smeared(&[1, 2, 4, 8, 16]))
        + w.count_ones();
    assert_eq!(package.run(), expected % 256);
}

#[test]
fn floating_sign_recognition_preserves_the_signed_zero_exception() {
    for (left, found) in [("4.0", true), ("-0.0", false)] {
        let package = Package::new(
            &format!("float-sign-{found}"),
            &format!(
                r"
            answer : (F64, Nat) -> Bool
            answer(value, 0) -> ({left} - value).sign_bit()
            answer(value, n) -> answer(value, n - 1)
            main!(host) -> host.exit!(answer(F64.of_nat(host.args.len()), host.args.len()).select(1, 0))
        "
            ),
        );
        let program = optimized(&package);
        let facts = program.analyze();
        let recognized = (0..program.exprs_count())
            .any(|at| facts.float_comparison(barb::ExprId::at(at)).is_some());
        assert_eq!(recognized, found);
    }
}

#[test]
fn final_tree_facts_identify_the_builder_and_reader_before_lowering() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../frontend/examples/benchmarks-game/binary-trees/main.fn");
    let mut program =
        faun_frontend::lower_to_barb(&path, &faun_driver::platform(&X86_64Linux)).unwrap();
    barb::optimize(&mut program);
    let facts = program.analyze();
    let roles: Vec<_> = program
        .function_ids()
        .map(|id| facts.tree_role(id))
        .collect();
    assert!(roles.contains(&barb::TreeRole::Builder));
    assert!(roles.contains(&barb::TreeRole::Reader));
}

#[test]
fn float_doubling_is_rewritten_before_machine_lowering() {
    let package = Package::new(
        "float-doubling",
        r"
        twice : (F64, Nat) -> F64
        twice(value, 0) -> value * 2.0 + value
        twice(value, n) -> twice(value, n - 1)
        main!(host) -> (
          host.print!(twice(F64.of_nat(host.args.len()), host.args.len()).to_str())
          host.exit!(0)
        )
    ",
    );
    let program = optimized(&package);
    let facts = with_floats(&program);
    let function = program
        .functions()
        .iter()
        .find(|f| program.name(f.name) == "twice")
        .unwrap();
    let mut adds = 0;
    for expr in expressions(&program, function.body().unwrap()) {
        if let barb::Expr::Call(id, _) = expr
            && let Some(float) = facts.float(id)
        {
            assert_ne!(float.operation, barb::float::Operation::Mul);
            adds += usize::from(float.operation == barb::float::Operation::Add);
        }
    }
    assert!(adds >= 2);
}

fn expressions(program: &barb::Program, body: barb::Body) -> Vec<barb::Expr> {
    let mut result = Vec::new();
    for id in program
        .bindings(body.bindings)
        .iter()
        .copied()
        .chain([body.tail])
    {
        let expr = program.expr(id);
        result.push(expr);
        if let barb::Expr::Match(_, arms) = expr {
            for arm in program.arms(arms) {
                result.extend(expressions(program, arm.body));
            }
        }
    }
    result
}

/// Every float operation the standard has, at both formats.
const FLOATS: [(&str, &str); 2] = [("F64", "Binary64"), ("F32", "Binary32")];
const OPERATIONS: [(&str, &str); 5] = [
    ("add", "Add"),
    ("sub", "Sub"),
    ("mul", "Mul"),
    ("div", "Div"),
    ("sqrt", "Sqrt"),
];

/// A program asking every float operation of both formats.
fn every_float_operation() -> Package {
    Package::new(
        "every-float-operation",
        r"
        main!(host) -> (
          n = host.args.len()
          wide = F64.of_nat(n)
          narrow = F32.of_nat(n)
          host.print!((wide + wide - wide * wide / wide).sqrt().to_str())
          host.print!((narrow + narrow - narrow * narrow / narrow).sqrt().to_str())
          host.exit!(0)
        )
    ",
    )
}

/// Each standard float operation's fingerprint, as its code comes to after
/// canonicalizing, on every target, with the operation and format it is.
fn reference_fingerprints() -> BTreeMap<u128, (&'static str, &'static str)> {
    let package = every_float_operation();
    let targets: [&dyn Target; 4] = [
        &X86_64Linux,
        &faun_aarch64_macos::Aarch64Macos,
        &faun_wasm32_wasi::Wasm32Wasi,
        &faun_mos6502_sim65::Mos6502Sim65,
    ];
    let mut known = BTreeMap::new();
    for target in targets {
        let mut program =
            faun_frontend::lower_to_barb(&package.entry(), &faun_driver::platform(target)).unwrap();
        barb::test_support::canonical(&mut program);
        for (float, format) in FLOATS {
            for (method, operation) in OPERATIONS {
                let name = format!("{float}.{method}");
                let id = program
                    .function_ids()
                    .find(|&id| program.name(program.function(id).name) == name)
                    .unwrap_or_else(|| panic!("the standard has no `{name}`"));
                let print = barb::test_support::fingerprint(&program, id)
                    .unwrap_or_else(|| panic!("`{name}` has no fingerprint"));
                known.insert(print, (operation, format));
            }
        }
    }
    known
}

/// The table Barb recognizes float operations by, as Rust.
fn reference_table() -> String {
    let mut table = String::from(
        "//! Written by `FAUN_BLESS=1 cargo nextest run -p faun --test barb_recognition`\n\
         //! from the standard's float operations, as the code of each comes to after\n\
         //! canonicalizing. Do not edit by hand.\n\n\
         use crate::float::{Float, Format, Operation};\n\n\
         pub(super) const KNOWN: &[(u128, Float)] = &[\n",
    );
    for (print, (operation, format)) in reference_fingerprints() {
        let digits = format!("{print:032x}");
        let print = digits
            .as_bytes()
            .chunks(4)
            .map(|chunk| std::str::from_utf8(chunk).unwrap())
            .collect::<Vec<_>>()
            .join("_");
        writeln!(
            table,
            "    (\n        0x{print},\n        Float {{\n            operation: Operation::{operation},\n            format: Format::{format},\n        }},\n    ),"
        )
        .unwrap();
    }
    table.push_str("];\n");
    table
}

#[test]
fn the_reference_float_table_is_the_standards_code() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../barb/src/floats/known.rs");
    let table = reference_table();
    if std::env::var_os("FAUN_BLESS").is_some_and(|value| value != "0") {
        std::fs::write(&path, &table).unwrap();
        return;
    }
    assert!(
        std::fs::read_to_string(&path).unwrap() == table,
        "the standard's float code changed: run `just bless` and read the diff"
    );
}

/// What a failing recognition says: the table no longer matches the
/// standard's code, and how to write it again.
const STALE: &str = "Barb's table of the standard's float code (crates/barb/src/floats/known.rs) \
     is stale, so this operation would run as the software routine on x86-64: \
     run `just bless` (FAUN_BLESS=1 cargo nextest run -p faun --test barb_recognition \
     the_reference_float_table) and read the diff";

/// A program asking `float`'s `method` and no other float operation.
fn asking(float: &str, method: &str) -> Package {
    let asked = if method == "sqrt" {
        "x.sqrt()".to_owned()
    } else {
        let symbol = match method {
            "add" => "+",
            "sub" => "-",
            "mul" => "*",
            _ => "/",
        };
        format!("x {symbol} x")
    };
    Package::new(
        &format!("float-{float}-{method}"),
        &format!(
            r"
        main!(host) -> (
          x = {float}.of_nat(host.args.len())
          host.print!(({asked}).to_str())
          host.exit!(0)
        )
    "
        ),
    )
}

/// Each operation the table covers, at each format, asked alone by a
/// program built for x86-64, whose float instructions answer it, is
/// recognized as itself: a library edit that changes a routine's code
/// without blessing the table would otherwise leave float code silently on
/// the software routines.
#[test]
fn every_float_operation_is_recognized_by_its_code() {
    for (float, format) in FLOATS {
        for (method, operation) in OPERATIONS {
            let name = format!("{float}.{method}");
            let program = optimized(&asking(float, method));
            let facts = with_floats(&program);
            // A recognized definition is never inlined, so one swept from a
            // program asking it was not recognized either.
            let recognized = program
                .function_ids()
                .find(|&id| program.name(program.function(id).name) == name)
                .and_then(|id| facts.float(id))
                .unwrap_or_else(|| panic!("`{name}` is not recognized: {STALE}"));
            assert_eq!(
                format!("{:?}", recognized.operation),
                operation,
                "{name}: {STALE}"
            );
            assert_eq!(
                format!("{:?}", recognized.format),
                format,
                "{name}: {STALE}"
            );
        }
    }
}
