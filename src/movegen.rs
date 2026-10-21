//! Move generation.
//!
//! Two entry points, because the search and the UI want different things:
//!
//! * `generate_pseudo_legal` is what the search uses.  It never allocates and it
//!   does not check whether a move leaves its own king in check — `make_move`
//!   does that and rejects the move, which is cheaper than testing every move up
//!   front.
//! * `generate_legal` is the filtered version the UI, the SAN writer and the
//!   perft harness use.
//!
//! Castling is generated last, and only when the king is not standing on or
//! passing through an attacked square, so a generated castle is always legal with
//! respect to check.

use crate::bitboard::{
    bishop_attacks, file_of, king_attacks, knight_attacks, pop_lsb, queen_attacks, rank_of,
    rook_attacks, square_bb, Bitboard, EAST, WEST,
};
use crate::board::{self, Board, BISHOP, KNIGHT, PAWN, QUEEN, ROOK, WHITE};
use crate::moves::{
    self, MoveList, PROMO_BISHOP_INDEX, PROMO_KNIGHT_INDEX, PROMO_QUEEN_INDEX, PROMO_ROOK_INDEX,
};

/// Promotions are generated queen first, so move ordering has the best promotion
/// already at the front and the UI's picker reads the same order.
pub const PROMOTION_ORDER: [usize; 4] = [
    PROMO_QUEEN_INDEX,
    PROMO_ROOK_INDEX,
    PROMO_BISHOP_INDEX,
    PROMO_KNIGHT_INDEX,
];

/// Every pseudo-legal move for the side to move.
pub fn generate_pseudo_legal(board: &Board, list: &mut MoveList) {
    let us = board.side;
    let occupied = board.occupied();
    let theirs = board.colour[1 - us];

    generate_pawn_moves(board, list, us, !occupied, theirs);
    generate_knight_moves(board, list, us);
    generate_sliding_moves(board, list, us, occupied);
    generate_king_moves(board, list, us);
    generate_castling(board, list, us, board.occupied());
}

/// Pawn pushes, double pushes, captures, promotions and en passant.
fn generate_pawn_moves(
    board: &Board,
    list: &mut MoveList,
    us: usize,
    empty: Bitboard,
    theirs: Bitboard,
) {
    // White walks up the board and black down, so the start and promotion ranks
    // are each other's mirror.
    let (up, start_rank, promotion_rank) = if us == WHITE {
        (8i32, 1i32, 6i32)
    } else {
        (-8i32, 6i32, 1i32)
    };

    let mut pawns = board.pieces_of(us, PAWN);
    while pawns != 0 {
        let from = pop_lsb(&mut pawns);
        let rank = rank_of(from) as i32;
        let file = file_of(from) as i32;
        let ahead = (from as i32 + up) as usize;
        let promoting = rank == promotion_rank;

        // Straight ahead, when that square is empty.
        if empty & square_bb(ahead) != 0 {
            if promoting {
                for piece in PROMOTION_ORDER {
                    list.push(moves::make_promotion(from, ahead, piece));
                }
            } else {
                list.push(moves::make(from, ahead));
                // A double push is only available from the pawn's home rank, and
                // only when both squares it crosses are empty.
                if rank == start_rank {
                    let far = (from as i32 + 2 * up) as usize;
                    if empty & square_bb(far) != 0 {
                        list.push(moves::make(from, far));
                    }
                }
            }
        }

        // Diagonal captures, one file either side.
        for step in [EAST, WEST] {
            let target_file = file + step;
            if !(0..8).contains(&target_file) {
                continue;
            }
            let to = (ahead as i32 + step) as usize;
            if theirs & square_bb(to) != 0 {
                if promoting {
                    for piece in PROMOTION_ORDER {
                        list.push(moves::make_promotion(from, to, piece));
                    }
                } else {
                    list.push(moves::make(from, to));
                }
            } else if board.ep != board::NO_EP
                && to == board.ep as usize
                && empty & square_bb(to) != 0
            {
                // En passant: the victim is not on the destination square, which
                // is exactly why `make_move` records where it found the capture.
                list.push(moves::make(from, to));
            }
        }
    }
}

/// Knights: one table lookup per knight, masked against the side's own pieces.
fn generate_knight_moves(board: &Board, list: &mut MoveList, us: usize) {
    let own = board.colour[us];
    let mut knights = board.pieces_of(us, KNIGHT);
    while knights != 0 {
        let from = pop_lsb(&mut knights);
        let mut targets = knight_attacks(from) & !own;
        while targets != 0 {
            list.push(moves::make(from, pop_lsb(&mut targets)));
        }
    }
}

