//! A scratch probe: replay moves, print the position, search and print the PV.
//!
//!     cargo run --release --bin probe -- "e2e4 e7e5 g1f3 b8c6 a2a3" 6 5000
//!
//! Kept out of the library and out of the tests: this is a torch for finding out
//! what the search is thinking, not part of the engine.

use pg_chess::board::Board;
use pg_chess::moves;
use pg_chess::search::{Level, SearchFeatures, SearchLimits, Searcher};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let line = arguments.first().cloned().unwrap_or_default();
    let depth: i32 = arguments
        .get(1)
        .and_then(|text| text.parse().ok())
        .unwrap_or(6);
    let time_ms: u64 = arguments
        .get(2)
        .and_then(|text| text.parse().ok())
        .unwrap_or(2_000);
    let baseline = arguments.get(3).map(|text| text == "old").unwrap_or(false);

    let mut board = Board::startpos();
    if line.contains('/') {
        board = Board::from_fen(&line).unwrap_or_else(|| panic!("bad FEN {}", line));
    } else {
        for text in line.split_whitespace() {
            let mv = moves::from_uci(text).unwrap_or_else(|| panic!("bad move {}", text));
            assert!(board.make_move(mv), "illegal move {}", text);
        }
    }
    println!("{}\n", board.to_fen());
    println!("{}", pg_chess::bitboard::to_diagram(board.occupied()));
    let legal = board.legal_moves();
    print!("\nlegal moves ({}):", legal.len());
    for index in 0..legal.len() {
        print!(" {}", moves::to_uci(legal.get(index)));
    }
    println!();

    let mut searcher = Searcher::new(32);
    if baseline {
        searcher.set_features(SearchFeatures::baseline());
    }
    println!(
        "\nstatic eval: all={} none={}",
        pg_chess::evaluate::evaluate(&board),
        pg_chess::evaluate::evaluate_with(&board, pg_chess::evaluate::EvalFeatures::none())
    );
    let limits = SearchLimits {
        max_depth: depth,
        time_ms,
        level: Level::Strong,
    };
    let result = searcher.search(&mut board, limits);
    println!(
        "\nbest {} score {} depth {} nodes {} in {}ms",
        moves::to_uci(result.best),
        result.score,
        result.depth,
        result.nodes,
        result.time_ms
    );
    print!("pv:");
    for mv in &result.pv {
        print!(" {}", moves::to_uci(*mv));
    }
    println!();
}
