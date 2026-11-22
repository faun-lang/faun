//! The crc library's example, built for this machine, on inputs too long
//! for its expectations: CRC-32 eight bytes a step and Adler-32 reduced
//! every few megabytes agree with checksums worked out here, a byte at a
//! time, at lengths that leave every tail.

#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

mod support;

use support::{Built, crc32, pipe};

fn adler32(bytes: &[u8]) -> u32 {
    let (mut low, mut high) = (1u32, 0u32);
    for &byte in bytes {
        low = (low + u32::from(byte)) % 65521;
        high = (high + low) % 65521;
    }
    high << 16 | low
}

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

/// What the example prints for `input`.
fn printed(built: &Built, input: &[u8]) -> String {
    let output = pipe(&mut built.command("basic"), input);
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn long_inputs_agree_with_checksums_worked_out_here() {
    let built = Built::new("crc", &["basic"]);
    // 0xff is the largest each sum can grow by, and 17 MB passes four
    // reductions of Adler-32; the lengths leave each tail of eight.
    let inputs = [
        vec![0xff; 17_000_003],
        noise(9_000_005),
        noise(7),
        Vec::new(),
    ];
    for input in inputs {
        let expected = format!(
            "crc32 {:08x}\nadler32 {:08x}\n",
            crc32(&input),
            adler32(&input)
        );
        assert_eq!(printed(&built, &input), expected, "{} bytes", input.len());
    }
}
