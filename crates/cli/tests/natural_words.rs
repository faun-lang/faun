//! A natural a loop carries is a machine word wherever its range is proven
//! to fit one, and exact everywhere else: a position stepped under a guard
//! comparing it with a list's length, and a count below a constant joined
//! to another test by `&`, cost what the same loop over `U64` costs, while
//! a natural whose guard is no length crosses every word width and stays
//! the number it is, on every target.
mod support;

use std::process::Command;
use support::{MACHINES, Package, faun, run_counted};

/// The fuel a program spends on `steps` bytes of input, and what it prints.
fn spent(package: &Package, steps: usize) -> (String, u64) {
    let input: Vec<u8> = (0..steps)
        .map(|at| u8::try_from(at * 7 % 13).unwrap())
        .collect();
    let (status, written, fuel) = run_counted(&package.entry(), &input);
    assert_eq!(status, 0);
    (written.trim().to_owned(), fuel)
}

/// What 100 bytes of input more cost, a byte.
fn per_byte(package: &Package) -> (String, u64) {
    let (_, first) = spent(package, 200);
    let (printed, second) = spent(package, 300);
    (printed, (second - first) / 100)
}

/// A program printing `walk(input.stdin)`, which `source` defines.
fn walking(name: &str, source: &str) -> Package {
    Package::new(
        name,
        &format!(
            r#"
            {source}

            app : {{stdin: List(U8), *}} -> Str
            app(input) -> "${{walk(input.stdin)}}"

            main!(host) -> host.run_stdin!(app)
            "#
        ),
    )
}

/// That the natural spelling prints what the workaround over `U64` prints,
/// and costs no more than `slack` percent more a byte.
fn as_cheap(name: &str, slack: u64, natural: &str, workaround: &str) {
    let (natural_printed, natural_byte) = per_byte(&walking(name, natural));
    let (hand_printed, hand_byte) = per_byte(&walking(&format!("{name}-u64"), workaround));
    assert_eq!(natural_printed, hand_printed);
    assert!(
        natural_byte * 100 <= hand_byte * (100 + slack),
        "a byte cost {natural_byte} fuel written naturally and {hand_byte} over U64"
    );
}

#[test]
fn a_position_below_a_length_is_a_word() {
    // `at + 2 < input.len()` bounds `at` itself, and the walk hands `at + 1`
    // on only there, so every position is below the length.
    as_cheap(
        "guarded-position",
        3,
        r"
        step : (List(U8), Nat, Nat) -> Nat
        step(input, at, total) -> (
          if at + 2 >= input.len() return total
          step(input, at + 1, total + input.get(at).ok_or(0).to_nat() + input.get(at + 2).ok_or(0).to_nat())
        )

        walk : List(U8) -> Nat
        walk(input) -> step(input, 0, 0)
        ",
        r"
        step : (List(U8), U64, Nat) -> Nat
        step(input, at, total) -> (
          if (at + 2).to_nat() >= input.len() return total
          step(input, at + 1, total + input.get(at.to_nat()).ok_or(0).to_nat() + input.get((at + 2).to_nat()).ok_or(0).to_nat())
        )

        walk : List(U8) -> Nat
        walk(input) -> step(input, U64.of_nat(0), 0)
        ",
    );
}

#[test]
fn a_count_below_a_constant_joined_by_and_is_a_word() {
    // `count < 16 & ...` is a `Bool` worked out by a match: its `True` says
    // the first comparison held, so the count handed on is below 17.
    as_cheap(
        "guarded-count",
        3,
        r"
        agree : (List(U8), Nat, Nat) -> Nat
        agree(input, at, count) if count < 16 & input.get(at + count).ok_or(0) == input.get(count).ok_or(0) ->
          agree(input, at, count + 1)
        agree(_, _, count) -> count

        step : (List(U8), Nat, Nat) -> Nat
        step(input, at, total) -> (
          if at >= input.len() return total
          step(input, at + 1, total + agree(input, at, 0))
        )

        walk : List(U8) -> Nat
        walk(input) -> step(input, 0, 0)
        ",
        r"
        agree : (List(U8), Nat, U64) -> U64
        agree(input, at, count) if count < 16 & input.get(at + count.to_nat()).ok_or(0) == input.get(count.to_nat()).ok_or(0) ->
          agree(input, at, count + 1)
        agree(_, _, count) -> count

        step : (List(U8), Nat, Nat) -> Nat
        step(input, at, total) -> (
          if at >= input.len() return total
          step(input, at + 1, total + agree(input, at, U64.of_nat(0)).to_nat())
        )

        walk : List(U8) -> Nat
        walk(input) -> step(input, 0, 0)
        ",
    );
}

