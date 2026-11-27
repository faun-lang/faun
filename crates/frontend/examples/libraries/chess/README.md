# chess

Chess for Faun: squares, pieces and positions, positions read from and
written as FEN, every legal move of a position, with castling, en passant
and promotion, moves read from and written as UCI, and perft, the count
that checks a move generator.

```ruby ignore
package {
  uses: {
    chess: "../src/chess"
  }
}

import chess {Position, Move, Square, Piece, White, Knight, DoublePush, perft}

expect Position.start.legal_moves().len() == 20
expect perft(Position.start, 2) == 400

# A move is read from UCI against the position it is played from, which
# says which piece moves and how, and `play` checks it is legal there.
expect Position.start.move_from_uci("e2e4").map_ok(Move.kind) == Ok(DoublePush)
expect Position.start.move_from_uci("e2e4").and_then(|move| Position.start.play(move).err_as(NoLegalMove)).map_ok(Position.to_fen)
    == Ok("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1")
expect Position.start.move_from_uci("e2e5") == Err(NoLegalMove)

expect Position.from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1") == Ok(Position.start)
expect Square.from_str("e4").map_ok(Square.to_nat) == Ok(28)
expect Piece(White, Knight).to_char() == 'N'
```

Every expectation above is one of the library's own, in [`src/`](src),
beneath the definition it shows.

## Positions

A `Position` is a game at one moment: the board, the side to move, the
castling rights left, the en passant square, and the two clocks, which is
everything FEN writes and everything the rules ask.

- `Position.start` is where a game begins.
- `Position.from_fen(text)` reads a FEN, and `position.to_fen()` writes
  one; `"${position}"` writes it too. The two clocks may be left out, as
  EPD leaves them.
- `position.legal_moves()` is every legal move of the side to move.
- `position.play(move)` plays a move, and `play_unchecked(move)` plays
  one `legal_moves` just gave, as below.
- `position.is_check()`, `is_checkmate()` and `is_stalemate()` say how
  the side to move stands.
- `position.piece_at(square)`, `pieces()`, `king(color)`, `turn()`,
  `castling()`, `en_passant()`, `halfmove_clock()` and
  `fullmove_number()` read it.

A FEN is refused with the reason as a tag, `Err(InvalidCastling)` say,
where a field is malformed, and also where the position it writes is one
no game could reach: a side without exactly one king (`InvalidKings`), a
pawn on the first or last rank (`PawnOnBackRank`), a castling right whose
king or rook has left its square, an en passant square no pawn just passed
over, or the side not to move in check (`OpponentInCheck`). So every
`Position` is one the rules can be asked of, and no method needs a
failure of its own for a board without a king.

## Moves

A `Move` is plain data: which piece goes from where to where, and what it
takes and becomes. It says nothing of the position it is played on, so
playing one checks it:

- `position.play(move)` answers the position after `move` where it is one
  of the position's legal moves, and `Err(IllegalMove(move))` where it is
  not, holding the move refused. This is the way to play a move from
  anywhere: a person, a UCI command, a file.
- `position.play_unchecked(move)` plays a move without asking. The move
  must be one of the position's `legal_moves()`: the loop that generates
  moves and then plays each, as `perft` does, needs no asking again. Any
  other move still answers a position, since nothing in Faun panics, but
  possibly one no game reaches, a king taken or the side not to move in
  check, and nothing asked of it then means anything.
- `position.move_from_uci(text)` reads a move as UCI writes it, and
  answers `Malformed` for text that is not UCI and `NoLegalMove` for text
  naming no legal move of the position. UCI gives only the squares and a
  promotion; the position says which piece moves, what it takes and what
  kind of move it is, so reading goes through it.

A move says `from()`, `to()`, the `role()` that moves, the role it
`captured()` and the role a pawn becomes, `promotion()`, each a `Result`
where there may be none, and its `kind()`: `Normal`, `DoublePush`,
`EnPassant` or `Castle(KingSide)` and `Castle(QueenSide)`.
`move.to_uci()`, and `"${move}"`, write it as UCI does: `e2e4`, `e7e8q`,
and a castle as the king's two squares, `e1g1`.

## Perft

