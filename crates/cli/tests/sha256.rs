//! The sha256 library's examples, built for this machine, on inputs too
//! long for its expectations: a million a's, as FIPS 180-4 gives it, and
//! inputs of many lengths, each digest the one the `sha2` crate gives,
//! whether the input is hashed whole or read 64 KiB at a time.

#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

mod support;

use std::fmt::Write;

use sha2::{Digest, Sha256};
use support::{Built, pipe};

/// Bytes from a fixed xorshift, so the input is the same every run.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[3]
        })
        .collect()
}

/// What the example `name` prints for `input`.
fn printed(built: &Built, name: &str, input: &[u8]) -> String {
    let output = pipe(&mut built.command(name), input);
    assert!(output.status.success(), "{name}");
    String::from_utf8(output.stdout).unwrap()
}

/// `bytes`' digest as 64 lowercase hex digits.
fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            write!(hex, "{byte:02x}").unwrap();
            hex
        })
}

#[test]
fn a_million_as_hash_to_the_digest_fips_180_4_gives() {
    let built = Built::new("sha256", &["stream"]);
    assert_eq!(
        printed(&built, "stream", &vec![b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0\n"
    );
}

/// Lengths either side of a block and of the padding's edge, and of the
/// stream example's reads of 64 KiB, so its pieces end everywhere a block
/// can.
#[test]
fn every_length_agrees_with_the_sha2_crates_digest() {
    let built = Built::new("sha256", &["basic", "stream"]);
    let lengths = [
        0, 1, 55, 56, 63, 64, 65, 119, 120, 128, 1000, 65_535, 65_536, 65_537, 65_600, 3_000_017,
    ];
    for len in lengths {
        let input = noise(len);
        let digest = hex(&input);
        assert_eq!(
            printed(&built, "basic", &input),
            format!("{digest}  -\n"),
            "basic, {len} bytes"
        );
        assert_eq!(
            printed(&built, "stream", &input),
            format!("{digest}\n"),
            "stream, {len} bytes"
        );
    }
}
