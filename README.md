# Faun

Faun is an experimental, compiled functional language. The goal is C-like
performance with a small language, immutable values, and no source-level
memory management.

## What stands out

- **Inferred types and structural records.** Signatures are optional. A
  function can ask for the fields it needs without naming a record type.
- **Pure code, explicit effects.** Pattern matches must cover every case.
  Names ending in `!` mark effects. Programs get their I/O capabilities from
  the host, and the compiler checks that resources such as open files are
  neither dropped nor used after closing.
- **Immutable values, reusable storage.** Reference counting tracks ownership.
  The compiler can reuse storage in place when nothing else can observe the
  change, without a tracing garbage collector.

## When it isn't a good fit

Faun is unfinished. There is no FFI, stable binary interface, or concurrency
model. You can pass known functions and call-site lambdas, but can't store or
return functions as values. Exhaustive matching doesn't guarantee recursion
will terminate.

## Examples

**Match on a record.** The `*` lets callers supply extra fields. Save as
`names.fn`:

```ruby
display_name : {name: Str, *} -> Str
display_name({name: "", *}) -> "anonymous"
display_name({name, *}) -> name

expect display_name({name: "Ada", age: 36}) == "Ada"
expect display_name({name: ""}) == "anonymous"
```

**Pass only the capability you need.** `greet!` gets stdout, not the whole
host. Save as `hello.fn`:

```ruby
greet!(stdout) -> stdout.write!("Hello, world!\n".to_utf8())

main!(host) -> (
  greet!(host.stdout)
  host.exit!(0)
)
```

## Get started

In a checkout, build with the [pinned Rust toolchain](rust-toolchain.toml)
and a C compiler. Save the two examples in the repository root, then run:

```sh
cargo build --locked
./target/debug/faun test names.fn
./target/debug/faun run hello.fn
```

The default target is native on x86-64 Linux and Apple Silicon macOS (limited
support). For other hosts, choose `--target wasm32-wasi` or
`--target mos6502-sim65`; `nix develop` includes their runners.

[Language notes](notes/README.md) · [Standard library](crates/frontend/standard/README.md) ·
[More examples](crates/frontend/examples) · [Helix highlighting](editors/tree-sitter-faun/README.md)

[0BSD](LICENSE), except where a separate license is noted.
