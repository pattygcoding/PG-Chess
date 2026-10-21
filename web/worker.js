//! The engine worker.
//!
//! The page never touches the wasm module directly: it sends commands here and
//! gets snapshots back, which is what keeps a search from blocking the board.  The
//! worker owns the game — board, move list, level — so the page's job is only to
//! draw what it is told and to send clicks.
//!
//! Commands (each carries an `id`, echoed back on the reply):
//!
//!   { cmd: 'new',   fen?, side? }   start a game, optionally from a FEN
//!   { cmd: 'state' }                ask for a snapshot
//!   { cmd: 'play',  uci }           play a legal move
//!   { cmd: 'undo',  plies }         take plies back
//!   { cmd: 'search', level }        think and reply with the move it would play
//!   { cmd: 'perft', depth }         count leaves (the move generator's test)
//!   { cmd: 'tt',    megabytes }     resize the transposition table
//!
//! Replies are `{ id, ok: true, ... }` or `{ id, ok: false, error }`, and a search
//! also streams `{ type: 'progress', ... }` messages as each depth completes.

import init, { Engine } from './pkg/pg_chess.js';

/**
 * The module's initialisation, started immediately.
 *
 * The handler below waits on this promise rather than the module awaiting it at
 * the top level: top-level await in a module worker is not supported everywhere,
 * and a worker that never finishes evaluating never answers a single message.  A
 * message that arrives before the wasm is ready is simply answered a moment later.
 */
const ready = init();

/** The engine handle, created on the first command. */
let engine = null;

/** The moves played this game, in SAN, so the page can show a move list. */
const played = [];

/** The level the page selected (0 casual, 1 club, 2 strong). */
let level = 1;

/** Depth caps and time budgets per level, mirrored from the Rust side. */
const LEVELS = [
  { depth: 4, time: 250 },
  { depth: 7, time: 700 },
  { depth: 10, time: 1500 },
];

/** The perft totals this harness checks against, for the starting position. */
const PERFT_EXPECTED = { 1: 20, 2: 400, 3: 8902, 4: 197281, 5: 4865609 };

/** A snapshot of everything the page draws. */
function snapshot() {
  return {
    fen: engine.fen(),
    placement: engine.piecePlacement(),
    side: engine.sideToMove(),
    legal: engine.legalMoves(),
    outcome: engine.outcome(),
    matedSide: engine.matedSide(),
    inCheck: engine.inCheck(),
    lastMove: engine.lastMove(),
    ply: engine.plyCount(),
    moves: played.slice(),
  };
}

/** A second engine, kept for measurements that must not disturb the game. */
let scratch = null;

/**
 * Converts a UCI principal variation into SAN.
 *
 * The line has to be replayed to write it down — SAN depends on the position it was
 * played from — so this borrows a scratch engine with a one-megabyte table rather
 * than touching the game's board.
 */
function pvToSan(fen, pv) {
  const moves = String(pv ?? '')
    .split(' ')
    .filter(Boolean);
  if (moves.length === 0) {
    return '';
  }
  if (!scratch) {
    scratch = new Engine();
    scratch.setTtSize(1);
  }
  scratch.newGameFromFen(fen);
  const parts = [];
  for (const uci of moves) {
    try {
      parts.push(scratch.play(uci));
    } catch {
      break;
    }
  }
  return parts.join(' ');
}

/**
 * Iterative deepening, driven from here one depth at a time.
 *
 * The engine's own `search` runs its own deepening loop; calling it once per depth
 * with the budget that is left gives the page a progress line and still leaves the
 * transposition table and the heuristics in place between calls, so the work is
 * never repeated.
 */
function think(levelIndex, fen) {
  const config = LEVELS[levelIndex] ?? LEVELS[1];
  const started = performance.now();
  let best = null;
  let spent = 0;

  for (let depth = 1; depth <= config.depth; depth += 1) {
    const budget = Math.max(30, config.time - spent);
    const info = engine.search(depth, budget, levelIndex);
    spent = performance.now() - started;
    best = info;
    self.postMessage({
      type: 'progress',
      depth: info.depth(),
      score: info.score(),
      mateIn: info.mateIn,
      nodes: info.nodes(),
      timeMs: info.timeMs,
      nodesPerSecond: info.nodesPerSecond,
      pv: pvToSan(fen, info.pv()),
      uci: info.uci(),
      san: info.san(),
    });
    if (info.mateIn !== 0 || spent >= config.time) {
      break;
    }
  }
  return best;
}

self.onmessage = async (event) => {
  const { id, cmd } = event.data;
  try {
    // Every command waits for the wasm to be ready, and the first one creates the
    // engine.  Nothing is dropped: a message that arrives early is answered late.
    await ready;
    if (!engine) {
      engine = new Engine();
    }
    switch (cmd) {
      case 'new': {
        if (event.data.fen) {
          engine.newGameFromFen(event.data.fen);
        } else {
          engine.newGame();
        }
        played.length = 0;
        if (event.data.level !== undefined) {
          level = event.data.level;
        }
        engine.resetSearch();
        self.postMessage({ id, ok: true, state: snapshot(), tt: engine.ttSize() });
        break;
      }
      case 'state': {
        self.postMessage({ id, ok: true, state: snapshot(), tt: engine.ttSize() });
        break;
      }
      case 'play': {
        const san = engine.play(event.data.uci);
        played.push(san);
        self.postMessage({ id, ok: true, san, state: snapshot() });
        break;
      }
      case 'undo': {
        let plies = event.data.plies ?? 1;
        while (plies > 0 && engine.undo()) {
          played.pop();
          plies -= 1;
        }
        self.postMessage({ id, ok: true, state: snapshot() });
        break;
      }
      case 'search': {
        const info = think(event.data.level ?? level, engine.fen());
        if (!info) {
          self.postMessage({ id, ok: false, error: 'the search returned nothing' });
          break;
        }
        self.postMessage({
          id,
          ok: true,
          search: {
            uci: info.uci(),
            san: info.san(),
            score: info.score(),
            mateIn: info.mateIn,
            depth: info.depth(),
            nodes: info.nodes(),
            timeMs: info.timeMs,
            nodesPerSecond: info.nodesPerSecond,
            pv: pvToSan(engine.fen(), info.pv()),
          },
        });
        break;
      }
      case 'perft': {
        // A throwaway engine on the starting position: the count has to be of the
        // standard position, not of whatever the game has reached, and the game's
        // own board and move history must not be disturbed.
        const depth = event.data.depth;
        const counter = new Engine();
        counter.setTtSize(1);
        const started = performance.now();
        const nodes = counter.perft(depth);
        const elapsed = performance.now() - started;
        counter.free();
        self.postMessage({
          id,
          ok: true,
          depth,
          nodes,
          expected: PERFT_EXPECTED[depth] ?? null,
          timeMs: elapsed,
        });
        break;
      }
      case 'tt': {
        engine.setTtSize(event.data.megabytes);
        self.postMessage({ id, ok: true, tt: engine.ttSize() });
        break;
      }
      default:
        self.postMessage({ id, ok: false, error: `unknown command: ${cmd}` });
    }
  } catch (error) {
    // wasm-bindgen turns a Rust error string into a thrown value, which is exactly
    // what an illegal move or an unreadable FEN produces.
    self.postMessage({ id, ok: false, error: String(error?.message ?? error) });
  }
};
