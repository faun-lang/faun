//! Representation and ownership tests use the scalar memory interpreter, not
//! only low exit statuses. Runtime inputs are parameters, never folded terms.
use num_bigint::BigUint;

use super::evaluate::{self, Evaluation, Host, HostResult, Memory};
use super::natural::Config;
use super::test_support::{live_objects, no_live_objects, word};
use super::*;

/// A cell with no fields is the same whatever makes it, so each of a
/// constructor's is one immortal object laid out before the program runs:
/// nothing allocates one, counts claims to it, or frees it.
#[test]
#[expect(
    clippy::redundant_closure_for_method_calls,
    reason = "the builder method is not lifetime-general enough as a direct callback"
)]
fn a_cell_with_no_fields_is_never_allocated() {
    let (mut source, main) = Program::new("main");
    let identity = source.declare("identity", &[Class::Cell], &[Class::Cell]);
    source.define(identity, |b, params| b.ret(params));
    source.define(main, |b, _| {
        let none = b.shape(&[]);
        let first = b.cell(1, none, &[]);
        let again = b.cell(1, none, &[]);
        let other = b.cell(2, none, &[]);
        let passed = b.call_one(identity, &[again]);
        let tags: Vec<_> = [first, passed, other]
            .iter()
            .map(|&cell| b.cell_tag(cell))
            .collect();
        let sum = b.binary(Binary::Add, tags[0], tags[1]);
        let sum = b.binary(Binary::Add, sum, tags[2]);
        b.ret(&[sum]);
    });
    for inline_counts in [false, true] {
        let config = Config {
            word_bits: 64,
            heap_bytes: 512,
            grows: false,
            inline_small: true,
            inline_counts,
            join_routines: false,
        };
        let legalized = legalize(&source, config).unwrap();
        let allocations = allocations_in(&legalized, &[main, identity]);
        assert_eq!(allocations, 0, "inline counts: {inline_counts}");
        let evaluation = run(&legalized, main, &[], 64);
        assert_eq!(evaluation.returned().unwrap(), &[BigUint::from(4u8)]);
        no_live_objects(&legalized, &evaluation, config);
    }
}

/// A buffer laid out before the program runs is immortal, so a table of
/// views into it counts no claims to it however many rows it pushes: no row
/// retains it, and the rows read back what was laid out.
#[test]
fn rows_of_an_immortal_buffer_count_no_claims_to_it() {
    let (mut source, main) = Program::new("main");
    source.define(main, |b, _| {
        let bytes = b.shape(&[Class::fixed(8)]);
        let data = b.data(b"table");
        let table = b.buffer_known(data, bytes, 1);
        let rows = b.shape(&[Class::Buffer, Class::Word, Class::Word]);
        let mut outer = b.buffer_empty(rows);
        let mut start = b.constant(Class::Word, 0);
        for (at, (first, len)) in [(0, 2), (2, 3)].into_iter().enumerate() {
            let held = b.constant(Class::Word, u64::try_from(at).unwrap());
            let first = b.constant(Class::Word, first);
            let len = b.constant(Class::Word, len);
            [outer, start] = b.buffer_push((outer, start, held), rows, &[table, first, len]);
        }
        let len = b.buffer_field((Class::Word, rows), (outer, start), 2);
        b.ret(&[len]);
    });
    let config = Config {
        word_bits: 64,
        heap_bytes: 512,
        grows: false,
        inline_small: true,
        inline_counts: false,
        join_routines: false,
    };
    let legalized = legalize(&source, config).unwrap();
    let evaluation = run(&legalized, main, &[], 64);
    assert_eq!(evaluation.returned().unwrap(), &[BigUint::from(3u8)]);
    let retain = legalized
        .functions()
        .iter()
        .position(|function| legalized.name(function.name) == "nat.retain");
    if let Some(retain) = retain {
        assert_eq!(evaluation.counts.calls[retain], 0, "{}", legalized.render());
    }
    no_live_objects(&legalized, &evaluation, config);
}

/// How many times the bodies of `functions` call the allocator: what the
/// program itself does, past the runtime it is given.
fn allocations_in(program: &Program, functions: &[FunctionId]) -> usize {
    functions
        .iter()
        .filter_map(|f| program.functions()[f.index()].body())
        .flat_map(|body| program.walk_within(body))
        .filter(|(op, _)| {
            matches!(op, Op::Call { function, .. }
                if program.name(program.functions()[function.index()].name) == "nat.allocate")
        })
        .count()
}

#[derive(Default)]
struct TestHost {
    calls: Vec<String>,
}

