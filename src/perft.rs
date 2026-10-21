//! Perft: the move generator's test rig.
//!
//! Perft counts the leaf nodes of the move tree to a given depth.  Every count is
//! published for these positions, so one wrong count points at a rule the
//! generator gets wrong — and because the counts are so sensitive (one missing
//! castle anywhere changes millions of nodes at depth five), it catches the bugs
//! a hand-written test list would miss.
//!
//! The suite is the standard one from the Chess Programming Wiki: the starting
//! position, Kiwipete (which is dense with castling, pins and en passant) and the
//! four positions that between them cover promotions, checks, discovered attacks
//! and the awkward en passant pins.
//!
//! `perft_divide` is the debugging companion: it counts per root move, so an
//! incorrect total can be narrowed to the branch that causes it.

use crate::board::Board;
use crate::movegen;
use crate::moves::{self, MoveList};

/// Counts leaf nodes to `depth`.
///
/// Illegal moves are filtered by `make_move`, which returns false and rolls the
/// move back, so this never walks into a position with a captured king.
pub fn perft(board: &mut Board, depth: i32) -> u64 {
    if depth <= 0 {
        return 1;
    }
    // One-ply shortcut: when the count only needs the number of moves, playing
    // each one just to undo it is pure overhead.
    if depth == 1 {
        let mut list = MoveList::new();
        movegen::generate_legal(board, &mut list);
        return list.len() as u64;
    }

    let mut list = MoveList::new();
    movegen::generate_pseudo_legal(board, &mut list);
    let mut nodes = 0;
    for mv in list.as_slice() {
        if board.make_move(*mv) {
            nodes += perft(board, depth - 1);
            board.unmake_move();
        }
    }
    nodes
}

/// Per-root-move counts, for narrowing a mismatch down.
pub fn perft_divide(board: &mut Board, depth: i32) -> Vec<(String, u64)> {
    let mut list = MoveList::new();
    movegen::generate_legal(board, &mut list);
    let mut out = Vec::new();
    for mv in list.as_slice() {
        if board.make_move(*mv) {
            let nodes = perft(board, depth - 1);
            out.push((moves::to_uci(*mv), nodes));
            board.unmake_move();
        }
    }
    out.sort_by(|first, second| first.0.cmp(&second.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Position name, FEN, and the published counts from depth 1 upwards.
    const SUITE: [(&str, &str, &[u64]); 6] = [
        (
            "startpos",
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            &[20, 400, 8902, 197281, 4865609],
        ),
        (
            "kiwipete",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            &[48, 2039, 97862, 4085603],
        ),
        (
            "position 3",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            &[14, 191, 2812, 43238, 674624],
        ),
        (
            "position 4",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            &[6, 264, 9467, 422333],
        ),
        (
            "position 5",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            &[44, 1486, 62379, 2103487],
        ),
        (
            "position 6",
            "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
            &[46, 2079, 89890, 3894594],
        ),
    ];

    #[test]
    fn the_published_counts_match_position_by_position() {
        for (name, fen, expected) in SUITE {
            let restored = Board::from_fen(fen).expect("legal FEN").to_fen();
            let mut board = Board::from_fen(fen).expect("legal FEN");
            for (index, want) in expected.iter().enumerate() {
                let depth = index as i32 + 1;
                let got = perft(&mut board, depth);
                assert_eq!(
                    got, *want,
                    "{} at depth {}: expected {}, counted {}",
                    name, depth, want, got
                );
                // Every count leaves the board exactly where it started.
                assert_eq!(board.to_fen(), restored, "{} left the board changed", name);
                assert_eq!(board.hash, board.compute_hash());
            }
        }
    }

    #[test]
    fn the_divide_sums_to_the_total_and_has_one_line_per_root_move() {
        let mut board = Board::from_fen(
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
        )
        .unwrap();
        let total = perft(&mut board, 3);
        let divide = perft_divide(&mut board, 3);
        assert_eq!(total, 97862);
        assert_eq!(divide.len(), 48, "Kiwipete has 48 legal moves");
        let summed: u64 = divide.iter().map(|(_, nodes)| nodes).sum();
        assert_eq!(total, summed);
        // No branch may be empty, and the counts have to be plausible numbers of
        // continuations rather than obviously wrong magnitudes.
        for (uci, nodes) in &divide {
            assert!(*nodes > 500, "{} counted only {}", uci, nodes);
            assert!(*nodes < 5000, "{} counted {}", uci, nodes);
        }
    }

    #[test]
    fn a_mated_position_counts_nothing() {
        // Ra8 is mate: there is no leaf to count at any depth.
        let mut board = Board::from_fen("7k/5ppp/8/8/8/8/8/R6K w - - 0 1").unwrap();
        assert!(board.make_move(moves::from_uci("a1a8").unwrap()));
        assert_eq!(perft(&mut board, 1), 0);
        assert_eq!(perft(&mut board, 5), 0);
        assert!(board.unmake_move());
        // Back to the start: the rook has thirteen squares and the king three.
        assert_eq!(perft(&mut board, 1), 16);
    }
}
