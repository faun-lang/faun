//! A recognized count answers what Rust's integers answer at every width,
//! nought included, and a target's set of counts holds what it was given.

use super::*;

#[test]
fn a_count_answers_as_rusts_integers_do_at_every_width() {
    let values = [
        0u64,
        1,
        0x58,
        0x80,
        0xff,
        0x0580,
        0x8000,
        0xffff,
        0x8000_0000,
    ];
    for bits in [8u16, 16, 32, 64] {
        let mask = u64::MAX >> (64 - bits);
        let top = 1u64 << (bits - 1);
        for value in values.into_iter().chain([top, mask, mask - 1, top | 1]) {
            let value = value & mask;
            let unused = 64 - u32::from(bits);
            let answer = |count| u32::try_from(BitCount { count, bits }.calculate(value)).unwrap();
            assert_eq!(
                answer(Count::Ones),
                value.count_ones(),
                "{bits}: {value:#x}"
            );
            assert_eq!(
                answer(Count::TrailingZeros),
                value.trailing_zeros().min(u32::from(bits)),
                "{bits}: {value:#x}"
            );
            assert_eq!(
                answer(Count::LeadingZeros),
                value.leading_zeros() - unused,
                "{bits}: {value:#x}"
            );
        }
        // Nought has no ones and the width of either count of zeros.
        for count in [Count::TrailingZeros, Count::LeadingZeros] {
            assert_eq!(BitCount { count, bits }.calculate(0), u64::from(bits));
        }
    }
}

#[test]
fn a_set_of_counts_holds_what_it_was_given() {
    let zeros = Counts::NONE
        .with(Count::TrailingZeros)
        .with(Count::LeadingZeros);
    assert!(!zeros.has(Count::Ones));
    assert!(zeros.has(Count::TrailingZeros) && zeros.has(Count::LeadingZeros));
    assert_eq!(
        Counts::ALL.without(zeros).iter().collect::<Vec<_>>(),
        [Count::Ones]
    );
    assert_eq!(Counts::NONE.iter().count(), 0);
    assert_eq!(Counts::default(), Counts::NONE);
}
