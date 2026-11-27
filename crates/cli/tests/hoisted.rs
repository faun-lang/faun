//! A value built from constants alone inside a loop is built once before
//! it, and the loop reads that one: what each round reads is the same, and
//! a round that changes it changes its own copy, on every target.

mod support;

use support::{MACHINES, Package, faun};

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

#[test]
fn a_list_of_strings_read_every_round_is_the_same_every_round() {
    // Every round reads a word of a constant list, and appends one to the
    // list it reads and measures it: built once, the list is shared by the
    // rounds, so each append is to a copy, and the next round reads the
    // list as it was.
    let package = Package::new(
        "hoisted-words",
        r#"
        walk : (Nat, Nat, List(U8)) -> Str
        walk(0, total, out) -> "${out.len()} ${total} ${Str.from_utf8(out.take(24)).ok_or("")}"
        walk(Succ(left), total, out) -> (
          words = ["the", "quick", "brown", "fox"].map(|word| word.to_utf8())
          word = words.get(left % 4).ok_or([])
          grown = words.append("over".to_utf8())
          walk(left, total + grown.len(), out.concat(word).append(' '))
        )

        main!(host) -> (
          printed = host.println!(walk(9 + host.args.len(), 0, []))
          host.exit!(0)
        )
        "#,
    );
    let words = ["the", "quick", "brown", "fox"];
    let mut out = String::new();
    for left in (0..10).rev() {
        out.push_str(words[left % 4]);
        out.push(' ');
    }
    let expected = format!("{} {} {}", out.len(), 10 * 5, &out[..24]);
    prints_everywhere(&package, &expected);
}

#[test]
fn a_constant_list_of_small_naturals_is_laid_out_in_the_image() {
    // Three hundred noughts and a table of digits are known before the
    // program runs, each element a natural held in place: they are data
    // in the image, read where they are, not built a push at a time.
    let package = Package::new(
        "laid-out-naturals",
        r#"
        counts : List(Nat)
        counts = List.repeat(0, 300)

        digits : List(Nat)
        digits = [3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9, 7, 9, 3]

        main!(host) -> (
          n = Nat.from_str(host.args.get(1).ok_or("7")).ok_or(7)
          bumped = counts.update(n, |count| count + 1)
          printed = host.println!("${bumped.fold(0, |sum, count| sum + count)} ${digits.get(n % 16).ok_or(0)} ${digits.fold(0, |sum, d| sum + d)}")
          host.exit!(0)
        )
        "#,
    );
    prints_everywhere(&package, "1 6 80");
    for target in MACHINES {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_faun"))
            .args(["emit", "--target", target, "machine"])
            .arg(package.entry())
            .output()
            .unwrap();
        let machine = String::from_utf8_lossy(&result.stdout);
        let pushes = machine.matches("buffer_push").count();
        assert!(pushes < 16, "{target}: {pushes} pushes\n{machine}");
    }
}
