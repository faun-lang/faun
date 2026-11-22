//! What `faun` says when it is asked for help or its version, or handed
//! arguments that ask for no command it can do: help and the version on
//! standard output, exiting 0; every misuse on standard error, exiting 2.
mod support;

use std::path::Path;
use std::process::Command;

use support::Package;

/// What `faun` with `args`, run in `directory`, exits with and writes to
/// standard output and to standard error.
fn faun_in(directory: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_faun"))
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn faun(args: &[&str]) -> (Option<i32>, String, String) {
    faun_in(&std::env::temp_dir(), args)
}

#[test]
fn bare_faun_prints_the_help_as_a_misuse() {
    let (status, out, help) = faun(&["--help"]);
    assert_eq!(status, Some(0));
    assert!(help.is_empty(), "{help}");
    let (status, nothing, said) = faun(&[]);
    assert_eq!(status, Some(2));
    assert!(nothing.is_empty(), "{nothing}");
    assert_eq!(said, out);
    assert!(said.starts_with("The Faun compiler\n\nUsage: faun COMMAND ...\n"));
    assert!(said.contains("\n    check  "), "{said}");
}

/// Each example the help closes with does what it says, in a package
/// holding a `hello.fn` and a `main.fn` with expectations.
#[test]
fn every_example_in_the_help_works() {
    let package = Package::new(
        "usage-examples",
        r"
        double : Nat -> Nat
        double(n) -> n + n
        expect double(2) == 4
        main!(host) -> host.exit!(0)",
    );
    let directory = package.entry().parent().unwrap().to_owned();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../frontend/examples/hello.fn"),
        directory.join("hello.fn"),
    )
    .unwrap();
    let (_, help, _) = faun(&["--help"]);
    let (_, examples) = help.split_once("\nExamples:\n").expect("help has examples");
    let invocations = examples
        .lines()
        .map(|line| {
            line.trim()
                .split_once("  ")
                .expect("an example and what it does")
                .0
        })
        .collect::<Vec<_>>();
    assert_eq!(invocations.len(), 5, "{examples}");
    for invocation in invocations {
        let args = invocation.strip_prefix("faun ").unwrap();
        let args = args.split(' ').collect::<Vec<_>>();
        let (status, out, said) = faun_in(&directory, &args);
        assert_eq!(status, Some(0), "{invocation}: {out}{said}");
    }
}

#[test]
fn a_source_file_given_as_a_command_is_shown_how_to_run_it() {
    let (status, out, said) = faun(&["hello.fn"]);
    assert_eq!(status, Some(2));
    assert!(out.is_empty(), "{out}");
    assert_eq!(
        said,
        "Error: expected `COMMAND ...`, got `hello.fn`. Pass `--help` for usage information\n\n\
         help: did you mean `faun run hello.fn`?\n"
    );
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let (_, _, said) = faun(&[manifest]);
    assert!(
        said.contains(&format!("did you mean `faun run {manifest}`?")),
        "{said}"
    );
}

#[test]
fn a_misspelt_command_is_told_the_nearest_without_a_file_hint() {
    let (status, _, said) = faun(&["rnu"]);
    assert_eq!(status, Some(2));
    assert_eq!(
        said,
        "Error: no such command or positional: `rnu`, did you mean `run`?\n"
    );
}

#[test]
fn a_command_missing_its_argument_says_what_and_its_usage() {
    let (status, out, said) = faun(&["run"]);
    assert_eq!(status, Some(2));
    assert!(out.is_empty(), "{out}");
    assert_eq!(
        said,
        "Error: faun run needs a source file\n\n\
         Usage: faun run [--color=WHEN] [--target=TARGET] PATH [ARGS]...\n\n\
         For more, try `faun run --help`\n"
    );
    for (args, needs) in [
        (&["check"][..], "faun check needs a source file"),
        (&["build", "--small"], "faun build needs a source file"),
        (&["test"], "faun test needs a source file"),
        (
            &["emit"],
            "faun emit needs a stage to print: barb, machine or target",
        ),
        (&["emit", "barb"], "faun emit needs a source file"),
        (&["fetch"], "faun fetch needs a package's root module"),
    ] {
        let (status, _, said) = faun(args);
        assert_eq!(status, Some(2), "{args:?}");
        assert!(
            said.starts_with(&format!("Error: {needs}\n\nUsage: faun ")),
            "{said}"
        );
    }
}

#[test]
fn the_version_is_one_line() {
    let (status, out, said) = faun(&["--version"]);
    assert_eq!(status, Some(0));
    assert!(said.is_empty(), "{said}");
    assert_eq!(out, concat!("faun ", env!("CARGO_PKG_VERSION"), "\n"));
}
