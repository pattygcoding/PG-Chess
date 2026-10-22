# PG-Chess

A chess engine written in Rust and compiled to WebAssembly, with a vanilla
JavaScript front end that runs the whole search in a Web Worker.

The engine uses 64-bit bitboards, iterative deepening negamax with alpha-beta
pruning and fail-soft bounds, aspiration windows, a 32 MB transposition table,
MVV-LVA ordering backed by killer and history heuristics, static exchange
evaluation, reverse futility and a quiescence search at the leaves.  It targets club
strength and reaches ten ply well inside a second in a browser tab.

* Live on the arcade: **<https://arcade.pattygcoding.com/chess>**
* The JavaScript interface is one `Engine` class; the page never sees a bitboard.

## Numbers

Measured with `cargo run --release --bin bench` on the development machine and, for
the Wasm column, by loading the same compiled module in Node and asking it the same
questions (`cargo run --release --bin probe` does it natively).  Node and a browser
tab run the same module, so the column is a browser number that anyone can reproduce
without a browser.

| Check | Native | Wasm |
| --- | --- | --- |
| perft(5) from the start position, 4,865,609 nodes | 0.15 s | 0.21 s |
| perft(4) from Kiwipete, 4,085,603 nodes | 0.12 s | — |
| Move generation rate | ~33 Mnodes/s | ~23 Mnodes/s |
| Search rate | ~1.5–2.4 Mnodes/s | ~1.4–2.0 Mnodes/s |
| Club level (0.7 s, 10 ply cap): depth reached | 10, in ~0.2 s | 10, in ~0.3 s |
| Strong level (1.5 s, 14 ply cap): Kiwipete | 13, in 1.2 s | 12, in 0.95 s |
| Compiled module | — | 108 KB `.wasm` + 20 KB glue |

Both levels finish inside their budget and are then held back by their depth caps, so
the caps — not the clock — are what makes a level's turn take the time it takes.

## How strong it is, and how that was checked

`src/bin/match.rs` plays the engine against itself with one technique switched off
per run, and reports a win/draw/loss count and an Elo estimate with a 95% interval.
Two hundred games at equal time against the engine as it was first released:

| Comparison | Result |
| --- | --- |
| Everything on vs the released engine (equal time, 300 games) | **+200 Elo** (+170 to +231) |
| Everything on vs the released engine (equal depth 8, 200 games) | +91 Elo (+56 to +125) |
| Everything on vs the same search without the new techniques (equal depth 6, 300 games) | +64 Elo (+43 to +86) |
| Everything on vs the same engine with the score read from white (equal time, 60 games) | won every game |
| Identical engines against each other (the control) | 50% of the points |

The last row is the one that says the harness works: with both sides configured the
same it reports +0 Elo over 120 games, so the numbers above are about the flags and
not about the opening lines or the first-move advantage.

Two real bugs came out of building that harness, and both had been shipped:

* **The evaluation was written from white's side.**  `negamax` negates a child's
  score, so an evaluation has to be the *mover's*; a score that was always white's
  made the search prefer its own worst move wherever the two disagreed, which is why
  the engine could hang a queen with a straight face.  Fixed in
  `evaluate_with`; `EvalFeatures::as_shipped` keeps the old behaviour, which is how
  the match above can play against it at all.
* **The principal variation was off by one.**  Each node copied one entry too many
  from its child, so a PV could contain empty moves (`a1a1`) — the UI's predicted
  line, quietly wrong.  `the_principal_variation_is_a_legal_line` is the test that
  now holds it down.

Each technique was then measured on its own,
`match --old full --without <flag>`, 120 games at equal depth and 160 at equal time.
The error bars are around ±20 Elo, so this is what the set is worth:

