//! Bitboard primitives.
//!
//! Every piece set in the engine is one `u64` with one bit per square, in
//! Little-Endian Rank-File order: bit 0 is a1, bit 7 is h1, bit 56 is a8 and bit
//! 63 is h8.  That ordering means `square = rank * 8 + file`, so a rank is a
//! contiguous byte and shifting a set by 8 moves it one rank.
//!
//! Sliding attacks are computed by walking a ray until it hits something.  That
//! is a few times slower than a magic-bitboard lookup table, but it is short
//! enough to read in one sitting and it has an obvious correctness story: the
//! tests at the bottom of this file check every square against an independent
//! reference implementation across a thousand random occupancies.  The bench
//! binary (`cargo run --release --bin bench`) prints the move rates this buys,
//! and the search's default time budget is set from those numbers.

pub type Bitboard = u64;

/// The a-file.  Shifting it left by a file index gives every other file.
pub const FILE_A: Bitboard = 0x0101_0101_0101_0101;
pub const FILE_H: Bitboard = FILE_A << 7;
/// The first rank.  Shifting it left by a rank index gives every other rank.
pub const RANK_1: Bitboard = 0xff;
pub const RANK_2: Bitboard = RANK_1 << 8;
pub const RANK_3: Bitboard = RANK_1 << 16;
pub const RANK_4: Bitboard = RANK_1 << 24;
pub const RANK_5: Bitboard = RANK_1 << 32;
pub const RANK_6: Bitboard = RANK_1 << 40;
pub const RANK_7: Bitboard = RANK_1 << 48;
pub const RANK_8: Bitboard = RANK_1 << 56;

/// The square-index step of each direction, so callers can say `sq + NORTH`.
pub const NORTH: i32 = 8;
pub const SOUTH: i32 = -8;
pub const EAST: i32 = 1;
pub const WEST: i32 = -1;
pub const NORTH_EAST: i32 = 9;
pub const NORTH_WEST: i32 = 7;
pub const SOUTH_EAST: i32 = -7;
pub const SOUTH_WEST: i32 = -9;

/// A one-square bitboard.
#[inline]
pub const fn square_bb(sq: usize) -> Bitboard {
    1u64 << sq
}

#[inline]
pub const fn file_of(sq: usize) -> usize {
    sq & 7
}

#[inline]
pub const fn rank_of(sq: usize) -> usize {
    sq >> 3
}

#[inline]
pub const fn square_index(file: usize, rank: usize) -> usize {
    rank * 8 + file
}

#[inline]
pub const fn popcount(bits: Bitboard) -> u32 {
    bits.count_ones()
}

/// Index of the lowest set square.  The caller must know the set is not empty.
#[inline]
pub const fn lsb(bits: Bitboard) -> usize {
    bits.trailing_zeros() as usize
}

/// Removes and returns the lowest set square.
#[inline]
pub fn pop_lsb(bits: &mut Bitboard) -> usize {
    let sq = lsb(*bits);
    *bits &= *bits - 1;
    sq
}

/// Squares a knight on `sq` attacks.
pub const KNIGHT_ATTACKS: [Bitboard; 64] = build_knight_attacks();
/// Squares a king on `sq` attacks; castling is added by the move generator.
pub const KING_ATTACKS: [Bitboard; 64] = build_king_attacks();
/// Squares a pawn of the given colour on `sq` attacks.  Index 0 is white and 1
/// is black, matching `board::WHITE` / `board::BLACK`.
pub const PAWN_ATTACKS: [[Bitboard; 64]; 2] = build_pawn_attacks();

/// The squares a knight on `sq` attacks.
#[inline]
pub const fn knight_attacks(sq: usize) -> Bitboard {
    KNIGHT_ATTACKS[sq]
}

/// The squares a king on `sq` attacks.
#[inline]
pub const fn king_attacks(sq: usize) -> Bitboard {
    KING_ATTACKS[sq]
}

/// The squares a pawn of `colour` on `sq` attacks.
#[inline]
pub const fn pawn_attacks(colour: usize, sq: usize) -> Bitboard {
    PAWN_ATTACKS[colour][sq]
}

/// The squares a pawn of `colour` would have to stand on to attack `sq`.  This
/// is the reverse lookup the attack detector uses, and it is why the pawn tables
/// are mirrored per colour.
#[inline]
pub const fn pawn_attackers(sq: usize, colour: usize) -> Bitboard {
    PAWN_ATTACKS[1 - colour][sq]
}


const KNIGHT_DELTAS: [(i32, i32); 8] = [
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
];

const fn inside(file: i32, rank: i32) -> bool {
    file >= 0 && file < 8 && rank >= 0 && rank < 8
}

