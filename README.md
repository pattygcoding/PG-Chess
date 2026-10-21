# PG-Chess

A chess engine written in Rust and compiled to WebAssembly, with a vanilla
JavaScript front end that runs the whole search in a Web Worker.

The engine uses 64-bit bitboards, iterative deepening negamax with alpha-beta
pruning and fail-soft bounds, a 32 MB transposition table, MVV-LVA ordering backed
by killer and history heuristics, and a quiescence search at the leaves.  It
targets club strength and thinks 6–8 plies plus quiescence well inside a second in a
browser tab.

* Live on the arcade: **<https://arcade.pattygcoding.com/chess>**
* The JavaScript interface is one `Engine` class; the page never sees a bitboard.

## Numbers

Measured with `cargo run --release --bin bench` on the development machine and, by
the harness's own perft panel, in the browser.

| Check | Native | Browser (Wasm) |
| --- | --- | --- |
| perft(5) from the start position, 4,865,609 nodes | 0.14 s | 0.16 s |
| perft(4) from Kiwipete, 4,085,603 nodes | 0.11 s | — |
| Move generation rate | ~35 Mnodes/s | ~30 Mnodes/s |
| Search rate (Club level) | ~3.5 Mnodes/s | ~2.9 Mnodes/s |
| Club level, start position: depth in 0.7 s | 7 (25 ms) | 7 (47 ms) |
| Compiled module | — | 98 KB `.wasm` + 20 KB glue |

The Club level's whole turn — depth 7 plus quiescence — takes a few tens of
milliseconds, so its 0.7 s budget is a ceiling rather than a cost.

## How it works

| File | What lives there |
| --- | --- |
| `src/bitboard.rs` | 64-bit sets, precomputed knight/king/pawn tables, ray attacks |
| `src/moves.rs` | 16-bit move encoding, `MoveList`, UCI parse and format |
| `src/board.rs` | Position, FEN reader/writer, incremental Zobrist, make/unmake, SAN |
| `src/movegen.rs` | Pseudo-legal generation plus a filtered legal version |
| `src/evaluate.rs` | PeSTO's piece-square tables, tapered, with pawn-structure terms |
| `src/tt.rs` | Compile-time Zobrist keys and the transposition table |
| `src/search.rs` | Iterative deepening, alpha-beta, null-move, LMR, quiescence |
| `src/perft.rs` | The move generator's test rig, and the published counts |
| `src/lib.rs` | The `wasm-bindgen` interface the page talks to |
| `web/` | The harness: `index.html`, `main.js`, `worker.js`, `pkg/` |

Three decisions are worth calling out:

**Sliding attacks are ray walks, not magic bitboards.**  A ray walk is a few times
slower than a magic lookup table, but it fits on a page and its correctness story is
a test: every square, across a thousand random occupancies, is checked against an
independent reference implementation.  The measured rate is about 35 Mnodes/s for
perft, far more than the search needs, and this README quotes it rather than
claiming it.

**`make_move` is the legality gate.**  The generator produces pseudo-legal moves;
playing one and asking whether the mover's king is attacked rejects the illegal
ones, and the move is rolled back.  That keeps the generator simple, and it means a
position with a captured king can never exist — which is what makes the search's
assumptions safe.

**The evaluation is PeSTO's.**  The middlegame and endgame piece-square tables come
from PeSTO, tapered linearly by the material left on the board, with a few
well-known terms on top: the bishop pair, doubled, isolated and passed pawns, rooks
on open files and a light king shield.  A test checks that a mirrored position
scores exactly the negated score, which is what catches an asymmetric table or a
mis-signed term.

## The move generator is verified, not asserted

Perft counts the leaves of the move tree, and every count below is published, so a
single wrong number points at a rule the generator gets wrong.  `cargo test` walks
all of them, and the harness runs the first five from the start position live in the
browser.

| Position | Depth 1 | 2 | 3 | 4 | 5 |
| --- | --- | --- | --- | --- | --- |
| Start | 20 | 400 | 8,902 | 197,281 | 4,865,609 |
| Kiwipete | 48 | 2,039 | 97,862 | 4,085,603 | |
| Position 3 | 14 | 191 | 2,812 | 43,238 | 674,624 |
| Position 4 | 6 | 264 | 9,467 | 422,333 | |
| Position 5 | 44 | 1,486 | 62,379 | 2,103,487 | |
| Position 6 | 46 | 2,079 | 89,890 | 3,894,594 | |

## Building

```sh
# The tests: the perft suite, the search, the board, the page-facing API
cargo test --release

# Engine speed report
cargo run --release --bin bench

# Build the Wasm module and the glue the page imports
wasm-pack build --target web --release --out-dir pkg

# Serve the harness (ES modules need http, not file://)
python -m http.server 8123 --directory web
```

After a `wasm-pack build`, copy the two artefacts the page loads:

```sh
cp pkg/pg_chess.js pkg/pg_chess_bg.wasm web/pkg/
```

That copy is committed, so the harness runs from a fresh clone without a Rust
toolchain.

## The JavaScript interface

```js
import init, { Engine } from './pkg/pg_chess.js';

await init();
const engine = new Engine('rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1');
engine.legalMoves();          // "b1a3 b1c3 ..."  (UCI, space separated)
engine.play('e2e4');          // "e4" (SAN); throws on an illegal move
engine.undo();
engine.search(7, 700, 1);     // depth, milliseconds, level 0|1|2
engine.perft(5);              // 4865609
engine.outcome();             // "playing" | "checkmate" | "stalemate" | ...
```

`search` returns an object with `uci()`, `san()`, `pv()`, `score()`, `depth()`,
`nodes()`, `timeMs`, `nodesPerSecond` and `mateIn`.  Scores are centipawns from the
side to move's point of view, and `mateIn` is the distance in moves when the score is
a forced mate.

The three levels differ in depth cap and time budget, and the lowest one also picks
among moves within 70 centipawns of the best: it still sees tactics, it just does not
always play the single best move, which is what stops the bottom setting from
feeling like a misunderstanding of chess.

| Level | Depth cap | Budget | "Nearly as good" margin |
| --- | --- | --- | --- |
| Casual (0) | 4 | 250 ms | 70 cp |
| Club (1) | 7 | 700 ms | 20 cp |
| Strong (2) | 10 | 1500 ms | 0 cp |

## Limitations

* Sliding attacks are ray walks; magic bitboards are the obvious next optimisation
  if the search ever needs the headroom.
* The transposition table ages rather than being replaced wholesale: each search
  bumps the generation, and a shallow entry may then be overwritten.
* No opening book and no endgame tablebases.  The engine plays the opening itself,
  which is fine at this strength but is the cheapest available strength gain.
* The engine is checked against perft, against hand-written tactical positions and
  against itself; it has not been run through a self-play Elo gauntlet.

## License

MIT — see [LICENSE](LICENSE).

