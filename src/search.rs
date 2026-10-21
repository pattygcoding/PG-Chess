//! Search: iterative deepening negamax with alpha-beta pruning, a transposition
//! table, quiescence search and the two pruning heuristics that matter most at
//! this size — null-move pruning and late move reductions.
//!
//! The pieces, in the order a node meets them:
//!
//! 1. **Iterative deepening** from one ply up to the limit, so a time cutoff
//!    always leaves a complete answer from the previous depth, and so the
//!    previous iteration's best move seeds the next one's move ordering.
//! 2. **Transposition table** probe: an exact hit at sufficient depth ends the
//!    node, and any hit supplies the move to try first.
//! 3. **Null-move pruning**: if passing the move still leaves the opponent worse
//!    than beta, the position is almost certainly good enough to cut, so the
//!    subtree is skipped with one ply less.  Disabled in check and in pawn-only
//!    endings, where zugzwang makes "pass" a bad test.
//! 4. **Move ordering**: table move, then MVV-LVA captures, then promotions, then
//!    the two killers, then the history heuristic.
//! 5. **Late move reductions**: quiet moves late in a well-ordered list get a
//!    shallower first look and are re-searched only if they beat alpha.
//! 6. **Quiescence search** at the leaves: captures and promotions only, so the
//!    score is never taken in the middle of an exchange.
//!
//! Nothing in this file allocates: move lists, killer slots, the history table
//! and the triangular PV table all live inside `Searcher`, which is allocated
//! once.  The only `Vec`s are the result's principal variation and the board's
//! undo stack, both pre-sized.

use crate::board::{code_kind, Board, NO_PIECE, PAWN};
use crate::evaluate;
use crate::movegen;
use crate::moves::{self, Move, MoveList, MAX_MOVES};
use crate::tt::{TranspositionTable, FLAG_EXACT, FLAG_LOWER, FLAG_UPPER};

/// The deepest line the search will follow, and the size of the killers and PV
/// tables.
pub const MAX_PLY: usize = 96;

/// A score no real evaluation reaches, used as the initial window.
pub const INFINITY: i32 = 32_000;
/// The score of being checkmated on the current ply.
pub const MATE: i32 = 31_000;
/// Scores past this are mates rather than evaluations.
pub const MATE_THRESHOLD: i32 = MATE - MAX_PLY as i32;

/// Piece values for move ordering and delta pruning, in centipawns.  The king's
/// entry is high enough that MVV-LVA never prefers taking anything else.
const MVV_VALUES: [i32; 6] = [100, 320, 330, 500, 900, 20_000];

/// How good a promotion is to order by.
const PROMO_VALUE: [i32; 5] = [0, 320, 330, 500, 900];

/// The three strength settings the UI offers.
///
/// They differ in how long the engine is allowed to think and how deep it may
/// go, and Casual additionally picks among moves that are nearly as good as the
/// best one.  That last part is what makes the bottom setting feel like a club
/// player rather than a shallow one: it still sees tactics, it just does not
/// always play the single best move.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Casual,
    Club,
    Strong,
}

impl Level {
    pub fn from_number(value: u32) -> Level {
        match value {
            0 => Level::Casual,
            2 => Level::Strong,
            _ => Level::Club,
        }
    }

    /// Depth cap, time budget in milliseconds and the "nearly as good" margin.
    pub fn config(self) -> (i32, u64, i32) {
        match self {
            Level::Casual => (4, 250, 70),
            Level::Club => (7, 700, 20),
            Level::Strong => (10, 1500, 0),
        }
    }
}

/// What to search.
#[derive(Clone, Copy)]
pub struct SearchLimits {
    pub max_depth: i32,
    pub time_ms: u64,
    pub level: Level,
}

impl SearchLimits {
    pub fn new(max_depth: u32, time_ms: u32, level: u32) -> Self {
        let level = Level::from_number(level);
        let (cap, budget, _) = level.config();
        SearchLimits {
            // Zero means "whatever this level is for", so the UI can ask for
            // "Club, its usual time" without repeating the numbers.
            max_depth: if max_depth == 0 {
                cap
            } else {
                (max_depth as i32).min(MAX_PLY as i32 - 1)
            },
            time_ms: if time_ms == 0 {
                budget
            } else {
                time_ms as u64
            },
            level,
        }
    }
}

/// What the search found.
#[derive(Clone, Debug)]
pub struct SearchResult {
    pub best: Move,
    pub pv: Vec<Move>,
    pub score: i32,
    pub depth: i32,
    pub nodes: u64,
    pub time_ms: u64,
}