`perft(position, depth)` counts the paths of `depth` legal moves from a
position. Tables of it are how a move generator is checked, since every
legal move must be found once and nothing else. `divide(position, depth)`
gives each first move with its share, so two generators that disagree can
be followed down to the move they first differ on. The library gives the
[Chess Programming Wiki's counts](https://www.chessprogramming.org/Perft_Results)
for all six of its positions, as deep as was tried: the start to depth 5
and Kiwipete to depth 4. Its expectations ask the shallow ones on the
reference interpreter, and
[`crates/cli/tests/chess.rs`](../../../../cli/tests/chess.rs) asks deeper
ones of the example built for the machine.

## How it is built, and why

The design follows two Rust crates, [shakmaty](https://docs.rs/shakmaty)
and [cozy-chess](https://docs.rs/cozy-chess), where Faun lets it, and
departs where Faun makes something simpler.

- **One position type.** shakmaty has a `Position` trait for its
  variants, with `Chess` for the standard game, and cozy-chess one
  `Board`. Faun has no traits and this library no variants, so
  `Position` is the one type, as cozy-chess's `Board` is.
- **A move is data, and playing it is checked.** As in shakmaty, a
  `Move` is plain data and `play` checks it, answering a `Result`, while
  `play_unchecked` trusts the caller; cozy-chess's `play` panics on an
  illegal move, and `try_play` checks. Most moves come from outside, so
  a check where they are played is where it belongs, and `legal_moves`
  keeps no position for each move, most of which are never played. A
  `Move`'s constructor stays in the library: a move is had from a
  position, which knows what it takes and what kind of move it is.
- **UCI through the position.** shakmaty has a `UciMove` type, turned
  into a move by `to_move(position)`. Here `move_from_uci` reads the text
  against the position in one step, since text alone cannot say which
  piece moves, and a type holding the text between the two would add a
  name and do nothing else.
- **Two refusals.** `play` refuses with `IllegalMove(move)`, holding the
  move as shakmaty's `PlayError` does; `move_from_uci` with `NoLegalMove`,
  having no move to hold. They are named apart so both may join one row
  through `?`.
- **Sealed values.** `File`, `Rank` and `Square` are numbers no one
  outside the library can make out of range, as shakmaty's and
  cozy-chess's enums are; `Square.from_nat`, `from_str` and the like
  check once. Squares are numbered as both crates number them, `a1` 0 to
  `h8` 63. A square steps to its neighbour by a `Direction`,
  `square.step(North)`, rather than by numbers added, so walking off the
  board is an `Err(OffBoard)` to match on rather than arithmetic to get
  right.
- **No flags.** Which side and which castle are `Color` and
  `CastlingSide`, never a `Bool`: `rights.has(White, KingSide)`.
  A `Piece` is `Piece(color, role)`, as shakmaty's is a color and a
  role.
- **Failures as tags.** FEN's errors are shakmaty's `ParseFenError` and
  `PositionError` in one row of tags, `FenError`, each named for its
  cause, so a caller's own failures join it through `?`.
- **The board is 64 squares, not bitboards.** cozy-chess keeps a
  bitboard a piece and finds sliding moves by magic tables, shakmaty
  likewise. Faun has `U64` and its bit operations, but a list of the 64
  squares reads as the rules do, and every rule is written once as the
  rules say it: each piece's moves, then each move played, then the king
  looked at. Pins, checks, discovered checks and the en passant capture
  that uncovers its king along a rank are not cases of their own.
- **FEN's en passant square.** A pawn's double step always leaves its
  square, as FEN's own rule says, whether or not a pawn could take it
  there; shakmaty writes it only where a capture is legal. Moves are the
  same either way, and a FEN read is written back as it was.

## Layout

Each type has a module of its own in [`src/`](src), named for it, as
`square.fn` holds `Square` and `castling_rights.fn` holds
`CastlingRights`, with the helpers that serve it beside it.
[`src/chess.fn`](src/chess.fn) is the root, and the one entry point: it
exports the whole public API, and `perft` and `divide` with it.

Not here yet: SAN, Chess960, the draw rules (repetition, fifty moves,
insufficient material) and Zobrist hashing.

## Speed

Written as one would teach it, in the most natural Faun: where that runs
slowly, the compiler has a bug to fix. On x86-64 it finds nearly two
million positions a second: the start's perft to depth 5, 4,865,609, in
about two and a half seconds. A plain C mailbox generator that plays and
tests each move the same way, the twin in
[`eval/workloads/perft`](../../../../../eval/workloads/perft), is some
twenty times as fast. The ratchet measures it, so making it faster
is the compiler's work.

## Example

[`examples/perft.fn`](examples/perft.fn) prints a perft divided by first
move, as engines' `go perft` does, for a depth and a FEN:

```sh
$ faun run examples/perft.fn 1 "4k3/8/8/8/8/8/8/4K2R w K - 0 1"
e1e2: 1
e1f1: 1
e1d1: 1
e1f2: 1
e1d2: 1
h1h2: 1
h1h3: 1
h1h4: 1
h1h5: 1
h1h6: 1
h1h7: 1
h1h8: 1
h1g1: 1
h1f1: 1
e1g1: 1

Nodes searched: 15
```

## Using it

Name the library's root in your package's `uses`, by its path from your
package, as above. Published on its own, it would be named by the URL of
its bundle, which `faun bundle src/chess.fn` makes.

## License

[Zero-Clause BSD](../../../../../LICENSE), as Faun is.
