//! Move encoding.
//!
//! A move is a 16-bit value, which is what lets the generator fill a
//! `[u16; 256]` array on the stack instead of a heap-allocated vector:
//!
//! ```text
//! bits  0-5   from square (0 = a1)
//! bits  6-11  to square
//! bits 12-13  promotion piece: 0 = queen, 1 = knight, 2 = bishop, 3 = rook
//! bit  14     set when the move is a promotion at all
//! bit  15     unused, always 0
//! ```
//!
//! The two special moves are derived rather than stored, because the board can
//! tell them apart without help: an en passant capture is a diagonal pawn move
//! onto the en passant square, and a castle is a king moving two files.

use crate::bitboard::{file_of, parse_square, rank_of, square_name};

pub type Move = u16;

/// Promotion flag: a pawn reaching the last rank always sets this.
pub const PROMO_FLAG: Move = 1 << 14;
/// The promotion field occupies bits 12-13.
pub const PROMO_MASK: Move = 0b11 << 12;
pub const PROMO_QUEEN: Move = 0;
pub const PROMO_KNIGHT: Move = 1 << 12;
pub const PROMO_BISHOP: Move = 2 << 12;
pub const PROMO_ROOK: Move = 3 << 12;

/// Piece indices as the promotion field spells them, matching
/// `board::PAWN`..`board::KING` for the four pieces a pawn can become.
pub const PROMO_KNIGHT_INDEX: usize = 1;
pub const PROMO_BISHOP_INDEX: usize = 2;
pub const PROMO_ROOK_INDEX: usize = 3;
pub const PROMO_QUEEN_INDEX: usize = 4;

/// A legal position never has more moves than this, so the search never has to
/// allocate a move list.
pub const MAX_MOVES: usize = 256;

/// Builds a quiet move.  Castles and en passant captures need no extra bits.
#[inline]
pub const fn make(from: usize, to: usize) -> Move {
    (from as Move) | ((to as Move) << 6)
}

/// Builds a promotion, `piece` being a `PROMO_*_INDEX`.
#[inline]
pub const fn make_promotion(from: usize, to: usize, piece: usize) -> Move {
    let field = match piece {
        PROMO_KNIGHT_INDEX => PROMO_KNIGHT,
        PROMO_BISHOP_INDEX => PROMO_BISHOP,
        PROMO_ROOK_INDEX => PROMO_ROOK,
        _ => PROMO_QUEEN,
    };
    make(from, to) | PROMO_FLAG | field
}

#[inline]
pub const fn from_square(mv: Move) -> usize {
    (mv & 0x3f) as usize
}

#[inline]
pub const fn to_square(mv: Move) -> usize {
    ((mv >> 6) & 0x3f) as usize
}

#[inline]
pub const fn is_promotion(mv: Move) -> bool {
    mv & PROMO_FLAG != 0
}

/// The piece a promotion turns into, as a `PROMO_*_INDEX`.
#[inline]
pub const fn promotion_piece(mv: Move) -> usize {
    match mv & PROMO_MASK {
        PROMO_KNIGHT => PROMO_KNIGHT_INDEX,
        PROMO_BISHOP => PROMO_BISHOP_INDEX,
        PROMO_ROOK => PROMO_ROOK_INDEX,
        _ => PROMO_QUEEN_INDEX,
    }
}

/// True when the move lands a pawn one file away from where it started.
#[inline]
pub const fn is_diagonal(mv: Move) -> bool {
    let from_file = file_of(from_square(mv));
    let to_file = file_of(to_square(mv));
    (from_file as i32 - to_file as i32).abs() == 1
}

/// True when a pawn moved two ranks, which is what sets the en passant square.
#[inline]
pub const fn is_double_push(mv: Move) -> bool {
    let from_rank = rank_of(from_square(mv)) as i32;
    let to_rank = rank_of(to_square(mv)) as i32;
    (from_rank - to_rank).abs() == 2
}

/// True when a king moved two files, which is the only way to castle.
#[inline]
pub const fn is_castle(mv: Move) -> bool {
    let from_file = file_of(from_square(mv)) as i32;
    let to_file = file_of(to_square(mv)) as i32;
    (from_file - to_file).abs() == 2
}

/// `e2e4`, or `e7e8q` for a promotion.
pub fn to_uci(mv: Move) -> String {
    let mut out = square_name(from_square(mv));
    out.push_str(&square_name(to_square(mv)));
    if is_promotion(mv) {
        out.push(promotion_letter(mv));
    }
    out
}

/// The lower-case letter a promotion is written with.
pub const fn promotion_letter(mv: Move) -> char {
    match promotion_piece(mv) {
        PROMO_KNIGHT_INDEX => 'n',
        PROMO_BISHOP_INDEX => 'b',
        PROMO_ROOK_INDEX => 'r',
        _ => 'q',
    }
}

/// Parses `e2e4` / `e7e8q`.  Anything else is rejected, including the null move
/// (the search uses that as a sentinel, never as input).
pub fn from_uci(text: &str) -> Option<Move> {
    let text = text.trim();
    if text.len() < 4 || text.len() > 5 {
        return None;
    }
    let from = parse_square(&text[0..2])?;
    let to = parse_square(&text[2..4])?;
    if text.len() == 4 {
        return Some(make(from, to));
    }
    let piece = match text.as_bytes()[4].to_ascii_lowercase() {
        b'n' => PROMO_KNIGHT_INDEX,
        b'b' => PROMO_BISHOP_INDEX,
        b'r' => PROMO_ROOK_INDEX,
        b'q' => PROMO_QUEEN_INDEX,
        _ => return None,
    };
    Some(make_promotion(from, to, piece))
}