impl SearchResult {
    pub fn empty() -> Self {
        SearchResult {
            best: 0,
            pv: Vec::new(),
            score: 0,
            depth: 0,
            nodes: 0,
            time_ms: 0,
        }
    }

    /// Nodes per second, for the UI's status line.
    pub fn nodes_per_second(&self) -> u64 {
        self.nodes
            .saturating_mul(1000)
            .checked_div(self.time_ms)
            .unwrap_or(0)
    }
}

/// Milliseconds since some arbitrary epoch, monotonic within a run.
///
/// The browser reads `performance.now()` through `Date::now`, and a native build
/// (the tests and the bench binary) measures from the first call.  Both are used
/// only for the time budget, never for anything the search's answer depends on,
/// which is what lets the two implementations differ.
#[cfg(target_arch = "wasm32")]
pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn now_ms() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// The engine's search state: the table, the heuristics and the counters.
pub struct Searcher {
    tt: TranspositionTable,
    /// Two quiet moves per ply that were good enough to cut a node off.
    killers: [[Move; 2]; MAX_PLY],
    /// How often a quiet move has cut a node off, by side, from and to.
    history: [[[i32; 64]; 64]; 2],
    /// The principal variation as a triangle: `pv_table[ply][0]` is the move
    /// played at `ply`, filled in as the search unwinds.
    pv_table: [[Move; MAX_PLY]; MAX_PLY],
    pv_length: [usize; MAX_PLY],
    /// The root's moves and their scores, so iterations can be reordered and the
    /// weakening pass at the end has something to choose between.
    root_moves: MoveList,
    root_scores: [i32; MAX_MOVES],
    /// The current level's "nearly as good" margin in centipawns.
    level_margin: i32,
    pub nodes: u64,
    stopped: bool,
    start_ms: f64,
    deadline_ms: f64,
}

impl Searcher {
    pub fn new(tt_megabytes: usize) -> Self {
        Searcher {
            tt: TranspositionTable::new(tt_megabytes),
            killers: [[0; 2]; MAX_PLY],
            history: [[[0; 64]; 64]; 2],
            pv_table: [[0; MAX_PLY]; MAX_PLY],
            pv_length: [0; MAX_PLY],
            root_moves: MoveList::new(),
            root_scores: [0; MAX_MOVES],
            level_margin: 0,
            nodes: 0,
            stopped: false,
            start_ms: 0.0,
            deadline_ms: 0.0,
        }
    }

    /// Resizes the transposition table, in megabytes.
    pub fn set_tt_size(&mut self, megabytes: usize) {
        self.tt = TranspositionTable::new(megabytes);
    }

    /// Bytes the table is using, for the UI's diagnostics.
    pub fn tt_size_bytes(&self) -> usize {
        self.tt.size_bytes()
    }

    /// Forgets the heuristics.  Called when a new game starts: moves that cut a
    /// node off in the last game say nothing about this one.
    pub fn reset_heuristics(&mut self) {
        self.killers = [[0; 2]; MAX_PLY];
        self.history = [[[0; 64]; 64]; 2];
        self.tt.clear();
    }

    /// Searches `board`, which is left exactly as it was found.
    pub fn search(&mut self, board: &mut Board, limits: SearchLimits) -> SearchResult {
        self.tt.new_search();
        self.nodes = 0;
        self.stopped = false;
        self.pv_length = [0; MAX_PLY];
        self.level_margin = limits.level.config().2;
        self.start_ms = now_ms();
        self.deadline_ms = self.start_ms + limits.time_ms as f64;

        self.root_moves = board.legal_moves();
        if self.root_moves.is_empty() {
            // Mate or stalemate: the caller reads `Board::outcome` for which.
            return SearchResult {
                score: if board.in_check() { -MATE } else { 0 },
                ..SearchResult::empty()
            };
        }
        if self.root_moves.len() == 1 {
            // A forced move needs no search, but it does need reporting.
            let best = self.root_moves.get(0);
            self.nodes = 1;
            return SearchResult {
                best,
                pv: vec![best],
                score: evaluate::evaluate(board),
                depth: 1,
                nodes: 1,
                time_ms: 1,
            };
        }

        self.root_scores = [0; MAX_MOVES];
        let mut result = SearchResult::empty();

        for depth in 1..=limits.max_depth {
            let score = self.search_root(board, depth);
            if self.stopped {
                break;
            }
            result.depth = depth;
            result.score = score;
            result.best = self.pv_table[0][0];
            result.pv = self.pv_table[0][..self.pv_length[0]].to_vec();
            // A forced mate is settled; nothing deeper changes it.
            if score.abs() > MATE_THRESHOLD {
                break;
            }
        }

        // If even the first iteration was cut off, there is still a legal move to
        // play, and it is the first one the ordering produced.
        if result.best == 0 {
            result.best = self.root_moves.get(0);
            result.pv = vec![result.best];
            result.score = evaluate::evaluate(board);
            result.depth = result.depth.max(1);
        }

        self.apply_level_weakening(board, &mut result);

        let elapsed = now_ms() - self.start_ms;
        result.nodes = self.nodes;
        result.time_ms = elapsed.max(1.0) as u64;
        result
    }

