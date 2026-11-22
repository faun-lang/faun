# Tags

A tag is a capitalised word naming one case of a value: `Ok`, `Empty`,
`NotFound`, `Circle(r)`. This note owns the two kinds of tag, structural and
nominal, the rows that structural tags form, how a word that could be either
is resolved, and how a module keeps its constructors to itself.

## Structural tags

A capitalised word that no visible declaration claims is a structural tag.
It exists by being written: it belongs to no module, needs no import, and is
identified by its spelling and its payload types. The standard list's
`first` is one:

```ruby
first : List(a) -> Result(a, [Empty])
first([]) -> Err(Empty)
first([head, *_]) -> Ok(head)
```

`[Empty]` is a variant row: a set of tags, each with its payload types. Rows
are row-polymorphic, which means a type may hold "these tags, and whatever
else is joined to them" while inference works it out. Where two rows meet,
unification joins them into one row holding the tags of both:

```ruby ignore
MyError = [NotFound, SomethingElse]
AnotherError = [Blah, NotFound]

cat : Nat -> Result(Nat, MyError)
dog : Nat -> Result(Nat, AnotherError)

foo : Nat -> Result(Nat, [Blah, NotFound, SomethingElse])
foo(n) -> (
  x = cat(n)?
  y = dog(n)?
  Ok(x + y)
)
```

Each `?` adds its producer's failure tags to the answer's row, and the two
`NotFound`s are one tag. No umbrella type and no conversion take part. The
same word may carry different payloads in rows that never meet. Where they
do meet, the payloads must unify: joining `[Invalid(Nat), Missing]` with
`[Denied, Invalid(Str)]` is refused, since one row cannot hold two
`Invalid`s.

A caller that needs to know which producer failed wraps each at the join.
A tag may be passed as a callback for this:

```ruby ignore
combined(n) -> (
  x = cat(n).map_err(CatFailure)?
  y = dog(n).map_err(DogFailure)?
  Ok(x + y)
)
```

The row now holds `CatFailure(...)` and `DogFailure(...)`, and the inner
`NotFound`s stay one tag.

### Closed and open

A row written in a signature is closed for the body and open to callers.
The body may answer only those tags: `first([]) -> Err(Nope)` under
`[Empty]` is refused with `` `Nope` is not among the tags of `[Empty]` ``.
A caller may still join `first`'s row with its own. A row in a parameter is
what the definition handles, and stays closed. Clauses, or an `is`'s arms,
that take a row apart with no catch-all close it too, so after `f(A) -> 1`
and `f(B) -> 2`, the call `f(C)` is refused.

A row may write `*` last, as a record does: `[A, *]` holds `A` and
whatever other tags the caller's row holds, a rest the caller chooses.
Where the definition receives the row, as a parameter or as what a
callback it is handed answers, the body must take it apart with a
catch-all, may name only the tags the row writes, and cannot compare or
print it:

```ruby
describe : [NotFound, Denied(Str), *] -> Str
describe(NotFound) -> "missing"
describe(Denied(who)) -> "denied to ${who}"
describe(_) -> "failed"

expect describe(Denied("ada")) == "denied to ada"
expect describe(TimedOut) == "failed"
```

Without `*`, `describe(TimedOut)` is refused, whatever catch-all the
clauses have. Where the definition gives the row, as its result or as a
callback's parameter, a written row is already closed for the body and
open to callers, so `[A, *]` there is `[A]`. Each `*` is a rest of its
own, so `[A, *] -> [A, *]` cannot answer what it was given; writing the
answer as `_` does. An alias's `*` is each use's own, and a declared type
holds no `*`: its tags, and what its tags hold, are its own.

A signature may write `_` for a row it leaves to its `?`s:
`parse_pair : (Str, Str) -> Result((Nat, Nat), _)` answers whatever its
producers and its own `Err(Zero)` join.

A closed row has `equals` and `to_str` without declaring them. Two values
are equal where they are one tag with equal payloads, and a tag is written
as its name with its payloads after it: `Invalid(3)`. A row with a
caller's rest has neither, as a record with `*` has none:

```text
error: `[A, *]` may hold tags a caller adds, which it cannot say `equals` of
```

## Nominal types

A capitalised `:` declaration mints a type and its constructors:

```ruby
Light : [Low, High]
Tree(a) : [Leaf, Node(Tree(a), a, Tree(a))]
Email : Str
```

`Email : Str` is short for a type with one constructor, `Email(Str)`. A
constructor belongs to its type, so `Match` and `Parse` may both declare
`Failed` and stay apart. A nominal type never unifies with a structural row
of the same shape. Methods, and the derived `equals`, `cmp`, `to_str` and
`hash_into`, live in the type's block ([methods](../forms/methods.md)). A
sum's own name may stand for all its constructors in a clause's parameters,
which is how a sum forwards a method to whichever value it holds; that is
the methods note's too.

## Resolution by position

A word that some visible declaration also claims is read as its position's
type decides. A position that wants a row takes the structural tag. A
position that wants a nominal type takes that type's constructor. Both
readings can stand in one clause:

```ruby
Light : [Low, High]

lift : [Low, Found(Nat)] -> Light
lift(Low) -> Low
lift(Found(_)) -> High
```

The pattern's `Low` is the row's tag, since the parameter is a row. The
body's `Low` is `Light`'s, since the result is a `Light`.

Where nothing decides, a word one declaration claims takes that declaration.
A word several declarations claim is an error:

```text
error: constructor `Failed` is ambiguous here
  = note: nothing decides between `Match.Failed` and `Parse.Failed`
  = help: qualify it, or let a signature decide
```

`Type.Ctor` names a constructor outright, in expressions and patterns:
`describe(Mode.Read) -> "read only"`. Inside a type's block, a bare word the
type declares always means its own constructor, and one that would resolve
to another type's is refused, with the qualified spelling as the fix.

## Constructor visibility

A module always sees its own constructors. Its export list decides what
importers see ([modules](../forms/modules.md)):

- `export {T}` publishes the type and its methods, and keeps construction
  and matching inside the module.
- `export {T.*}` publishes the constructors too, and an importer may then
  take one alone: `import std.host {Read}`.

A sealed type cannot be forged. Outside the module, `Email("x")` can only be
a structural tag, and no `Email` parameter takes one:

```text
expected `Email`, found `[Email(Str), *]`
```

Qualifying does not get around it: `Failure.NotFound` outside a module that
exported only `Failure` is refused as having no visible constructor.

## Data holds no functions

No payload, structural or nominal, may hold a function:

```text
Bad : [B((Nat -> Nat))]
error: constructor `B` holds a function
  = note: data may not store functions; a function is passed into a call
```

Why is under Rationale; what functions may do is
[functions](functions.md)'s.

## Rationale

### Why both nominal and structural tags?

They serve different things. Structural rows are for outcomes, the answers
that flow between functions, where a caller may join in a new failure
without anyone's permission and nobody asks whose `Missing` a value is. The
host's errors show it: `ReadError`, `WriteError` and `OpenError` are rows
holding only what each operation can meet, and `?` joins them where a
program uses several. Nominal types are for things: values with methods,
with laws the compiler checks against the block (a type with `cmp` takes its
`equals` from it), and, when the constructors are sealed, with a guarantee
such as `Str`'s valid UTF-8.

The cost is two kinds of identity to learn. A same-shaped structural value
never passes for a nominal one, which is the point of a seal but is also
what a newcomer trips on first. Reach for the structural form by default;
declare a type when it needs methods or laws, and seal it when it needs a
wall.

### Why resolve a tag by its position?

Because the information is already there, and numerals and methods already
resolve this way. A clause's parameter types and its result type say which
type each tag position wants before the word is read.

Ori first resolved by declaration: a declared constructor took its word
everywhere in the module. A module using `Absent` in structural scan rows
could not declare a type with an `Absent` case, and two types in a module
could not both have a `Failed`, so constructors wore their type's name,
`MatchFailed`, `ParsedAbsent`, on both sides of every clause. Faun's clap
library now declares `Failed` in both `Match` and `ParsedArg`, and shares
`Absent` with its scan rows. The cost is that a reader must know a
position's type to read its tag, and the one real ambiguity is reported
rather than guessed.

