use super::test_support::returned;
use super::{Class, Op, Program, Relation, hoist::hoist, validate};

/// How many instructions `wanted` picks out inside the entry's loop.
fn in_loop(p: &Program, wanted: impl Fn(&Op) -> bool) -> usize {
    let entry = p.function(p.entry()).body().unwrap();
    let body = p
        .every(entry)
        .find_map(|op| match *op {
            Op::Loop { body, .. } => Some(body),
            _ => None,
        })
        .expect("a loop");
    p.every(body).filter(|op| wanted(op)).count()
}

/// Four rounds, each building `[10, 20, 30]` from constants and adding the
/// element the round counts to: 10 + 20 + 30 + 10.
fn rounds() -> Program {
    let (mut p, main) = Program::new("main");
    p.define(main, |b, _| {
        let zero = b.constant(Class::Word, 0);
        let answer = b.loop_(&[zero, zero], &[Class::Word], |b, state| {
            let (round, sum) = (state[0], state[1]);
            let four = b.constant(Class::Word, 4);
            let done = b.compare(Relation::Equal, round, four);
            b.leave_if(done, |b| b.break_(&[sum]));
            let word = b.shape(&[Class::Word]);
            let empty = b.buffer_empty(word);
            let mut list = (
                empty,
                b.constant(Class::Word, 0),
                b.constant(Class::Word, 0),
            );
            for value in [30, 20, 10] {
                let value = b.constant(Class::Word, value);
                let [buffer, start] = b.buffer_push(list, word, &[value]);
                let one = b.constant(Class::Word, 1);
                let len = b.binary(super::Binary::Add, list.2, one);
                list = (buffer, start, len);
            }
            let three = b.constant(Class::Word, 3);
            let at = b.binary(super::Binary::Mod, round, three);
            let at = b.binary(super::Binary::Add, list.1, at);
            let read = b.buffer_field((Class::Word, word), (list.0, at), 0);
            let sum = b.binary(super::Binary::Add, sum, read);
            let one = b.constant(Class::Word, 1);
            let next = b.binary(super::Binary::Add, round, one);
            b.continue_(&[next, sum]);
        });
        b.ret(&answer);
    });
    p
}

#[test]
fn a_list_built_from_constants_every_round_is_built_once_before_the_loop() {
    let source = rounds();
    validate(&source).unwrap();
    let pushes = |op: &Op| matches!(op, Op::BufferPush { .. });
    assert_eq!(in_loop(&source, pushes), 3);
    let held = hoist(&source);
    validate(&held).unwrap();
    assert_eq!(in_loop(&held, pushes), 0);
    assert_eq!(
        returned(&held, held.entry()),
        returned(&source, source.entry())
    );
    assert_eq!(returned(&held, held.entry()), [70u8.into()]);
}