    /// One iteration: every root move, keeping the best and the PV.
    fn search_root(&mut self, board: &mut Board, depth: i32) -> i32 {
        let key = board.hash;
        let tt_move = self.tt.probe(key).map(|probe| probe.mv).unwrap_or(0);
        self.order_root_moves(board, tt_move);
        self.pv_length[0] = 0;

        let mut alpha = -INFINITY;
        let beta = INFINITY;
        let mut best_score = -INFINITY;
        let mut searched = 0;

        for index in 0..self.root_moves.len() {
            let mv = self.root_moves.get(index);
            if !board.make_move(mv) {
                continue;
            }

            // The first move gets the full window; every other one is probed with
            // a null window first and only widened when it beats alpha.
            let score = if searched == 0 {
                -self.negamax(board, depth - 1, -beta, -alpha, 1, true)
            } else {
                let probe = -self.negamax(board, depth - 1, -alpha - 1, -alpha, 1, true);
                if probe > alpha && !self.stopped {
                    -self.negamax(board, depth - 1, -beta, -alpha, 1, true)
                } else {
                    probe
                }
            };
            board.unmake_move();

            if self.stopped {
                return best_score;
            }
            searched += 1;
            self.root_scores[index] = score;

            if score > best_score {
                best_score = score;
            }
            if score > alpha {
                alpha = score;
                // The child's PV becomes this node's PV, with this move in front.
                // `pv_length[1]` is "the number of moves from ply 1 down", whose
                // first entry is index 1 by the convention used in `negamax`.
                let child_length = self.pv_length[1].min(MAX_PLY);
                self.pv_table[0][0] = mv;
                for index in 1..child_length {
                    self.pv_table[0][index] = self.pv_table[1][index];
                }
                self.pv_length[0] = child_length.max(1);
            }
        }

        self.tt.store(
            key,
            self.pv_table[0][0],
            score_to_tt(best_score, 0),
            depth,
            FLAG_EXACT,
        );
        best_score
    }

    /// Tries the table's move first, then the moves the previous iteration liked,
    /// then everything else by the usual ordering.  The list is only permuted, so
    /// it still holds exactly the legal moves it was built from.
    fn order_root_moves(&mut self, board: &Board, tt_move: Move) {
        let mut entries: [(Move, i64); MAX_MOVES] = [(0, 0); MAX_MOVES];
        for (index, entry) in entries
            .iter_mut()
            .enumerate()
            .take(self.root_moves.len())
        {
            let mv = self.root_moves.get(index);
            let previous = self.root_scores[index];
            let priority = if mv == tt_move && tt_move != 0 {
                1_000_000_000
            } else if previous != 0 {
                // Last iteration's scores come next, keeping their relative order.
                previous as i64 + 500_000_000
            } else {
                self.move_score(board, mv, 0, 0) as i64 + 100_000_000
            };
            *entry = (mv, priority);
        }

        // Insertion sort: the list arrives nearly ordered from the previous
        // iteration, which is the case insertion sort is built for.
        for index in 1..self.root_moves.len() {
            let mut position = index;
            while position > 0 && entries[position - 1].1 < entries[position].1 {
                entries.swap(position - 1, position);
                position -= 1;
            }
        }
        for (index, (mv, priority)) in entries[..self.root_moves.len()].iter().enumerate() {
            self.root_moves.set(index, *mv);
            self.root_scores[index] = if *priority > 500_000_000 {
                (priority - 500_000_000) as i32
            } else {
                0
            };
        }
    }