impl Host for TestHost {
    fn call(
        &mut self,
        name: &str,
        args: &[BigUint],
        _: &mut Memory<'_>,
    ) -> Result<HostResult, evaluate::Error> {
        self.calls.push(name.to_owned());
        if name == "exit!" {
            Ok(HostResult::Halt(args.to_vec()))
        } else if name == "mark" {
            Ok(HostResult::Return(vec![]))
        } else {
            Err(evaluate::Error::Host(name.to_owned()))
        }
    }
}

fn run(p: &Program, f: FunctionId, args: &[BigUint], word_bits: u16) -> Evaluation {
    evaluate::evaluate(
        p,
        f,
        args,
        evaluate::Config {
            word_bits,
            fuel: 100_000_000,
            ..evaluate::Config::default()
        },
        &mut TestHost::default(),
    )
    .expect("reference execution")
}

fn decoded(p: &Program, evaluation: &Evaluation, bits: u16) -> BigUint {
    let address = word(&evaluation.returned().unwrap()[0].to_bytes_le());
    // An immediate is twice its number; a handle names its object one past
    // it.
    if address.is_multiple_of(2) {
        return BigUint::from(address >> 1);
    }
    let address = address - 1;
    let globals = usize::try_from(evaluate::globals_base(p).unwrap()).unwrap();
    let data = usize::try_from(evaluate::DATA_BASE).unwrap();
    let (bytes, start) = if address >= globals {
        (&evaluation.globals[..], address - globals)
    } else {
        (p.data(), address - data)
    };
    let w = usize::from(bits / 8);
    let len = word(&bytes[start + 2 * w..start + 3 * w]);
    // Each digit is a word, and the last is not nought.
    let digits = &bytes[start + 3 * w..start + 3 * w + len * w];
    assert!(
        digits.is_empty() || digits[digits.len() - w..].iter().any(|&byte| byte != 0),
        "normalized"
    );
    if address >= globals {
        assert_eq!(
            word(&bytes[start + w..start + 2 * w]),
            1,
            "one returned claim"
        );
    }
    BigUint::from_bytes_le(digits)
}

/// Around half a word, where a natural stops being an immediate and starts
/// being an object: every operation, on every pair of numbers either side,
/// answers what the reference answers, at every width.
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "differential fixture over widths, operations, and pairs either side of the boundary"
)]
fn arithmetic_across_the_immediate_boundary_matches_the_reference() {
    for bits in [16, 32, 64] {
        let half = natural::half(bits);
        let most = u64::MAX >> (64 - bits);
        let values = [0, 1, half - 2, half - 1, half, half + 1, most];
        let ops = [
            Ok(Binary::Add),
            Ok(Binary::Sub),
            Ok(Binary::Mul),
            Ok(Binary::Div),
            Ok(Binary::Mod),
            Err(Relation::Equal),
            Err(Relation::Less),
        ];
        for op in ops {
            let (mut p, main) = Program::new("main");
            p.define(main, |b, _| {
                let zero = b.constant(Class::Word, 0);
                b.ret(&[zero]);
            });
            let result = if op.is_ok() {
                Class::Natural
            } else {
                Class::Word
            };
            let test = p.declare("test", &[Class::Word, Class::Word], &[result]);
            p.define(test, |b, args| {
                let left = b.convert(Class::Natural, args[0]);
                let right = b.convert(Class::Natural, args[1]);
                let answer = match op {
                    Ok(op) => b.binary(op, left, right),
                    Err(relation) => b.compare(relation, left, right),
                };
                b.ret(&[answer]);
            });
            let config = Config {
                word_bits: bits,
                heap_bytes: 2048,
                grows: false,
                inline_small: true,
                inline_counts: false,
                join_routines: false,
            };
            let scalar = legalize(&p, config).expect("legalized");
            for &x in &values {
                for &y in &values {
                    let args = [BigUint::from(x), BigUint::from(y)];
                    let expected = run(&p, test, &args, bits).returned().unwrap()[0].clone();
                    let actual = run(&scalar, test, &args, bits);
                    let answer = if op.is_ok() {
                        decoded(&scalar, &actual, bits)
                    } else {
                        actual.returned().unwrap()[0].clone()
                    };
                    assert_eq!(answer, expected, "{bits} {op:?} {x} {y}");
                    // An answer below half a word is an immediate, and one
                    // at or past it the one object still claimed.
                    let object = op.is_ok() && answer >= BigUint::from(half);
                    assert_eq!(
                        live_objects(&scalar, &actual, config),
                        usize::from(object),
                        "{bits} {op:?} {x} {y}"
                    );
                }
            }
        }
    }
}