#[test]
fn a_words_difference_from_nought_is_worked_out_in_the_word() {
    // `0 - w` on a `U64` is `(0 + 2^64) - w` modulo 2^64, whose sum is a
    // number known before the program runs: it is the word's own negation,
    // as `!w + 1` is, never a natural as wide as 2^64.
    as_cheap(
        "word-negated",
        3,
        r"
        step : (List(U8), Nat, U64, U64) -> U64
        step(input, at, word, total) -> (
          if at >= input.len() return total
          next = word * 6364136223846793005 + U64.of_nat(input.get(at).ok_or(0).to_nat())
          step(input, at + 1, next, total + (next & (0 - next)))
        )

        walk : List(U8) -> U64
        walk(input) -> step(input, 0, U64.of_nat(1), U64.of_nat(0))
        ",
        r"
        step : (List(U8), Nat, U64, U64) -> U64
        step(input, at, word, total) -> (
          if at >= input.len() return total
          next = word * 6364136223846793005 + U64.of_nat(input.get(at).ok_or(0).to_nat())
          step(input, at + 1, next, total + (next & (!next + 1)))
        )

        walk : List(U8) -> U64
        walk(input) -> step(input, 0, U64.of_nat(1), U64.of_nat(0))
        ",
    );
}

/// That `package` prints `expected` built for every target.
fn prints_everywhere(package: &Package, expected: &str) {
    for target in MACHINES {
        let (status, written, said) =
            faun(&["run", "--target", target], &package.entry(), &[], b"");
        assert_eq!(
            (status, written.trim()),
            (Some(0), expected),
            "{target}: {said}"
        );
    }
}

/// The sum `walk` prints from `start` stepping by `step` while below
/// `limit`, and its last position, worked out as the program should.
fn stepped(start: u128, step: u128, limit: u128) -> String {
    let (mut at, mut sum, mut last) = (start, 0u128, start);
    while at + step <= limit {
        sum += at;
        last = at;
        at += step;
    }
    format!("{sum} {last}")
}

#[test]
fn a_natural_guarded_by_no_length_crosses_every_word_exactly() {
    // The limit is no list's length, so nothing bounds the position but the
    // natural it is: each walk starts a few steps below a word's top and
    // goes past it, which a position held in a word would wrap.
    let package = Package::new(
        "crossing-words",
        r#"
        walk : (Nat, Nat, Nat, Nat) -> Str
        walk(at, limit, sum, last) -> (
          if at + 3 > limit return "${sum} ${last}"
          walk(at + 3, limit, sum + at, at)
        )

        main!(host) -> (
          lift = host.args.len()
          lines = [16, 32, 64].map(|bits| walk(2.pow(bits) - 7 + lift, 2.pow(bits) + 8, 0, 0))
          printed = host.println!(lines.join(" | "))
          host.exit!(0)
        )
        "#,
    );
    let expected: Vec<String> = [16u32, 32, 64]
        .iter()
        .map(|&bits| stepped((1u128 << bits) - 7 + 1, 3, (1u128 << bits) + 8))
        .collect();
    prints_everywhere(&package, &expected.join(" | "));
}

#[test]
fn a_natural_a_branch_bounds_is_narrowed_where_it_is_operated_on() {
    // Below ten the branch proves `n` a digit, and the operation writing it
    // out reads it as one, on every backend, rather than as the widest
    // number a word pair holds, which a 32-bit target splits in two and the
    // 6502 in four.
    let package = Package::new(
        "branch-narrowed",
        r#"
        digits : (Nat, Nat) -> Nat
        digits(n, total) -> (
          if n < 10 return total + n % 10
          digits(n / 10, total + n % 10)
        )

        main!(host) -> (
          printed = host.println!("${digits(12345 + host.args.len(), 0)}")
          host.exit!(0)
        )
        "#,
    );
    prints_everywhere(&package, "16");
    for target in MACHINES {
        let result = Command::new(env!("CARGO_BIN_EXE_faun"))
            .args(["emit", "--target", target, "machine"])
            .arg(package.entry())
            .output()
            .unwrap();
        let machine = String::from_utf8_lossy(&result.stdout);
        assert!(machine.contains("to 0..9\n"), "{target}: {machine}");
        assert!(
            !machine.contains(&format!("to 0..{}\n", u64::MAX)),
            "{target}: {machine}"
        );
    }
}