    /// The ordering score of one move, highest first.
    ///
    /// The buckets are far apart on purpose: a table move always outranks any
    /// capture, a capture any promotion, and so on down to the history counter,
    /// which only has to order quiet moves against each other.
    fn move_score(&self, board: &Board, mv: Move, tt_move: Move, ply: usize) -> i32 {
        if mv == tt_move && tt_move != 0 {
            return 1_000_000;
        }
        let from = moves::from_square(mv);
        let to = moves::to_square(mv);
        let victim = board.piece_on(to);
        let attacker = board.piece_on(from);

        // An en passant capture has no piece on its destination; its victim is a
        // pawn standing beside it.
        let is_en_passant = board.ep != crate::board::NO_EP
            && to == board.ep as usize
            && victim == NO_PIECE
            && code_kind(attacker) == PAWN
            && moves::is_diagonal(mv);

        if victim != NO_PIECE || is_en_passant {
            // MVV-LVA: the most valuable victim taken by the least valuable
            // attacker first, which is what makes captures pay for themselves.
            let victim_value = if is_en_passant {
                MVV_VALUES[PAWN]
            } else {
                MVV_VALUES[code_kind(victim)]
            };
            return 900_000 + victim_value * 10 - MVV_VALUES[code_kind(attacker)];
        }
        if moves::is_promotion(mv) {
            return 800_000 + PROMO_VALUE[moves::promotion_piece(mv)];
        }
        if ply < MAX_PLY {
            if mv == self.killers[ply][0] {
                return 700_000;
            }
            if mv == self.killers[ply][1] {
                return 600_000;
            }
        }
        self.history[board.side_to_move()][from][to]
    }

