//! The JavaScript interface.
//!
//! Everything the page needs to play a game goes through `Engine`: it owns the
//! board and the searcher, and every method either reads state or plays one move.
//! The page never sees a bitboard or a `u64`, only strings, numbers and booleans,
//! which is what keeps the worker thin and the boundary cheap.
//!
//! Two conventions the page depends on:
//!
//! * Moves cross the boundary as UCI strings (`e2e4`, `e7e8q`).
//! * Scores are centipawns from the side to move's point of view, and a score past
//!   `MATE_THRESHOLD` is a mate whose distance `mate_in` reports.

use wasm_bindgen::prelude::*;

pub mod bitboard;
pub mod board;
pub mod evaluate;
pub mod movegen;
pub mod moves;
pub mod perft;
pub mod search;
pub mod tt;

use board::{Board, Outcome, WHITE};
use moves::Move;
use search::{SearchLimits, Searcher};

/// The design's transposition table size.
const DEFAULT_TT_MEGABYTES: usize = 32;

/// A wasm-side engine handle.
#[wasm_bindgen]
pub struct Engine {
    board: Board,
    searcher: Searcher,
    /// The moves played since the start, so the page can walk back through them.
    history: Vec<Move>,
}

#[wasm_bindgen]
impl Engine {
    /// A fresh engine on the standard starting position.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Engine {
        Engine {
            board: Board::startpos(),
            searcher: Searcher::new(DEFAULT_TT_MEGABYTES),
            history: Vec::with_capacity(1024),
        }
    }

    /// A fresh engine on a FEN position.  Returns an error string when the FEN is
    /// not readable, which the page shows next to the input.
    #[wasm_bindgen(js_name = fromFen)]
    pub fn from_fen(fen: &str) -> Result<Engine, String> {
        let board = Board::from_fen(fen)
            .ok_or_else(|| format!("unreadable FEN: {}", fen))?;
        Ok(Engine {
            board,
            searcher: Searcher::new(DEFAULT_TT_MEGABYTES),
            history: Vec::with_capacity(1024),
        })
    }

    /// The current position as FEN, which is one of the two ways the page can
    /// recreate the board.
    pub fn fen(&self) -> String {
        self.board.to_fen()
    }

    /// `"white"` or `"black"`.
    #[wasm_bindgen(js_name = sideToMove)]
    pub fn side_to_move(&self) -> String {
        if self.board.side_to_move() == WHITE {
            "white".to_string()
        } else {
            "black".to_string()
        }
    }

    /// Every legal move as UCI, space separated.  The page uses this both to
    /// validate a click and to light up the squares the selected piece can reach.
    #[wasm_bindgen(js_name = legalMoves)]
    pub fn legal_moves(&self) -> String {
        self.board
            .legal_moves()
            .as_slice()
            .iter()
            .map(|&mv| moves::to_uci(mv))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// True when the side to move is in check.
    #[wasm_bindgen(js_name = inCheck)]
    pub fn in_check(&self) -> bool {
        self.board.in_check()
    }

    /// `playing`, `checkmate`, `stalemate`, `fifty-move`, `repetition` or
    /// `insufficient-material`.
    pub fn outcome(&self) -> String {
        self.board.outcome().slug().to_string()
    }

    /// True once no further moves can be played.
    #[wasm_bindgen(js_name = isGameOver)]
    pub fn is_game_over(&self) -> bool {
        self.board.outcome().is_over()
    }

    /// The side that has been mated, or `""` while the game is still running.
    ///
    /// `outcome() == "checkmate"` says somebody was mated; this says who, and the
    /// answer is always the side to move.
    #[wasm_bindgen(js_name = matedSide)]
    pub fn mated_side(&self) -> String {
        if self.board.outcome() == Outcome::Checkmate {
            self.side_to_move()
        } else {
            String::new()
        }
    }

    /// The board as 64 characters, `a8` first and `h1` last: `P` is a white pawn,
    /// `n` a black knight, `.` an empty square.
    ///
    /// The page draws this rather than parsing the FEN again, because a flat
    /// string needs no parsing at all.
    #[wasm_bindgen(js_name = piecePlacement)]
    pub fn piece_placement(&self) -> String {
        let mut out = String::with_capacity(64);
        for rank in (0..8).rev() {
            for file in 0..8 {
                let square = bitboard::square_index(file, rank);
                let code = self.board.piece_on(square);
                out.push(if code == board::NO_PIECE {
                    '.'
                } else {
                    board::piece_letter(code)
                });
            }
        }
        out
    }

    /// Plays a legal move given in UCI and returns the SAN of what was played.
    ///
    /// Returns an error string when the move is not legal here, which is what lets
    /// the page treat a rejection as "that click was not a move".
    pub fn play(&mut self, uci: &str) -> Result<String, String> {
        let mv = moves::from_uci(uci).ok_or_else(|| format!("unreadable move: {}", uci))?;
        if !self.board.legal_moves().contains(mv) {
            return Err(format!("illegal move: {}", uci));
        }
        // SAN is read before the move is played, because it describes the move
        // from the position it was made in.
        let san = self.board.move_san(mv);
        self.board.make_move(mv);
        self.history.push(mv);
        Ok(san)
    }

    /// Takes one ply back.  The page calls it twice to take back a full move.
    pub fn undo(&mut self) -> bool {
        if self.board.unmake_move() {
            self.history.pop();
            true
        } else {
            false
        }
    }

    /// Plies played since this engine was created or restarted.
    #[wasm_bindgen(js_name = plyCount)]
    pub fn ply_count(&self) -> u32 {
        self.board.ply_count() as u32
    }

    /// The last move played, as UCI, or `""` at the start of a game.
    #[wasm_bindgen(js_name = lastMove)]
    pub fn last_move(&self) -> String {
        self.board
            .last_move()
            .map(moves::to_uci)
            .unwrap_or_default()
    }

    /// The SAN of a move without playing it.
    pub fn san(&self, uci: &str) -> String {
        match moves::from_uci(uci) {
            Some(mv) => self.board.move_san(mv),
            None => String::new(),
        }
    }

    /// The static evaluation in centipawns, from the side to move's point of view.
    pub fn evaluate(&self) -> i32 {
        evaluate::evaluate(&self.board)
    }

    /// Searches and returns the result.  A `max_depth` or `time_ms` of zero means
    /// "whatever this level's setting is".
    pub fn search(&mut self, max_depth: u32, time_ms: u32, level: u32) -> SearchInfo {
        let limits = SearchLimits::new(max_depth, time_ms, level);
        let result = self.searcher.search(&mut self.board, limits);
        SearchInfo {
            uci: moves::to_uci(result.best),
            san: self.board.move_san(result.best),
            pv: result
                .pv
                .iter()
                .map(|&mv| moves::to_uci(mv))
                .collect::<Vec<_>>()
                .join(" "),
            score: result.score,
            mate_in: mate_in(result.score),
            depth: result.depth.max(0) as u32,
            nodes: result.nodes.min(u32::MAX as u64) as u32,
            time_ms: result.time_ms.min(u32::MAX as u64) as u32,
            nodes_per_second: result.nodes_per_second().min(u32::MAX as u64) as u32,
        }
    }

    /// Counts leaf nodes to `depth`, for the harness's own perft checks.
    pub fn perft(&mut self, depth: u32) -> f64 {
        perft::perft(&mut self.board, depth as i32) as f64
    }

    /// The per-root-move counts, as `"e2e4:20 g1f3:20"`.
    #[wasm_bindgen(js_name = perftDivide)]
    pub fn perft_divide(&mut self, depth: u32) -> String {
        perft::perft_divide(&mut self.board, depth as i32)
            .into_iter()
            .map(|(uci, nodes)| format!("{}:{}", uci, nodes))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Resizes the transposition table, in megabytes.
    #[wasm_bindgen(js_name = setTtSize)]
    pub fn set_tt_size(&mut self, megabytes: u32) {
        self.searcher.set_tt_size(megabytes.max(1) as usize);
    }

    /// The table's size in bytes.
    #[wasm_bindgen(js_name = ttSize)]
    pub fn tt_size(&self) -> f64 {
        self.searcher.tt_size_bytes() as f64
    }

    /// Forgets the search heuristics, for a new game.
    #[wasm_bindgen(js_name = resetSearch)]
    pub fn reset_search(&mut self) {
        self.searcher.reset_heuristics();
    }

    /// Starts a fresh game from the standard position.
    #[wasm_bindgen(js_name = newGame)]
    pub fn new_game(&mut self) {
        self.board = Board::startpos();
        self.history.clear();
        self.searcher.reset_heuristics();
    }

    /// Starts a fresh game from a FEN.
    #[wasm_bindgen(js_name = newGameFromFen)]
    pub fn new_game_from_fen(&mut self, fen: &str) -> Result<(), String> {
        self.board =
            Board::from_fen(fen).ok_or_else(|| format!("unreadable FEN: {}", fen))?;
        self.history.clear();
        self.searcher.reset_heuristics();
        Ok(())
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

/// What a search returns to JavaScript.
#[wasm_bindgen]
pub struct SearchInfo {
    uci: String,
    san: String,
    pv: String,
    score: i32,
    mate_in: i32,
    depth: u32,
    nodes: u32,
    time_ms: u32,
    nodes_per_second: u32,
}

#[wasm_bindgen]
impl SearchInfo {
    /// The best move in UCI.
    pub fn uci(&self) -> String {
        self.uci.clone()
    }

    /// The best move in SAN.
    pub fn san(&self) -> String {
        self.san.clone()
    }

    /// The principal variation in UCI, space separated.
    pub fn pv(&self) -> String {
        self.pv.clone()
    }

    /// Centipawns, from the side to move's point of view.
    pub fn score(&self) -> i32 {
        self.score
    }

    /// Moves until mate when the score is a mate, otherwise 0.  Negative means the
    /// side to move is the one being mated.
    #[wasm_bindgen(getter, js_name = mateIn)]
    pub fn mate_in(&self) -> i32 {
        self.mate_in
    }

    /// The deepest completed iteration.
    pub fn depth(&self) -> u32 {
        self.depth
    }

    /// Nodes visited.
    pub fn nodes(&self) -> u32 {
        self.nodes
    }

    /// Milliseconds spent.
    #[wasm_bindgen(getter, js_name = timeMs)]
    pub fn time_ms(&self) -> u32 {
        self.time_ms
    }

    /// Nodes per second.
    #[wasm_bindgen(getter, js_name = nodesPerSecond)]
    pub fn nodes_per_second(&self) -> u32 {
        self.nodes_per_second
    }
}

/// Moves to mate when the score is a mate score, otherwise 0.
fn mate_in(score: i32) -> i32 {
    if score > search::MATE_THRESHOLD {
        (search::MATE - score + 1) / 2
    } else if score < -search::MATE_THRESHOLD {
        -((search::MATE + score + 1) / 2)
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_engine_starts_on_the_standard_position() {
        let engine = Engine::new();
        assert_eq!(
            engine.fen().split(' ').next(),
            Some("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR")
        );
        assert_eq!(engine.side_to_move(), "white");
        assert_eq!(engine.ply_count(), 0);
        assert_eq!(engine.piece_placement().len(), 64);
        assert!(engine.piece_placement().starts_with("rnbqkbnr"));
        assert_eq!(engine.legal_moves().split(' ').count(), 20);
        assert_eq!(engine.outcome(), "playing");
        assert!(!engine.in_check());
        assert_eq!(engine.last_move(), "");
    }

    #[test]
    fn playing_and_undoing_walks_the_game_forwards_and_back() {
        let mut engine = Engine::new();
        let start = engine.fen();
        assert_eq!(engine.play("e2e4").expect("e2e4 is legal"), "e4");
        assert_eq!(engine.side_to_move(), "black");
        assert_eq!(engine.ply_count(), 1);
        assert_eq!(engine.last_move(), "e2e4");
        assert_eq!(engine.play("e7e5").expect("e7e5 is legal"), "e5");
        assert_eq!(engine.ply_count(), 2);

        assert!(engine.undo());
        assert!(engine.undo());
        assert_eq!(engine.fen(), start);
        assert_eq!(engine.ply_count(), 0);
        assert!(!engine.undo(), "there is nothing left to undo");
    }

    #[test]
    fn an_illegal_move_is_rejected_and_changes_nothing() {
        let mut engine = Engine::new();
        let before = engine.fen();
        assert!(engine.play("e2e5").is_err());
        assert!(engine.play("not a move").is_err());
        assert!(engine.play("d2d4d").is_err());
        assert_eq!(engine.fen(), before);
        assert_eq!(engine.ply_count(), 0);
    }

    #[test]
    fn the_page_facing_summary_matches_what_was_searched() {
        let mut engine = Engine::new();
        let info = engine.search(4, 1500, 2);
        assert!(!info.uci().is_empty());
        assert!(!info.san().is_empty());
        assert!(info.depth() >= 1);
        assert!(info.nodes() > 0);
        assert!(info.time_ms() <= 4000);
        // The move it reports has to be playable.
        assert!(engine.play(&info.uci()).is_ok());
    }

    #[test]
    fn a_mate_is_reported_as_a_distance_in_moves() {
        let mut engine = Engine::from_fen("7k/5ppp/8/8/8/8/8/R6K w - - 0 1").unwrap();
        let info = engine.search(4, 2000, 2);
        assert_eq!(info.uci(), "a1a8");
        assert_eq!(info.mate_in(), 1);
        assert!(info.san().ends_with('#'), "san was {}", info.san());
    }

    #[test]
    fn perft_is_reachable_through_the_page_api() {
        let mut engine = Engine::new();
        assert_eq!(engine.perft(3) as u64, 8902);
        let divide = engine.perft_divide(2);
        assert_eq!(divide.split(' ').count(), 20);
        assert!(divide.contains("e2e4:20"));
    }

    #[test]
    fn a_new_game_clears_the_board_and_the_search() {
        let mut engine = Engine::new();
        engine.play("e2e4").unwrap();
        engine.play("e7e5").unwrap();
        engine.new_game();
        assert_eq!(engine.ply_count(), 0);
        assert_eq!(engine.last_move(), "");
        assert_eq!(engine.side_to_move(), "white");

        engine
            .new_game_from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1")
            .unwrap();
        assert_eq!(engine.outcome(), "insufficient-material");
        assert!(engine.new_game_from_fen("nonsense").is_err());
    }

    #[test]
    fn checkmate_names_the_side_that_lost() {
        let mut engine = Engine::from_fen("7k/5ppp/8/8/8/8/8/R6K w - - 0 1").unwrap();
        engine.play("a1a8").unwrap();
        assert_eq!(engine.outcome(), "checkmate");
        assert_eq!(engine.mated_side(), "black");
        assert!(engine.is_game_over());
    }

    #[test]
    fn the_transposition_table_can_be_resized() {
        let mut engine = Engine::new();
        assert_eq!(engine.tt_size(), (32 * 1024 * 1024) as f64);
        engine.set_tt_size(8);
        assert_eq!(engine.tt_size(), (8 * 1024 * 1024) as f64);
    }
}

