//! Static exchange evaluation.
//!
//! SEE answers "what does this capture win or lose if both sides keep recapturing
//! on that square, taking with the cheapest piece each time?"  It is what separates
//! taking a pawn with a queen *because the pawn is defended* from taking it because
//! it is not.
//!
//! The implementation is a minimax over the capture sequence, driven by the board's
//! own attacker queries: at each step the side to move takes with its cheapest
//! attacker, that piece leaves the occupancy, and the question repeats.  `see_ge` is
//! the same walk with the threshold in hand, so it stops as soon as the answer is
//! settled — most calls resolve in a ply or two.
//!
//! The test at the bottom checks both against a deliberately naive reference: a
//! recursion that clones the board and really plays the sequence.  Two
//! implementations, one of them obviously correct, agreeing on every legal capture
//! of a set of positions is the evidence that this one works.

use crate::bitboard::{pop_lsb, square_bb, Bitboard};
use crate::board::{self, Board, KING, PAWN};
use crate::moves::{self, Move};

/// The piece values SEE trades in.  The king is never captured, so its entry only
/// has to be larger than everything else.
pub const SEE_VALUES: [i32; 6] = [100, 320, 330, 500, 900, 20_000];

/// The static exchange value of `mv`, from the moving side's point of view.
///
/// Positive means the sequence wins material, negative means it loses it, and zero
/// means an even trade.
pub fn see_value(board: &Board, mv: Move) -> i32 {
    let to = moves::to_square(mv);
    let from = moves::from_square(mv);

    // What the move wins outright, before any recapture.
    let target = victim_value(board, mv);

    // The first capture has already happened, so the opponent is to move and the
    // moving piece has left its square.  A promotion arrives as the piece it
    // promotes to, which is why the value is read that way round.
    let occupant = occupant_value(board, mv);
    let occupied = board.occupied() & !square_bb(from);
    target - exchange(board, to, occupied, 1 - board.side_to_move(), occupant).max(0)
}

/// True when the capture's exchange value reaches `threshold`.
///
/// A capture that wins material outright, or loses it whatever happens, is settled
/// without walking the sequence at all: the king's value bounds what any recapture
/// can swing, so one comparison answers those cases.
pub fn see_ge(board: &Board, mv: Move, threshold: i32) -> bool {
    let target = victim_value(board, mv);
    if target - SEE_VALUES[KING] >= threshold {
        return true;
    }
    if target + SEE_VALUES[KING] < threshold {
        return false;
    }

    let to = moves::to_square(mv);
    let from = moves::from_square(mv);
    let occupant = occupant_value(board, mv);
    let occupied = board.occupied() & !square_bb(from);
    target - exchange(board, to, occupied, 1 - board.side_to_move(), occupant) >= threshold
}

/// What the piece standing on the destination square is worth after the move: a
/// promoted pawn is worth the piece it became.
fn occupant_value(board: &Board, mv: Move) -> i32 {
    if moves::is_promotion(mv) {
        SEE_VALUES[moves::promotion_piece(mv)]
    } else {
        SEE_VALUES[board::code_kind(board.piece_on(moves::from_square(mv)))]
    }
}

/// The most the side to move can win by capturing on `to`, where the piece standing
/// there is worth `target`, always taking with its cheapest attacker.
///
/// Returns 0 when nothing can capture there, which is what ends the walk.  The walk
/// also stops when the cheapest attacker is a king: a king cannot be taken back, so
/// its capture settles the exchange and no further arithmetic is needed.
fn exchange(board: &Board, to: usize, occupied: Bitboard, side: usize, target: i32) -> i32 {
    let attackers = board.attackers_to(to, side, occupied) & occupied;
    if attackers == 0 {
        return 0;
    }
    let (attacker_square, attacker_kind) = cheapest_attacker(attackers, board);
    if attacker_kind == KING {
        return target;
    }

    // The opponent now faces the same question, with this piece standing on `to`.
    let reply = exchange(
        board,
        to,
        occupied & !square_bb(attacker_square),
        1 - side,
        SEE_VALUES[attacker_kind],
    );
    (target - reply).max(0)
}

/// The least valuable attacker in a set; among equals, the lowest square, so the
/// walk is deterministic.
fn cheapest_attacker(attackers: Bitboard, board: &Board) -> (usize, usize) {
    let mut best_square = 0;
    let mut best_kind = KING;
    let mut best_value = i32::MAX;
    let mut remaining = attackers;
    while remaining != 0 {
        let square = pop_lsb(&mut remaining);
        let kind = board::code_kind(board.piece_on(square));
        let value = SEE_VALUES[kind];
        if value < best_value || (value == best_value && square < best_square) {
            best_value = value;
            best_kind = kind;
            best_square = square;
        }
    }
    (best_square, best_kind)
}

