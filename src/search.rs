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
use crate::see;
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

/// The exchange value a capture has to reach to be searched in quiescence.
const SEE_PRUNE_THRESHOLD: i32 = 0;

/// Aspiration window widths, tried in order around the previous iteration's score.
/// A zero means "the full window", which is the last resort and cannot fail, so a
/// score that moves further than the widest guess is simply searched normally.
const ASPIRATION_WINDOWS: [i32; 4] = [25, 100, 400, 0];

/// Futility: how much a quiet move may be worth, per ply of remaining depth, before
/// it is not worth searching at all.
const FUTILITY_MARGIN: i32 = 100;

/// Reverse futility: how far above beta the static score must be, per ply, before
/// the node is cut without searching a single move.
const REVERSE_FUTILITY_MARGIN: i32 = 90;

/// Late move pruning: how many quiet moves are worth searching at a low depth.
/// Indexed by depth, so at depth two the twenty-first quiet move onwards is skipped.
///
/// Deliberately generous.  An earlier version cut at five moves (depth 1) and ten
/// (depth 2) and cost the engine 144 Elo at equal depth in the matches: a search that
/// is three plies deep is not yet good enough at ordering to be sure the twentieth
/// quiet move is worthless.
const LATE_MOVE_PRUNING: [usize; 4] = [0, 8, 20, 40];

/// Iterative deepening only seeds the ordering from this depth on, where a blind
/// first move actually costs something.
const IID_MINIMUM_DEPTH: i32 = 5;

/// ProbCut: how far above beta a shallow search of a capture has to come back before the
/// node is cut, and how many plies shallower that search is.  The margin is what keeps
/// it honest — the shallow search is a weaker player, so it is only believed when it
/// wins by more than its own error.
const PROBCUT_MARGIN: i32 = 200;
const PROBCUT_DEPTH: i32 = 4;

/// An iteration is only started while the budget can absorb this share of it, and the
/// next iteration is assumed to cost this multiple of the last one.  See the loop in
/// `search` for why a guess beats a fixed share of the clock.
///
/// The multiple is an *estimate*, not a worst case: an iteration that runs out of time
/// changes nothing about the answer, because the search returns the last depth that
/// finished, so the whole cost of starting one that does not finish is the wall-clock
/// time the caller was willing to spend anyway.  Being pessimistic here throws depth
/// away for nothing.  The bench shows an iteration costing between 1.3 and 2.4 times
/// the one before it depending on the position, so 2.0 was leaving a third of the
/// budget unused; 1.4 with a 5% margin spends it.
const ITERATION_BUDGET_SHARE: f64 = 0.95;
const ITERATION_GROWTH: f64 = 1.4;

/// Which of the search's strength techniques are switched on.
///
/// Every one of these is standard, and every one of them trades nodes for work: the
/// exchange walks cost time at each capture, the reductions risk missing a move that
/// only looks quiet.  They are flags rather than plain code because the match binary
/// plays the engine against itself with them on and off, which is the only way to say
/// whether they made it stronger rather than merely cleverer.
///
/// `all()` is the configuration that ships, and it is *not* "every flag true": each
/// flag below was measured on its own with `match --old full --without <flag>` (or
/// `--with <flag>` for one that ships off), 120 games at equal depth and 160 at equal
/// time, and the ones that measured neutral or worse are off by default even though
/// the code stays behind the flag.
///
/// | flag | equal depth | equal time |
/// | --- | --- | --- |
/// | `see_pruning` | +137 | |
/// | `aspiration` | +134 | |
/// | `king_safety` | +114 | |
/// | `reverse_futility` | +67 | |
/// | `passed_pawn_king_proximity` | +67 | |
/// | `mobility` (safe squares only) | +22 | -9 |
/// | `lmr_refinements` | -22 | +29 |
/// | `log_reductions` | +23 | +9 |
/// | `futility` | 0 | -15 |
/// | `soft_time_limit` | n/a | 0 |
/// | `probcut` | | +31, +2, -19 |
/// | `threats` | 0 | +3 |
/// | `counter_moves` | -2 | -37 |
/// | `see_ordering` | -44 | +17 |
/// | `internal_iterative_deepening` | 0 | -41 |
/// | `late_move_pruning` | -144, -89 milder | -7 |
/// | `king_storm` | +44 | -46 with `drawish_endings` |
/// | `drawish_endings` | 0 | " |
/// | `connected_pawns` | 0 | |
///
/// The error bars are around ±20, so the top five are wins, `futility` and the soft
/// time limit are free (they cost nothing measurable and save nodes or waiting), and
/// the last nine earn nothing: they are the interesting negative results.
///
/// The three positional terms in round four are the odd ones out, and they are what
/// taught this project the most.  `king_storm` measured **+44 Elo at equal depth** and
/// the pair of it and `drawish_endings` measured **−46 at equal time**, and that is not
/// a contradiction: a term that adds to the score also makes the score a worse
/// predictor of the search's own answer, and every pruning decision here — reverse
/// futility, futility, ProbCut, delta pruning in quiescence — leans on that prediction.
/// Strength per ply and strength per second are different questions, and a term has to
/// answer the second one to ship.
///
/// Rows measured in different configurations are not additive, and the last four were
/// measured against a configuration that differed from the one that ships.  Re-measured
/// in the final configuration, safe mobility and the check exception in the reductions
/// are each worth around ±20 Elo — small enough that a 240-game sample can flip a sign
/// — which is why the number the README quotes for them is the *pair*: +42 Elo at equal
/// time over 300 games against the previous release.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchFeatures {
    /// Quiescence skips captures that lose material even after the recaptures.
    pub see_pruning: bool,
    /// Captures are ordered by exchange value rather than by piece values.  Off: the
    /// walk costs time at every node and the ordering it produces measured worse than
    /// the simpler MVV-LVA one.
    pub see_ordering: bool,
    /// Each iteration is searched inside a window around the previous score.
    pub aspiration: bool,
    /// A node whose static score is far above beta is cut without a search.
    pub reverse_futility: bool,
    /// Quiet moves that cannot raise alpha at a low depth are skipped.
    pub futility: bool,
    /// The tail of the move list is skipped at a low depth.  Off: at equal depth it
    /// cost the engine 89 Elo even with a generous cut-off, which is the clearest
    /// negative result of the set.
    pub late_move_pruning: bool,
    /// A node with no move to try first searches itself shallowly to find one.  Off:
    /// it measured 41 Elo *worse* at equal time, because the table already supplies
    /// most of the ordering it is there to provide.
    pub internal_iterative_deepening: bool,
    /// Reductions follow the log-of-depth-times-log-of-move curve.
    pub log_reductions: bool,
    /// The move that refuted the opponent's last move is tried early.
    pub counter_moves: bool,
    /// Reductions are adjusted: a move that gives check is not reduced at all.
    pub lmr_refinements: bool,
    /// A capture that wins enough material lets the node be cut with a shallow search
    /// of that capture alone, instead of a full search of every move.
    pub probcut: bool,
    /// Iterations that cannot finish are not started.
    pub soft_time_limit: bool,
    /// Which evaluation terms the search asks for.
    pub eval: evaluate::EvalFeatures,
}