/// The last immediate and the first boxed word meet at the same boundary
/// for every layout. Only the guarded arm may decode by shifting the handle.
#[test]
fn guarded_naturals_decode_only_within_the_immediate_range() {
    for (bits, limit) in [16u16, 32, 64]
        .into_iter()
        .flat_map(|bits| [1, 256, 1u64 << (bits - 1)].map(|limit| (bits, limit)))
    {
        let (mut p, main) = Program::new_with_params("main", &[("n", Class::Word)]);
        p.define(main, |b, args| {
            let n = b.convert(Class::Natural, args[0]);
            let bound = b.constant(Class::fixed(bits), limit);
            let bound = b.convert(Class::Natural, bound);
            let below = b.compare(Relation::Less, n, bound);
            let result = b.if_(
                below,
                &[Class::Word],
                |b| {
                    let word = b.convert(Class::Word, n);
                    b.yield_(&[word]);
                },
                |b| {
                    let word = b.convert(Class::Word, n);
                    b.yield_(&[word]);
                },
            );
            b.ret(&result);
        });
        let config = fixed_heap(bits, 512);
        let scalar = legalize(&p, config).unwrap();
        for n in [0, limit - 1, limit, limit + 1, u64::MAX >> (64 - bits)] {
            let args = [BigUint::from(n)];
            let actual = run(&scalar, main, &args, bits);
            assert_eq!(actual.outcome, run(&p, main, &args, bits).outcome);
            // Past the guard an immediate is still read where it is; only
            // an object asks the helper.
            assert_eq!(
                actual.counts.calls_to(&scalar, "nat.to_word"),
                u64::from(n >= natural::half(bits))
            );
            no_live_objects(&scalar, &actual, config);
        }
    }
}

/// A call that does not come back ends its region, so a claim the caller
/// still holds when it is made is never released after it: the call reads
/// a cell it borrows and leaves the program, and nothing follows it. The
/// 6502 hands every argument past its fourth over in a cell, so a function
/// taking five that never comes back, as md5's loop over its input does,
/// is called this way.
#[test]
fn nothing_is_released_after_a_call_that_does_not_come_back() {
    let (mut p, main) = Program::new("main");
    let exit = crate::platform::Routines::builtin().declare(&mut p, "exit!");
    let leave = p.declare_never("leave", &[Class::Cell]);
    let word = p.shape(&[Class::Word]);
    p.define(leave, |b, params| {
        let status = b.cell_field((Class::Word, word), params[0], 0);
        b.call(exit, &[status]);
    });
    p.define(main, |b, _| {
        let status = b.constant(Class::Word, 42);
        let cell = b.cell(0, word, &[status]);
        b.call(leave, &[cell]);
    });
    for bits in [16, 32, 64] {
        let legalized = legalize(&p, fixed_heap(bits, 256)).unwrap();
        let ops = legalized.ops(legalized.function(main).body().unwrap());
        assert!(matches!(ops.last(), Some(Op::Call { function, .. }) if *function == leave));
        let mut host = TestHost::default();
        let config = evaluate::Config {
            word_bits: bits,
            ..evaluate::Config::default()
        };
        let result = evaluate::evaluate(&legalized, main, &[], config, &mut host).unwrap();
        assert_eq!(result.halted().unwrap(), [BigUint::from(42u8)]);
    }
}

/// A heap of `heap_bytes` that exits when it is full.
const fn fixed_heap(word_bits: u16, heap_bytes: usize) -> Config {
    Config {
        word_bits,
        heap_bytes,
        grows: false,
        inline_small: true,
        inline_counts: false,
        join_routines: false,
    }
}

#[test]
fn unsupported_representations_are_errors_not_pointer_reinterpretations() {
    for (source, target) in [
        (Class::Address, Class::Natural),
        (Class::Natural, Class::Address),
    ] {
        let (mut p, main) = Program::new("main");
        p.define(main, |b, _| {
            let value = b.constant(source, 1);
            b.convert(target, value);
            let zero = b.constant(Class::Word, 0);
            b.ret(&[zero]);
        });
        assert!(legalize(&p, fixed_heap(64, 256)).is_err());
    }
    let (mut p, main) = Program::new("main");
    // Wider than the machine's word, which splitting holds as words first;
    // legalizing it as it stands would read it as one.
    p.define(main, |b, _| {
        b.constant(Class::fixed(17), 1);
        let zero = b.constant(Class::Word, 0);
        b.ret(&[zero]);
    });
    assert!(
        legalize(&p, fixed_heap(16, 256))
            .unwrap_err()
            .contains("held as words")
    );
    assert!(legalize(&split(&p, 16), fixed_heap(16, 256)).is_ok());
}

