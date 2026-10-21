//! A native benchmark: move generation and search speed, printed as a report.
//!
//!     cargo run --release --bin bench
//!
//! Perft is the move generator's speed and its correctness at once — a wrong count
//! shows up next to a timing instead of only in a test — and the search rows are
//! what the browser's time budget is chosen from.  The numbers here are the ones
//! the README quotes, so they are measured rather than claimed.

use pg_chess::board::Board;
use pg_chess::perft::perft;
use pg_chess::search::{SearchLimits, Searcher};
use std::time::Instant;

fn main() {
    println!("PG-Chess native benchmark\n");

    perft_report();
    search_report();
}

fn perft_report() {
    println!("Move generation (perft)");
    let cases = [
        (
            "startpos",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            5,
        ),
        (
            "kiwipete",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            4,
        ),
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

fn search_report() {
    println!("Search (level 2: 10 ply cap, 1.5 s budget)");
    println!(
        "  {:<12} {:>5} {:>10} {:>8} {:>8} {:>12}",
        "position", "depth", "score", "nodes", "time", "nodes/s"
    );

    let positions = [
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
        (
            "endgame",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        ),
    ];

    let mut searcher = Searcher::new(32);
    for (name, fen) in positions {
        let mut board = Board::from_fen(fen).expect("legal FEN");
        let result = searcher.search(&mut board, SearchLimits::new(0, 1500, 2));
        println!(
            "  {:<12} {:>5} {:>10} {:>8} {:>7}ms {:>12}",
            name,
            result.depth,
            result.score,
            result.nodes,
            result.time_ms,
            result.nodes_per_second()
        );
    }

    // The budget the page actually uses, so the report answers the question the
    // browser raises: how deep does a Club-level turn get in the time it has?
    println!("\nClub level budget (0.7 s, 7 ply cap)");
    for (name, fen) in positions {
        let mut board = Board::from_fen(fen).expect("legal FEN");
        let result = searcher.search(&mut board, SearchLimits::new(0, 0, 1));
        println!(
            "  {:<12} depth {:>2}  score {:>7}  {:>7} nodes  {:>4}ms",
            name, result.depth, result.score, result.nodes, result.time_ms
        );
    }
}