impl SearchFeatures {
    pub const fn all() -> Self {
        SearchFeatures {
            see_pruning: true,
            see_ordering: false,
            aspiration: true,
            reverse_futility: true,
            futility: true,
            late_move_pruning: false,
            internal_iterative_deepening: false,
            log_reductions: true,
            // Off: keeping this table measured 2 Elo worse at equal depth and 37 worse
            // at equal time over 200 games, so it is not a return for the work of
            // maintaining it.  The likely reason is that the killers and the history
            // counter already cover the case it is for.
            counter_moves: false,
            lmr_refinements: true,
            // Off: the node count it saves is real, but the score is not, and two
            // equal-time measurements over 200 games each put it at +2 and −19 — one on
            // each side of zero.  Off like the rest of the neutral set, and `match --old
            // full --with probcut` re-tests it without rewriting it.
            probcut: false,
            soft_time_limit: true,
            eval: evaluate::EvalFeatures::all(),
        }
    }

    /// The engine as it was before any of the above: the half of the match that the
    /// other half has to beat.
    pub const fn baseline() -> Self {
        SearchFeatures {
            see_pruning: false,
            see_ordering: false,
            aspiration: false,
            reverse_futility: false,
            futility: false,
            late_move_pruning: false,
            internal_iterative_deepening: false,
            log_reductions: false,
            counter_moves: false,
            lmr_refinements: false,
            probcut: false,
            soft_time_limit: false,
            eval: evaluate::EvalFeatures::none(),
        }
    }
}

impl Default for SearchFeatures {
    fn default() -> Self {
        SearchFeatures::all()
    }
}


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
            // The depth cap is a backstop against a runaway node count, not a target:
            // the clock is what stops the search.  These caps used to be 5, 10 and 14,
            // which the engine now reaches in a fraction of its budget (depth ten in
            // 211 ms of a 700 ms move), so the cap, not the clock, was choosing how
            // deep it looked.  The bench shows the depth each level now reaches and the
            // milliseconds it took to get there.
            Level::Casual => (12, 250, 70),
            Level::Club => (16, 700, 20),
            Level::Strong => (20, 1500, 0),
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