| Technique | Equal depth | Equal time | Kept? |
| --- | --- | --- | --- |
| Static exchange evaluation in quiescence | +137 | | yes |
| Aspiration windows | +134 | | yes |
| King safety (attackers on the king's ring) | +114 | | yes |
| Reverse futility | +67 | | yes |
| King proximity to a passed pawn | +67 | | yes |
| Log-shaped late move reductions | +23 | +9 | yes |
| Futility | 0 | -15 | yes: it saves nodes for nothing |
| Soft time limit | — | 0 | yes: it saves the user's time for nothing |
| Mobility | -20, -44 | -4 | no |
| Exchange-value capture ordering | -44 | +17 | no |
| Internal iterative deepening | 0 | -41 | no |
| Late move pruning | -144, -89 | -7 | no |

The negative results are the point of the exercise.  Late move pruning and internal
iterative deepening are both standard, both measured *worse*, and both are off by
default with the flag and the measurement left in place.  Mobility is off for the
same reason: counted this way it cost more than it paid, and the next thing to try is
the usual refinement — counting only the squares the enemy's pawns do not cover —
rather than more weight.

## How it works

| File | What lives there |
| --- | --- |
| `src/bitboard.rs` | 64-bit sets, precomputed knight/king/pawn tables, ray attacks |
| `src/moves.rs` | 16-bit move encoding, `MoveList`, UCI parse and format |
| `src/board.rs` | Position, FEN reader/writer, incremental Zobrist, make/unmake, SAN |
| `src/movegen.rs` | Pseudo-legal generation plus a filtered legal version |
| `src/evaluate.rs` | PeSTO's piece-square tables, tapered, with pawn-structure, king-safety and passed-pawn terms |
| `src/see.rs` | Static exchange evaluation, with a brute-force reference in its tests |
| `src/tt.rs` | Compile-time Zobrist keys and the transposition table |
| `src/search.rs` | Iterative deepening, alpha-beta, aspiration, null-move, SEE, LMR, quiescence |
| `src/perft.rs` | The move generator's test rig, and the published counts |
| `src/lib.rs` | The `wasm-bindgen` interface the page talks to |
| `src/bin/match.rs` | Self-play matches between feature sets, with Elo and its interval |
| `src/bin/bench.rs` | The speed report above |
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

**The evaluation is PeSTO's, from the mover's point of view.**  The middlegame and
endgame piece-square tables come from PeSTO, tapered linearly by the material left on
the board, with a few well-known terms on top: the bishop pair, doubled, isolated and
passed pawns, rooks on open files, a light king shield, king safety and the king's
proximity to a passed pawn.  It is scored for the side to move, and two tests hold
that down from both directions: a mirrored position has to score *the same* (the
mover is equally well off in the mirrored world, tempo included), and a side that is
down a queen has to score negative *whichever* colour it is.  The second test is the
one that matters — the first release only checked the mirror, which a score written
from white's side satisfies perfectly, and that is how a sign error survived into
production.

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

# Self-play matches, which is where the strength claims come from
cargo run --release --bin match -- --games 300 --ms 60 --old shipped

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
toolchain.  If `wasm-pack` is not installed, the two steps it wraps are
`cargo build --release --target wasm32-unknown-unknown` followed by
`wasm-bindgen --target web --out-dir pkg target/wasm32-unknown-unknown/release/pg_chess.wasm`.

## The JavaScript interface

```js
import init, { Engine } from './pkg/pg_chess.js';

await init();
// The constructor takes no position: a FEN goes through the static factory or
// through `newGameFromFen`.  `new Engine(fen)` compiles and quietly ignores the
// argument, which is a trap worth naming here.
const engine = Engine.fromFen('rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1');
engine.legalMoves();          // "b1a3 b1c3 ..."  (UCI, space separated)
engine.play('e2e4');          // "e4" (SAN); throws on an illegal move
engine.undo();
engine.search(0, 0, 1);       // depth, milliseconds, level 0|1|2 (0 = the level's own)
engine.perft(5);              // 4865609
engine.outcome();             // "playing" | "checkmate" | "stalemate" | ...
```

`search` returns an object with `uci()`, `san()`, `pv()`, `score()`, `depth()`,
`nodes()`, `timeMs`, `nodesPerSecond` and `mateIn`.  Scores are centipawns from the
side to move's point of view, and `mateIn` is the distance in moves when the score is
a forced mate.

The three levels differ in depth cap and time budget, and the lower ones also pick
among moves the search scores within a margin of the best one, favouring the better
end of that band: they still see tactics — both take a free queen — they just do not
always play the single best move.

| Level | Depth cap | Budget | "Nearly as good" margin |
| --- | --- | --- | --- |
| Casual (0) | 5 | 250 ms | 70 cp |
| Club (1) | 10 | 700 ms | 20 cp |
| Strong (2) | 14 | 1500 ms | 0 cp |

A weakened level's margin is a promise the search has to keep, which means the moves
inside the band need their *real* scores.  Probing a root move at alpha returns a
bound, so every move that fails looks one centipawn from the best, and a "within 70
centipawns" level plays moves far worse than that:
`every_move_the_weakening_offers_is_really_within_its_margin` re-scores every root
move with a full window and holds the band to what it says.

## Limitations

* Sliding attacks are ray walks; magic bitboards are the obvious next optimisation
  if the search ever needs the headroom.
* The transposition table ages rather than being replaced wholesale: each search
  bumps the generation, and a shallow entry may then be overwritten.
* No opening book and no endgame tablebases.  The engine plays the opening itself,
  which is fine at this strength but is the cheapest available strength gain.
* The matches are the engine against itself at short time controls, which measures
  the *differences* between configurations well and absolute strength not at all.
* The evaluation has no mobility term (it did not pay for itself) and no threats,
  safe squares or rook-lift ideas; the pawn structure terms are the textbook three.

## License

MIT — see [LICENSE](LICENSE).

