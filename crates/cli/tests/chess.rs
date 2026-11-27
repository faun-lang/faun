//! The chess library's perft example, built for this machine, at depths
//! too deep for its expectations on the reference interpreter: every
//! position of the Chess Programming Wiki's perft results gives the
//! counts the wiki gives, and the start position's divide is the one
//! engines print.

#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

mod support;

use support::Built;

/// The last line the example prints for a depth and a FEN.
fn nodes(built: &Built, depth: u32, fen: &str) -> String {
    let output = built
        .command("perft")
        .arg(depth.to_string())
        .args(fen.split(' '))
        .output()
        .unwrap();
    assert!(output.status.success(), "perft {depth} {fen}");
    let text = String::from_utf8(output.stdout).unwrap();
    text.lines().last().unwrap().to_owned()
}

/// The wiki's six positions, Kiwipete second and the fourth both ways
/// round, each with a depth and its count.
const POSITIONS: [(&str, u32, u64); 7] = [
    (
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        4,
        197_281,
    ),
    (
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        3,
        97_862,
    ),
    ("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1", 4, 43_238),
    (
        "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        3,
        9_467,
    ),
    (
        "r2q1rk1/pP1p2pp/Q4n2/bbp1p3/Np6/1B3NBn/pPPP1PPP/R3K2R b KQ - 0 1",
        3,
        9_467,
    ),
    (
        "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        3,
        62_379,
    ),
    (
        "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
        3,
        89_890,
    ),
];

#[test]
fn every_wiki_position_gives_the_wikis_counts() {
    let built = Built::new("chess", &["perft"]);
    for (fen, depth, count) in POSITIONS {
        assert_eq!(
            nodes(&built, depth, fen),
            format!("Nodes searched: {count}"),
            "{fen} at {depth}"
        );
    }
}

#[test]
fn the_start_positions_divide_is_the_one_engines_print() {
    let built = Built::new("chess", &["perft"]);
    let output = built.command("perft").arg("2").output().unwrap();
    assert!(output.status.success());
    let expected = "\
b1a3: 20\nb1c3: 20\ng1f3: 20\ng1h3: 20\n\
a2a3: 20\na2a4: 20\nb2b3: 20\nb2b4: 20\nc2c3: 20\nc2c4: 20\n\
d2d3: 20\nd2d4: 20\ne2e3: 20\ne2e4: 20\nf2f3: 20\nf2f4: 20\n\
g2g3: 20\ng2g4: 20\nh2h3: 20\nh2h4: 20\n\nNodes searched: 400\n";
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}

#[test]
fn a_fen_it_cannot_read_is_refused_with_why() {
    let built = Built::new("chess", &["perft"]);
    let output = built
        .command("perft")
        .args(["1", "8/8/8/8/8/8/8/8", "w", "-", "-", "0", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "perft: InvalidKings\n"
    );
}