### Why does a word nothing decides take the declared reading?

Faun checks each definition once and specializes from the types checking
recorded, so an unsigned definition cannot be one type's constructor for
one caller and a row's tag for another. Ori deferred such a tag to each
caller as a bound; Faun does not. Taking the declared reading keeps an
unsigned definition's meaning the one a reader of the module would guess.

The cost is real: once `Light` declares `Low`, an unsigned `glow(_) -> Low`
answers a `Light`, and a caller wanting the row `[Low, Other]` is refused.
Declaring a type can therefore change what an unsigned definition below it
means. A signature on `glow` is the recourse.

### Why may a row write `*`?

Ori had no written tail, and Faun kept that at first: a variant is
matched, and matching covers the whole row, so "at least these tags"
seemed a promise about a catch-all clause rather than a fact about a
type. But the promise could not be written where a caller reads it. A
clause list with a catch-all takes any tag, yet `f : [A] -> Nat` with
`f(A) -> 1` and `f(_) -> 0` refused a caller's `f(C)`, so such a
definition stayed unsigned. And inference makes open rows, which messages
print, `Result({}, [BrokenPipe, NoSpace, Other(Nat), *])`, and which no
signature could then write.

`*` means what it means in a record: a rest its caller chooses, never
named, each `*` a rest of its own. What it changes depends on which way
the row flows, not on how deep it lies.

- **Where a definition receives the row**: a parameter, or the answer of
  a callback it is handed. `f : [A, *] -> Nat` takes any row that holds
  `A` or may: the literal `B`, a closed `[A, B]`, an unsigned producer's
  open row. Like `{name: Str, *}` refusing a record with no `name`, it
  refuses a closed row lacking `A`. The body sees the rest as a caller's,
  and three things are refused. Clauses that take the row apart without
  a catch-all are refused as any clauses not covering their type are,
  `` no clause matches `f(_)` ``, and arms the same way: `` `[A, *]` may
  hold tags a caller adds, which no pattern here matches: write a
  catch-all ``. A pattern naming a tag the row does not write, as
  `r.age` under `{name: Str, *}` is refused: `` `B` is not among the
  tags of `[A, *]`, whose others are a caller's ``, with help to write it
  in the row before its `*`; so is passing the value where `[A, B, *]`
  is wanted, as a record with `*` cannot be passed where it would need a
  field it may not have. And `equals` or `to_str` of the row, `` `[A, *]`
  may hold tags a caller adds, which it cannot say `equals` of ``. The
  value may be bound, passed where a row with `*` is
  wanted, and matched; it may not be answered where another `*` is.
- **Where a definition gives the row**: its result, or a callback's
  parameter. A written row there is already closed for the body and open
  to callers, so `[A, *]` and `[A]` are one type. `*` is allowed rather
  than refused as redundant, so that a type a message prints may be
  pasted where it was printed; a message shows such a row as `[A]`, as
  it is most often written.
- **Inside other types**: `Result(a, [A, *])` and `List([A, *])` take the
  position of the type around them. As a parameter, a match on the
  `Err`'s row needs a catch-all; as a result, either spelling is the
  same. `(Nat -> [A, *]) -> Nat` is handed a callback that may answer
  tags of the caller's; `([A, *] -> Nat) -> Nat` gives its callback rows,
  and is `([A] -> Nat) -> Nat`.
- **Several `*`**: each is its own rest, as for records.
  `([A, *], [A, *]) -> Bool` takes two rows whose rests the caller
  chooses apart, and `[A, *] -> [A, *]` written `f(x) -> x` is refused:
  the answer's rest is not the parameter's. The message tells the two
  apart, `` expected `[A]`, found `[A, *]` ``, numbering them where they
  would still read alike, with help to write the answer as `_`.
- **Payloads**: `[A(Nat), *]` fixes the written tags' payloads; a
  caller's other tags carry what they carry, and a caller's `A` must
  carry a `Nat`, since one row cannot hold two `A`s.