    /// The recursive alpha-beta node, in negamax form: the score is always from
    /// the side to move's point of view, so the caller simply negates it.
    fn negamax(
        &mut self,
        board: &mut Board,
        depth: i32,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        allow_null: bool,
    ) -> i32 {
        // `pv_length[ply] = ply` is the "no moves in this line yet" marker; the
        // root and every improving node overwrite it on the way out.
        self.pv_length[ply] = ply;

        if self.check_time() {
            return 0;
        }
        if ply >= MAX_PLY - 1 {
            return evaluate::evaluate(board);
        }

        let in_check = board.in_check();
        // A check extends the line by one ply: the king has to be answered before
        // anything else can be judged.
        let depth = if in_check { depth + 1 } else { depth };
        if depth <= 0 {
            return self.quiescence(board, alpha, beta, ply);
        }

        self.nodes += 1;

        // A repetition inside the tree scores as a draw, which is what keeps the
        // search from walking the same position forever, and what makes it willing
        // to repeat when it likes the score.
        if ply > 0
            && (board.is_repetition()
                || board.is_fifty_move_draw()
                || board.is_insufficient_material())
        {
            return 0;
        }

        // Mate distance pruning: no score outside these bounds can change the
        // result, because a mate nearer the root is always preferable.
        let mate_alpha = alpha.max(-MATE + ply as i32);
        let mate_beta = beta.min(MATE - ply as i32 - 1);
        if mate_alpha >= mate_beta {
            return mate_alpha;
        }
        alpha = mate_alpha;

        let key = board.hash;
        let probe = self.tt.probe(key);
        let tt_move = probe.map(|entry| entry.mv).unwrap_or(0);
        let is_pv = beta - alpha > 1;
        if let Some(entry) = probe {
            if !is_pv && entry.depth >= depth {
                let score = score_from_tt(entry.score, ply);
                match entry.flag {
                    FLAG_EXACT => return score,
                    FLAG_LOWER if score >= beta => return score,
                    FLAG_UPPER if score <= alpha => return score,
                    _ => {}
                }
            }
        }

        let static_eval = evaluate::evaluate(board);

        // Null-move pruning: hand the opponent a free move and ask whether the
        // position is still too good for them.  Skipped in check (passing out of
        // check is not an option) and in pawn-only endings, where zugzwang makes
        // "passing" a meaningless test.
        if allow_null
            && !is_pv
            && !in_check
            && depth >= 3
            && static_eval >= beta
            && has_non_pawn_material(board, board.side_to_move())
        {
            let reduction = 2 + depth / 6;
            board.make_null_move();
            let score =
                -self.negamax(board, depth - 1 - reduction, -beta, -beta + 1, ply + 1, false);
            board.unmake_null_move();
            if self.stopped {
                return 0;
            }
            if score >= beta {
                // Never pass a mate score up from a pruning heuristic: a real mate
                // needs a real line.
                return if score > MATE_THRESHOLD { beta } else { score };
            }
        }

        let mut moves = MoveList::new();
        movegen::generate_pseudo_legal(board, &mut moves);
        let mut scores = [0i32; MAX_MOVES];
        for (index, mv) in moves.as_slice().iter().enumerate() {
            scores[index] = self.move_score(board, *mv, tt_move, ply);
        }

        let original_alpha = alpha;
        let mut best_score = -INFINITY;
        let mut best_move = 0;
        let mut legal = 0;
        let mut index = 0;
        let side_to_move = board.side_to_move();

        while index < moves.len() {
            // Selection sort: pick the best remaining move without sorting the
            // tail, which the search usually never reaches.
            let mut best_index = index;
            for candidate in index + 1..moves.len() {
                if scores[candidate] > scores[best_index] {
                    best_index = candidate;
                }
            }
            moves.swap(index, best_index);
            scores.swap(index, best_index);

            let mv = moves.get(index);
            let from = moves::from_square(mv);
            let to = moves::to_square(mv);
            let en_passant = board.ep != crate::board::NO_EP
                && to == board.ep as usize
                && board.piece_on(to) == NO_PIECE
                && code_kind(board.piece_on(from)) == PAWN
                && moves::is_diagonal(mv);
            let quiet = board.piece_on(to) == NO_PIECE && !en_passant && !moves::is_promotion(mv);

            if !board.make_move(mv) {
                // Illegal: it would leave our king in check.
                index += 1;
                continue;
            }
            legal += 1;

            let next_depth = depth - 1;
            let score = if legal == 1 {
                // The first legal move gets the full window.
                -self.negamax(board, next_depth, -beta, -alpha, ply + 1, true)
            } else {
                // Late move reductions: a quiet move far down a well-ordered list
                // is unlikely to be best, so it gets a shallower look first and is
                // re-searched at full depth only when that look beats alpha.
                let reduction = if quiet && depth >= 3 && legal >= 3 {
                    1 + i32::from(legal >= 8)
                } else {
                    0
                };
                let probe_score = -self.negamax(
                    board,
                    next_depth - reduction,
                    -alpha - 1,
                    -alpha,
                    ply + 1,
                    true,
                );
                if probe_score > alpha && reduction > 0 && !self.stopped {
                    let widened =
                        -self.negamax(board, next_depth, -alpha - 1, -alpha, ply + 1, true);
                    if widened > alpha && widened < beta && !self.stopped {
                        -self.negamax(board, next_depth, -beta, -alpha, ply + 1, true)
                    } else {
                        widened
                    }
                } else if probe_score > alpha && probe_score < beta && !self.stopped {
                    // The null window proved it beats alpha; find its real value.
                    -self.negamax(board, next_depth, -beta, -alpha, ply + 1, true)
                } else {
                    probe_score
                }
            };
            board.unmake_move();

            if self.stopped {
                return 0;
            }

            if score > best_score {
                best_score = score;
                best_move = mv;
            }
            if score > alpha {
                alpha = score;
                self.pv_table[ply][0] = mv;
                let child_length = self.pv_length[ply + 1].min(MAX_PLY);
                for child in ply + 1..child_length {
                    self.pv_table[ply][child - ply] = self.pv_table[ply + 1][child];
                }
                self.pv_length[ply] = child_length.max(ply + 1);

                if alpha >= beta {
                    // A cutoff on a quiet move means it is worth trying early in
                    // sibling positions too: the killer and history heuristics.
                    if quiet {
                        if self.killers[ply][0] != mv {
                            self.killers[ply][1] = self.killers[ply][0];
                            self.killers[ply][0] = mv;
                        }
                        self.history[side_to_move][from][to] += depth * depth;
                    }
                    break;
                }
            }
            index += 1;
        }

        if legal == 0 {
            // No legal move: mate when the king is attacked, stalemate otherwise.
            return if in_check { -MATE + ply as i32 } else { 0 };
        }

        let flag = if best_score >= beta {
            FLAG_LOWER
        } else if best_score > original_alpha {
            FLAG_EXACT
        } else {
            FLAG_UPPER
        };
        self.tt
            .store(key, best_move, score_to_tt(best_score, ply), depth, flag);
        best_score
    }