/// A fixed-capacity move list.  Nothing here allocates: the search holds one per
/// ply and clears it on the way back up.
#[derive(Clone)]
pub struct MoveList {
    moves: [Move; MAX_MOVES],
    len: usize,
}

impl MoveList {
    pub fn new() -> Self {
        Self {
            moves: [0; MAX_MOVES],
            len: 0,
        }
    }

    #[inline]
    pub fn push(&mut self, mv: Move) {
        // A legal position never comes close to 256 moves; the assert stays
        // because a silent overflow here would be a search bug that is very hard
        // to see from the outside.
        debug_assert!(self.len < MAX_MOVES, "move list overflow");
        self.moves[self.len] = mv;
        self.len += 1;
    }

    #[inline]
    pub fn clear(&mut self) {
        self.len = 0;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn get(&self, index: usize) -> Move {
        self.moves[index]
    }

    /// Overwrites an entry.  The search uses this to write a reordered root list
    /// back into the list it came from.
    #[inline]
    pub fn set(&mut self, index: usize, mv: Move) {
        debug_assert!(index < self.len);
        self.moves[index] = mv;
    }

    #[inline]
    pub fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }

    #[inline]
    pub fn swap(&mut self, first: usize, second: usize) {
        self.moves.swap(first, second);
    }

    pub fn contains(&self, mv: Move) -> bool {
        self.as_slice().contains(&mv)
    }

    /// True when one of the moves satisfies `predicate`, used by the stalemate
    /// and checkmate checks.
    pub fn has(&self, predicate: impl Fn(Move) -> bool) -> bool {
        self.as_slice().iter().copied().any(predicate)
    }
}

impl Default for MoveList {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> IntoIterator for &'a MoveList {
    type Item = Move;
    type IntoIter = std::iter::Copied<std::slice::Iter<'a, Move>>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter().copied()
    }
}

/// The four promotion choices, in the order the UI's picker offers them.
pub const PROMOTION_CHOICES: [&str; 4] = ["q", "r", "b", "n"];

#[cfg(test)]
mod tests {
    use super::*;

    const KNIGHT: usize = PROMO_KNIGHT_INDEX;
    const QUEEN: usize = PROMO_QUEEN_INDEX;

    #[test]
    fn every_field_survives_a_round_trip() {
        for from in 0..64 {
            for to in 0..64 {
                for piece in [KNIGHT, PROMO_BISHOP_INDEX, PROMO_ROOK_INDEX, QUEEN] {
                    let mv = make_promotion(from, to, piece);
                    assert_eq!(from_square(mv), from);
                    assert_eq!(to_square(mv), to);
                    assert!(is_promotion(mv));
                    assert_eq!(promotion_piece(mv), piece);
                }
                let quiet = make(from, to);
                assert_eq!(from_square(quiet), from);
                assert_eq!(to_square(quiet), to);
                assert!(!is_promotion(quiet));
            }
        }
    }

    #[test]
    fn uci_round_trips_including_promotions() {
        assert_eq!(to_uci(make(12, 28)), "e2e4");
        assert_eq!(from_uci("e2e4"), Some(make(12, 28)));
        assert_eq!(to_uci(make_promotion(52, 60, KNIGHT)), "e7e8n");
        assert_eq!(from_uci("e7e8n"), Some(make_promotion(52, 60, KNIGHT)));
        assert_eq!(to_uci(make_promotion(52, 60, QUEEN)), "e7e8q");
        assert_eq!(from_uci("e7e8q"), Some(make_promotion(52, 60, QUEEN)));
        // Case-insensitive: the UI's picker sends lower case, some GUIs upper.
        assert_eq!(from_uci("E7E8Q"), Some(make_promotion(52, 60, QUEEN)));
    }

    #[test]
    fn malformed_uci_is_rejected() {
        for text in ["", "e2", "e2e", "e2e44", "e2e9", "z2z4", "e2e4k"] {
            assert_eq!(from_uci(text), None, "{} should not parse", text);
        }
    }

    #[test]
    fn derived_specials_agree_with_the_board_events() {
        // e2-e4 is a double push, d4xe5 is a diagonal step, e1-g1 is a castle.
        let double = make(12, 28);
        assert!(is_double_push(double));
        assert!(!is_castle(double));
        let ep = make(27, 36);
        assert!(is_diagonal(ep));
        assert!(!is_double_push(ep));
        let castle = make(4, 6);
        assert!(is_castle(castle));
        assert!(!is_diagonal(castle));
    }

    #[test]
    fn the_move_list_keeps_order_and_reports_its_size() {
        let mut list = MoveList::new();
        assert!(list.is_empty());
        list.push(make(0, 8));
        list.push(make(52, 60));
        list.push(make_promotion(52, 60, KNIGHT));
        assert_eq!(list.len(), 3);
        assert_eq!(list.as_slice()[1], make(52, 60));
        assert!(list.contains(make(0, 8)));
        assert!(!list.contains(make(1, 9)));
        assert!(list.has(|mv| to_square(mv) == 60));
        assert!(!list.has(|mv| to_square(mv) == 61));
        list.clear();
        assert!(list.is_empty());
    }
}