#[test]
fn natural_host_signatures_do_not_become_address_signatures() {
    let mut routines = crate::platform::Routines::builtin();
    routines.push(crate::platform::Routine {
        name: "foreign".into(),
        offered: false,
        authority: false,
        takes: vec![crate::platform::Takes::Scalar(Class::Natural)],
        returns: crate::platform::Returns::Value(Class::Natural),
        built: None,
        consumes: false,
    });
    let (mut p, main) = Program::new("main");
    let foreign = routines.declare(&mut p, "foreign");
    p.define(main, |b, _| {
        let natural = b.constant(Class::Natural, 1);
        b.call(foreign, &[natural]);
        let zero = b.constant(Class::Word, 0);
        b.ret(&[zero]);
    });
    let error = legalize(
        &p,
        Config {
            word_bits: 64,
            heap_bytes: 256,
            grows: false,
            inline_small: true,
            inline_counts: false,
            join_routines: false,
        },
    )
    .unwrap_err();
    assert!(error.contains("bigint ABI"), "{error}");
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "host setup, allocation fixture and observed failure ordering"
)]
fn exhaustion_preserves_effect_order_and_exits_instead_of_returning_zero() {
    for bits in [16, 32, 64] {
        let mut routines = crate::platform::Routines::builtin();
        routines.push(crate::platform::Routine {
            name: "mark".into(),
            offered: false,
            authority: false,
            takes: vec![],
            returns: crate::platform::Returns::Unit,
            built: None,
            consumes: false,
        });
        let (mut p, main) = Program::new("main");
        let mark = routines.declare(&mut p, "mark");
        p.define(main, |b, _| {
            b.call(mark, &[]);
            // Past where immediates stop, so each is an object.
            let word = b.constant(Class::Word, natural::half(bits) + 100);
            // Both are read after the second is made, so both are live when
            // it asks for room: a dead first one would already be released.
            let first = b.convert(Class::Natural, word);
            let second = b.convert(Class::Natural, word);
            b.compare(Relation::Equal, first, second);
            b.call(mark, &[]);
            b.ret(&[word]);
        });
        let scalar = legalize(
            &p,
            Config {
                word_bits: bits,
                heap_bytes: usize::from(bits / 8) * 4,
                grows: false,
                inline_small: true,
                inline_counts: false,
                join_routines: false,
            },
        )
        .unwrap();
        let mut host = TestHost::default();
        let result = evaluate::evaluate(
            &scalar,
            main,
            &[],
            evaluate::Config {
                word_bits: bits,
                ..evaluate::Config::default()
            },
            &mut host,
        )
        .unwrap();
        assert_eq!(result.halted().unwrap(), [BigUint::from(2u8)]);
        assert_eq!(host.calls, ["mark", "exit!"]);
    }
}

#[test]
fn source_host_arguments_are_not_implicitly_truncated() {
    // A sum of two counts a word holds may not fit a word, and a status must.
    let routines = crate::platform::Routines::builtin().with_word(64);
    let mut library = barb::fixture::Library::new();
    let (nat, add) = (library.nat, library.add);
    let world = tests::host(&mut library.program, nat);
    let main =
        library
            .program
            .declare_named("main!", &[("host", world.host), ("argc", nat)], world.never);
    // The count is a word because the target says so of a parameter named
    // for it, when lowering.
    library.program.define(main, |b, args| {
        let n = args[1];
        let doubled = b.call(add, &[n, n]);
        b.call(world.exit, &[args[0], doubled])
    });
    library.program.set_entry(main);
    let error = tests::machine(library.program, &routines).unwrap_err();
    assert!(
        error.contains("host arguments must be proven to fit"),
        "{error}"
    );
    assert!(
        error.contains("no bound was found for what `main!` hands it"),
        "{error}"
    );

    // A status known to be small fits even a narrow word.
    let routines = crate::platform::Routines::builtin().with_word(16);
    let mut library = barb::fixture::Library::new();
    let one = library.number(1);
    let world = tests::host(&mut library.program, library.nat);
    let main = library
        .program
        .declare_named("main!", &[("host", world.host)], world.never);
    library.program.define(main, |b, args| {
        let one = b.known(one);
        b.call(world.exit, &[args[0], one])
    });
    library.program.set_entry(main);
    tests::machine(library.program, &routines).unwrap();
}

#[test]
fn scalar_fast_path_has_no_runtime_code_data_or_globals() {
    let (mut p, main) = Program::new("main");
    p.define(main, |b, _| {
        let answer = b.constant(Class::Word, 42);
        b.ret(&[answer]);
    });
    let scalar = legalize(
        &p,
        Config {
            word_bits: 64,
            heap_bytes: 0,
            grows: false,
            inline_small: true,
            inline_counts: false,
            join_routines: false,
        },
    )
    .unwrap();
    assert_eq!(format!("{p:?}"), format!("{scalar:?}"));
}