#[test]
fn a_quotient_or_remainder_by_a_power_of_two_is_a_shift_or_a_mask() {
    // `x` holds sixteen bits, and `k` is read while the program runs: below
    // the width, at it, past it, and as a natural no word holds, whose power
    // would never finish being built. Each answer is `x` moved down `k`
    // places, or its low `k` bits, on every target.
    let package = Package::new(
        "power-of-two-divisor",
        r#"
        main!(host) -> (
          lift = host.args.len()
          x = (1000 + lift) * 97 % 65536
          ks = [0, 1, 7, 15, 16, 17, 63, 64, 1000, 2.pow(64)].map(|k| k + lift - 1)
          lines = ks.map(|k| "${x / 2.pow(k)} ${x % 2.pow(k)}")
          printed = host.println!(lines.join(" | "))
          host.exit!(0)
        )
        "#,
    );
    let x = 1001u128 * 97 % 65536;
    let expected: Vec<String> = [0u32, 1, 7, 15, 16, 17, 63, 64, 1000, u32::MAX]
        .iter()
        .map(|&k| {
            let (quotient, remainder) = if k < 16 {
                (x >> k, x & ((1 << k) - 1))
            } else {
                (0, x)
            };
            format!("{quotient} {remainder}")
        })
        .collect();
    prints_everywhere(&package, &expected.join(" | "));
}

#[test]
fn a_power_of_two_divided_by_is_never_built() {
    // `k` is known only while the program runs, so `2.pow(k)` is no
    // constant, and nothing else in the program multiplies: built, the
    // power would be squared and multiplied in, a step for each bit of `k`.
    // Read as a shift and a mask, it is never built, on any target.
    let package = Package::new(
        "power-of-two-unbuilt",
        r#"
        walk : (List(U8), Nat, Nat, Nat) -> Nat
        walk([], _, _, sum) -> sum
        walk([byte, *rest], k, x, sum) ->
          walk(rest, k, (x + byte.to_nat() + 7) % 65536, sum + x / 2.pow(k) + x % 2.pow(k))

        main!(host) -> (
          k = host.args.len() + 3
          printed = host.println!("${walk([5, 6, 7, 200], k, 1, 0)}")
          host.exit!(0)
        )
        "#,
    );
    prints_everywhere(&package, "35");
    for target in MACHINES {
        let result = Command::new(env!("CARGO_BIN_EXE_faun"))
            .args(["emit", "--target", target, "machine"])
            .arg(package.entry())
            .output()
            .unwrap();
        let machine = String::from_utf8_lossy(&result.stdout);
        assert!(!machine.contains(" = Mul "), "{target}: {machine}");
    }
}

#[test]
fn a_natural_moves_by_whole_digits_and_the_bits_between() {
    // Numbers of one digit and of several, on every word size, moved by
    // distances below a digit, at one, between two and past every digit
    // they have, each distance read while the program runs. Moved up and
    // back down, a number is itself; moved down, it is its quotient.
    let package = Package::new(
        "natural-shifts",
        r#"
        main!(host) -> (
          lift = host.args.len() - 1
          xs = [5, 65535, 4294967297, 2.pow(100) + 12345, 3.pow(90)].map(|x| x + lift)
          ks = [0, 1, 3, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 200, 2.pow(70)].map(|k| k + lift)
          lines = xs.map(|x| ks.map(|k| "${x >> k} ${(x << k.min(300)) >> k.min(300) == x}").join(" "))
          printed = host.println!(lines.join(" | "))
          host.exit!(0)
        )
        "#,
    );
    let expected = moved_lines();
    prints_everywhere(&package, &expected.join(" | "));
}

/// The distances `a_natural_moves_by_whole_digits_and_the_bits_between`
/// moves by, the last standing for one no immediate holds.
const DISTANCES: [u32; 16] = [
    0,
    1,
    3,
    15,
    16,
    17,
    31,
    32,
    33,
    63,
    64,
    65,
    127,
    128,
    200,
    u32::MAX,
];

/// What `a_natural_moves_by_whole_digits_and_the_bits_between` prints: each
/// number moved down each distance, and that each moved up and back is
/// itself, a line a number.
fn moved_lines() -> Vec<String> {
    use num_bigint::BigUint;
    let xs = [
        BigUint::from(5u8),
        BigUint::from(65535u32),
        BigUint::from(4_294_967_297u64),
        (BigUint::from(1u8) << 100u32) + 12345u32,
        BigUint::from(3u8).pow(90),
    ];
    xs.iter()
        .map(|x| {
            let moved = DISTANCES.map(|k| {
                let down = if k == u32::MAX {
                    BigUint::default()
                } else {
                    x >> k
                };
                format!("{down} True")
            });
            moved.join(" ")
        })
        .collect()
}
