//! The `faun` command line: what each subcommand takes, parsed into a
//! [`Command`].

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use bpaf::{
    Args, Doc, OptionParser, ParseFailure, Parser, choice, construct, long, positional, short,
};
use faun_core::targets::{Favour, SymbolTable, Target};

use crate::{NATIVE, TARGETS};

#[derive(Clone, Debug)]
pub(crate) enum Command {
    Check {
        target: &'static dyn Target,
        source: PathBuf,
    },
    Build {
        target: &'static dyn Target,
        source: PathBuf,
        output: PathBuf,
        favour: Favour,
        symbols: SymbolTable,
    },
    Run {
        target: &'static dyn Target,
        source: PathBuf,
        /// What the program is run with, after its own name.
        args: Vec<String>,
    },
    Test {
        target: &'static dyn Target,
        source: PathBuf,
    },
    Emit {
        target: &'static dyn Target,
        stage: Stage,
        source: PathBuf,
    },
    Bundle {
        target: &'static dyn Target,
        source: PathBuf,
        output: PathBuf,
    },
    Fetch {
        source: PathBuf,
    },
    Uses {
        source: PathBuf,
    },
}

/// An intermediate stage `emit` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Barb,
    Machine,
    /// The target's own rendering of what it encodes.
    Target,
}

/// Parse this process's arguments, printing help, the version or an error
/// and exiting where they ask for no command.
pub(crate) fn parse() -> Command {
    let given: Vec<OsString> = std::env::args_os().skip(1).collect();
    match answer(&given) {
        Answer::Command(command) => command,
        Answer::Out(text) => {
            println!("{text}");
            std::process::exit(0)
        }
        Answer::Misused(text) => {
            eprintln!("{text}");
            std::process::exit(MISUSED)
        }
    }
}

/// How `faun` exits when its arguments ask for nothing it can do, as
/// cargo, go and clap's tools do, apart from a program that fails to
/// compile, which exits 1.
const MISUSED: i32 = 2;

/// What arguments ask for.
enum Answer {
    Command(Command),
    /// Help or the version, asked for: written to standard output.
    Out(String),
    /// No command, or one missing what it needs: written to standard error.
    Misused(String),
}

/// What `given`, the arguments after the program's name, ask for.
fn answer(given: &[OsString]) -> Answer {
    let parser = parser();
    if given.is_empty() {
        return Answer::Misused(help(&parser, &["--help"]));
    }
    match parser.run_inner(Args::from(given).set_name("faun")) {
        Ok(command) => Answer::Command(command),
        Err(ParseFailure::Stdout(doc, full)) => {
            let text = doc.monochrome(full);
            let text = text.trim_end();
            if text == concat!("Version: ", env!("CARGO_PKG_VERSION")) {
                return Answer::Out(concat!("faun ", env!("CARGO_PKG_VERSION")).to_owned());
            }
            Answer::Out(text.to_owned())
        }
        Err(ParseFailure::Completion(text)) => Answer::Out(text),
        Err(ParseFailure::Stderr(doc)) => {
            let message = doc.monochrome(true);
            let message = message.trim_end();
            let first = given[0].to_string_lossy();
            Answer::Misused(
                missing(&parser, &first, message)
                    .unwrap_or_else(|| format!("Error: {message}{}", file_hint(&given[0]))),
            )
        }
    }
}

/// Every command, parsed into a [`Command`].
fn parser() -> OptionParser<Command> {
    choice([
        check().boxed(),
        build().boxed(),
        run().boxed(),
        test().boxed(),
        emit().boxed(),
        bundle().boxed(),
        fetch().boxed(),
        uses().boxed(),
    ])
    .to_options()
    .descr("The Faun compiler")
    .version(env!("CARGO_PKG_VERSION"))
    .footer(examples())
}

/// The examples closing the help, each a real invocation and what it does.
const EXAMPLES: [(&str, &str); 5] = [
    ("faun run hello.fn", "Compile hello.fn and run it"),
    (
        "faun build -o hello hello.fn",
        "Compile hello.fn into ./hello",
    ),
    ("faun check hello.fn", "Check hello.fn for errors"),
    (
        "faun test main.fn",
        "Run the expectations in main.fn's package",
    ),
    (
        "faun build --target wasm32-wasi hello.fn",
        "Compile hello.fn for WebAssembly",
    ),
];

/// [`EXAMPLES`] as a section of help, a column each, indented as bpaf
/// indents its own sections.
fn examples() -> Doc {
    let width = EXAMPLES
        .iter()
        .map(|(invocation, _)| invocation.len())
        .max()
        .unwrap_or(0);
    let mut doc = Doc::default();
    doc.emphasis("Examples:");
    for (invocation, what) in EXAMPLES {
        // A newline then a space is a line break to bpaf, which a newline
        // then four spaces would make a code block, left out of short help.
        doc.text("\n ");
        doc.literal(&format!("    {invocation:width$}  "));
        doc.text(what);
    }
    doc
}

/// The help `args` ask `parser` for, as it would print it.
fn help(parser: &OptionParser<Command>, args: &[&str]) -> String {
    match parser.run_inner(Args::from(args).set_name("faun")) {
        Err(ParseFailure::Stdout(doc, full)) => doc.monochrome(full).trim_end().to_owned(),
        _ => unreachable!("--help is always answered with help"),
    }
}

