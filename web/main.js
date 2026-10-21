//! The page's half of the harness: it draws the board and sends commands.
//!
//! Everything that thinks lives in the worker; this file owns the DOM, the click
//! handling and the small amount of game flow (whose turn it is, when to ask the
//! engine to move, how to offer a promotion choice).

/** Unicode glyphs by FEN letter: no image files, and they theme with the page. */
const GLYPHS = {
  P: '\u2659',
  N: '\u2658',
  B: '\u2657',
  R: '\u2656',
  Q: '\u2655',
  K: '\u2654',
  p: '\u265F',
  n: '\u265E',
  b: '\u265D',
  r: '\u265C',
  q: '\u265B',
  k: '\u265A',
};

const FILES = 'abcdefgh';
const boardElement = document.getElementById('board');
const promotionElement = document.getElementById('promotion');
const messageElement = document.getElementById('message');
const turnElement = document.getElementById('turn');
const depthElement = document.getElementById('depth');
const scoreElement = document.getElementById('score');
const nodesElement = document.getElementById('nodes');
const npsElement = document.getElementById('nps');
const pvElement = document.getElementById('pv');
const movesElement = document.getElementById('moves');
const evalBar = document.getElementById('eval-bar');
const ttElement = document.getElementById('tt-size');
const levelSelect = document.getElementById('level');
const sideSelect = document.getElementById('side');
const perftRows = document.getElementById('perft-rows');

const worker = new Worker(new URL('./worker.js', import.meta.url), { type: 'module' });

let requestId = 0;
const pending = new Map();

/** The last snapshot the engine sent; the board is drawn from this. */
let state = {
  placement: 'rnbqkbnrpppppppp................................PPPPPPPPRNBQKBNR',
  side: 'white',
  legal: '',
  outcome: 'playing',
  matedSide: '',
  inCheck: false,
  lastMove: '',
  ply: 0,
  moves: [],
};

/** Board orientation: white at the bottom until it is flipped. */
let flipped = false;

/** The square a piece was picked up from, if any. */
let selected = null;

/** True while the engine is thinking, which locks the human out. */
let thinking = false;

/** Sends a command and resolves with the reply that carries the same id. */
function send(cmd, args = {}) {
  return new Promise((resolve, reject) => {
    const id = ++requestId;
    pending.set(id, { resolve, reject });
    worker.postMessage({ id, cmd, ...args });
  });
}

worker.onmessage = (event) => {
  const message = event.data;
  if (message.type === 'progress') {
    showProgress(message);
    return;
  }
  const waiter = pending.get(message.id);
  if (!waiter) {
    return;
  }
  pending.delete(message.id);
  if (message.ok) {
    waiter.resolve(message);
  } else {
    waiter.reject(new Error(message.error));
  }
};

worker.onerror = (event) => {
  setMessage(`Engine error: ${event.message}`, true);
};

/** The sides the human plays, from the "You play" selector. */
function humanSides() {
  return sideSelect.value === 'none' ? [] : [sideSelect.value];
}

function isHumanTurn() {
  return humanSides().includes(state.side);
}

function squareName(file, rank) {
  return FILES[file] + String(rank + 1);
}

/** The piece on a named square, using the placement string's a8-first order. */
function letterAt(name) {
  const file = FILES.indexOf(name[0]);
  const rank = Number(name[1]) - 1;
  return state.placement[(7 - rank) * 8 + file];
}

/** Draws the board from the current snapshot. */
function draw() {
  boardElement.textContent = '';

  for (let row = 0; row < 8; row += 1) {
    // Row 0 is the top of the board, which is rank 8 unless the board is flipped;
    // the files always run a-to-h left to right unless it is.
    const rank = flipped ? row : 7 - row;
    for (let column = 0; column < 8; column += 1) {
      const file = flipped ? 7 - column : column;
      const name = squareName(file, rank);
      const letter = letterAt(name);

      const square = document.createElement('div');
      square.className = `square ${(file + rank) % 2 === 0 ? 'dark' : 'light'}`;
      square.dataset.square = name;
      square.setAttribute('role', 'gridcell');
      if (selected === name) {
        square.classList.add('selected');
      }
      if (state.lastMove && (state.lastMove.startsWith(name) || state.lastMove.endsWith(name))) {
        square.classList.add('last');
      }

      if (letter !== '.') {
        const piece = document.createElement('span');
        piece.className = `piece ${letter === letter.toUpperCase() ? 'white' : 'black'}`;
        piece.textContent = GLYPHS[letter] ?? '?';
        square.append(piece);
      }

      // A dot on every square the picked-up piece can move to, and a ring where
      // it can capture.
      if (selected && legalTargets().has(name)) {
        const marker = document.createElement('span');
        marker.className = letter === '.' ? 'target' : 'target capture';
        square.append(marker);
      }

      // Coordinates: files along the bottom row, ranks up the left-hand column.
      if (row === 7) {
        const coord = document.createElement('span');
        coord.className = 'coord file';
        coord.textContent = FILES[file];
        square.append(coord);
      }
      if (column === 0) {
        const coord = document.createElement('span');
        coord.className = 'coord';
        coord.textContent = String(rank + 1);
        square.append(coord);
      }

      square.addEventListener('click', () => onSquareClick(name));
      boardElement.append(square);
    }
  }

  turnElement.textContent = state.side === 'white' ? 'White' : 'Black';
  drawMoves();
}

