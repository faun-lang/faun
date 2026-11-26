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
