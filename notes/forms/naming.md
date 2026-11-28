# Naming

This note owns how Faun names what a type or a library offers: the
standard library's conventions for conversions, questions, fallbacks and
errors, and where a library's names live, so a library reads like the
standard one. Rust's [API guidelines](https://rust-lang.github.io/api-guidelines/naming.html)
are the nearest counterpart; where Faun differs, it says why.

## Conversions

Name a conversion by the source or destination type, unless the caller is
choosing a particular way to lose information:

- `T.of_x(x) -> T` makes a `T` from an `x`, always: `Int.of_nat`,
  `U64.of_u8`.
- `T.from_x(x) -> Result(T, …)` makes a `T` from an `x` where one fits,
  checked: `U8.from_nat`, `Nat.from_str`, `F64.from_nat`.
- `t.to_x() -> X` is an `X` from a `t`: `to_str`, `to_nat`, `to_list`,
  `to_bits`. For new numeric conversions, use plain `to_*` only when the
  numeric value is preserved. The older `U64.to_u8` and `U64.to_u32` wrap
  for compatibility; use the destination's `wrap` in new code.

```ruby
expect U8.wrap(300) == 44
expect U8.from_nat(300) == Err(TooBig)
expect Nat.from_str("42") == Ok(42)
expect U8.wrap(300).to_nat() == 44
```

- Use `wrap` for deliberate modular reduction, not for clamping, rounding
  or arbitrary bit masks. Put it on the destination: `U8.wrap(word)`
  retains the low eight bits, and changing the destination changes the
  modulus, not the spelling. A library's cyclic type should document its
  modulus and representative range.
- Use `from_` when a value must fit: it answers `Err` with why, rather
  than wrapping or rounding.
- Use `of_` for exact value conversions such as `Int.of_nat`. Some
  existing `of_` operations lose information: the word types' `of_nat`
  wraps and is still their numeral constructor. Some float conversions,
  such as `F64.of_nat` and `F64.of_int`, round. For new modular conversions
  prefer the explicit `wrap`.
- The word types' `wrap` accepts a source with `to_nat : a -> Nat`.
  Document `to_nat` as an exact natural value, never a clamp, a rounded
  approximation or an absolute value. `Nat.to_nat` is the identity;
  `Bool.to_nat` already gives 0 and 1 and therefore qualifies. For a domain
  type with a numeric identifier but no natural value, name the projection
  for what it returns and call it explicitly. `Int` does not qualify:
  negative values have no exact natural projection.
- `to_str` is also how a value prints itself: `"${value}"` calls it, and
  `Printable(a)` asks for it.

There is no `as_` and no `into_`. Rust tells a borrowed view, `as_`, from
a copy, `to_`, and from a conversion that takes ownership, `into_`; Faun's
types hold no borrows and its memory reuses what nothing else holds, so
the three are one, `to_`.

## Questions, fallbacks and failures

- `is_x` asks a yes-or-no question: `is_empty`, `is_ok`, `is_digit`,
  `is_nan`.
- A lookup that may find nothing answers a `Result`, `list.get(i)` and
  `map.get(key)`, and has no twin that takes a fallback: the fallback is
  `Result`'s, `list.get(i).ok_or(0)`, so every lookup gets it, and
  `?`, `|` and `map_ok` too, without each collection repeating them.
- `x_or(fallback)` answers the value, or the fallback where there is none:
  `ok_or`, `err_or`.
- `update(key, change)` changes what a key or an index holds by a
  function, and leaves a container without one unchanged, as `remove`
  does a map and `put` a list: `Map.update`, `SortedMap.update`,
  `List.update`. `update_or(key, missing, change)` changes `missing`
  where there is nothing.
- `try_x` is the version of `x` that stops at the first `Err`: `try_map`,
  `try_fold`.
- `x_unchecked` is the version of `x` that skips a check its caller has
  already made, and answers a plain value where `x` answers a `Result`:
  a chess position's `play_unchecked` takes a move from its own
  `legal_moves`, where `play` checks any move. Its doc comment says what
  the caller must guarantee. With no panics, breaking that answers a value
  whose answers mean nothing, never a crash; a library offers one only
  where the check costs more than the work it guards, and only beside the
  checked `x`.
- A failure is a tag named by its cause, never text: `TooBig`,
  `Malformed`, `OutOfBounds`, `Missing`, `NotAscii`. Callers match on it,
  and two libraries refusing for one reason may share a word.
- A constant is a value on its type: `Nat.zero`, `Str.empty`.
- A verb names a function that does something, `encode`, `split`, `sort`;
  a noun names what it answers, `len`, `keys`, `first`. Neither takes a
  `get_` prefix.

## Operators

An operator is a method, `a | b` calling `a.or(b)`, so naming a method
`add`, `mul`, `or` and the rest gives a type the symbol too. Give one only
where it has one obvious meaning: closed, two of a type answering that
type, or else with the one partner the type is plainly used with, as a
price is scaled by a number. Where several meanings are natural, name them
instead. [Operators](operators.md) owns the conventions and their laws.

```ruby
expect (Nat.from_str("x") | Nat.from_str("7")) == Ok(7)
expect Nat.from_str("x").ok_or(0) == 0
```

`Result`'s `or` is the first `Ok`, closed as `|` is everywhere else; the
fallback to a plain value, which answers another type, is `ok_or`.

## Where names live

A type's block is its namespace: what concerns a type is a method of it,
called `value.method()` or `Type.method(…)`. A capitalised word before a
dot is a type and a lowercase one a value, so the case alone says which.

A library's other functions are exported plainly, `export {encode,
decode}`, and imported by name. Where two libraries export one name, the
importer renames one, `import base64 {encode: base64_encode}`; a library
does not prefix its names for it, nor declare a type only to hold them.
Which file a type lives in is the modules note's to say: [a module for each type](modules.md#dividing-a-library), behind a root that exports the
library's surface.

## Open questions

- **How to name existing rounded conversions.** The word types now have
  `wrap` for modular conversions. Float rounding is a separate policy;
  the names and migration for `F64.of_nat` and similar functions remain
  open.
