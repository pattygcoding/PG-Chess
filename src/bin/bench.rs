//! A native benchmark: move generation and search speed, printed as a report.
//!
//!     cargo run --release --bin bench
//!
//! Perft is the move generator's speed and its correctness at once — a wrong count
//! shows up next to a timing instead of only in a test — and the search rows are
//! what the browser's time budget is chosen from.  The numbers here are the ones
//! the README quotes, so they are measured rather than claimed.
//!
//! The search section prints two columns for the same position and the same depth:
//! `new` is the engine as it ships, `old` is the same engine with every technique
//! introduced after the first release switched off.  Both are exact node counts, so
//! the comparison is a measurement rather than an opinion, and the last section
//! answers the question the page actually asks: how deep does a turn get in the time
//! it has?

use pg_chess::board::Board;
use pg_chess::perft::perft;
use pg_chess::search::{SearchFeatures, SearchLimits, Searcher};
use std::time::Instant;

fn main() {
    println!("PG-Chess native benchmark\n");

    perft_report();
    fixed_depth_report();
    budget_report();
}

const POSITIONS: [(&str, &str); 4] = [
    (
        "startpos",
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
    ),
    (
        "italian",
        "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
    ),
    (
        "kiwipete",
        "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
    ),
    ("endgame", "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1"),
];

fn perft_report() {
    println!("Move generation (perft)");
    let cases = [
        ("startpos", POSITIONS[0].1, 5),
        ("kiwipete", POSITIONS[2].1, 4),
        (
            "position 5",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            4,
        ),
    ];

    for (name, fen, depth) in cases {
        let mut board = Board::from_fen(fen).expect("legal FEN");
        let start = Instant::now();
        let nodes = perft(&mut board, depth);
        let elapsed = start.elapsed().as_secs_f64();
        let rate = nodes as f64 / elapsed / 1_000_000.0;
        println!(
            "  {:<12} depth {} {:>12} nodes  {:>7.3}s  {:>6.1} Mnodes/s",
            name, depth, nodes, elapsed, rate
        );
    }
    println!();
}

/// Both feature sets, same positions, same depth, no clock: every number here is an
/// exact node count, so it does not depend on how busy the machine is.
fn fixed_depth_report() {
    println!("Search to a fixed depth (this engine vs the techniques switched off)");
    println!(
        "  {:<12} {:>4} {:>12} {:>12} {:>9} {:>9}",
        "position", "ply", "nodes now", "nodes old", "now ms", "old ms"
    );

    let mut now = Searcher::new(32);
    now.set_features(SearchFeatures::all());
    let mut old = Searcher::new(32);
    old.set_features(SearchFeatures::baseline());

    for depth in [6, 8] {
        for (name, fen) in POSITIONS {
            let mut board = Board::from_fen(fen).expect("legal FEN");
            // A day of budget and the depth cap as the only limit, so the iteration
            // always finishes and the counts are reproducible.
            let limits = SearchLimits::new(depth, 86_400_000, 2);
            now.reset_heuristics();
            let start = Instant::now();
            let this_run = now.search(&mut board, limits);
            let now_ms = start.elapsed().as_secs_f64() * 1000.0;
            old.reset_heuristics();
            let start = Instant::now();
            let that_run = old.search(&mut board, limits);
            let old_ms = start.elapsed().as_secs_f64() * 1000.0;
            println!(
                "  {:<12} {:>4} {:>12} {:>12} {:>8.0} {:>8.0}",
                name,
                this_run.depth,
                format!("{}", this_run.nodes),
                format!("{}", that_run.nodes),
                now_ms,
                old_ms
            );
        }
    }
    println!();
}

/// The two budgets the page uses, with the depth each feature set reaches inside
/// them.  This is the row that decides whether a level needs re-tuning.
fn budget_report() {
    println!("Depth reached inside the UI's budgets");
    println!(
        "  {:<12} {:>11} {:>8} {:>9} {:>8} {:>9}",
        "position", "budget", "depth now", "nodes now", "depth old", "nodes old"
    );

    let mut now = Searcher::new(32);
    now.set_features(SearchFeatures::all());
    let mut old = Searcher::new(32);
    old.set_features(SearchFeatures::baseline());

    for (level_name, level) in [("Club 0.7s", 1u32), ("Strong 1.5s", 2u32)] {
        for (name, fen) in POSITIONS {
            let mut board = Board::from_fen(fen).expect("legal FEN");
            now.reset_heuristics();
            let this_run = now.search(&mut board, SearchLimits::new(0, 0, level));
            old.reset_heuristics();
            let that_run = old.search(&mut board, SearchLimits::new(0, 0, level));
            println!(
                "  {:<12} {:>11} {:>8} {:>9} {:>8} {:>9}",
                name,
                level_name,
                format!("{} ({}ms)", this_run.depth, this_run.time_ms),
                format!("{}", this_run.nodes),
                format!("{} ({}ms)", that_run.depth, that_run.time_ms),
                format!("{}", that_run.nodes)
            );
        }
    }
}
