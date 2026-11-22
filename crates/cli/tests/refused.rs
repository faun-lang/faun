//! Programs the compiler must refuse, and exactly what it says about each.
//!
//! Every case under `tests/refused/` is a program `faun check` refuses: a
//! file `<case>.fn`, or a directory `<case>/` whose entry is `main.fn` for a
//! case needing more than one module. Beside it, `<case>.err` holds what
//! `faun check` writes to standard error, verbatim. Comparing all of it, not
//! a phrase from it, means a diagnostic that loses its place, its caret or
//! its help is a diff rather than a test still passing.
//!
//! When a message is meant to change, run `just bless` to write every
//! `.err` again from what the compiler says now, and read the diff: that
//! review is why they are kept.
//!
//! Each case is its own test, named for its file, so one can be run alone
//! and one failing does not hide another.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use libtest_mimic::{Arguments, Failed, Trial};

fn main() {
    let arguments = Arguments::from_args();
    let directory = refused();
    let mut trials = Vec::new();
    for case in cases(&directory) {
        let name = case.name.clone();
        let trial = Trial::test(name, move || check(&case));
        trials.push(trial);
    }
    trials.push(Trial::test("every_err_file_has_its_case", move || {
        orphans(&directory)
    }));
    trials.push(Trial::test("color_always_paints_what_it_says", painted));
    libtest_mimic::run(&arguments, trials).exit();
}

/// Where the cases are.
fn refused() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/refused")
}

/// A program to refuse.
struct Case {
    name: String,
    /// The entry `faun check` is given, from the cases' directory.
    entry: PathBuf,
    /// Where what it says is kept.
    golden: PathBuf,
}

/// Every case under `directory`, by name.
fn cases(directory: &Path) -> Vec<Case> {
    let mut cases = Vec::new();
    for entry in fs::read_dir(directory).expect("tests/refused is readable") {
        let path = entry.expect("tests/refused is readable").path();
        let name = path.file_stem().expect("a case has a name");
        let name = name.to_string_lossy().into_owned();
        let entry = if path.is_dir() {
            PathBuf::from(&name).join("main.fn")
        } else if path.extension().is_some_and(|extension| extension == "fn") {
            PathBuf::from(format!("{name}.fn"))
        } else {
            continue;
        };
        let golden = directory.join(format!("{name}.err"));
        cases.push(Case {
            name,
            entry,
            golden,
        });
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

/// Check the case as someone would from the cases' directory, and hold
/// what it says to its `.err`.
fn check(case: &Case) -> Result<(), Failed> {
    let output = Command::new(env!("CARGO_BIN_EXE_faun"))
        .args(["check", "--target", "x86_64-linux"])
        .arg(&case.entry)
        .current_dir(refused())
        // Not on a terminal, it is uncoloured unless the environment
        // forces colour, which these must not inherit.
        .env_remove("CLICOLOR_FORCE")
        .env_remove("FORCE_COLOR")
        .output()
        .map_err(|error| format!("cannot run faun: {error}"))?;
    if output.status.success() {
        return Err("faun check accepted this; a refused case must be refused".into());
    }
    let told = String::from_utf8(output.stderr).map_err(|_| "faun wrote non-UTF-8")?;
    if blessing() {
        fs::write(&case.golden, &told)
            .map_err(|error| format!("cannot write {}: {error}", case.golden.display()))?;
        return Ok(());
    }
    let Ok(expected) = fs::read_to_string(&case.golden) else {
        let message = format!(
            "{} is missing; run `just bless` and read what it wrote\n\n{told}",
            case.golden.display()
        );
        return Err(message.into());
    };
    if told != expected {
        let message = format!(
            "faun check said something else.\n\n\
             --- expected ({}) ---\n{expected}\n\
             --- said ---\n{told}\n\
             If what it says now is better, run `just bless` and read the diff.",
            case.golden.display()
        );
        return Err(message.into());
    }
    Ok(())
}

/// `--color=always` paints a diagnostic in rustc's colours, even where
/// standard error is not a terminal, and says what its `.err` says beneath
/// them.
fn painted() -> Result<(), Failed> {
    let case = "a_method_given_fewer_arguments_than_it_takes";
    let output = Command::new(env!("CARGO_BIN_EXE_faun"))
        .args(["check", "--color=always", "--target", "x86_64-linux"])
        .arg(format!("{case}.fn"))
        .current_dir(refused())
        .output()
        .map_err(|error| format!("cannot run faun: {error}"))?;
    let told = String::from_utf8(output.stderr).map_err(|_| "faun wrote non-UTF-8")?;
    let painted = [
        // `error` bold red, the message bold.
        "\x1b[1;31merror\x1b[0m\x1b[1m: `blocks` takes 1 argument, but 0 were given\x1b[0m\n",
        // The gutter bold blue.
        " \x1b[1;34m-->\x1b[0m a_method",
        "\x1b[1;34m4 |\x1b[0m halves",
        // Where it happened bold red, the other place and what it says
        // bold blue.
        "\x1b[1;31m^^^^^^\x1b[0m\x1b[1;34m--\x1b[0m \x1b[1;34man argument of type `Nat` is missing\x1b[0m",
        // `help` bright cyan.
        "\x1b[1;34m =\x1b[0m \x1b[1;96mhelp\x1b[0m: provide the argument",
    ];
    if let Some(missing) = painted.iter().find(|part| !told.contains(*part)) {
        return Err(format!("no {missing:?} in {told:?}").into());
    }
    let plain = strip(&told);
    let golden = fs::read_to_string(refused().join(format!("{case}.err")))
        .map_err(|error| format!("cannot read the golden: {error}"))?;
    if plain != golden {
        return Err(format!("beneath its colours it said {plain:?}, not {golden:?}").into());
    }
    Ok(())
}

/// `text` without its colours.
fn strip(text: &str) -> String {
    let mut plain = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('\x1b') {
        plain.push_str(&rest[..at]);
        let after = &rest[at..];
        rest = after.find('m').map_or("", |end| &after[end + 1..]);
    }
    plain.push_str(rest);
    plain
}

/// Every `.err` belongs to a case: one left behind when its case moved or
/// was removed is a failure, not a file nobody reads.
fn orphans(directory: &Path) -> Result<(), Failed> {
    let cases = cases(directory);
    let mut orphans = Vec::new();
    for entry in fs::read_dir(directory).expect("tests/refused is readable") {
        let path = entry.expect("tests/refused is readable").path();
        if path.extension().is_none_or(|extension| extension != "err") {
            continue;
        }
        if !cases.iter().any(|case| case.golden == path) {
            orphans.push(path.display().to_string());
        }
    }
    if orphans.is_empty() {
        return Ok(());
    }
    Err(format!("no case for {}", orphans.join(", ")).into())
}

/// Whether to write each `.err` from what the compiler says now.
fn blessing() -> bool {
    std::env::var_os("FAUN_BLESS").is_some_and(|value| value != "0")
}
