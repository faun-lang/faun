//! The flate library's gzip example, built for this machine, compressing:
//! every input at every level, in every container, reads back through the
//! library's own inflate, and through `gzip -d` and Python's zlib, as the
//! input; and comes out within a little of zlib's size at the same level.

#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

mod support;

use std::process::Command;

use support::{Built, file_name as name, library, pipe};

/// What `gzip` writes, given `args`, with `input` on its standard input.
fn run(gzip: &Built, args: &[&str], input: &[u8]) -> Vec<u8> {
    let output = pipe(gzip.command("gzip").args(args), input);
    assert!(
        output.status.success(),
        "gzip {args:?}: {:?}",
        output.status
    );
    output.stdout
}

/// Python's zlib reading `stream` as `format` (`gzip`, `zlib` or `raw`)
/// and writing what it holds.
const PYTHON_READ: &str = "import sys, zlib
bits = {'gzip': 31, 'zlib': 15, 'raw': -15}[sys.argv[1]]
d = zlib.decompressobj(bits)
sys.stdout.buffer.write(d.decompress(sys.stdin.buffer.read()) + d.flush())
assert d.eof and not d.unused_data";

/// The size of zlib's own `format` stream of `input` at `level`.
const PYTHON_SIZE: &str = "import sys, zlib
bits = {'gzip': 31, 'zlib': 15, 'raw': -15}[sys.argv[1]]
c = zlib.compressobj(int(sys.argv[2]), zlib.DEFLATED, bits)
print(len(c.compress(sys.stdin.buffer.read()) + c.flush()))";

fn python(script: &str, args: &[&str], input: &[u8]) -> Vec<u8> {
    let output = pipe(
        Command::new("python3")
            .arg("-I")
            .arg("-c")
            .arg(script)
            .args(args),
        input,
    );
    assert!(output.status.success(), "python {args:?}");
    output.stdout
}

/// The inflate fixtures' inputs, and inputs made to reach the window's
/// edges: a run longer than any match, every byte value, and random blocks
/// repeated exactly a window apart and just past one.
fn inputs() -> Vec<(String, Vec<u8>)> {
    let dir = library("flate").join("fixtures/input");
    let mut found: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    found.sort();
    let mut all: Vec<_> = found
        .iter()
        .map(|path| (name(path).to_owned(), std::fs::read(path).unwrap()))
        .collect();
    let mut seed: u64 = 1951;
    let mut random = |size: usize| -> Vec<u8> {
        (0..size)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed.to_le_bytes()[0]
            })
            .collect()
    };
    all.push(("run".to_owned(), vec![b'z'; 1000]));
    all.push(("bytes".to_owned(), (0..=255).cycle().take(3000).collect()));
    for size in [32767, 32768, 32769] {
        let block = random(size);
        all.push((format!("window-{size}"), [block.clone(), block].concat()));
    }
    all
}

#[test]
fn every_level_reads_back_through_every_reader() {
    let gzip = Built::new("flate", &["gzip"]);
    for (name, input) in inputs() {
        for level in 0..=9 {
            let flag = format!("-{level}");
            for format in ["gzip", "zlib", "raw"] {
                let mut args = vec![flag.as_str()];
                if format != "gzip" {
                    args.push(if format == "zlib" { "--zlib" } else { "--raw" });
                }
                let stream = run(&gzip, &args, &input);
                let mut back = vec!["-d"];
                back.extend(&args[1..]);
                let what = format!("{name} at {flag} as {format}");
                assert!(run(&gzip, &back, &stream) == input, "{what}: our inflate");
                assert!(
                    python(PYTHON_READ, &[format], &stream) == input,
                    "{what}: Python's zlib"
                );
                if format == "gzip" {
                    let output = pipe(Command::new("gzip").arg("-dc"), &stream);
                    assert!(output.status.success(), "{what}: gzip -d");
                    assert!(output.stdout == input, "{what}: gzip -d");
                }
            }
        }
    }
}

/// Within 2% and 16 bytes of zlib at the same level, on every input: the
/// search and the codes are zlib's, so falling further behind is a fault.
#[test]
fn sizes_stay_near_zlib() {
    let gzip = Built::new("flate", &["gzip"]);
    for (name, input) in inputs() {
        for level in [1, 6, 9] {
            let ours = run(&gzip, &[&format!("-{level}"), "--raw"], &input).len();
            let theirs: usize =
                String::from_utf8(python(PYTHON_SIZE, &["raw", &level.to_string()], &input))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
            assert!(
                ours <= theirs + theirs / 50 + 16,
                "{name} at -{level}: {ours} bytes against zlib's {theirs}"
            );
        }
    }
}