/// Late move reductions by remaining depth and moves searched: the usual
/// `0.77 + ln(depth) * ln(move) / 2.36`, floored, and never as deep as the search
/// itself.  The two logarithms are what give the curve its shape — a fast rise for
/// the first few moves, then almost flat — and building it once keeps floating point
/// out of the hot loop.
fn build_reductions() -> [[i32; 64]; 64] {
    let mut table = [[0i32; 64]; 64];
    for (depth, row) in table.iter_mut().enumerate().skip(1) {
        for (index, cell) in row.iter_mut().enumerate().skip(1) {
            let value = 0.77 + (depth as f64).ln() * (index as f64).ln() / 2.36;
            *cell = (value as i32).clamp(0, depth as i32 - 1);
        }
    }
    table
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
    /// The move that refuted a move reaching (from, to): `counter_moves[from][to]` is
    /// the quiet move that cut the node off last time the opponent played (from, to).
    ///
    /// Keyed by the squares of the move being refuted rather than by the piece that
    /// made it, because the piece is not knowable here: by the time a node sees the
    /// opponent's move, its capture may have already replaced the piece on the square
    /// it landed on.  The squares are exact.
    counter_moves: [[Move; 64]; 64],
    /// The principal variation as a triangle: `pv_table[ply][0]` is the move
    /// played at `ply`, filled in as the search unwinds.
    pv_table: [[Move; MAX_PLY]; MAX_PLY],
    pv_length: [usize; MAX_PLY],
    /// The root's moves and their scores, so iterations can be reordered and the
    /// weakening pass at the end has something to choose between.
    root_moves: MoveList,
    root_scores: [i32; MAX_MOVES],
    /// `root_scores` as of the last iteration that *finished*, together with the move
    /// order it was written against.  An iteration that runs out of time is abandoned
    /// mid-way, so its scores are a mixture of the values it published and the ones it
    /// never reached, and the next iteration re-sorts the list under them; the weakening
    /// pass has to choose between moves on the last *complete* set, in the order that
    /// set was ranked in.
    completed_moves: MoveList,
    completed_root_scores: [i32; MAX_MOVES],
    /// The current level's "nearly as good" margin in centipawns.
    level_margin: i32,
    /// Which strength techniques are switched on.  Everything the search does
    /// differently from the version before it lives behind one of these flags.
    features: SearchFeatures,
    /// Late move reductions by remaining depth and moves searched, from the usual
    /// log-of-both curve.  Built once, since the curve needs floating point.
    reductions: [[i32; 64]; 64],
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
            counter_moves: [[0; 64]; 64],
            pv_table: [[0; MAX_PLY]; MAX_PLY],
            pv_length: [0; MAX_PLY],
            root_moves: MoveList::new(),
            root_scores: [0; MAX_MOVES],
            completed_moves: MoveList::new(),
            completed_root_scores: [0; MAX_MOVES],
            level_margin: 0,
            features: SearchFeatures::all(),
            reductions: build_reductions(),
            nodes: 0,
            stopped: false,
            start_ms: 0.0,
            deadline_ms: 0.0,
        }
    }

    /// Chooses which techniques the search uses.  The default is everything on;
    /// the matches ask for `SearchFeatures::baseline()` on one side.
    pub fn set_features(&mut self, features: SearchFeatures) {
        self.features = features;
    }

    pub fn features(&self) -> SearchFeatures {
        self.features
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
        self.counter_moves = [[0; 64]; 64];
        self.completed_moves = MoveList::new();
        self.completed_root_scores = [0; MAX_MOVES];
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
                score: evaluate::evaluate_with(board, self.features.eval),
                depth: 1,
                nodes: 1,
                time_ms: 1,
            };
        }

        self.root_scores = [0; MAX_MOVES];
        let mut result = SearchResult::empty();
        let mut last_iteration_ms = 0.0;
        // Nothing has finished yet in *this* search.  The snapshot has to start empty:
        // an abandoned iteration would otherwise leave the weakening pass choosing
        // between moves from the previous position, which are not even legal here.
        self.completed_moves = MoveList::new();
        self.completed_root_scores = [0; MAX_MOVES];

        for depth in 1..=limits.max_depth {
            // Whether to start another iteration at all is a bet on it finishing: a
            // cut-off iteration changes nothing about the answer, so its time is spent
            // for nothing.  The guess is the previous iteration's duration doubled,
            // because a search that just finished depth d in t milliseconds is roughly
            // as likely to spend 2t on depth d+1 — a better guess than a fixed share of
            // the clock, which either wastes the tail of the budget or gives up on an
            // iteration that would have fit.
            if self.features.soft_time_limit && depth > 1 {
                let elapsed = now_ms() - self.start_ms;
                let expected = (last_iteration_ms * ITERATION_GROWTH).max(1.0);
                if elapsed + expected > limits.time_ms as f64 * ITERATION_BUDGET_SHARE {
                    break;
                }
            }
            let iteration_started = now_ms();
            let score = self.search_root(board, depth, result.score);
            last_iteration_ms = now_ms() - iteration_started;
            if self.stopped {
                break;
            }
            result.depth = depth;
            result.score = score;
            result.best = self.pv_table[0][0];
            result.pv = self.pv_table[0][..self.pv_length[0]].to_vec();
            // The iteration finished, so its scores are the ones the level is allowed to
            // choose between: snapshot them with the order they were ranked in before
            // the next iteration re-sorts the list under them.
            self.completed_moves = self.root_moves.clone();
            self.completed_root_scores = self.root_scores;
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
            result.score = evaluate::evaluate_with(board, self.features.eval);
            result.depth = result.depth.max(1);
        }

        self.apply_level_weakening(board, &mut result);

        let elapsed = now_ms() - self.start_ms;
        result.nodes = self.nodes;
        result.time_ms = elapsed.max(1.0) as u64;
        result
    }

    /// One iteration, with the aspiration window wrapped around it.
    ///
    /// Most iterations land within a pawn of the previous one, and a narrow window
    /// prunes harder than a wide one, so the iteration is tried inside
    /// `previous ± 25` first and only widened when that guess turns out to be wrong.
    /// A wrong guess costs the iteration, not the answer: the iteration is re-searched
    /// inside the next width, and the last width is the full window, which cannot
    /// fail.
    fn search_root(&mut self, board: &mut Board, depth: i32, previous: i32) -> i32 {
        let mut score = previous;
        for window in ASPIRATION_WINDOWS {
            // The first iterations are cheap enough to search exactly, and a mate
            // score is not a score a window around it means anything for.  A level
            // that weakens its choice also searches the full window, because it has to
            // score every move near the top exactly to pick between them.
            let narrow = self.features.aspiration
                && depth > 2
                && window > 0
                && previous.abs() < MATE_THRESHOLD
                && self.level_margin == 0;
            let (alpha, beta) = if narrow {
                (previous.saturating_sub(window), previous.saturating_add(window))
            } else {
                (-INFINITY, INFINITY)
            };

            let (value, failed_low, failed_high) =
                self.search_root_window(board, depth, alpha, beta);
            score = value;
            if self.stopped || (!failed_low && !failed_high) {
                return score;
            }
        }
        score
    }

    /// Every root move for one depth, inside an explicit window.
    ///
    /// Returns the score, and whether it landed outside the window on either side,
    /// which is what the caller needs to know to widen it and try again.
    fn search_root_window(
        &mut self,
        board: &mut Board,
        depth: i32,
        alpha: i32,
        beta: i32,
    ) -> (i32, bool, bool) {
        let key = board.hash;
        let tt_move = self.tt.probe(key).map(|probe| probe.mv).unwrap_or(0);
        self.order_root_moves(board, tt_move);
        self.pv_length[0] = 0;

        let mut alpha = alpha;
        let alpha_floor = alpha;
        let mut best_score = -INFINITY;
        let mut searched = 0;

        for index in 0..self.root_moves.len() {
            let mv = self.root_moves.get(index);
            if !board.make_move(mv) {
                continue;
            }

            // The first move gets the full window; every other one is probed with
            // a null window first and only widened when it beats alpha.
            // The bar a move has to clear to be worth searching exactly.  It is alpha —
            // the best score so far — except at a level that weakens its choice, where
            // it is the margin *below* the best.  A score from a probe at alpha is only
            // an upper bound, so probing there makes every move that fails look as
            // though it were one centipawn from the best one, which is how a "within
            // twenty-five centipawns" level ends up playing moves twenty-five
            // centipawns *and a queen* worse.
            let bar = if self.level_margin > 0 {
                (best_score - self.level_margin).max(-INFINITY + 1)
            } else {
                alpha
            };
            let score = if searched == 0 {
                -self.negamax(board, depth - 1, -beta, -alpha, 1, true)
            } else {
                let probe = -self.negamax(board, depth - 1, -bar - 1, -bar, 1, true);
                if probe > bar && !self.stopped {
                    // Close enough to matter: its exact value decides the choice.
                    -self.negamax(board, depth - 1, -beta, -bar, 1, true)
                } else {
                    probe
                }
            };
            board.unmake_move();

            if self.stopped {
                return (best_score, false, false);
            }
            searched += 1;
            self.root_scores[index] = score;

            if score > best_score {
                best_score = score;
            }
            if score > alpha {
                alpha = score;
                // The child's PV becomes this node's PV, with this move in front.
                self.pv_table[0][0] = mv;
                let child_moves = self.pv_length[1].min(MAX_PLY - 1);
                for step in 0..child_moves {
                    self.pv_table[0][step + 1] = self.pv_table[1][step];
                }
                self.pv_length[0] = child_moves + 1;
            }
        }

        // The flag has to describe the window that was actually searched: a score
        // from a failed probe is a bound, not an exact value, and storing it as
        // exact would poison the table for every later visit.
        let flag = if best_score >= beta {
            FLAG_LOWER
        } else if best_score > alpha_floor {
            FLAG_EXACT
        } else {
            FLAG_UPPER
        };
        self.tt.store(
            key,
            self.pv_table[0][0],
            score_to_tt(best_score, 0),
            depth,
            flag,
        );
        (best_score, best_score <= alpha_floor, best_score >= beta)
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
                self.move_score(board, mv, 0, 0, self.features.see_ordering) as i64 + 100_000_000
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
    /// which only has to order quiet moves against each other.  `by_exchange` asks
    /// for the exchange value of captures, which is a walk and is therefore not worth
    /// paying for inside quiescence, where the losing ones have already been pruned.
    fn move_score(
        &self,
        board: &Board,
        mv: Move,
        tt_move: Move,
        ply: usize,
        by_exchange: bool,
    ) -> i32 {
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
            let victim_value = if is_en_passant {
                MVV_VALUES[PAWN]
            } else {
                MVV_VALUES[code_kind(victim)]
            };
            if by_exchange && !is_en_passant && !moves::is_promotion(mv) {
                // MVV-LVA guesses what a capture is worth from the two pieces on the
                // square it starts on; the exchange value walks the whole sequence
                // and is the better guess.  Both parts are clamped into the capture
                // band, so every capture still outranks every promotion and quiet
                // move, which is the ordering the rest of the search expects.
                let exchange = see::see_value(board, mv).clamp(-1_000, 1_000);
                let material = (victim_value - MVV_VALUES[code_kind(attacker)]).clamp(-100, 100);
                return 900_000 + exchange * 10 + material;
            }
            // MVV-LVA: the most valuable victim taken by the least valuable
            // attacker first, which is what makes captures pay for themselves.
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
        // The move that refuted the opponent's last one is a better guess than the
        // history counter, because it is about this position rather than this ply.
        if self.features.counter_moves {
            if let Some(previous) = board.last_move() {
                let refutation =
                    self.counter_moves[moves::from_square(previous)][moves::to_square(previous)];
                if refutation != 0 && refutation == mv {
                    return 550_000;
                }
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
        // `pv_length[ply] = 0` is the "no moves in this line yet" marker; the root and
        // every node that improves alpha overwrite it on the way out.
        self.pv_length[ply] = 0;

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
        let mut tt_move = probe.map(|entry| entry.mv).unwrap_or(0);
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

        let static_eval = evaluate::evaluate_with(board, self.features.eval);

        // Internal iterative deepening: with no move to try first, every move is
        // ordered blind, and a shallow search is the cheapest way to find out which
        // one deserves the full depth.  Its score is thrown away; only the move
        // matters, and it lands in the table where the ordering below reads it.
        if self.features.internal_iterative_deepening
            && tt_move == 0
            && is_pv
            && !in_check
            && depth >= IID_MINIMUM_DEPTH
        {
            self.negamax(board, depth / 2, alpha, beta, ply, false);
            if self.stopped {
                return 0;
            }
            tt_move = self.tt.probe(key).map(|entry| entry.mv).unwrap_or(0);
        }

        // Reverse futility: the static score is so far above beta that the opponent
        // would have to be handed two free moves to bring the node back into range.
        // Shallow depths only, and never in a pawn ending, where "passing" is exactly
        // the move that loses.
        if self.features.reverse_futility
            && !is_pv
            && !in_check
            && depth <= 5
            && static_eval < MATE_THRESHOLD
            && static_eval - REVERSE_FUTILITY_MARGIN * depth >= beta
            && has_non_pawn_material(board, board.side_to_move())
        {
            return static_eval;
        }

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

        // ProbCut: some captures settle a node on their own.  A capture that wins
        // material even after the recaptures — the exchange evaluator says so — is
        // searched at a reduced depth with the window raised by a margin, and if it
        // still comes back above that, the node is cut and the result stored as a lower
        // bound so later visits to this position skip the work.  The margin is what
        // makes it sound: a shallow search is a weaker player, so it is only believed
        // when it wins by more than its own error.
        if self.features.probcut
            && !is_pv
            && !in_check
            && depth >= PROBCUT_DEPTH + 1
            && beta.abs() < MATE_THRESHOLD
        {
            let prob_beta = beta + PROBCUT_MARGIN;
            let shallow = depth - PROBCUT_DEPTH;
            let mut captures = MoveList::new();
            movegen::generate_captures(board, &mut captures);
            for index in 0..captures.len() {
                let mv = captures.get(index);
                // Only a capture that wins material outright is worth the shallow
                // search; one that loses it cannot cut, whatever the shallow search
                // says.
                if !see::see_ge(board, mv, prob_beta - static_eval) {
                    continue;
                }
                if !board.make_move(mv) {
                    continue;
                }
                let score =
                    -self.negamax(board, shallow, -prob_beta, -prob_beta + 1, ply + 1, false);
                board.unmake_move();
                if self.stopped {
                    return 0;
                }
                if score >= prob_beta {
                    self.tt.store(
                        key,
                        mv,
                        score_to_tt(score, ply),
                        shallow,
                        FLAG_LOWER,
                    );
                    return score;
                }
            }
        }

        let mut moves = MoveList::new();
        movegen::generate_pseudo_legal(board, &mut moves);
        let mut scores = [0i32; MAX_MOVES];
        for (index, mv) in moves.as_slice().iter().enumerate() {
            scores[index] = self.move_score(board, *mv, tt_move, ply, self.features.see_ordering);
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

            // Two pruning rules for quiet moves, both only at low depth, only away
            // from check, and never at a PV node, where any move might be the one
            // that matters:
            //
            // * Late move pruning: the list is ordered, so a quiet move this far
            //   down it is not going to be the best one at depth three.
            // * Futility: the position is already so far below alpha that a quiet
            //   move worth less than the margin cannot bring it back.
            if self.features.late_move_pruning
                && !is_pv
                && !in_check
                && quiet
                && legal > 0
                && depth <= 3
                && legal >= LATE_MOVE_PRUNING[depth as usize]
            {
                index += 1;
                continue;
            }
            if self.features.futility
                && !is_pv
                && !in_check
                && quiet
                && legal > 0
                && depth <= 3
                && static_eval + FUTILITY_MARGIN * depth <= alpha
            {
                index += 1;
                continue;
            }

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
                    let mut base = if self.features.log_reductions {
                        self.reductions[(depth.min(63)) as usize][legal.min(63)]
                    } else {
                        1 + i32::from(legal >= 8)
                    };
                    if self.features.lmr_refinements && board.in_check() {
                        // A move that gives check is not a late move: the opponent has
                        // to answer it, and the search is about to extend the child by a
                        // ply for exactly that reason, so reducing here would cancel the
                        // extension out.  (An earlier version also reduced one ply more
                        // off the principal variation, and that measured 42 Elo *worse*
                        // at equal depth: the log curve is already steep enough that
                        // adding to it prunes real moves.)
                        base = 0;
                    }
                    base
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
                // The child's line, one move longer with this move in front.  The
                // count is what matters: the child's row always starts at index 0, and
                // a child that returned through a table hit has no line to copy, so
                // the length decides how much of the row is real.
                let child_moves = self.pv_length[ply + 1].min(MAX_PLY - ply - 1);
                for step in 0..child_moves {
                    self.pv_table[ply][step + 1] = self.pv_table[ply + 1][step];
                }
                self.pv_length[ply] = child_moves + 1;

                if alpha >= beta {
                    // A cutoff on a quiet move means it is worth trying early in
                    // sibling positions too: the killer and history heuristics.
                    if quiet {
                        if self.killers[ply][0] != mv {
                            self.killers[ply][1] = self.killers[ply][0];
                            self.killers[ply][0] = mv;
                        }
                        self.history[side_to_move][from][to] += depth * depth;
                        // The same move is often the answer to the opponent's last one
                        // wherever it is played, so remember it against those squares.
                        // The board is back to this node's position here, which is what
                        // makes `last_move` the opponent's move.
                        if self.features.counter_moves {
                            if let Some(previous) = board.last_move() {
                                self.counter_moves[moves::from_square(previous)]
                                    [moves::to_square(previous)] = mv;
                            }
                        }
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
            scores[index] = self.move_score(board, *mv, 0, ply, false);
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
            // * Losing captures: a capture that the exchange says loses material is
            //   not worth a node, whatever the two piece values suggest.  (Without
            //   the exchange evaluator this falls back to comparing the victim's
            //   value against the attacker's.)
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
                if !en_passant && !moves::is_promotion(mv) {
                    if self.features.see_pruning {
                        if !see::see_ge(board, mv, SEE_PRUNE_THRESHOLD) {
                            index += 1;
                            continue;
                        }
                    } else if gain + 150 < MVV_VALUES[code_kind(attacker)] {
                        index += 1;
                        continue;
                    }
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
        if margin <= 0 || self.completed_moves.len() < 2 || result.depth < 2 {
            return;
        }
        if result.score.abs() > MATE_THRESHOLD {
            // Never throw away a forced mate.
            return;
        }

        // Candidates are the moves the search scored *strictly* inside the margin.  The
        // comparison has to be strict: a move whose probe failed is scored at exactly the
        // bar — that is what a bounded probe returns — so an inclusive test would offer
        // moves the search believes are far worse than the margin allows, and
        // `every_move_the_weakening_offers_is_really_within_its_margin` is the test that
        // says so.
        let mut candidates = 0;
        for index in 0..self.completed_moves.len() {
            let mv = self.completed_moves.get(index);
            let score = self.completed_root_scores[index];
            if mv != 0 && score != 0 && score > result.score - margin {
                candidates += 1;
            }
        }
        if candidates < 2 {
            return;
        }

        // A deterministic step from the position's own key: the same board always picks
        // the same move, so a loss is reproducible.  Nothing about the search's own
        // effort goes into it — a level that played differently depending on how many
        // nodes it happened to visit would not be reproducible at all.
        let mut state = board.hash;
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;

        // The draw is triangular over the candidates in the order the search would try
        // them, which is the previous iteration's ranking: the best move comes up most
        // often, the next one less, and so on.  Drawing uniformly instead is what makes
        // a weakened level play the fifteenth-best move as readily as the best one, and
        // in an opening, where fifteen moves sit inside a quarter of a pawn, that is a
        // knight to the rim.
        let total = candidates * (candidates + 1) / 2;
        let mut draw = (state % total as u64) as usize;
        let mut pick = 0;
        while draw >= candidates - pick {
            draw -= candidates - pick;
            pick += 1;
        }

        let mut seen = 0;
        for index in 0..self.completed_moves.len() {
            let mv = self.completed_moves.get(index);
            let score = self.completed_root_scores[index];
            if mv == 0 || score == 0 || score <= result.score - margin {
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
        // The content of this test is the legality of every move and the hash staying
        // consistent with the position (both asserted above).  Length is only a sanity
        // check that the engine is playing rather than falling over: a short game is
        // fine when it ends by a rule — a mate at ply twenty-nine is a result, not a
        // bug — but a game that stops early while still in play is not.
        let outcome = board.outcome();
        assert!(
            played >= 30 || outcome.is_over(),
            "self-play stopped after {} plies still {}",
            played,
            outcome.slug()
        );
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
        for (level, expected_cap) in [(0u32, 12), (1, 16), (2, 20)] {
            let (cap, _, _) = Level::from_number(level).config();
            assert_eq!(cap, expected_cap);
        }
        // And each level is more permissive than the one below it, in depth, time and
        // how close to the best move it insists on playing.
        let levels = [Level::Casual, Level::Club, Level::Strong];
        for pair in levels.windows(2) {
            let (cap, time, margin) = pair[0].config();
            let (next_cap, next_time, next_margin) = pair[1].config();
            assert!(cap < next_cap, "{:?} is not shallower than {:?}", pair[0], pair[1]);
            assert!(time <= next_time);
            assert!(margin > next_margin);
        }
        // A depth of zero means "whatever this level is for".
        let limits = SearchLimits::new(0, 0, 1);
        assert_eq!(limits.max_depth, 16);
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
    fn a_hanging_queen_is_taken_with_black_to_move_too() {
        // The mirror of `a_hanging_queen_is_taken`: the same hunt from the other side,
        // with enough material left on the board that the win is real rather than a
        // dead draw.  This is the shape of test that catches a score written from
        // white's side rather than from the mover's, because for black the whole
        // search comes out upside down and the engine prefers its own worst move.
        let (mut board, result) = search_fen("2r1k3/8/2n5/8/3Q4/8/8/4K3 b - - 0 1", 6, 3000, 2);
        assert_eq!(
            moves::to_uci(result.best),
            "c6d4",
            "black should take the queen"
        );
        assert!(board.make_move(result.best));
        assert_eq!(board.pieces_of(crate::board::WHITE, crate::board::QUEEN), 0);
        assert!(result.score > 400, "black scored only {}", result.score);
    }

    #[test]
    fn the_search_never_prefers_its_own_worst_move() {
        // The same position twice, mirrored: each time the side to move can win a
        // queen with a knight, and each time the search has to see a big win for the
        // mover.  One number above zero from each side is exactly what cannot happen
        // when the evaluation is written from white's side.
        for (fen, expected) in [
            ("4k3/8/8/3q4/8/2N5/8/2R1K3 w - - 0 1", "c3d5"),
            ("2r1k3/8/2n5/8/3Q4/8/8/4K3 b - - 0 1", "c6d4"),
        ] {
            let (_, result) = search_fen(fen, 4, 3000, 2);
            assert_eq!(moves::to_uci(result.best), expected, "in {}", fen);
            assert!(result.score > 300, "{} scored {}", fen, result.score);
        }
    }

    #[test]
    fn the_principal_variation_is_a_legal_line() {
        // The PV is what the UI shows as the engine's plan and what the analysis
        // reads, so every move in it has to be playable in turn.
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "4k3/8/8/3q4/8/2n5/8/3QK3 b - - 0 1",
        ] {
            let (mut board, result) = search_fen(fen, 6, 3000, 2);
            assert!(!result.pv.is_empty(), "{} produced no line", fen);
            assert_eq!(result.pv[0], result.best, "{} PV starts elsewhere", fen);
            for (step, mv) in result.pv.iter().enumerate() {
                assert_ne!(*mv, 0, "{} has an empty move at step {}", fen, step);
                assert!(
                    board.make_move(*mv),
                    "{} plays the illegal move {} at step {}",
                    fen,
                    moves::to_uci(*mv),
                    step
                );
            }
        }
    }

    #[test]
    fn a_weakened_level_still_takes_a_free_queen() {
        // The weakening picks among moves within a margin of the best one, and that must
        // never reach far enough to miss a piece.  Both weakened levels have to take a
        // queen standing in front of a knight, where the capture is nine pawns clear of
        // every alternative: the band cannot contain anything else.
        for level in [0u32, 1] {
            let (mut board, result) = search_fen("4k3/8/8/3q4/8/2N5/8/2R1K3 w - - 0 1", 6, 3_000, level);
            assert_eq!(
                moves::to_uci(result.best),
                "c3d5",
                "level {} missed the queen",
                level
            );
            assert!(board.make_move(result.best));
            assert_eq!(board.pieces_of(crate::board::BLACK, crate::board::QUEEN), 0);
        }
    }

    #[test]
    fn every_move_the_weakening_offers_is_really_within_its_margin() {
        // The claim is that a move the level is willing to play is within `margin` of
        // the best.  The way to check a claim like that is to re-score every root move
        // with a full window — the truth the weakening is approximating — and compare.
        let (_, _, margin) = Level::Club.config();
        assert!(margin > 0, "this test is about a weakened level");
        let depth = 6;
        let mut board = Board::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let mut searcher = Searcher::new(16);
        let limits = SearchLimits {
            max_depth: depth,
            time_ms: 86_400_000,
            level: Level::Club,
        };
        searcher.search(&mut board, limits);

        let mut rescored = Vec::new();
        for index in 0..searcher.root_moves.len() {
            let mv = searcher.root_moves.get(index);
            let recorded = searcher.root_scores[index];
            if !board.make_move(mv) {
                continue;
            }
            let exact = -searcher.negamax(&mut board, depth - 1, -INFINITY, INFINITY, 1, true);
            board.unmake_move();
            rescored.push((mv, recorded, exact));
        }
        let best = rescored
            .iter()
            .map(|(_, _, exact)| *exact)
            .max()
            .expect("moves to score");

        let mut offered = 0;
        for (mv, recorded, exact) in rescored {
            // Strictly inside the margin, which is what the weakening offers: a move
            // whose probe failed is scored exactly at the bar, and that is not the same
            // as being within the margin.
            if recorded > best - margin {
                offered += 1;
                assert!(
                    exact >= best - margin,
                    "{} was offered as near-best: recorded {} but really {} against a best of {}",
                    moves::to_uci(mv),
                    recorded,
                    exact,
                    best
                );
            }
        }
        assert!(offered >= 1, "no move was offered at all");
    }

    #[test]
    fn a_quiet_refutation_is_remembered_as_a_counter_move() {
        // Kiwipete at six ply is deep enough to refute quiet moves, so with the flag on
        // the table has to come out of the search holding something — and stay empty
        // with it off, which is the shipped default and what makes the match between the
        // two mean anything.
        let kiwipete = "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1";
        let stored_after = |features: SearchFeatures| {
            let mut board = Board::from_fen(kiwipete).unwrap();
            let mut searcher = Searcher::new(4);
            searcher.set_features(features);
            searcher.search(&mut board, SearchLimits::new(6, 10_000, 2));
            searcher
                .counter_moves
                .iter()
                .flatten()
                .filter(|mv| **mv != 0)
                .count()
        };

        assert!(
            stored_after(SearchFeatures {
                counter_moves: true,
                ..SearchFeatures::all()
            }) > 0,
            "the search stored no counter-move at all"
        );
        assert_eq!(
            stored_after(SearchFeatures::all()),
            0,
            "the shipped default stores counter-moves, which the matches say it should not"
        );
    }

    #[test]
    fn the_counter_move_is_ordered_ahead_of_an_ordinary_quiet_move() {
        // Pretend the opponent has just played e2e4 and the refutation is d7d5.  With
        // the flag on the ordering has to prefer the stored refutation; with it off —
        // the shipped default — the table is not consulted at all.
        let mut board = Board::startpos();
        let opponent_move = moves::from_uci("e2e4").unwrap();
        assert!(board.make_move(opponent_move));
        let refutation = moves::from_uci("d7d5").unwrap();
        let ordinary = moves::from_uci("d7d6").unwrap();
        let key = (
            moves::from_square(opponent_move),
            moves::to_square(opponent_move),
        );

        let mut searcher = Searcher::new(1);
        searcher.set_features(SearchFeatures {
            counter_moves: true,
            ..SearchFeatures::all()
        });
        searcher.counter_moves[key.0][key.1] = refutation;
        let preferred = searcher.move_score(&board, refutation, 0, 1, false);
        let other = searcher.move_score(&board, ordinary, 0, 1, false);
        assert_eq!(preferred, 550_000);
        assert!(
            preferred > other,
            "counter-move scored {} against {}",
            preferred,
            other
        );

        let searcher = Searcher::new(1);
        assert!(!searcher.features().counter_moves, "the default is off");
        let mut searcher = searcher;
        searcher.counter_moves[key.0][key.1] = refutation;
        assert_eq!(
            searcher.move_score(&board, refutation, 0, 1, false),
            0,
            "the flag is off and the move was still preferred"
        );
    }

    #[test]
    fn the_search_uses_most_of_the_budget_it_is_given() {
        // A position that cannot be finished inside the budget by any margin, so the
        // engine has to keep starting iterations rather than stopping at the first one
        // that happens to fit.  A budget that is quietly ignored is as much of a bug as
        // one that is overrun.
        let mut board = Board::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let mut searcher = Searcher::new(16);
        let result = searcher.search(&mut board, SearchLimits::new(20, 300, 2));
        assert!(
            result.time_ms >= 100,
            "gave up after {} ms of a 300 ms budget",
            result.time_ms
        );
        assert!(
            result.time_ms <= 300,
            "overran the budget: {} ms",
            result.time_ms
        );
    }

    #[test]
    fn a_weakened_level_plays_a_legal_move_even_when_its_search_is_cut_off() {
        // The weakening pass chooses among the moves of the last iteration that
        // *finished*, and the budget here is small enough that most iterations are
        // abandoned part-way.  A snapshot taken from the previous search — or from an
        // unfinished one — would let the level play a move that is not legal in the
        // position it is looking at, so this plays a few moves of a game with a level
        // that weakens its choice and a budget of one millisecond per move.
        for (level, _) in [(0u32, Level::Casual), (1, Level::Club)] {
            let mut board = Board::startpos();
            let mut searcher = Searcher::new(4);
            for ply in 0..6 {
                let legal = board.legal_moves();
                if legal.is_empty() {
                    break;
                }
                let result = searcher.search(&mut board, SearchLimits::new(0, 1, level));
                assert_ne!(result.best, 0, "no move at ply {}", ply);
                assert!(
                    legal.contains(result.best),
                    "ply {}: {} is not legal in {}",
                    ply,
                    moves::to_uci(result.best),
                    board.to_fen()
                );
                assert!(board.make_move(result.best));
            }
        }
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