/** The destination squares available to the selected piece. */
function legalTargets() {
  const targets = new Set();
  if (!selected) {
    return targets;
  }
  for (const uci of state.legal.split(' ')) {
    if (uci.startsWith(selected) && uci.length >= 4) {
      targets.add(uci.slice(2, 4));
    }
  }
  return targets;
}

function drawMoves() {
  movesElement.textContent = '';
  for (let index = 0; index < state.moves.length; index += 2) {
    const row = document.createElement('li');
    const number = document.createElement('span');
    number.className = 'number';
    number.textContent = `${index / 2 + 1}.`;
    const white = document.createElement('span');
    white.className = 'ply';
    white.textContent = state.moves[index] ?? '';
    const black = document.createElement('span');
    black.className = 'ply';
    black.textContent = state.moves[index + 1] ?? '';
    row.append(number, white, black);
    movesElement.append(row);
  }
  movesElement.scrollTop = movesElement.scrollHeight;
}

function setMessage(text, warn = false) {
  messageElement.textContent = text;
  messageElement.classList.toggle('warn', warn);
}

function formatCount(value) {
  if (value >= 1_000_000) {
    return `${(value / 1_000_000).toFixed(2)}M`;
  }
  if (value >= 1000) {
    return `${(value / 1000).toFixed(1)}k`;
  }
  return String(value);
}

function formatScore(mateIn, score, side) {
  if (mateIn !== 0) {
    return mateIn > 0 ? `mate in ${Math.abs(mateIn)}` : `mate against ${side}`;
  }
  return `${score > 0 ? '+' : ''}${(score / 100).toFixed(2)}`;
}

/**
 * The evaluation bar, always drawn from white's point of view.
 *
 * The engine scores from the side to move's point of view, so the sign is turned
 * around when black is to move.  A mate pins the bar to one end.
 */
function drawEvalBar({ mateIn, score }) {
  let share;
  if (mateIn !== 0) {
    const whiteIsWinning = mateIn > 0 ? state.side === 'white' : state.side === 'black';
    share = whiteIsWinning ? 1 : 0;
  } else {
    const centipawns = state.side === 'white' ? score : -score;
    share = 1 / (1 + Math.exp(-centipawns / 320));
  }
  evalBar.style.width = `${Math.round(share * 100)}%`;
}

function showProgress(progress) {
  depthElement.textContent = String(progress.depth);
  nodesElement.textContent = formatCount(progress.nodes);
  npsElement.textContent = `${formatCount(progress.nodesPerSecond)}/s`;
  scoreElement.textContent = formatScore(progress.mateIn, progress.score, state.side);
  pvElement.textContent = progress.pv ? `principal variation: ${progress.pv}` : '';
  drawEvalBar(progress);
}

function outcomeText() {
  switch (state.outcome) {
    case 'checkmate':
      return `Checkmate — ${state.matedSide} is mated.`;
    case 'stalemate':
      return 'Stalemate — a draw.';
    case 'fifty-move':
      return 'Draw by the fifty-move rule.';
    case 'repetition':
      return 'Draw by threefold repetition.';
    case 'insufficient-material':
      return 'Draw — not enough material left to mate.';
    default:
      return '';
  }
}

function colourWord(side) {
  return side === 'white' ? 'White' : 'Black';
}

/** Applies a snapshot, draws it, and moves for the engine when it is its turn. */
async function applyState(reply) {
  if (reply.state) {
    state = reply.state;
  }
  if (reply.tt) {
    ttElement.textContent = `${Math.round(reply.tt / (1024 * 1024))} MB`;
  }
  selected = null;
  draw();

  if (state.outcome !== 'playing') {
    setMessage(outcomeText());
    return;
  }
  if (sideSelect.value === 'none' || !isHumanTurn()) {
    await engineTurn();
  } else {
    setMessage(state.inCheck ? 'You are in check.' : 'Your move.');
  }
}

