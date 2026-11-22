//! The png library's example, built for this machine and run on `PngSuite`:
//! every image decodes to the pixels pypng gives, by the CRC-32 the
//! fixtures' `expected.txt` holds, and every file `PngSuite` marks corrupt,
//! or a valid one cut short or with a bit flipped, is refused, never a
//! crash.

#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

mod support;

use std::path::PathBuf;
use std::process::Stdio;

use support::{Built, crc32, exit_code, file_name as name, library, pipe};

/// The status the example exits with on `input`, and the pixels it wrote
/// after the PAM header.
fn decode(decoder: &Built, input: &[u8]) -> (i32, Vec<u8>) {
    let output = pipe(decoder.command("topam").stderr(Stdio::null()), input);
    let end = b"ENDHDR\n";
    let pixels = output
        .stdout
        .windows(end.len())
        .position(|window| window == end)
        .map_or_else(Vec::new, |at| output.stdout[at + end.len()..].to_vec());
    (exit_code(&output), pixels)
}

/// What a file should decode to: its pixels' count and CRC-32, or nothing
/// where it should be refused.
type Expected = Option<(usize, u32)>;

fn suite() -> Vec<(PathBuf, Expected)> {
    let dir = library("png").join("fixtures");
    std::fs::read_to_string(dir.join("expected.txt"))
        .unwrap()
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.split(' ').collect();
            let path = dir.join("pngsuite").join(fields[0]);
            let expected = match fields[1..] {
                ["error"] => None,
                [width, height, crc] => {
                    let width: usize = width.parse().unwrap();
                    let height: usize = height.parse().unwrap();
                    Some((width * height * 4, u32::from_str_radix(crc, 16).unwrap()))
                }
                _ => panic!("{line}"),
            };
            (path, expected)
        })
        .collect()
}

#[test]
fn crc32_is_zlibs() {
    assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
}

#[test]
fn every_image_decodes_to_the_reference_and_every_corrupt_one_is_refused() {
    let decoder = Built::new("png", &["topam"]);
    let all = suite();
    assert!(all.len() >= 170, "{} images", all.len());
    for (path, expected) in all {
        let (status, pixels) = decode(&decoder, &std::fs::read(&path).unwrap());
        match expected {
            Some((len, crc)) => {
                assert_eq!(status, 0, "{}", name(&path));
                assert_eq!(pixels.len(), len, "{}", name(&path));
                assert_eq!(
                    crc32(&pixels),
                    crc,
                    "{} decodes to pypng's pixels",
                    name(&path)
                );
            }
            None => assert_eq!(status, 1, "{} is refused", name(&path)),
        }
    }
}

/// Every file cut short is refused, since a cut loses IEND. A bit flipped
/// anywhere is refused too: in the signature it is no PNG, and anywhere
/// after, every chunk's length, type, data and CRC-32 are checked by that
/// CRC-32, ancillary chunks' as well.
#[test]
fn a_file_cut_short_or_flipped_is_refused() {
    let decoder = Built::new("png", &["topam"]);
    let mut seed: u64 = 2003;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for (path, expected) in suite().into_iter().step_by(5) {
        if expected.is_none() {
            continue;
        }
        let file = std::fs::read(&path).unwrap();
        let len = file.len();
        for cut in (0..len).step_by((len / 8).max(1)).chain(len - 3..len) {
            let (status, _) = decode(&decoder, &file[..cut]);
            assert_eq!(status, 1, "{} cut to {cut} of {len}", name(&path));
        }
        for _ in 0..4 {
            let mut flipped = file.clone();
            let at = usize::try_from(next()).unwrap() % len;
            flipped[at] ^= 1 << (next() % 8);
            let (status, _) = decode(&decoder, &flipped);
            assert_eq!(status, 1, "{} flipped at {at}", name(&path));
        }
    }
}
