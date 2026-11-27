use super::test_support::returned;
use super::{Binary, Class, Op, Program, claims::place, validate};

/// How many claims and releases `p` writes.
fn counted(p: &Program) -> (usize, usize) {
    let body = p.function(p.entry()).body().unwrap();
    let claims = p
        .every(body)
        .filter(|op| matches!(op, Op::Claim { .. }))
        .count();
    let releases = p
        .every(body)
        .filter(|op| matches!(op, Op::Release { .. }))
        .count();
    (claims, releases)
}

/// A one-element buffer pushed onto twice: the first push reads it before
/// its last use, so it takes a claim of its own, and the second moves the
/// buffer's claim; each longer buffer is given up after it is read.
#[test]
fn a_use_that_takes_a_claim_moves_it_where_it_is_last_and_takes_one_otherwise() {
    let (mut p, main) = Program::new("main");
    p.define(main, |b, _| {
        let word = b.shape(&[Class::Word]);
        let zero = b.constant(Class::Word, 0);
        let empty = b.buffer_empty(word);
        let one = b.constant(Class::Word, 1);
        let [first, start] = b.buffer_push((empty, zero, zero), word, &[one]);
        let two = b.constant(Class::Word, 2);
        let left = b.buffer_push((first, start, one), word, &[two]);
        let three = b.constant(Class::Word, 3);
        let right = b.buffer_push((first, start, one), word, &[three]);
        let a = b.buffer_field((Class::Word, word), left.into(), 0);
        let c = b.buffer_field((Class::Word, word), right.into(), 0);
        let sum = b.binary(Binary::Add, a, c);
        b.ret(&[sum]);
    });
    validate(&p).unwrap();
    let placed = place(&p, 64, true);
    validate(&placed).unwrap();
    assert_eq!(counted(&p), (0, 0));
    assert_eq!(counted(&placed), (1, 2));
    assert_eq!(returned(&placed, placed.entry()), returned(&p, p.entry()));
    assert_eq!(returned(&placed, placed.entry()), [5u8.into()]);
}
