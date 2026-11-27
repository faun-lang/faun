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

/// Every claim to a natural a word holds goes: a byte widened, and that
/// doubled, are each held in a word, so neither is counted.
#[test]
fn a_natural_held_in_its_word_takes_no_claim() {
    let (mut p, main) = Program::new("main");
    let keep = p.declare("keep", &[Class::Natural, Class::Natural], &[Class::Word]);
    p.define(keep, |b, params| {
        let small = b.convert(Class::Natural, params[0]);
        let _ = small;
        let zero = b.constant(Class::Word, 0);
        b.ret(&[zero]);
    });
    p.define(main, |b, _| {
        let byte = b.constant(Class::fixed(8), 200);
        let wide = b.convert(Class::Natural, byte);
        let doubled = b.binary(Binary::Add, wide, wide);
        let answer = b.call(keep, &[doubled, wide]);
        b.ret(&answer);
    });
    validate(&p).unwrap();
    let placed = place(&p, 64, true);
    let elided = super::claims::elide(&placed, 64);
    validate(&elided).unwrap();
    assert_eq!(counted(&elided), (0, 0));
    assert_eq!(returned(&elided, elided.entry()), returned(&p, p.entry()));
}

/// A loop reading a buffer it hands on as it came, round after round,
/// borrows it: the loop takes no claim to it, carries none round and gives
/// none up, and the function handing it in borrows it from its caller.
#[test]
fn a_loop_reading_what_it_hands_on_as_it_came_borrows_it() {
    use super::ownership::owned_parameters;
    let (p, sum) = summed();
    validate(&p).unwrap();
    assert!(!owned_parameters(&p)[sum.index()][0]);
    let placed = place(&p, 64, true);
    validate(&placed).unwrap();
    let body = placed.function(sum).body().unwrap();
    assert_eq!(
        placed
            .every(body)
            .filter(|op| matches!(op, Op::Claim { .. } | Op::Release { .. }))
            .count(),
        0
    );
    assert_eq!(returned(&placed, placed.entry()), [7u8.into()]);
}

/// A function summing a buffer it hands its loop as it came, called once.
fn summed() -> (Program, super::FunctionId) {
    let (mut p, main) = Program::new("main");
    let word = p.shape(&[Class::Word]);
    let sum = p.declare(
        "sum",
        &[Class::Buffer, Class::Word, Class::Word],
        &[Class::Word],
    );
    p.define(sum, |b, params| {
        let zero = b.constant(Class::Word, 0);
        let answer = b.loop_(&[params[0], zero, zero], &[Class::Word], |b, state| {
            let (buffer, at, total) = (state[0], state[1], state[2]);
            let done = b.compare(super::Relation::Equal, at, params[2]);
            b.leave_if(done, |b| b.break_(&[total]));
            let start = b.binary(Binary::Add, params[1], at);
            let read = b.buffer_field((Class::Word, word), (buffer, start), 0);
            let total = b.binary(Binary::Add, total, read);
            let one = b.constant(Class::Word, 1);
            let next = b.binary(Binary::Add, at, one);
            b.continue_(&[buffer, next, total]);
        });
        b.ret(&answer);
    });
    p.define(main, |b, _| {
        let zero = b.constant(Class::Word, 0);
        let empty = b.buffer_empty(word);
        let seven = b.constant(Class::Word, 7);
        let [one, start] = b.buffer_push((empty, zero, zero), word, &[seven]);
        let count = b.constant(Class::Word, 1);
        let answer = b.call(sum, &[one, start, count]);
        b.ret(&answer);
    });
    (p, sum)
}