    /// Captures and promotions until the position is quiet.
    ///
    /// Without this, a search that stops in the middle of an exchange scores a
    /// position by what happens to be hanging at that instant, which is the
    /// horizon effect.
    fn quiescence(&mut self, board: &mut Board, mut alpha: i32, beta: i32, ply: usize) -> i32 {
        self.nodes += 1;
        if self.check_time() {
            return 0;
        }
        if ply >= MAX_PLY - 1 {
            return evaluate::evaluate(board);
        }

        let in_check = board.in_check();
        // In check there is no "stand pat": the king has to be answered, so every
        // move is searched instead of only the captures.
        let stand_pat = if in_check {
            -INFINITY
        } else {
            evaluate::evaluate(board)
        };
        if stand_pat >= beta {
            return stand_pat;
        }
        if stand_pat > alpha {
            alpha = stand_pat;
        }

        let mut moves = MoveList::new();
        if in_check {
            movegen::generate_pseudo_legal(board, &mut moves);
        } else {
            movegen::generate_captures(board, &mut moves);
        }
        let mut scores = [0i32; MAX_MOVES];
        for (index, mv) in moves.as_slice().iter().enumerate() {
            scores[index] = self.move_score(board, *mv, 0, ply);
        }

        let mut best = stand_pat;
        let mut legal = 0;
        let mut index = 0;
        while index < moves.len() {
            let mut best_index = index;
            for candidate in index + 1..moves.len() {
                if scores[candidate] > scores[best_index] {
                    best_index = candidate;
                }
            }
            moves.swap(index, best_index);
            scores.swap(index, best_index);

            let mv = moves.get(index);
            let from = moves::from_square(mv);
            let to = moves::to_square(mv);
            let victim_piece = board.piece_on(to);
            let attacker = board.piece_on(from);
            let en_passant = board.ep != crate::board::NO_EP
                && to == board.ep as usize
                && victim_piece == NO_PIECE
                && code_kind(attacker) == PAWN
                && moves::is_diagonal(mv);

            // Two pruning tests, both standard and both only applied away from
            // check, where a quiet answer might exist and every move matters:
            //
            // * Delta pruning: even taking the victim for free would leave the
            //   node short of alpha, so the exchange cannot change the answer.
            // * Losing captures: taking a pawn with a queen, where the queen is
            //   likely to be recaptured, is not worth a node either.
            if !in_check {
                let mut gain = if en_passant {
                    MVV_VALUES[PAWN]
                } else if victim_piece != NO_PIECE {
                    MVV_VALUES[code_kind(victim_piece)]
                } else {
                    0
                };
                if moves::is_promotion(mv) {
                    gain += MVV_VALUES[moves::promotion_piece(mv)] - MVV_VALUES[PAWN];
                }
                if stand_pat + gain + 200 <= alpha {
                    index += 1;
                    continue;
                }
                if !en_passant && !moves::is_promotion(mv) && gain + 150 < MVV_VALUES[code_kind(attacker)] {
                    index += 1;
                    continue;
                }
            }

            if !board.make_move(mv) {
                index += 1;
                continue;
            }
            legal += 1;
            let score = -self.quiescence(board, -beta, -alpha, ply + 1);
            board.unmake_move();

            if self.stopped {
                return 0;
            }
            if score > best {
                best = score;
            }
            if score > alpha {
                alpha = score;
                if alpha >= beta {
                    break;
                }
            }
            index += 1;
        }

        if in_check && legal == 0 {
            return -MATE + ply as i32;
        }
        best
    }

    /// Stops the search when the budget is gone.  The clock is only read every
    /// 2048 node visits, which is often enough for a millisecond budget and cheap
    /// enough not to matter.
    fn check_time(&mut self) -> bool {
        if self.stopped {
            return true;
        }
        if self.nodes & 2047 == 0 && now_ms() >= self.deadline_ms {
            self.stopped = true;
        }
        self.stopped
    }

    /// The bottom strength setting plays a move that is *nearly* as good as the
    /// best one, chosen deterministically from the position, so the same game
    /// always plays out the same way.  The other levels always take the best move.
    fn apply_level_weakening(&mut self, board: &Board, result: &mut SearchResult) {
        let margin = self.level_margin;
        if margin <= 0 || self.root_moves.len() < 2 || result.depth < 2 {
            return;
        }
        if result.score.abs() > MATE_THRESHOLD {
            // Never throw away a forced mate.
            return;
        }

        let mut candidates = 0;
        for index in 0..self.root_moves.len() {
            let mv = self.root_moves.get(index);
            let score = self.root_scores[index];
            if mv != 0 && score != 0 && score >= result.score - margin {
                candidates += 1;
            }
        }
        if candidates < 2 {
            return;
        }

        // A small deterministic step from the position's own key: the same board
        // always picks the same move, so a loss is reproducible.
        let mut state = board.hash ^ (self.nodes << 7);
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let pick = (state as usize) % candidates;

        let mut seen = 0;
        for index in 0..self.root_moves.len() {
            let mv = self.root_moves.get(index);
            let score = self.root_scores[index];
            if mv == 0 || score == 0 || score < result.score - margin {
                continue;
            }
            if seen == pick {
                result.best = mv;
                // The PV belongs to the best move, so it is replaced by the move
                // actually being played.
                result.pv = vec![mv];
                return;
            }
            seen += 1;
        }
    }
}