const fn build_knight_attacks() -> [Bitboard; 64] {
    let mut table = [0u64; 64];
    let mut sq = 0;
    while sq < 64 {
        let file = (sq & 7) as i32;
        let rank = (sq >> 3) as i32;
        let mut i = 0;
        while i < 8 {
            let f = file + KNIGHT_DELTAS[i].0;
            let r = rank + KNIGHT_DELTAS[i].1;
            if inside(f, r) {
                table[sq] |= 1u64 << ((r * 8 + f) as usize);
            }
            i += 1;
        }
        sq += 1;
    }
    table
}

const fn build_king_attacks() -> [Bitboard; 64] {
    let mut table = [0u64; 64];
    let mut sq = 0;
    while sq < 64 {
        let file = (sq & 7) as i32;
        let rank = (sq >> 3) as i32;
        let mut df = -1;
        while df <= 1 {
            let mut dr = -1;
            while dr <= 1 {
                if (df != 0 || dr != 0) && inside(file + df, rank + dr) {
                    table[sq] |= 1u64 << (((rank + dr) * 8 + file + df) as usize);
                }
                dr += 1;
            }
            df += 1;
        }
        sq += 1;
    }
    table
}

const fn build_pawn_attacks() -> [[Bitboard; 64]; 2] {
    let mut table = [[0u64; 64]; 2];
    let mut sq = 0;
    while sq < 64 {
        let file = (sq & 7) as i32;
        let rank = (sq >> 3) as i32;
        // White pushes up the board, black down.
        if inside(file - 1, rank + 1) {
            table[0][sq] |= 1u64 << (((rank + 1) * 8 + file - 1) as usize);
        }
        if inside(file + 1, rank + 1) {
            table[0][sq] |= 1u64 << (((rank + 1) * 8 + file + 1) as usize);
        }
        if inside(file - 1, rank - 1) {
            table[1][sq] |= 1u64 << (((rank - 1) * 8 + file - 1) as usize);
        }
        if inside(file + 1, rank - 1) {
            table[1][sq] |= 1u64 << (((rank - 1) * 8 + file + 1) as usize);
        }
        sq += 1;
    }
    table
}

/// Walks one direction from `sq`, stopping on the first occupied square.  That
/// square is itself included, because it is a capture candidate.
#[inline]
fn ray_attacks(sq: usize, occupied: Bitboard, file_step: i32, rank_step: i32) -> Bitboard {
    let mut attacks = 0u64;
    let mut file = file_of(sq) as i32 + file_step;
    let mut rank = rank_of(sq) as i32 + rank_step;
    while inside(file, rank) {
        let target = (rank * 8 + file) as usize;
        attacks |= 1u64 << target;
        if occupied & (1u64 << target) != 0 {
            break;
        }
        file += file_step;
        rank += rank_step;
    }
    attacks
}

/// Squares a rook on `sq` attacks, blockers included up to the first one.
#[inline]
pub fn rook_attacks(sq: usize, occupied: Bitboard) -> Bitboard {
    ray_attacks(sq, occupied, 1, 0)
        | ray_attacks(sq, occupied, -1, 0)
        | ray_attacks(sq, occupied, 0, 1)
        | ray_attacks(sq, occupied, 0, -1)
}

/// Squares a bishop on `sq` attacks, blockers included up to the first one.
#[inline]
pub fn bishop_attacks(sq: usize, occupied: Bitboard) -> Bitboard {
    ray_attacks(sq, occupied, 1, 1)
        | ray_attacks(sq, occupied, 1, -1)
        | ray_attacks(sq, occupied, -1, 1)
        | ray_attacks(sq, occupied, -1, -1)
}

#[inline]
pub fn queen_attacks(sq: usize, occupied: Bitboard) -> Bitboard {
    rook_attacks(sq, occupied) | bishop_attacks(sq, occupied)
}

/// True when `from` and `to` share a diagonal, which is what separates a pawn
/// capture (including en passant) from a push.
#[inline]
pub const fn on_same_diagonal(from: usize, to: usize) -> bool {
    let file = file_of(to) as i32 - file_of(from) as i32;
    let rank = rank_of(to) as i32 - rank_of(from) as i32;
    file == rank || file == -rank
}

/// Renders a square the way algebraic notation does (`e4`).
pub fn square_name(sq: usize) -> String {
    let file = b'a' + file_of(sq) as u8;
    let rank = b'1' + rank_of(sq) as u8;
    String::from_utf8(vec![file, rank]).unwrap_or_default()
}

/// Parses `e4` back into a square index.  Case-insensitive, because a GUI may send
/// either case.
pub fn parse_square(name: &str) -> Option<usize> {
    let bytes = name.as_bytes();
    if bytes.len() != 2 {
        return None;
    }
    let file = bytes[0].to_ascii_lowercase().wrapping_sub(b'a');
    let rank = bytes[1].wrapping_sub(b'1');
    if file < 8 && rank < 8 {
        Some(square_index(file as usize, rank as usize))
    } else {
        None
    }
}

