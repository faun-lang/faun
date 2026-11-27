use super::{Class, Offset, Program, Relation, Width, conversions_taking_nothing};

/// `v < 4` bounds `v` in the arm where it holds, so two bits keep all of
/// it there; in the other arm, and before the branch, they need not.
#[test]
fn a_comparison_bounds_what_it_compares_in_the_arm_it_holds() {
    let (mut program, main) = Program::new_with_params("main", &[("count", Class::Word)]);
    let two_bits = Class::fixed(2);
    let (mut before, mut under, mut over) = (None, None, None);
    program.define(main, |b, args| {
        let count = args[0];
        before = Some(b.convert(two_bits, count));
        let four = b.constant(Class::Word, 4);
        let below = b.compare(Relation::Less, count, four);
        let kept = b.if_(
            below,
            &[two_bits],
            |b| {
                let held = b.convert(two_bits, count);
                under = Some(held);
                b.yield_(&[held]);
            },
            |b| {
                let held = b.convert(two_bits, count);
                over = Some(held);
                b.yield_(&[held]);
            },
        );
        let wide = b.convert(Class::Word, kept[0]);
        b.ret(&[wide]);
    });
    let whole = conversions_taking_nothing(&program);
    assert!(!whole[before.unwrap().index()]);
    assert!(whole[under.unwrap().index()]);
    assert!(!whole[over.unwrap().index()]);
}

/// `c < v` bounds `v` by `c` where it does not hold.
#[test]
fn a_comparison_the_other_way_bounds_the_arm_it_fails_in() {
    let (mut program, main) = Program::new_with_params("main", &[("count", Class::Word)]);
    let two_bits = Class::fixed(2);
    let (mut above, mut at_most) = (None, None);
    program.define(main, |b, args| {
        let count = args[0];
        let three = b.constant(Class::Word, 3);
        let exceeds = b.compare(Relation::Less, three, count);
        let kept = b.if_(
            exceeds,
            &[two_bits],
            |b| {
                let held = b.convert(two_bits, count);
                above = Some(held);
                b.yield_(&[held]);
            },
            |b| {
                let held = b.convert(two_bits, count);
                at_most = Some(held);
                b.yield_(&[held]);
            },
        );
        let wide = b.convert(Class::Word, kept[0]);
        b.ret(&[wide]);
    });
    let whole = conversions_taking_nothing(&program);
    assert!(!whole[above.unwrap().index()]);
    assert!(whole[at_most.unwrap().index()]);
}

/// A byte a load read is below what eight bits keep, wherever it is read,
/// and a word it read is not.
#[test]
fn a_load_narrower_than_a_word_bounds_what_it_reads() {
    let (mut program, main) = Program::new_with_params("main", &[("address", Class::Word)]);
    let (mut byte, mut word) = (None, None);
    program.define(main, |b, args| {
        let read = b.load(Width::Byte, args[0], Offset::Bytes(0));
        byte = Some(b.convert(Class::fixed(8), read));
        let whole = b.load(Width::Word, args[0], Offset::Bytes(0));
        word = Some(b.convert(Class::fixed(8), whole));
        let wide = b.convert(Class::Word, byte.unwrap());
        b.ret(&[wide]);
    });
    let whole = conversions_taking_nothing(&program);
    assert!(whole[byte.unwrap().index()]);
    assert!(!whole[word.unwrap().index()]);
}