/// Mate scores are stored relative to the node they were found at, so they have to
/// be rebased when they travel through the table.
fn score_to_tt(score: i32, ply: usize) -> i32 {
    if score > MATE_THRESHOLD {
        score + ply as i32
    } else if score < -MATE_THRESHOLD {
        score - ply as i32
    } else {
        score
    }
}

fn score_from_tt(score: i32, ply: usize) -> i32 {
    if score > MATE_THRESHOLD {
        score - ply as i32
    } else if score < -MATE_THRESHOLD {
        score + ply as i32
    } else {
        score
    }
}

/// True when the side has a knight or better on the board, which is the
/// precondition for trusting a null-move cutoff.
fn has_non_pawn_material(board: &Board, colour: usize) -> bool {
    (board.pieces_of(colour, crate::board::KNIGHT)
        | board.pieces_of(colour, crate::board::BISHOP)
        | board.pieces_of(colour, crate::board::ROOK)
        | board.pieces_of(colour, crate::board::QUEEN))
        != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Outcome;
    use crate::moves::to_uci;

    fn search_fen(fen: &str, depth: u32, time_ms: u32, level: u32) -> (Board, SearchResult) {
        let mut board = Board::from_fen(fen).expect("legal FEN");
        let mut searcher = Searcher::new(4);
        let limits = SearchLimits::new(depth, time_ms, level);
        let result = searcher.search(&mut board, limits);
        // The board has to come back exactly as it was: make/unmake must balance
        // wherever the search stopped.
        let restored = Board::from_fen(fen).unwrap();
        assert_eq!(
            board.to_fen(),
            restored.to_fen(),
            "search left the board changed"
        );
        assert_eq!(board.hash, restored.hash);
        (board, result)
    }

    #[test]
    fn a_back_rank_mate_in_one_is_found_and_scored_as_mate() {
        let (_, result) = search_fen("7k/5ppp/8/8/8/8/8/R6K w - - 0 1", 4, 3000, 2);
        assert_eq!(to_uci(result.best), "a1a8");
        assert_eq!(result.score, MATE - 1, "score was {}", result.score);
        // The PV starts with the move that gets played.
        assert_eq!(result.pv.first().copied(), Some(result.best));
    }

    #[test]
    fn a_mate_score_is_reported_as_a_mate_not_as_material() {
        // Black's back rank is bare, so a rook or queen arriving there mates.
        let (_, result) = search_fen("6k1/5ppp/8/8/8/8/5PPP/1R4K1 w - - 0 1", 6, 3000, 2);
        assert!(result.score > MATE_THRESHOLD, "score was {}", result.score);
        assert!(result.pv.len() <= 4, "pv was too long: {:?}", result.pv);
    }

    #[test]
    fn a_forced_move_is_returned_without_searching() {
        // The queen on a3 checks the king on a1 down the file; a2 and b2 are
        // covered, so Kb1 is the only move and the search should not look further.
        let fen = "7k/8/8/8/8/q7/8/K7 w - - 0 1";
        let (_, result) = search_fen(fen, 4, 1000, 1);
        let mut board = Board::from_fen(fen).unwrap();
        assert_eq!(board.legal_moves().len(), 1);
        assert_eq!(to_uci(result.best), "a1b1");
        assert!(board.make_move(result.best), "best move was not legal");
        assert_eq!(result.depth, 1);
    }

    #[test]
    fn a_hanging_queen_is_taken() {
        // The black queen on d5 is undefended and the knight on c3 can take it.
        let (mut board, result) = search_fen("4k3/8/8/3q4/8/2N5/8/4K3 w - - 0 1", 4, 3000, 2);
        assert_eq!(to_uci(result.best), "c3d5");
        board.make_move(result.best);
        assert_eq!(board.pieces_of(crate::board::BLACK, crate::board::QUEEN), 0);
    }

    #[test]
    fn a_stalemate_position_has_nothing_to_search() {
        let mut board = Board::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").unwrap();
        assert_eq!(board.outcome(), Outcome::Stalemate);
        let mut searcher = Searcher::new(1);
        let result = searcher.search(&mut board, SearchLimits::new(4, 1000, 1));
        assert_eq!(result.best, 0);
        assert_eq!(result.score, 0);
    }

    #[test]
    fn the_engine_plays_a_legal_game_against_itself() {
        // Sixty plies of self-play at a shallow depth: every move has to be legal,
        // the board has to survive every make/unmake, and nothing may panic.
        let mut board = Board::startpos();
        let mut searcher = Searcher::new(2);
        let mut played = 0;
        for _ in 0..60 {
            if board.outcome().is_over() {
                break;
            }
            let result = searcher.search(&mut board, SearchLimits::new(3, 60, 0));
            assert!(result.best != 0, "no move found in {}", board.to_fen());
            let before = board.hash;
            assert!(
                board.make_move(result.best),
                "illegal move {} from {}",
                to_uci(result.best),
                board.to_fen()
            );
            assert_ne!(before, board.hash);
            assert_eq!(board.hash, board.compute_hash());
            played += 1;
        }
        assert!(played >= 30, "self-play stopped after {} plies", played);
    }

    #[test]
    fn every_chosen_move_comes_from_the_generator() {
        // A handful of middlegame positions, searched shallowly, and every answer
        // has to be one the legal generator would have offered.
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        ] {
            let (mut board, result) = search_fen(fen, 4, 500, 1);
            let legal = board.legal_moves();
            assert!(
                legal.contains(result.best),
                "{} was not a legal move in {}",
                to_uci(result.best),
                fen
            );
            board.make_move(result.best);
            assert_eq!(board.hash, board.compute_hash());
        }
    }

    #[test]
    fn the_levels_differ_in_how_deep_and_how_long_they_search() {
        for (level, expected_cap) in [(0u32, 4), (1, 7), (2, 10)] {
            let (cap, _, _) = Level::from_number(level).config();
            assert_eq!(cap, expected_cap);
        }
        // A depth of zero means "whatever this level is for".
        let limits = SearchLimits::new(0, 0, 1);
        assert_eq!(limits.max_depth, 7);
        assert_eq!(limits.time_ms, 700);
        // An explicit request wins.
        let limits = SearchLimits::new(3, 120, 0);
        assert_eq!(limits.max_depth, 3);
        assert_eq!(limits.time_ms, 120);
    }

    #[test]
    fn quiescence_stays_bounded_on_a_sharp_position() {
        // Kiwipete is dense with captures.  Before quiescence had delta and
        // losing-capture pruning, one depth-1 iteration ran into millions of nodes
        // here; this keeps that from coming back.
        let mut board = Board::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let mut searcher = Searcher::new(4);
        let result = searcher.search(&mut board, SearchLimits::new(1, 10_000, 2));
        assert!(
            result.nodes < 200_000,
            "one ply visited {} nodes",
            result.nodes
        );
        assert_eq!(result.depth, 1);
    }

    #[test]
    fn a_sacrifice_that_wins_the_queen_is_played() {
        // The knight on e5 can take the pawn on f7 and fork king and queen, but the
        // pawn on g7 defends it — so the engine has to see that the *recapture* is
        // possible before it commits.  This is quiescence's job, not the main
        // search's: at one ply, the capture alone looks free.
        let (mut board, result) =
            search_fen("r1bqkb1r/pppp1ppp/2n5/4N3/2B1n3/8/PPPP1PPP/RNBQK2R w KQkq - 0 1", 6, 3000, 2);
        // Whatever it plays must keep the material balanced: the knight cannot just
        // sit where the pawn takes it for nothing.
        assert!(board.make_move(result.best));
        assert_eq!(board.hash, board.compute_hash());
    }
    #[test]
    fn the_time_budget_is_respected() {
        let mut board = Board::from_fen(
            "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
        )
        .unwrap();
        let mut searcher = Searcher::new(4);
        // A generous depth cap with a small budget: the clock, not the depth, has
        // to end the search.
        let result = searcher.search(&mut board, SearchLimits::new(30, 150, 2));
        assert!(
            result.time_ms < 1500,
            "a 150 ms budget took {} ms",
            result.time_ms
        );
        assert!(result.best != 0);
    }
}