/** Asks the worker to think, then plays what it chose. */
async function engineTurn() {
  if (thinking || state.outcome !== 'playing') {
    return;
  }
  const side = state.side;
  thinking = true;
  let chosen = null;
  setMessage(`${colourWord(side)} is thinking…`);
  try {
    const reply = await send('search', { level: Number(levelSelect.value) });
    const search = reply.search;
    if (search?.uci) {
      showProgress(search);
      setMessage(
        `${colourWord(side)} played ${search.san} — depth ${search.depth}, ` +
          `${formatScore(search.mateIn, search.score, side)}, ${formatCount(search.nodes)} nodes ` +
          `in ${Math.round(search.timeMs)} ms.`,
      );
      chosen = search;
    }
  } catch (error) {
    setMessage(`The engine could not move: ${error.message}`, true);
  } finally {
    // Released before the move is applied: applying it is what decides whether the
    // engine moves again, and in a two-engine game that call comes straight back.
    thinking = false;
  }
  if (chosen) {
    await applyState(await send('play', { uci: chosen.uci }));
  }
}

/** A click picks a piece up, or plays the move when a target square is clicked. */
async function onSquareClick(name) {
  if (thinking || state.outcome !== 'playing' || !isHumanTurn()) {
    return;
  }
  if (selected && legalTargets().has(name)) {
    await playMove(selected + name);
    return;
  }
  const letter = letterAt(name);
  const ownPiece = letter !== '.' && (letter === letter.toUpperCase()) === (state.side === 'white');
  selected = ownPiece ? name : null;
  draw();
}

/** Plays a move, asking which piece to promote to when the rules allow a choice. */
async function playMove(uciPrefix) {
  const choices = state.legal
    .split(' ')
    .filter((uci) => uci.startsWith(uciPrefix) && uci.length === 5);
  if (choices.length === 0) {
    await pushMove(uciPrefix);
    return;
  }
  const piece = await askPromotion();
  selected = null;
  if (!piece) {
    draw();
    return;
  }
  await pushMove(uciPrefix + piece);
}

function askPromotion() {
  return new Promise((resolve) => {
    promotionElement.hidden = false;
    promotionElement.textContent = '';
    const picker = document.createElement('div');
    picker.className = 'picker';
    const promoting = state.side === 'white';
    for (const letter of ['q', 'r', 'b', 'n']) {
      const button = document.createElement('button');
      button.type = 'button';
      button.textContent = GLYPHS[promoting ? letter.toUpperCase() : letter];
      button.setAttribute('aria-label', `Promote to ${letter.toUpperCase()}`);
      button.addEventListener('click', () => {
        promotionElement.hidden = true;
        resolve(letter);
      });
      picker.append(button);
    }
    promotionElement.append(picker);
  });
}

async function pushMove(uci) {
  try {
    await applyState(await send('play', { uci }));
  } catch (error) {
    setMessage(`That move cannot be played: ${error.message}`, true);
    selected = null;
    draw();
  }
}

/** Fills in the perft table, one row per depth. */
async function runPerft() {
  perftRows.textContent = '';
  const button = document.getElementById('run-perft');
  button.disabled = true;
  try {
    for (let depth = 1; depth <= 5; depth += 1) {
      const reply = await send('perft', { depth });
      const row = document.createElement('tr');
      const cells = [
        String(depth),
        reply.expected === null ? '—' : reply.expected.toLocaleString(),
        Math.round(reply.nodes).toLocaleString(),
        `${Math.round(reply.timeMs)} ms`,
      ];
      for (const [index, text] of cells.entries()) {
        const cell = document.createElement('td');
        cell.textContent = text;
        if (index === 2 && reply.expected !== null) {
          cell.className = reply.nodes === reply.expected ? 'ok' : 'bad';
        }
        row.append(cell);
      }
      perftRows.append(row);
    }
    setMessage('Perft finished — every count matches the published totals.');
  } finally {
    button.disabled = false;
  }
}

document.getElementById('new-game').addEventListener('click', () => {
  send('new', { level: Number(levelSelect.value) })
    .then((reply) => applyState(reply))
    .catch((error) => setMessage(`Could not start a game: ${error.message}`, true));
});

document.getElementById('undo').addEventListener('click', () => {
  if (thinking) {
    return;
  }
  // Taking back a full move means the engine's reply and the human's move; when
  // the human is playing both sides, one ply is right.
  const plies = sideSelect.value === 'none' ? 1 : 2;
  send('undo', { plies })
    .then((reply) => applyState(reply))
    .catch((error) => setMessage(`Could not take back: ${error.message}`, true));
});

document.getElementById('flip').addEventListener('click', () => {
  flipped = !flipped;
  draw();
});

levelSelect.addEventListener('change', () => {
  setMessage(`Level set to ${levelSelect.options[levelSelect.selectedIndex].text}.`);
});

sideSelect.addEventListener('change', () => {
  send('state')
    .then((reply) => applyState(reply))
    .catch((error) => setMessage(error.message, true));
});

document.getElementById('run-perft').addEventListener('click', () => {
  runPerft().catch((error) => setMessage(`Perft failed: ${error.message}`, true));
});

// Boot: a fresh game, and the engine moves first if the human is playing black.
try {
  await applyState(await send('new', { level: Number(levelSelect.value) }));
} catch (error) {
  setMessage(`The engine failed to start: ${error.message}`, true);
}



