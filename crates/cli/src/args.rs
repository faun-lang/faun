//! The `faun` command line: what each subcommand takes, parsed into a
//! [`Command`].

use std::path::PathBuf;

use bpaf::{Parser, choice, construct, long, positional, short};
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

/// Whether diagnostics are coloured, as `--color` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Color {
    /// Where standard error is a terminal, unless the environment says.
    Auto,
    Always,
    Never,
}

/// An intermediate stage `emit` prints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Barb,
    Machine,
    /// The target's own rendering of what it encodes.
    Target,
}

/// Parse this process's arguments, printing help or an error and exiting
/// where they ask for no command.
pub(crate) fn parse() -> (Command, Color) {
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
    .run()
}

fn check() -> impl Parser<(Command, Color)> {
    let target = target();
    let source = positional::<PathBuf>("PATH").help("The source file to check");
    colored(construct!(Command::Check { target, source }))
        .to_options()
        .descr("Check a Faun program against the selected machine's host")
        .command("check")
}

fn build() -> impl Parser<(Command, Color)> {
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
    colored(construct!(Command::Build {
        output,
        symbols,
        target,
        favour,
        source,
    }))
    .to_options()
    .descr("Compile a Faun program")
    .command("build")
}

fn run() -> impl Parser<(Command, Color)> {
    let target = target();
    let source = positional::<PathBuf>("PATH").help("The source file to compile and run");
    let args = positional::<String>("ARGS")
        .help("What the program is run with; after `--`, even what looks like an option")
        .many();
    colored(construct!(Command::Run {
        target,
        source,
        args
    }))
    .to_options()
    .descr("Compile a Faun program for the selected machine and run it")
    .command("run")
}

fn test() -> impl Parser<(Command, Color)> {
    let target = target();
    let source = positional::<PathBuf>("PATH").help("The source file whose package to test");
    colored(construct!(Command::Test { target, source }))
        .to_options()
        .descr("Run every `expect` in a Faun package on the selected machine")
        .command("test")
}

fn emit() -> impl Parser<(Command, Color)> {
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
    colored(construct!(Command::Emit {
        target,
        stage,
        source,
    }))
    .to_options()
    .descr("Print an intermediate stage of a Faun program")
    .command("emit")
}

fn bundle() -> impl Parser<(Command, Color)> {
    let target = target();
    let output = short('o')
        .long("output")
        .help("The directory to write the bundle into")
        .argument::<PathBuf>("DIR")
        .fallback(PathBuf::from("."))
        .debug_fallback();
    let source = positional::<PathBuf>("PATH").help("The root module of the package to bundle");
    colored(construct!(Command::Bundle {
        target,
        output,
        source,
    }))
    .to_options()
    .descr("Write a package as a bundle named by its hash, for a URL dependency to name")
    .command("bundle")
}

fn fetch() -> impl Parser<(Command, Color)> {
    let source = positional::<PathBuf>("PATH").help("The root module of the package");
    colored(construct!(Command::Fetch { source }))
        .to_options()
        .descr("Download every URL dependency of a package, so a later build is offline")
        .command("fetch")
}

fn uses() -> impl Parser<(Command, Color)> {
    let source = positional::<PathBuf>("PATH").help("The root module of the package");
    colored(construct!(Command::Uses { source }))
        .to_options()
        .descr("Print the packages a package uses, as a tree")
        .command("uses")
}

/// A subcommand's own arguments, with `--color`.
fn colored(command: impl Parser<Command>) -> impl Parser<(Command, Color)> {
    let color = long("color")
        .help("When to colour diagnostics: auto, always or never")
        .argument::<String>("WHEN")
        .parse(|when| match when.as_str() {
            "auto" => Ok(Color::Auto),
            "always" => Ok(Color::Always),
            "never" => Ok(Color::Never),
            _ => Err("expected auto, always or never"),
        })
        .fallback(Color::Auto);
    // Options go before what a subcommand takes positionally.
    construct!(color, command).map(|(color, command)| (command, color))
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
