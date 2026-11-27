//! The gzip library's examples, built for this machine, against the system's
//! `gzip`: what the library writes, at every level and with every header
//! field, `gzip -d` reads back as the input; what `gzip` writes, one member
//! or several, the library reads back, with the name and time `gzip` put in
//! the header; and a file cut short or corrupted is refused, never a crash.

#![cfg(all(target_arch = "x86_64", target_os = "linux"))]

mod support;

use std::process::{Command, Stdio};

use support::{Built, exit_code, library, pipe};

/// The examples, built once for each test.
fn built() -> Built {
    Built::new("gzip", &["gzip", "gunzip"])
}

/// The status a command leaves with on `input`, and what it wrote.
fn run(command: &mut Command, input: &[u8]) -> (i32, Vec<u8>) {
    let output = pipe(command.stderr(Stdio::null()), input);
    (exit_code(&output), output.stdout)
}

/// The flate fixtures' inputs, a run, and bytes from a fixed xorshift.
fn inputs() -> Vec<(String, Vec<u8>)> {
    let dir = library("flate").join("fixtures/input");
    let mut found: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    found.sort();
    let mut all: Vec<_> = found
        .iter()
        .map(|path| {
            let name = support::file_name(path).to_owned();
            (name, std::fs::read(path).unwrap())
        })
        .collect();
    let mut seed: u64 = 1952;
    let noise = (0..100_000)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed.to_le_bytes()[0]
        })
        .collect();
    all.push(("noise".to_owned(), noise));
    all.push(("run".to_owned(), vec![b'z'; 100_000]));
    all
}

#[test]
fn gzip_reads_what_the_library_writes() {
    let built = built();
    for (name, input) in inputs() {
        for level in [0, 1, 6, 9] {
            let flag = format!("-{level}");
            let header = ["--name", "notes.txt", "--comment", "a comment"];
            for args in [
                vec![flag.as_str()],
                [&[flag.as_str()][..], &header].concat(),
            ] {
                let what = format!("{name} with {args:?}");
                let (status, file) = run(built.command("gzip").args(&args), &input);
                assert_eq!(status, 0, "{what}");
                let (status, back) = run(Command::new("gzip").arg("-dc"), &file);
                assert_eq!(status, 0, "{what}: gzip -d");
                assert!(back == input, "{what}: gzip -d reads back the input");
                let (status, back) = run(&mut built.command("gunzip"), &file);
                assert_eq!(status, 0, "{what}: gunzip");
                assert!(back == input, "{what}: the library reads back the input");
            }
        }
    }
}

/// `gzip -lN` finds the name the library put in the header.
#[test]
fn gzip_finds_the_name_the_library_wrote() {
    let built = built();
    let (_, file) = run(built.command("gzip").args(["--name", "kept.txt"]), b"hello");
    let directory = std::env::temp_dir().join(format!("faun-gzip-name-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("renamed.gz");
    std::fs::write(&path, &file).unwrap();
    let output = Command::new("gzip").arg("-lN").arg(&path).output().unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    let listed = String::from_utf8(output.stdout).unwrap();
    assert!(output.status.success(), "{listed}");
    assert!(listed.contains("kept.txt"), "{listed}");
}

#[test]
fn the_library_reads_what_gzip_writes() {
    let built = built();
    let all = inputs();
    for (name, input) in &all {
        for level in ["-1", "-6", "-9"] {
            let (_, file) = run(Command::new("gzip").args(["-c", level]), input);
            let (status, back) = run(&mut built.command("gunzip"), &file);
            assert_eq!(status, 0, "{name} at {level}");
            assert!(back == *input, "{name} at {level} reads back");
        }
    }
    // Every input as a member of its own, one after another, as `cat`
    // joins them, and padded with zeros as a tape pads them.
    let mut joined = Vec::new();
    let mut file = Vec::new();
    for (_, input) in &all {
        file.extend(run(Command::new("gzip").arg("-c"), input).1);
        joined.extend(input);
    }
    let (status, back) = run(&mut built.command("gunzip"), &file);
    assert_eq!(status, 0, "the joined members");
    assert!(back == joined, "the joined members read back");
    file.extend([0; 7]);
    let (status, back) = run(&mut built.command("gunzip"), &file);
    assert!(status == 0 && back == joined, "zeros after the members");
    file.push(1);
    assert_eq!(
        run(&mut built.command("gunzip"), &file).0,
        1,
        "a byte after them"
    );
}

/// `gzip` given a file puts its name and time in the header, and a member
/// Python wrote with every optional field reads back with them.
#[test]
fn the_library_reads_the_header_gzip_wrote() {
    let built = built();
    let directory = std::env::temp_dir().join(format!("faun-gzip-header-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("story.txt");
    std::fs::write(&path, b"once upon a time\n").unwrap();
    let status = Command::new("touch")
        .args(["-d", "@1700000000"])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    let status = Command::new("gzip").arg(&path).status().unwrap();
    assert!(status.success());
    let file = std::fs::read(directory.join("story.txt.gz")).unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    let (status, listed) = run(built.command("gunzip").arg("--list"), &file);
    assert_eq!(status, 0);
    assert_eq!(
        String::from_utf8(listed).unwrap(),
        "17 1700000000 Unix story.txt -\n"
    );
    let fixture = library("flate").join("fixtures/deflate/text.header.gz");
    let (status, listed) = run(
        built.command("gunzip").arg("--list"),
        &std::fs::read(fixture).unwrap(),
    );
    assert_eq!(status, 0);
    assert_eq!(
        String::from_utf8(listed).unwrap(),
        "70000 0 Unknown name.txt a comment\n"
    );
}

/// Every file cut short is refused with status one, and a bit flipped
/// anywhere is refused or read as the same input, never a crash.
#[test]
fn a_damaged_file_is_refused() {
    let built = built();
    let mut seed: u64 = 1953;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for (name, input) in inputs() {
        let (_, file) = run(
            built
                .command("gzip")
                .args(["--name", "n", "--comment", "c"]),
            &input,
        );
        let len = file.len();
        let cuts = (0..len).step_by((len / 12).max(1)).chain(len - 4..len);
        for cut in cuts {
            let (status, _) = run(&mut built.command("gunzip"), &file[..cut]);
            assert_eq!(status, 1, "{name} cut to {cut} of {len}");
        }
        for _ in 0..8 {
            let mut flipped = file.clone();
            // The time, the extra flags and the system are not checked, and
            // a byte of the name or the comment changes only the header;
            // the rest is checked or covered by a checksum.
            let at = loop {
                let at = usize::try_from(next()).unwrap() % len;
                if !(4..10).contains(&at) {
                    break at;
                }
            };
            flipped[at] ^= 1 << (next() % 8);
            let (status, back) = run(&mut built.command("gunzip"), &flipped);
            let read_alike = status == 0 && back == input;
            assert!(
                status == 1 || read_alike,
                "{name} flipped at {at}: {status}"
            );
        }
    }
}