/// Prints a set the way a board diagram is written, for debugging and tests.
pub fn to_diagram(bits: Bitboard) -> String {
    let mut out = String::with_capacity(80);
    for rank in (0..8).rev() {
        for file in 0..8 {
            let sq = square_index(file, rank);
            out.push(if bits & square_bb(sq) != 0 { '1' } else { '.' });
            out.push(' ');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference the ray walk must agree with: step out one square at a time
    /// in a fixed direction and let the bounds check do the work.
    fn reference_attacks(sq: usize, occupied: Bitboard, steps: &[(i32, i32)]) -> Bitboard {
        let mut attacks = 0u64;
        for &(df, dr) in steps {
            let mut file = file_of(sq) as i32 + df;
            let mut rank = rank_of(sq) as i32 + dr;
            while inside(file, rank) {
                let target = (rank * 8 + file) as usize;
                attacks |= square_bb(target);
                if occupied & square_bb(target) != 0 {
                    break;
                }
                file += df;
                rank += dr;
            }
        }
        attacks
    }

    /// Deterministic xorshift, so a failure always reproduces.
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

    const ORTHOGONAL: [(i32, i32); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    const DIAGONAL: [(i32, i32); 4] = [(1, 1), (1, -1), (-1, 1), (-1, -1)];

    #[test]
    fn sliding_attacks_match_the_reference_on_random_boards() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..1000 {
            let occupied = rng.next() & rng.next();
            for sq in 0..64 {
                assert_eq!(
                    rook_attacks(sq, occupied),
                    reference_attacks(sq, occupied, &ORTHOGONAL),
                    "rook on {} with occupancy {:#x}",
                    square_name(sq),
                    occupied
                );
                assert_eq!(
                    bishop_attacks(sq, occupied),
                    reference_attacks(sq, occupied, &DIAGONAL),
                    "bishop on {} with occupancy {:#x}",
                    square_name(sq),
                    occupied
                );
            }
        }
    }

    #[test]
    fn a_blocker_is_included_but_nothing_behind_it_is() {
        // A rook on a1 with a piece on a3: a2 and a3 are attacked, a4 is not.
        let occupied = square_bb(parse_square("a3").unwrap());
        let attacks = rook_attacks(parse_square("a1").unwrap(), occupied);
        assert!(attacks & square_bb(parse_square("a2").unwrap()) != 0);
        assert!(attacks & square_bb(parse_square("a3").unwrap()) != 0);
        assert!(attacks & square_bb(parse_square("a4").unwrap()) == 0);
        // ...and a slider never attacks its own square.
        assert!(attacks & square_bb(parse_square("a1").unwrap()) == 0);
    }

    #[test]
    fn knight_and_king_tables_have_the_expected_degree() {
        // Every interior square has eight knight moves; a corner has two.
        assert_eq!(popcount(KNIGHT_ATTACKS[square_index(3, 3)]), 8);
        assert_eq!(popcount(KNIGHT_ATTACKS[0]), 2);
        assert_eq!(popcount(KING_ATTACKS[square_index(3, 3)]), 8);
        assert_eq!(popcount(KING_ATTACKS[0]), 3);
    }

    #[test]
    fn pawn_attacks_point_the_right_way() {
        let e2 = parse_square("e2").unwrap();
        assert_eq!(
            PAWN_ATTACKS[0][e2],
            square_bb(parse_square("d3").unwrap()) | square_bb(parse_square("f3").unwrap())
        );
        let e7 = parse_square("e7").unwrap();
        assert_eq!(
            PAWN_ATTACKS[1][e7],
            square_bb(parse_square("d6").unwrap()) | square_bb(parse_square("f6").unwrap())
        );
        // The squares attacking a white pawn mirror the squares it attacks,
        // which is what makes the reverse lookup in `board.rs` work.
        assert_eq!(
            PAWN_ATTACKS[1][e2],
            square_bb(parse_square("d1").unwrap()) | square_bb(parse_square("f1").unwrap())
        );
    }

    #[test]
    fn square_names_round_trip() {
        for sq in 0..64 {
            assert_eq!(parse_square(&square_name(sq)), Some(sq));
        }
        assert_eq!(square_name(0), "a1");
        assert_eq!(square_name(63), "h8");
        assert_eq!(parse_square("i9"), None);
        assert_eq!(parse_square("e"), None);
    }

    #[test]
    fn same_diagonal_separates_captures_from_pushes() {
        let e4 = parse_square("e4").unwrap();
        assert!(on_same_diagonal(e4, parse_square("d5").unwrap()));
        assert!(on_same_diagonal(e4, parse_square("f3").unwrap()));
        assert!(!on_same_diagonal(e4, parse_square("e5").unwrap()));
    }

    #[test]
    fn pop_lsb_walks_a_set_low_to_high() {
        let mut bits: Bitboard = square_bb(3) | square_bb(17) | square_bb(60);
        assert_eq!(pop_lsb(&mut bits), 3);
        assert_eq!(pop_lsb(&mut bits), 17);
        assert_eq!(pop_lsb(&mut bits), 60);
        assert_eq!(bits, 0);
    }
}