- **Form**: `*` comes last, once, and is never named, as in a record:
  `[*]` is any row, which the body can only bind, pass on or take with a
  catch-all; `[A, *r]` and `[*, A]` are refused by the grammar.
- **`_`**: a hole is whatever the clauses make it, so `_` and `*` do not
  meet. In a signature that writes `_` somewhere, its written types are
  where inference starts and a `*` is a rest inference may fill, as a
  record's `*` is there.
- **Aliases**: transparent, so `Failure = [NotFound, *]` gives each use
  its own rest, read in the position of that use.
- **Declared types**: a declared type's tags are its own, so
  `Light : [Low, High, *]` is refused, "a declared type's tags are its
  own", and so is a `*` anywhere in what a declared type holds, a
  record's included, "a declared type holds no row with `*`": the type
  would hold a rest no one chose.
- **Coverage**: a row with a caller's rest is covered only by a
  catch-all, which the coverage check says of any open row.
- **Derived members**: a row with a caller's rest has no `equals` or
  `to_str` in the body, as a record with `*` has none; no tag row has
  `hash_into`. A caller, who knows the rest, compares and prints its own
  rows.

Messages print only writable types. A row still open to inference, or
one whose `*` a signature wrote where the definition receives it, shows
`*`; the rest of a row a definition gives, which no one need write, does
not. At a use, a row its signature closed shows `*`, `[Green, Red, *]`
for a constant written `[Red, Green]`, since the use may add tags to it.

The cost is a second spelling of a result row, and one more thing a
reader of a parameter must notice. Wrapping a producer's row in a tag
with a variable payload, `[TimedOut, Inner(e)]`, is still the way to keep
provenance rather than merge rows.

### Why can exports hide constructors?

So a module can promise something about every value of a type. Only
`parse_email` builds an `Email`, so every `Email` was checked. Only the
target builds a `Host` or a `File`, so a program cannot forge authority or a
descriptor ([effects](../forms/effects.md)). Visibility does not reserve a
spelling: outside the module, the same word is just a structural tag that
no parameter of the sealed type accepts.

Why not mark visibility at the declaration? There are three named forms:
the transparent alias (`=`), the open nominal type (`:` with `T.*`) and the
sealed one (`:` with `T`). Two declaration operators cannot name three
things, and the export list is where a reader already looks for a module's
contract. The cost is that an importer cannot match a sealed type; it uses
the methods the module exports.

### Why can't data hold functions?

Faun's functions are second order. A definition taking a function is made
once for each set of callbacks it is handed, so every call of a parameter
is a direct call, and no closure exists at run time. A function stored in a
constructor would outlive the call that knows what it is, and would need
exactly the closures the language does without: a code pointer, captured
values and an indirect call.

Ori's reason was its totality checker: a recursive type holding a function
of itself lets a program recurse with no recursive call in sight. Faun does
not check termination, so that premise is gone; the representation reason
stands. The cost is that a table of handlers or a strategy cannot be data.
The recourse is data that says what to call. The rock-paper-scissors arena
holds its players as a sum, `Bot : [AlwaysRock(AlwaysRock), ...]`, each
constructor carrying a bot's state, and `Bot`'s methods forward to each
bot's own.

## History

- **2026-11-21:** a row may write `*`, `[A, *]`, a rest its caller
  chooses, as a record's `*` is. Before, the grammar refused it, "expected
  one of `]`, a type name, found `*`", though inference made such rows
  and messages printed them, and a signature could not say that clauses
  with a catch-all take any tag. A declared type holding a record with
  `*` in a constructor, `P : [A({x: Nat, *})]`, came to be refused at
  the same time; it had been accepted, and its program failed to build.

## Related

- [Records](records.md): the other row, read by field, whose `*` a tag
  row's follows
- [Types](../types.md): inference, rows in general, and `where` bounds
- [Methods](../forms/methods.md): type blocks, derived members, and a sum's
  name standing for its constructors
- [Pattern matching](../forms/pattern-matching.md): coverage, which is what
  closes a row
- [Modules](../forms/modules.md): export lists and imports
- [Functions](functions.md): what a function may and may not be