/// Where `message` says `command`, the first argument, was given without an argument it needs,
/// that said in words, with the command's usage.
fn missing(parser: &OptionParser<Command>, command: &str, message: &str) -> Option<String> {
    let metavar = message
        .strip_prefix("expected `")?
        .strip_suffix("`, pass `--help` for usage information")?;
    let needed = needed(command, metavar)?;
    let usage = help(parser, &[command, "--help"])
        .lines()
        .find(|line| line.starts_with("Usage:"))?
        .to_owned();
    Some(format!(
        "Error: faun {command} needs {needed}\n\n{usage}\n\nFor more, try `faun {command} --help`"
    ))
}

/// What each command's positional argument is, in words.
fn needed(command: &str, metavar: &str) -> Option<&'static str> {
    match (command, metavar) {
        ("check" | "build" | "run" | "test" | "emit", "PATH") => Some("a source file"),
        ("bundle" | "fetch" | "uses", "PATH") => Some("a package's root module"),
        ("emit", "STAGE") => Some("a stage to print: barb, machine or target"),
        _ => None,
    }
}

/// Where `first`, the first argument, names a source file rather than a
/// command, how to run it.
fn file_hint(first: &OsStr) -> String {
    let path = Path::new(first);
    let file = path.extension().is_some_and(|extension| extension == "fn") || path.is_file();
    if file && !first.to_string_lossy().starts_with('-') {
        format!("\n\nhelp: did you mean `faun run {}`?", path.display())
    } else {
        String::new()
    }
}

fn check() -> impl Parser<Command> {
    let target = target();
    let source = positional::<PathBuf>("PATH").help("The source file to check");
    construct!(Command::Check { target, source })
        .to_options()
        .descr("Check a program for errors without building it")
        .command("check")
}

fn build() -> impl Parser<Command> {
    let target = target();
    let source = positional::<PathBuf>("PATH").help("The source file to compile");
    let output = short('o')
        .long("output")
        .help("Where to write artifacts")
        .argument::<PathBuf>("PATH")
        .fallback(PathBuf::from("a.out"))
        .debug_fallback();
    let favour = long("small")
        .help("Favour a smaller program over a faster one")
        .switch()
        .map(|small| if small { Favour::Size } else { Favour::Speed });
    let symbols = long("symbols")
        .help("Name each function in the image and chain their frames, for a profiler or a disassembler")
        .switch()
        .map(|named| {
            if named {
                SymbolTable::Written
            } else {
                SymbolTable::Omitted
            }
        });
    construct!(Command::Build {
        output,
        symbols,
        target,
        favour,
        source,
    })
    .to_options()
    .descr("Compile a program into an executable")
    .command("build")
}

fn run() -> impl Parser<Command> {
    let target = target();
    let source = positional::<PathBuf>("PATH").help("The source file to compile and run");
    let args = positional::<String>("ARGS")
        .help("What the program is run with; after `--`, even what looks like an option")
        .many();
    construct!(Command::Run {
        target,
        source,
        args
    })
    .to_options()
    .descr("Compile a program and run it")
    .command("run")
}

fn test() -> impl Parser<Command> {
    let target = target();
    let source = positional::<PathBuf>("PATH").help("The source file whose package to test");
    construct!(Command::Test { target, source })
        .to_options()
        .descr("Run the expectations in a program's package")
        .command("test")
}

fn emit() -> impl Parser<Command> {
    let target = target();
    let stage = positional::<String>("STAGE")
        .help("The stage to print: barb, machine or target")
        .parse(|stage| match stage.as_str() {
            "barb" => Ok(Stage::Barb),
            "machine" => Ok(Stage::Machine),
            "target" => Ok(Stage::Target),
            _ => Err("expected barb, machine or target"),
        });
    let source = positional::<PathBuf>("PATH").help("The source file to compile");
    construct!(Command::Emit {
        target,
        stage,
        source,
    })
    .to_options()
    .descr("Print an intermediate stage of a program")
    .command("emit")
}

fn bundle() -> impl Parser<Command> {
    let target = target();
    let output = short('o')
        .long("output")
        .help("The directory to write the bundle into")
        .argument::<PathBuf>("DIR")
        .fallback(PathBuf::from("."))
        .debug_fallback();
    let source = positional::<PathBuf>("PATH").help("The root module of the package to bundle");
    construct!(Command::Bundle {
        target,
        output,
        source,
    })
    .to_options()
    .descr("Write a package as a hash-named bundle for URL dependencies")
    .command("bundle")
}

fn fetch() -> impl Parser<Command> {
    let source = positional::<PathBuf>("PATH").help("The root module of the package");
    construct!(Command::Fetch { source })
        .to_options()
        .descr("Download a package's URL dependencies for offline builds")
        .command("fetch")
}

fn uses() -> impl Parser<Command> {
    let source = positional::<PathBuf>("PATH").help("The root module of the package");
    construct!(Command::Uses { source })
        .to_options()
        .descr("Print the tree of packages a package uses")
        .command("uses")
}

/// `--target`, defaulting to [`NATIVE`] and required where there is none.
fn target() -> impl Parser<&'static dyn Target> {
    long("target")
        .help("Machine to compile for")
        .argument::<String>("TARGET")
        .parse(|name| {
            TARGETS
                .into_iter()
                .find(|target| target.name() == name)
                .ok_or_else(|| unknown_target(&name))
        })
        .fallback_with(|| NATIVE.ok_or(NO_NATIVE))
        .debug_fallback()
}

const NO_NATIVE: &str = "no target runs on this machine, so --target is required";

fn unknown_target(name: &str) -> String {
    let known = TARGETS
        .iter()
        .map(|target| target.name())
        .collect::<Vec<_>>()
        .join(", ");
    format!("unknown target `{name}`, expected: {known}")
}