/// Bishops, rooks and queens, through the ray attacks.
fn generate_sliding_moves(board: &Board, list: &mut MoveList, us: usize, occupied: Bitboard) {
    let own = board.colour[us];

    let mut bishops = board.pieces_of(us, BISHOP);
    while bishops != 0 {
        let from = pop_lsb(&mut bishops);
        let mut targets = bishop_attacks(from, occupied) & !own;
        while targets != 0 {
            list.push(moves::make(from, pop_lsb(&mut targets)));
        }
    }

    let mut rooks = board.pieces_of(us, ROOK);
    while rooks != 0 {
        let from = pop_lsb(&mut rooks);
        let mut targets = rook_attacks(from, occupied) & !own;
        while targets != 0 {
            list.push(moves::make(from, pop_lsb(&mut targets)));
        }
    }

    let mut queens = board.pieces_of(us, QUEEN);
    while queens != 0 {
        let from = pop_lsb(&mut queens);
        let mut targets = queen_attacks(from, occupied) & !own;
        while targets != 0 {
            list.push(moves::make(from, pop_lsb(&mut targets)));
        }
    }
}

/// The king's eight steps.  Castling is generated separately.
fn generate_king_moves(board: &Board, list: &mut MoveList, us: usize) {
    let own = board.colour[us];
    let from = board.kings[us];
    let mut targets = king_attacks(from) & !own;
    while targets != 0 {
        list.push(moves::make(from, pop_lsb(&mut targets)));
    }
}

/// Castles, when the rights are still there, the path is clear and the king does
/// not start, pass through or land on an attacked square.
fn generate_castling(board: &Board, list: &mut MoveList, us: usize, occupied: Bitboard) {
    let them = 1 - us;

    /// One wing's squares.
    struct Wing {
        king_from: usize,
        king_to: usize,
        rook_from: usize,
        /// Squares that must be empty: the king's path, and b1/b8 for the long
        /// castle even though the king never crosses it.
        empty: &'static [usize],
        /// Squares that must not be attacked: the king's walk, including where
        /// it starts.
        walk: &'static [usize],
        rights: u8,
    }

    let wings: [Wing; 2] = if us == WHITE {
        [
            Wing {
                king_from: 4,
                king_to: 6,
                rook_from: 7,
                empty: &[5, 6],
                walk: &[4, 5, 6],
                rights: board::CASTLE_WK,
            },
            Wing {
                king_from: 4,
                king_to: 2,
                rook_from: 0,
                empty: &[1, 2, 3],
                walk: &[4, 3, 2],
                rights: board::CASTLE_WQ,
            },
        ]
    } else {
        [
            Wing {
                king_from: 60,
                king_to: 62,
                rook_from: 63,
                empty: &[61, 62],
                walk: &[60, 61, 62],
                rights: board::CASTLE_BK,
            },
            Wing {
                king_from: 60,
                king_to: 58,
                rook_from: 56,
                empty: &[57, 58, 59],
                walk: &[60, 59, 58],
                rights: board::CASTLE_BQ,
            },
        ]
    };

    for wing in wings {
        if board.castling & wing.rights == 0 {
            continue;
        }
        // The rook has to actually be there: a FEN can claim a right the board
        // does not support.
        if board.piece_on(wing.rook_from) != board::piece_code(us, ROOK) {
            continue;
        }
        if wing
            .empty
            .iter()
            .any(|&sq| occupied & square_bb(sq) != 0)
        {
            continue;
        }
        if wing.walk.iter().any(|&sq| board.is_attacked(sq, them)) {
            continue;
        }
        list.push(moves::make(wing.king_from, wing.king_to));
    }
}

/// Every legal move, by generating pseudo-legal moves and playing each one.
pub fn generate_legal(board: &Board, list: &mut MoveList) {
    let mut pseudo = MoveList::new();
    generate_pseudo_legal(board, &mut pseudo);
    // One probe board for every move: make/unmake pairs restore it exactly, so
    // the copy is reused instead of cloned per move.
    let mut probe = board.clone();
    for mv in pseudo.as_slice() {
        if probe.make_move(*mv) {
            list.push(*mv);
            probe.unmake_move();
        }
    }
}

