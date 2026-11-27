use super::test_support::returned;
use super::{Binary, Class, Op, Program, Relation, merge_ifs::merge_ifs, validate};

/// How many branches `p`'s entry holds.
fn branches(p: &Program) -> usize {
    let body = p.function(p.entry()).body().unwrap();
    p.every(body)
        .filter(|op| matches!(op, Op::If { .. }))
        .count()
}

/// Two branches on one condition, the second reading what the first
/// answered, are one branch answering the same.
#[test]
fn branches_on_one_condition_one_after_another_are_one_branch() {
    let (mut p, main) = Program::new("main");
    p.define(main, |b, _| {
        let three = b.constant(Class::Word, 3);
        let four = b.constant(Class::Word, 4);
        let below = b.compare(Relation::Less, three, four);
        let first = b.if_(
            below,
            &[Class::Word],
            |b| {
                let ten = b.constant(Class::Word, 10);
                b.yield_(&[ten]);
            },
            |b| {
                let twenty = b.constant(Class::Word, 20);
                b.yield_(&[twenty]);
            },
        );
        let second = b.if_(
            below,
            &[Class::Word],
            |b| {
                let more = b.binary(Binary::Add, first[0], three);
                b.yield_(&[more]);
            },
            |b| b.yield_(&[four]),
        );
        let sum = b.binary(Binary::Add, first[0], second[0]);
        b.ret(&[sum]);
    });
    validate(&p).unwrap();
    let merged = merge_ifs(&p, usize::MAX);
    validate(&merged).unwrap();
    assert_eq!(branches(&p), 2);
    assert_eq!(branches(&merged), 1);
    assert_eq!(returned(&merged, merged.entry()), returned(&p, p.entry()));
    assert_eq!(returned(&merged, merged.entry()), [23u8.into()]);
}

/// A target carrying two values out of a branch keeps a run answering
/// three as two branches: the first two merged, the third as it was.
#[test]
fn a_run_is_merged_only_as_wide_as_a_branch_carries() {
    let (mut p, main) = Program::new("main");
    p.define(main, |b, _| {
        let three = b.constant(Class::Word, 3);
        let four = b.constant(Class::Word, 4);
        let below = b.compare(Relation::Less, three, four);
        let mut answers = Vec::new();
        for (yes, no) in [(1, 2), (10, 20), (100, 200)] {
            let answered = b.if_(
                below,
                &[Class::Word],
                |b| {
                    let yes = b.constant(Class::Word, yes);
                    b.yield_(&[yes]);
                },
                |b| {
                    let no = b.constant(Class::Word, no);
                    b.yield_(&[no]);
                },
            );
            answers.push(answered[0]);
        }
        let some = b.binary(Binary::Add, answers[0], answers[1]);
        let all = b.binary(Binary::Add, some, answers[2]);
        b.ret(&[all]);
    });
    let merged = merge_ifs(&p, 2);
    validate(&merged).unwrap();
    assert_eq!(branches(&merged), 2);
    assert_eq!(returned(&merged, merged.entry()), [111u8.into()]);
}

/// What the first branch answers that only the second's arms read is not
/// answered by the merged branch.
#[test]
fn what_only_later_arms_read_stays_inside_the_merged_branch() {
    let (mut p, main) = Program::new("main");
    p.define(main, |b, _| {
        let three = b.constant(Class::Word, 3);
        let four = b.constant(Class::Word, 4);
        let below = b.compare(Relation::Less, three, four);
        let first = b.if_(
            below,
            &[Class::Word],
            |b| b.yield_(&[three]),
            |b| b.yield_(&[four]),
        );
        let second = b.if_(
            below,
            &[Class::Word],
            |b| {
                let more = b.binary(Binary::Add, first[0], three);
                b.yield_(&[more]);
            },
            |b| b.yield_(&[first[0]]),
        );
        b.ret(&[second[0]]);
    });
    let merged = merge_ifs(&p, usize::MAX);
    validate(&merged).unwrap();
    let body = merged.function(merged.entry()).body().unwrap();
    let answered: Vec<usize> = merged
        .walk(body)
        .filter(|(op, _)| matches!(op, Op::If { .. }))
        .map(|(_, results)| results.len())
        .collect();
    assert_eq!(answered, [1]);
    assert_eq!(returned(&merged, merged.entry()), [6u8.into()]);
}