/// What `mv` takes: a pawn for an en passant capture, otherwise whatever stands on
/// the destination, and nothing at all for a quiet move.
fn victim_value(board: &Board, mv: Move) -> i32 {
    let to = moves::to_square(mv);
    let from = moves::from_square(mv);
    let victim = board.piece_on(to);
    if victim != board::NO_PIECE {
        return SEE_VALUES[board::code_kind(victim)];
    }
    // En passant is the one capture whose victim is not on the destination square.
    let moving = board::code_kind(board.piece_on(from));
    if moving == PAWN && moves::is_diagonal(mv) && board.ep != board::NO_EP && to == board.ep as usize
    {
        return SEE_VALUES[PAWN];
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{NO_PIECE, ROOK};
    use crate::moves::{to_square, MoveList};

    /// The reference SEE: clone the board, play the capture, then let each side
    /// recapture with its cheapest attacker, alternating until nobody can.
    ///
    /// It plays *pseudo-legal* captures, not legal ones, because SEE is a static
    /// estimate that ignores pins: a rook that cannot legally recapture because it
    /// would expose its king still counts here.  The point of the reference is not to
    /// be a better evaluator than SEE but to execute the same rule with real board
    /// moves instead of occupancy arithmetic, which is what makes agreeing with it
    /// evidence that the arithmetic is right.
    ///
    /// Only the *reply* is clamped at zero: the first capture itself may well lose
    /// material, which is the whole point of asking.
    fn reference_see(board: &Board, mv: Move) -> i32 {
        let target = victim_value(board, mv);
        let to = moves::to_square(mv);
        let mut probe = board.clone();
        if probe.piece_on(to) == king_code(probe.side_to_move()) {
            return 0;
        }
        probe.make_move_unchecked(mv);
        let value = target - reference_exchange(&mut probe, to).max(0);
        probe.unmake_move();
        value
    }

    fn king_code(colour: usize) -> u8 {
        board::piece_code(colour, crate::board::KING)
    }

    /// The most the side to move wins by capturing the piece on `square`, taking
    /// with its cheapest attacker; a king's capture ends the sequence.
    fn reference_exchange(board: &mut Board, square: usize) -> i32 {
        let occupant = board.piece_on(square);
        if occupant == NO_PIECE {
            return 0;
        }
        let target = SEE_VALUES[board::code_kind(occupant)];

        let mut chosen: Option<Move> = None;
        let mut chosen_value = i32::MAX;
        let mut list = MoveList::new();
        crate::movegen::generate_pseudo_legal(board, &mut list);
        for mv in list.as_slice() {
            if to_square(*mv) != square {
                continue;
            }
            let victim = board.piece_on(square);
            if victim != NO_PIECE && board::code_kind(victim) == crate::board::KING {
                // Capturing a king is not part of the exchange.
                continue;
            }
            let kind = board::code_kind(board.piece_on(moves::from_square(*mv)));
            if SEE_VALUES[kind] < chosen_value {
                chosen_value = SEE_VALUES[kind];
                chosen = Some(*mv);
            }
        }

        let mv = match chosen {
            Some(mv) => mv,
            None => return 0,
        };
        if chosen_value == SEE_VALUES[crate::board::KING] {
            return target;
        }
        board.make_move_unchecked(mv);
        let reply = reference_exchange(board, square);
        board.unmake_move();
        (target - reply).max(0)
    }

    /// Every legal capture in the position.
    fn captures_of(board: &Board) -> Vec<Move> {
        let legal = board.legal_moves();
        let mut list = MoveList::new();
        crate::movegen::generate_captures(board, &mut list);
        list.as_slice()
            .iter()
            .copied()
            .filter(|mv| legal.contains(*mv))
            .collect()
    }

    /// Deterministic xorshift, so a failure reproduces.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
    }

    #[test]
    fn a_free_pawn_is_worth_a_pawn() {
        // The knight on c3 takes a pawn on d5 that nothing defends.
        let board = Board::from_fen("4k3/8/8/3p4/8/2N5/8/4K3 w - - 0 1").unwrap();
        let mv = moves::from_uci("c3d5").unwrap();
        assert_eq!(see_value(&board, mv), SEE_VALUES[PAWN]);
        assert!(see_ge(&board, mv, 0));
        assert!(see_ge(&board, mv, SEE_VALUES[PAWN]));
        assert!(!see_ge(&board, mv, SEE_VALUES[PAWN] + 1));
    }

    #[test]
    fn a_defended_pawn_costs_the_piece_that_took_it() {
        // A rook takes a pawn on d5 that the pawn on e6 defends: rook for pawn.
        let board = Board::from_fen("4k3/8/4p3/3p4/8/8/8/3RK3 w - - 0 1").unwrap();
        let mv = moves::from_uci("d1d5").unwrap();
        assert_eq!(see_value(&board, mv), SEE_VALUES[PAWN] - SEE_VALUES[ROOK]);
        assert!(!see_ge(&board, mv, 0));
    }

    #[test]
    fn a_recapture_chain_ends_with_the_side_holding_the_last_attacker() {
        // Rook takes a pawn on d5, the knight on c7 recaptures it, and the bishop
        // on g2 takes the knight back: pawn - rook + knight = -80 for the rook.
        let board = Board::from_fen("4k3/2n5/8/3p4/8/8/6B1/3RK3 w - - 0 1").unwrap();
        let mv = moves::from_uci("d1d5").unwrap();
        assert_eq!(
            see_value(&board, mv),
            SEE_VALUES[PAWN] + SEE_VALUES[board::KNIGHT] - SEE_VALUES[board::ROOK]
        );
        assert!(!see_ge(&board, mv, 0));
    }

    #[test]
    fn a_queen_taking_into_two_defenders_is_negative() {
        // Queen takes a pawn on d5 defended by pawns on c6 and e6.
        let board = Board::from_fen("4k3/8/2p1p3/3p4/8/8/8/3QK3 w - - 0 1").unwrap();
        let mv = moves::from_uci("d1d5").unwrap();
        assert!(see_value(&board, mv) < 0, "valued {}", see_value(&board, mv));
        assert!(!see_ge(&board, mv, 0));
    }

    #[test]
    fn an_en_passant_capture_sees_the_recapture_that_follows_it() {
        // White pawn e5 takes d6 en passant, and the black queen on d8 takes it
        // back down the open file: even, so not worth a node.  (This is precisely
        // the difference between SEE and a value comparison: the capture itself
        // looks free until the recapture is accounted for.)
        let board =
            Board::from_fen("rnbqkbnr/ppp1pppp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 3").unwrap();
        let mv = moves::from_uci("e5d6").unwrap();
        assert_eq!(see_value(&board, mv), 0);
    }

    /// The same capture with nothing able to recapture on the landing square.
    #[test]
    fn an_en_passant_capture_nothing_defends_is_worth_a_pawn() {
        // Nothing black owns reaches d6: the b8 and g8 knights do not, the c8 bishop
        // stops on d7, and there is no pawn on c7 or e7 to take back.
        let board =
            Board::from_fen("1nb1k1nr/pp4pp/8/3pP3/8/8/PPPP1PPP/RNBQKBNR w KQkq d6 0 3").unwrap();
        let mv = moves::from_uci("e5d6").unwrap();
        assert_eq!(see_value(&board, mv), SEE_VALUES[PAWN]);
        assert!(see_ge(&board, mv, 0));
    }

    #[test]
    fn a_promotion_capture_counts_the_new_piece_as_a_capturer() {
        // A pawn takes a rook on b8 and becomes a queen, and the knight on c6 can
        // take it back: rook - queen.
        let board = Board::from_fen("1r2k3/1P6/2n5/8/8/8/8/4K3 w - - 0 1").unwrap();
        let mv = moves::make_promotion(
            crate::bitboard::parse_square("b7").unwrap(),
            crate::bitboard::parse_square("b8").unwrap(),
            moves::PROMO_QUEEN_INDEX,
        );
        assert_eq!(
            see_value(&board, mv),
            SEE_VALUES[board::ROOK] - SEE_VALUES[board::QUEEN]
        );
    }

    #[test]
    fn the_value_matches_a_reference_on_every_legal_capture() {
        let mut rng = Rng(0x51ed_270b_1a2b_3c4d);
        let positions = [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
        ];
        let mut checked = 0;
        for fen in positions {
            // Walk a few random legal plies, so the captures are not all opening ones.
            let mut board = Board::from_fen(fen).unwrap();
            for _ in 0..14 {
                for mv in captures_of(&board) {
                    let value = see_value(&board, mv);
                    assert_eq!(
                        value,
                        reference_see(&board, mv),
                        "{} in {} disagrees with the reference",
                        moves::to_uci(mv),
                        board.to_fen()
                    );
                    // The threshold form has to agree with the value everywhere,
                    // including at the ends of the range where it short-circuits.
                    for threshold in [-1_000, -500, -1, 0, 1, 100, 500, 1_000] {
                        assert_eq!(
                            see_ge(&board, mv, threshold),
                            value >= threshold,
                            "{} at threshold {} in {}",
                            moves::to_uci(mv),
                            threshold,
                            board.to_fen()
                        );
                    }
                    checked += 1;
                }
                let legal = board.legal_moves();
                if legal.is_empty() {
                    break;
                }
                let mut probe = board.clone();
                assert!(probe.make_move(legal.get((rng.next() as usize) % legal.len())));
                board = probe;
            }
        }
        assert!(checked > 40, "only checked {} captures", checked);
    }
}