/// Captures and promotions only, which is what quiescence search needs.
///
/// Quiet promotions count as tactical: a pawn turning into a queen changes the
/// material balance as much as a capture does, so omitting them would hand the
/// horizon effect exactly the material it wants to hide.
pub fn generate_captures(board: &Board, list: &mut MoveList) {
    let us = board.side;
    let occupied = board.occupied();
    let theirs = board.colour[1 - us];
    let (up, promotion_rank) = if us == WHITE { (8i32, 6i32) } else { (-8i32, 1i32) };

    let mut pawns = board.pieces_of(us, PAWN);
    while pawns != 0 {
        let from = pop_lsb(&mut pawns);
        let rank = rank_of(from) as i32;
        let file = file_of(from) as i32;
        let ahead = (from as i32 + up) as usize;
        let promoting = rank == promotion_rank;

        if promoting && !occupied & square_bb(ahead) != 0 {
            for piece in PROMOTION_ORDER {
                list.push(moves::make_promotion(from, ahead, piece));
            }
        }
        for step in [EAST, WEST] {
            if !(0..8).contains(&(file + step)) {
                continue;
            }
            let to = (ahead as i32 + step) as usize;
            if theirs & square_bb(to) != 0 {
                if promoting {
                    for piece in PROMOTION_ORDER {
                        list.push(moves::make_promotion(from, to, piece));
                    }
                } else {
                    list.push(moves::make(from, to));
                }
            } else if board.ep != board::NO_EP && to == board.ep as usize {
                list.push(moves::make(from, to));
            }
        }
    }

    let knights = board.pieces_of(us, KNIGHT);
    let bishops = board.pieces_of(us, BISHOP);
    let rooks = board.pieces_of(us, ROOK);
    let queens = board.pieces_of(us, QUEEN);

    let mut minors = knights | bishops;
    while minors != 0 {
        let from = pop_lsb(&mut minors);
        let attacks = if knights & square_bb(from) != 0 {
            knight_attacks(from)
        } else {
            bishop_attacks(from, occupied)
        };
        let mut targets = attacks & theirs;
        while targets != 0 {
            list.push(moves::make(from, pop_lsb(&mut targets)));
        }
    }

    let mut sliders = rooks | queens;
    while sliders != 0 {
        let from = pop_lsb(&mut sliders);
        let attacks = if rooks & square_bb(from) != 0 {
            rook_attacks(from, occupied)
        } else {
            queen_attacks(from, occupied)
        };
        let mut targets = attacks & theirs;
        while targets != 0 {
            list.push(moves::make(from, pop_lsb(&mut targets)));
        }
    }

    let king_from = board.kings[us];
    let mut targets = king_attacks(king_from) & theirs;
    while targets != 0 {
        list.push(moves::make(king_from, pop_lsb(&mut targets)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::BLACK;
    use crate::board::code_colour;

    fn legal_ucis(fen: &str) -> Vec<String> {
        let board = Board::from_fen(fen).expect("legal FEN");
        let mut list = MoveList::new();
        generate_legal(&board, &mut list);
        let mut out: Vec<String> = list.as_slice().iter().map(|&mv| moves::to_uci(mv)).collect();
        out.sort();
        out
    }

    fn offers(fen: &str, uci: &str) -> bool {
        legal_ucis(fen).iter().any(|candidate| candidate == uci)
    }

    #[test]
    fn the_starting_position_has_twenty_moves() {
        let moves = legal_ucis("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        assert_eq!(moves.len(), 20);
        assert!(moves.contains(&"e2e4".to_string()));
        assert!(moves.contains(&"b1a3".to_string()));
        assert!(!moves.contains(&"g8h6".to_string()), "black moves when it is white's turn");
    }

    #[test]
    fn check_leaves_only_legal_answers() {
        // The black rook on e2 checks the white king down the e-file and covers the
        // whole second rank, so the king may step to d1, to f1 — or take the rook,
        // which is undefended.
        let board = Board::from_fen("4k3/8/8/8/8/8/4r3/4K3 w - - 0 1").expect("legal FEN");
        assert!(board.in_check());
        let answers = board.legal_moves();
        assert_eq!(answers.len(), 3);
        for mv in &answers {
            let mut probe = board.clone();
            assert!(probe.make_move(mv));
            assert!(
                !probe.in_check(),
                "{} still leaves the king in check",
                moves::to_uci(mv)
            );
        }
    }

    #[test]
    fn castling_needs_rights_a_clear_path_and_a_safe_walk() {
        let both = "r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1";
        assert!(offers(both, "e1g1"));
        assert!(offers(both, "e1c1"));
        // A bishop on f1 blocks the short castle, not the long one.
        let blocked_short = "r3k2r/8/8/8/8/8/8/R3KB1R w KQkq - 0 1";
        assert!(!offers(blocked_short, "e1g1"));
        assert!(offers(blocked_short, "e1c1"));
        // A rook between king and queen rook blocks the long castle.
        assert!(!offers("r3k2r/8/8/8/8/8/8/R2RK3 w KQkq - 0 1", "e1c1"));
        // A rook covering e1 means the king may not castle at all.
        assert!(!offers("4r3/8/8/8/8/8/8/R3K2R w KQ - 0 1", "e1g1"));
        // A rook covering f1 kills only the short castle.
        assert!(!offers("5r2/8/8/8/8/8/8/R3K2R w KQ - 0 1", "e1g1"));
        assert!(offers("5r2/8/8/8/8/8/8/R3K2R w KQ - 0 1", "e1c1"));
        // Rights with no rook: including the right in a FEN is tolerated, but no
        // castle is offered.
        assert!(!offers("4k3/8/8/8/8/8/8/4K3 w KQ - 0 1", "e1g1"));
    }

    #[test]
    fn en_passant_removes_the_pawn_behind_the_target() {
        let fen = "rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 3";
        assert!(offers(fen, "e5d6"));
        let board = Board::from_fen(fen).unwrap();
        let mut probe = board.clone();
        assert!(probe.make_move(moves::from_uci("e5d6").unwrap()));
        assert_eq!(probe.pieces_of(BLACK, PAWN).count_ones(), 7);
        assert_eq!(probe.pieces_of(WHITE, PAWN).count_ones(), 8);
        // ...and unmaking puts the black pawn back where it stood.
        assert!(probe.unmake_move());
        assert_eq!(probe.to_fen(), board.to_fen());
    }

    #[test]
    fn promotions_offer_all_four_pieces_for_both_colours() {
        // Both pawns are one step from promoting, and both promotion squares are
        // empty: the kings stand on e1 and g1, out of the way.
        let white = legal_ucis("8/P7/8/8/8/8/7p/4K1k1 w - - 0 1");
        for piece in ["q", "r", "b", "n"] {
            assert!(white.contains(&format!("a7a8{}", piece)), "white {}", piece);
        }
        let black = legal_ucis("8/P7/8/8/8/8/7p/4K1k1 b - - 0 1");
        for piece in ["q", "r", "b", "n"] {
            assert!(black.contains(&format!("h2h1{}", piece)), "black {}", piece);
        }
    }

    #[test]
    fn captures_only_lists_tactical_moves() {
        let board =
            Board::from_fen("rnbqkbnr/ppp1pppp/8/3p4/4P3/8/PPPP1PPP/RNBQKBNR w KQkq - 0 2").unwrap();
        let mut list = MoveList::new();
        generate_captures(&board, &mut list);
        let found: Vec<String> = list.as_slice().iter().map(|&mv| moves::to_uci(mv)).collect();
        assert_eq!(found, vec!["e4d5".to_string()]);
    }

    #[test]
    fn every_generated_move_starts_on_one_of_our_pieces() {
        // The generator's one structural promise: a move's from-square holds the
        // piece that is moving.  Walking a few plies of each suite position is
        // enough to catch a generator that invents a move from an empty square.
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        ] {
            let mut board = Board::from_fen(fen).expect("legal FEN");
            walk_generation(&mut board, 3, fen);
        }
    }

    fn walk_generation(board: &mut Board, depth: i32, root: &str) {
        let mut list = MoveList::new();
        generate_pseudo_legal(board, &mut list);
        for mv in list.as_slice() {
            let from = moves::from_square(*mv);
            let code = board.piece_on(from);
            assert!(
                code != crate::board::NO_PIECE && code_colour(code) == board.side_to_move(),
                "{} starts on an empty square in {} (from {})",
                moves::to_uci(*mv),
                board.to_fen(),
                root
            );
        }
        if depth <= 1 {
            return;
        }
        for mv in list.as_slice() {
            if board.make_move(*mv) {
                walk_generation(board, depth - 1, root);
                board.unmake_move();
            }
        }
    }

    #[test]
    fn a_king_may_not_walk_into_check() {
        // The black rook on e8 covers the e-file, so e2 is off limits even though
        // nothing occupies it.
        let moves = legal_ucis("4r3/8/8/8/8/8/8/4K3 w - - 0 1");
        assert!(!moves.contains(&"e1e2".to_string()));
        assert!(moves.contains(&"e1d1".to_string()));
    }

    #[test]
    fn a_pinned_piece_may_not_leave_the_line() {
        // The white knight on e2 is pinned to the king on e1 by the rook on e8, so
        // no knight move is available — and a knight can never stay on the file.
        let moves = legal_ucis("4r3/8/8/8/8/8/4N3/4K3 w - - 0 1");
        assert!(!moves.iter().any(|mv| mv.starts_with("e2")));
        // The king may still step aside, which is what proves the knight is the
        // only piece being restricted.
        assert!(moves.contains(&"e1d1".to_string()));
    }
}



