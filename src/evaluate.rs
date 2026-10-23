//! Static evaluation: PeSTO's piece-square tables, tapered between the middlegame
//! and the endgame.
//!
//! The tables are the ones PeSTO (Ronald Friederich's engine, the source of the
//! tuning most hobby engines start from) publishes, and they are written here in
//! the orientation PeSTO indexes them in: with the eighth rank first.  A white
//! piece therefore reads its square mirrored (`sq ^ 56`) and a black piece reads
//! it directly, which is what keeps the two colours symmetric — the
//! `evaluation_is_symmetric_under_colour_reversal` test is what holds that claim
//! honest.
//!
//! The taper is PeSTO's: every remaining knight and bishop counts one, every rook
//! two and every queen four, and the score slides linearly from the middlegame
//! reading at a full board (24) to the endgame reading as material comes off.
//! On top of the tables sit a few classic terms — the bishop pair, doubled,
//! isolated and passed pawns, rooks on open files and a light king shield —
//! because piece-square tables alone cannot see pawn structure.

use crate::bitboard::{
    bishop_attacks, king_attacks, knight_attacks, pawn_attacks, pop_lsb, popcount, queen_attacks,
    rook_attacks, square_bb, Bitboard,
};
use crate::board::{Board, BISHOP, BLACK, KNIGHT, PAWN, QUEEN, ROOK, WHITE};

/// The material value of each piece, middlegame and endgame, in centipawns.
pub const MG_VALUES: [i32; 6] = [82, 337, 365, 477, 1025, 0];
pub const EG_VALUES: [i32; 6] = [94, 281, 297, 512, 936, 0];

/// How much a piece counts towards the middlegame share of the taper.
pub const PHASE_WEIGHTS: [i32; 6] = [0, 1, 1, 2, 4, 0];

/// A full board of knights, bishops, rooks and queens.
pub const MAX_PHASE: i32 = 24;

/// The side to move gets a small nudge: having the move is worth something.
const TEMPO: i32 = 12;

/// The per-file bitboards, used by the pawn structure terms.
const FILE_MASKS: [Bitboard; 8] = [
    0x0101_0101_0101_0101,
    0x0202_0202_0202_0202,
    0x0404_0404_0404_0404,
    0x0808_0808_0808_0808,
    0x1010_1010_1010_1010,
    0x2020_2020_2020_2020,
    0x4040_4040_4040_4040,
    0x8080_8080_8080_8080,
];

/// Passed pawn bonus by how far it has travelled, in centipawns (mg, eg).
const PASSED_PAWN_BONUS: [(i32, i32); 8] = [
    (0, 0),
    (5, 10),
    (10, 20),
    (20, 35),
    (35, 60),
    (60, 100),
    (100, 160),
    (0, 0),
];

/// Bishop pair, doubled pawns, isolated pawns, rooks on open files, king shield.
const BISHOP_PAIR: (i32, i32) = (25, 40);
const DOUBLED_PAWN: (i32, i32) = (-10, -20);
const ISOLATED_PAWN: (i32, i32) = (-12, -16);
const ROOK_OPEN_FILE: (i32, i32) = (20, 10);
const ROOK_SEMI_OPEN_FILE: (i32, i32) = (10, 5);
const KING_SHIELD_PAWN: i32 = 8;

/// What one square of mobility is worth, by piece type (mg, eg).  A queen is worth
/// little per square because it has so many of them, a bishop more than a rook for
/// the same reason in the endgame, where the board opens up.
pub const MOBILITY: [(i32, i32); 6] = [(0, 0), (4, 4), (3, 5), (2, 4), (1, 2), (0, 0)];

/// How much of the king's safety one attacking piece is worth, by type.  The
/// squares a piece attacks matter less than which piece it is.
pub const KING_ATTACK_WEIGHT: [i32; 6] = [0, 2, 3, 4, 6, 0];

/// How much a passed pawn's promotion square is worth per step of king proximity,
/// for the escorting king (a bonus) and the defending king (a penalty).
const PASSED_PAWN_KING_STEP: i32 = 5;

/// What a pawn defending another pawn is worth (mg, eg): a chain cannot be undermined
/// by a single attacker, which is what the piece-square tables cannot see.
const CONNECTED_PAWN: (i32, i32) = (8, 12);

/// How many king-attack units one enemy pawn storming towards the king is worth.  A
/// pawn arriving in front of a castled king is worth more than the pawn that made it.
const KING_STORM_UNIT: i32 = 4;

/// What the score is multiplied by in a position that is drawn by its nature, out of
/// `SCALE_ONE`.
pub const SCALE_ONE: i32 = 32;
const SCALE_DRAWN_BISHOPS: i32 = 16;
const SCALE_DRAWN_MINOR: i32 = 8;

const MG_PAWN_TABLE: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, //
    98, 134, 61, 95, 68, 126, 34, -11, //
    -6, 7, 26, 31, 65, 56, 25, -20, //
    -14, 13, 6, 21, 23, 12, 17, -23, //
    -27, -2, -5, 12, 17, 6, 10, -25, //
    -26, -4, -4, -10, 3, 3, 33, -12, //
    -35, -1, -20, -23, -15, 24, 38, -22, //
    0, 0, 0, 0, 0, 0, 0, 0, //
];

const EG_PAWN_TABLE: [i32; 64] = [
    0, 0, 0, 0, 0, 0, 0, 0, //
    178, 173, 158, 134, 147, 132, 165, 187, //
    94, 100, 85, 67, 56, 53, 82, 84, //
    32, 24, 13, 5, -2, 4, 17, 17, //
    13, 9, -3, -7, -7, -8, 3, -1, //
    4, 7, -6, 1, 0, -5, -1, -8, //
    13, 8, 8, 10, 13, 0, 2, -7, //
    0, 0, 0, 0, 0, 0, 0, 0, //
];

const MG_KNIGHT_TABLE: [i32; 64] = [
    -167, -89, -34, -49, 61, -97, -15, -107, //
    -73, -41, 72, 36, 23, 62, 7, -17, //
    -47, 60, 37, 65, 84, 129, 73, 44, //
    -9, 17, 19, 53, 37, 69, 18, 22, //
    -13, 4, 16, 13, 28, 19, 21, -8, //
    -23, -9, 12, 10, 19, 17, 25, -16, //
    -29, -53, -12, -3, -1, 18, -14, -19, //
    -105, -21, -58, -33, -17, -28, -19, -23, //
];

const EG_KNIGHT_TABLE: [i32; 64] = [
    -58, -38, -13, -28, -31, -27, -63, -99, //
    -25, -8, -25, -2, -9, -25, -24, -52, //
    -24, -20, 10, 9, -1, -9, -19, -41, //
    -17, 3, 22, 22, 22, 11, 8, -18, //
    -18, -6, 16, 25, 16, 17, 4, -18, //
    -23, -3, -1, 15, 10, -3, -20, -22, //
    -42, -20, -10, -5, -2, -20, -23, -44, //
    -29, -51, -23, -15, -22, -18, -50, -64, //
];

const MG_BISHOP_TABLE: [i32; 64] = [
    -29, 4, -82, -37, -25, -42, 7, -8, //
    -26, 16, -18, -13, 30, 59, 18, -47, //
    -16, 37, 43, 40, 35, 50, 37, -2, //
    -4, 5, 19, 50, 37, 37, 7, -2, //
    -6, 13, 13, 26, 34, 12, 10, 4, //
    0, 15, 15, 15, 14, 27, 18, 10, //
    4, 15, 16, 0, 7, 21, 33, 1, //
    -33, -3, -14, -21, -13, -12, -39, -21, //
];

const EG_BISHOP_TABLE: [i32; 64] = [
    -14, -21, -11, -8, -7, -9, -17, -24, //
    -8, -4, 7, -12, -3, -13, -4, -14, //
    2, -8, 0, -1, -2, 6, 0, 4, //
    -3, 9, 12, 9, 14, 10, 3, 2, //
    -6, 3, 13, 19, 7, 10, -3, -9, //
    -12, -3, 8, 10, 13, 3, -7, -15, //
    -14, -18, -7, -1, 4, -9, -15, -27, //
    -23, -9, -23, -5, -9, -16, -5, -17, //
];

const MG_ROOK_TABLE: [i32; 64] = [
    32, 42, 32, 51, 63, 9, 31, 43, //
    27, 32, 58, 62, 80, 67, 26, 44, //
    -5, 19, 26, 36, 17, 45, 61, 16, //
    -24, -11, 7, 26, 24, 35, -8, -20, //
    -36, -26, -12, -1, 9, -7, 6, -23, //
    -45, -25, -16, -17, 3, 0, -5, -33, //
    -44, -16, -20, -9, -1, 11, -6, -71, //
    -19, -13, 1, 17, 16, 7, -37, -26, //
];

const EG_ROOK_TABLE: [i32; 64] = [
    13, 10, 18, 15, 12, 12, 8, 5, //
    11, 13, 13, 11, -3, 3, 8, 3, //
    7, 7, 7, 5, 4, -3, -5, -3, //
    4, 3, 13, 1, 2, 1, -1, 2, //
    3, 5, 8, 4, -5, -6, -8, -11, //
    -4, 0, -5, -1, -7, -12, -8, -16, //
    -6, -6, 0, 2, -9, -9, -11, -3, //
    -9, 2, 3, -1, -5, -13, 4, -20, //
];

const MG_QUEEN_TABLE: [i32; 64] = [
    -28, 0, 29, 12, 59, 44, 43, 45, //
    -24, -39, -5, 1, -16, 57, 28, 54, //
    -13, -17, 7, 8, 29, 56, 47, 57, //
    -27, -27, -16, -16, -1, 17, -2, 1, //
    -9, -26, -9, -10, -2, -4, 3, -3, //
    -14, 2, -11, -2, -5, 2, 14, 5, //
    -35, -8, 11, 2, 8, 15, -3, 1, //
    -1, -18, -9, 10, -15, -25, -31, -50, //
];

const EG_QUEEN_TABLE: [i32; 64] = [
    -9, 22, 22, 27, 27, 19, 10, 20, //
    -17, 20, 32, 41, 58, 25, 30, 0, //
    -20, 6, 9, 49, 47, 35, 19, 9, //
    3, 22, 24, 45, 57, 40, 57, 36, //
    -18, 28, 19, 47, 31, 34, 39, 23, //
    -16, -27, 15, 6, 9, 17, 10, 5, //
    -22, -23, -30, -16, -16, -23, -36, -32, //
    -33, -28, -22, -43, -5, -32, -20, -41, //
];

const MG_KING_TABLE: [i32; 64] = [
    -65, 23, 16, -15, -56, -34, 2, 13, //
    29, -1, -20, -7, -8, -4, -38, -29, //
    -9, 24, 2, -16, -20, 6, 22, -22, //
    -17, -20, -12, -27, -30, -25, -14, -36, //
    -49, -1, -27, -39, -46, -44, -33, -51, //
    -14, -14, -22, -46, -44, -30, -15, -27, //
    1, 7, -8, -64, -43, -16, 9, 8, //
    -15, 36, 12, -54, 8, -28, 24, 14, //
];

const EG_KING_TABLE: [i32; 64] = [
    -74, -35, -18, -18, -11, 15, 4, -17, //
    -12, 17, 14, 17, 17, 38, 23, 11, //
    10, 17, 23, 15, 20, 45, 44, 13, //
    -8, 22, 24, 27, 26, 33, 26, 3, //
    -18, -4, 21, 24, 27, 23, 9, -11, //
    -19, -3, 11, 21, 23, 16, 7, -9, //
    -27, -11, 4, 13, 14, 4, -5, -17, //
    -53, -34, -21, -11, -28, -14, -24, -43, //
];

const MG_TABLES: [&[i32; 64]; 6] = [
    &MG_PAWN_TABLE,
    &MG_KNIGHT_TABLE,
    &MG_BISHOP_TABLE,
    &MG_ROOK_TABLE,
    &MG_QUEEN_TABLE,
    &MG_KING_TABLE,
];

const EG_TABLES: [&[i32; 64]; 6] = [
    &EG_PAWN_TABLE,
    &EG_KNIGHT_TABLE,
    &EG_BISHOP_TABLE,
    &EG_ROOK_TABLE,
    &EG_QUEEN_TABLE,
    &EG_KING_TABLE,
];

/// Which of the position-aware terms beyond material and the piece-square tables
/// are switched on.
///
/// PeSTO's tables alone cannot see that a knight in the corner is idle or that the
/// king in front of it is under attack, so these three terms are the difference
/// between "counting material well" and "understanding the position".  They are
/// flags because they cost time at every leaf, and because the match binary plays
/// the engine against itself with them on and off — which is the only honest way to
/// claim any of them earned its keep.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EvalFeatures {
    /// How many squares each piece can move to, counted from its attack sets and
    /// leaving out the squares the enemy's pawns cover.
    pub mobility: bool,
    /// Enemy pieces this side's pawns are attacking.
    pub threats: bool,
    /// Pawns defended by other pawns.
    pub connected_pawns: bool,
    /// Enemy pawns advancing towards this side's king.
    pub king_storm: bool,
    /// Scale the score towards zero in positions that are drawn however much material
    /// one side is up: opposite-coloured bishops, a lone minor piece with no pawns,
    /// and a rook pawn with the wrong bishop.
    pub drawish_endings: bool,
    /// How many enemy pieces are bearing down on the king's neighbourhood.
    pub king_safety: bool,
    /// How close each side's king is to the passed pawn it is escorting or chasing.
    pub passed_pawn_king_proximity: bool,
    /// Read the position from white's side rather than from the side to move.
    ///
    /// This is how the engine's first release scored positions, and it was wrong:
    /// `negamax` negates a child's score, so an evaluation has to be written from the
    /// mover's point of view, and a score that was always white's made the search
    /// prefer its own worst move wherever the two disagreed.  The flag exists so the
    /// match binary can play the fixed engine against the one that shipped and put a
    /// number on the fix; nothing that ships turns it on, and
    /// `the_orientation_of_the_score_is_the_side_to_moves` is what holds the
    /// convention down.
    pub white_positive_score: bool,
}

impl EvalFeatures {
    /// The configuration that ships.
    ///
    /// `mobility` was measured *negative* in its first form — counting every square a
    /// piece attacks — and is in the shipped set in a different form: it leaves out the
    /// squares the enemy's pawns cover, and *that* version measured +22 Elo at equal
    /// depth.  Same weights, different squares, opposite result.
    pub const fn all() -> Self {
        EvalFeatures {
            mobility: true,
            // Off: the pawn-threat term is the one piece of this round that measured
            // nothing, at equal depth and at equal time, so it is a cost for no return.
            // The refinement worth trying is threats from *pieces* as well as pawns,
            // which needs the defended/undefended distinction to mean anything.
            threats: false,
            // Off: pawns defending pawns measured nothing at equal depth over 240 games,
            // which is not a return for the work at every leaf.
            connected_pawns: false,
            // Off: +44 Elo at *equal depth*, and −46 with its companion term at equal
            // time, over 200 to 300 games each way.  The two are not a contradiction:
            // a positional term makes the static score a worse predictor of the search's
            // own answer, and every pruning decision in the search — reverse futility,
            // futility, ProbCut, delta pruning — leans on that prediction.  Worth
            // knowing: this is the first time a term measured positive per depth and
            // still lost Elo per second.  The tests keep the code honest so a later
            // round can retry it with a better-behaved static score.
            king_storm: false,
            drawish_endings: false,
            king_safety: true,
            passed_pawn_king_proximity: true,
            white_positive_score: false,
        }
    }

    pub const fn none() -> Self {
        EvalFeatures {
            mobility: false,
            threats: false,
            connected_pawns: false,
            king_storm: false,
            drawish_endings: false,
            king_safety: false,
            passed_pawn_king_proximity: false,
            white_positive_score: false,
        }
    }

    /// The terms as the first release had them: no mobility, no king safety, no king
    /// proximity to a passed pawn, and the score read from white's side.
    pub const fn as_shipped() -> Self {
        EvalFeatures {
            mobility: false,
            threats: false,
            connected_pawns: false,
            king_storm: false,
            drawish_endings: false,
            king_safety: false,
            passed_pawn_king_proximity: false,
            white_positive_score: true,
        }
    }
}

impl Default for EvalFeatures {
    fn default() -> Self {
        EvalFeatures::all()
    }
}

/// How much of the score a position is worth, out of `SCALE_ONE`, when material says
/// one side is up and the position says nobody is winning.
///
/// Two rules, both of them cases where the draw follows from the *material alone* plus
/// the square colours, which is what makes them safe to encode statically:
///
/// * one bishop each, on opposite colours, with no knights or majors and the same
///   number of pawns: the defender blocks on the squares the attacking bishop cannot
///   reach, and half a score is the most the position can be worth.
/// * at most one minor piece each with no pawns at all: none of those combinations can
///   force mate, whatever the pieces are, and `is_insufficient_material` only catches
///   the ones where it is provable (two knights cannot, but they *can* be mated).
///
/// The textbook third case — a rook pawn with a bishop that cannot cover the promotion
/// square — is deliberately *not* here.  It is a draw only when the defending king can
/// reach the corner region, which is a question about the position rather than about
/// the material, and encoding it as a dead draw would throw away won positions where
/// the defender's king is cut off.
fn drawishness_scale(board: &Board) -> i32 {
    let pawns_white = popcount(board.pieces_of(WHITE, PAWN));
    let pawns_black = popcount(board.pieces_of(BLACK, PAWN));
    let minors_white = board.pieces_of(WHITE, KNIGHT) | board.pieces_of(WHITE, BISHOP);
    let minors_black = board.pieces_of(BLACK, KNIGHT) | board.pieces_of(BLACK, BISHOP);
    let knights = board.pieces_of(WHITE, KNIGHT) | board.pieces_of(BLACK, KNIGHT);
    let majors = board.pieces_of(WHITE, ROOK)
        | board.pieces_of(BLACK, ROOK)
        | board.pieces_of(WHITE, QUEEN)
        | board.pieces_of(BLACK, QUEEN);

    if pawns_white == 0
        && pawns_black == 0
        && majors == 0
        && (minors_white | minors_black) != 0
        && popcount(minors_white) <= 1
        && popcount(minors_black) <= 1
    {
        return SCALE_DRAWN_MINOR;
    }

    if pawns_white == pawns_black && knights == 0 && majors == 0 {
        if let (Some(white), Some(black)) = (
            single_square(board.pieces_of(WHITE, BISHOP)),
            single_square(board.pieces_of(BLACK, BISHOP)),
        ) {
            if square_colour(white) != square_colour(black) {
                return SCALE_DRAWN_BISHOPS;
            }
        }
    }

    SCALE_ONE
}

/// The one square in a set, or `None` if it does not hold exactly one.
fn single_square(bits: Bitboard) -> Option<usize> {
    if popcount(bits) == 1 {
        Some(crate::bitboard::lsb(bits))
    } else {
        None
    }
}

/// Which colour complex a square belongs to: 0 for the dark squares, 1 for the light.
fn square_colour(square: usize) -> usize {
    (crate::bitboard::file_of(square) + crate::bitboard::rank_of(square)) & 1
}

/// The static score of the position, in centipawns, from the side to move's point
/// of view: positive means the side to move stands better.
pub fn evaluate(board: &Board) -> i32 {
    evaluate_with(board, EvalFeatures::all())
}

/// The static score with individual terms switched off, which is what the
/// self-play matches use to measure what each one is worth.
pub fn evaluate_with(board: &Board, features: EvalFeatures) -> i32 {
    let mut mg = [0i32; 2];
    let mut eg = [0i32; 2];
    let mut phase = 0;

    // The two covers are wanted by both of the terms below and by opposite sides of
    // them: our pawns' cover is what we threaten and what the enemy has to avoid, and
    // theirs is the mirror.  Computed once here rather than once per colour per term.
    let (white_cover, black_cover) = if features.mobility || features.threats {
        (pawn_cover(board, WHITE), pawn_cover(board, BLACK))
    } else {
        (0, 0)
    };

    for colour in [WHITE, BLACK] {
        let (our_cover, their_cover) = if colour == WHITE {
            (white_cover, black_cover)
        } else {
            (black_cover, white_cover)
        };

        for kind in 0..6 {
            let mut pieces = board.pieces_of(colour, kind);
            while pieces != 0 {
                let square = pop_lsb(&mut pieces);
                // PeSTO's tables are stored eighth rank first, so white reads
                // them mirrored and black straight; that is what makes mirrored
                // positions score exactly opposite.
                let index = if colour == WHITE { square ^ 56 } else { square };
                mg[colour] += MG_TABLES[kind][index] + MG_VALUES[kind];
                eg[colour] += EG_TABLES[kind][index] + EG_VALUES[kind];
                phase += PHASE_WEIGHTS[kind];
            }
        }

        let (structure_mg, structure_eg) = structure_terms(board, colour, features);
        mg[colour] += structure_mg;
        eg[colour] += structure_eg;

        if features.mobility {
            let (mobility_mg, mobility_eg) = mobility_terms(board, colour, their_cover);
            mg[colour] += mobility_mg;
            eg[colour] += mobility_eg;
        }
        if features.threats {
            let (threat_mg, threat_eg) = threat_terms(board, colour, our_cover);
            mg[colour] += threat_mg;
            eg[colour] += threat_eg;
        }
        if features.connected_pawns {
            // A pawn standing on a square another of our pawns attacks is defended, so
            // taking it costs material.  The cover is already computed for the terms
            // above.
            let defended = popcount(board.pieces_of(colour, PAWN) & our_cover) as i32;
            mg[colour] += defended * CONNECTED_PAWN.0;
            eg[colour] += defended * CONNECTED_PAWN.1;
        }
        if features.king_safety {
            mg[colour] -= king_danger(board, colour, features.king_storm);
        }
    }

    let side = board.side;
    let other = 1 - side;
    // The score has to be the *mover's*: `negamax` negates whatever a child returns,
    // so a score read from white's side is upside down for every black-to-move node,
    // and the search would then prefer its own worst move.  `white_positive_score` is
    // the first release's mistake, kept only so a match can measure how much the fix
    // is worth.
    let (midgame, endgame) = if features.white_positive_score {
        (mg[WHITE] - mg[BLACK], eg[WHITE] - eg[BLACK])
    } else {
        (mg[side] - mg[other], eg[side] - eg[other])
    };
    let phase = phase.clamp(0, MAX_PHASE);

    // PeSTO's taper: the middlegame reading holds while the board is full and
    // gives way linearly as material leaves it.
    let score = (midgame * phase + endgame * (MAX_PHASE - phase)) / MAX_PHASE;
    // Positions that are drawn however much material one side is up are worth less
    // than their material: without this the search plays into them believing it is
    // winning and, from the other side, defends them as though they were lost.
    let score = if features.drawish_endings {
        score * drawishness_scale(board) / SCALE_ONE
    } else {
        score
    };
    // Having the move is worth something, and it is worth it to the side that has it.
    // The first release instead signed the bonus with white, which is the same
    // mistake as the positional part and is kept alive only by the flag.
    let tempo = if features.white_positive_score && board.side != WHITE {
        -TEMPO
    } else {
        TEMPO
    };
    score + tempo
}

/// How many squares each of a side's pieces attacks, weighted by piece type.
///
/// A knight with eight moves is doing more than a knight with two, and counting the
/// attack sets — rather than only the empty squares they reach — is what makes this
/// cheap: the sets are one lookup for the leapers and one ray walk per slider.
///
/// The squares an enemy pawn covers are left out, which is the whole difference
/// between this and the version that measured *worse* than nothing.  A square a pawn
/// attacks is a square a piece cannot sit on, so a rook that "has" eight moves into a
/// pawn's cover has none of them; counting those eight as mobility was rewarding
/// activity that loses material.
fn mobility_terms(board: &Board, colour: usize, enemy_pawns_cover: Bitboard) -> (i32, i32) {
    let area = !board.colour_of(colour) & !enemy_pawns_cover;
    let occupied = board.occupied();
    let mut mg = 0;
    let mut eg = 0;

    for kind in [KNIGHT, BISHOP, ROOK, QUEEN] {
        let mut pieces = board.pieces_of(colour, kind);
        while pieces != 0 {
            let square = pop_lsb(&mut pieces);
            let attacks = piece_attacks(square, kind, occupied);
            let squares = popcount(attacks & area) as i32;
            mg += squares * MOBILITY[kind].0;
            eg += squares * MOBILITY[kind].1;
        }
    }
    (mg, eg)
}

/// Every square one side's pawns attack, whether or not anything stands there.
///
/// Two terms want this and neither wants the other's copy of it: mobility subtracts
/// it (a square a pawn covers is not mobility) and the threat term intersects it with
/// the enemy's pieces.  One loop over the pawns, shared.
fn pawn_cover(board: &Board, colour: usize) -> Bitboard {
    let mut pawns = board.pieces_of(colour, PAWN);
    let mut cover = 0;
    while pawns != 0 {
        cover |= pawn_attacks(colour, pop_lsb(&mut pawns));
    }
    cover
}

/// What it is worth to have a pawn attacking an enemy piece, by the piece attacked
/// (mg, eg).  These are deliberately smaller than the piece values: a threatened
/// piece usually just moves, so the threat is pressure rather than material.
const PAWN_THREAT: [(i32, i32); 6] = [(0, 0), (25, 20), (25, 20), (35, 25), (50, 30), (0, 0)];

/// What this side's pawns are attacking.
///
/// A pawn attacking a knight is worth more than a pawn attacking nothing, and the
/// term costs one intersection per piece type because the pawn cover is already
/// computed for the mobility term.  Pieces attacked by pieces (rather than by pawns)
/// would be the next refinement, and it needs the defended/undefended distinction to
/// be worth anything, which is a walk per piece.
fn threat_terms(board: &Board, colour: usize, own_pawns_cover: Bitboard) -> (i32, i32) {
    let mut mg = 0;
    let mut eg = 0;
    for kind in [KNIGHT, BISHOP, ROOK, QUEEN] {
        let attacked = board.pieces_of(1 - colour, kind) & own_pawns_cover;
        let count = popcount(attacked) as i32;
        mg += count * PAWN_THREAT[kind].0;
        eg += count * PAWN_THREAT[kind].1;
    }
    (mg, eg)
}

/// How dangerous the enemy pieces are to this side's king, as a middlegame penalty.
///
/// Each enemy knight, bishop, rook or queen that attacks one of the eight squares
/// around the king contributes units by type; the total is squared, because two
/// attackers are more than twice as dangerous as one.  This is the classic shape of
/// the term, and it is what stops the engine from leaving its king in a position
/// where a sacrifice opens it up.
fn king_danger(board: &Board, colour: usize, count_storm: bool) -> i32 {
    let king = board.king_square(colour);
    let ring = king_attacks(king) | square_bb(king);
    let occupied = board.occupied();
    let mut units = 0;

    for kind in [KNIGHT, BISHOP, ROOK, QUEEN] {
        let mut pieces = board.pieces_of(1 - colour, kind);
        while pieces != 0 {
            let square = pop_lsb(&mut pieces);
            if piece_attacks(square, kind, occupied) & ring != 0 {
                units += KING_ATTACK_WEIGHT[kind];
            }
        }
    }

    if count_storm {
        units += king_storm_units(board, colour);
    }
    units * units / 4
}

/// Enemy pawns on their way to this side's king, as king-attack units.
///
/// A pawn two files from the king and past the middle of the board is a threat the
/// piece count cannot see: the ring around the king is where it is going to arrive,
/// and the squares it covers are squares the king's defenders cannot use.  Only the
/// three files around the king are counted, and only from the half of the board the
/// pawn came from.
fn king_storm_units(board: &Board, colour: usize) -> i32 {
    let king_file = crate::bitboard::file_of(board.king_square(colour));
    let mut files = FILE_MASKS[king_file];
    if king_file > 0 {
        files |= FILE_MASKS[king_file - 1];
    }
    if king_file < 7 {
        files |= FILE_MASKS[king_file + 1];
    }

    // For a white king the enemy pawns worth counting are the ones already on ranks
    // five to eight; for a black king, ranks one to four.
    let far_half = if colour == WHITE {
        crate::bitboard::RANK_5 | crate::bitboard::RANK_6 | crate::bitboard::RANK_7 | crate::bitboard::RANK_8
    } else {
        crate::bitboard::RANK_1 | crate::bitboard::RANK_2 | crate::bitboard::RANK_3 | crate::bitboard::RANK_4
    };

    let storming = board.pieces_of(1 - colour, PAWN) & files & far_half;
    popcount(storming) as i32 * KING_STORM_UNIT
}

/// The attack set of one piece, which is a table lookup for the leapers and a ray
/// walk for the sliders.
fn piece_attacks(square: usize, kind: usize, occupied: Bitboard) -> Bitboard {
    match kind {
        KNIGHT => knight_attacks(square),
        BISHOP => bishop_attacks(square, occupied),
        ROOK => rook_attacks(square, occupied),
        QUEEN => queen_attacks(square, occupied),
        _ => 0,
    }
}

/// The terms that care about where the pieces are rather than which they are.
fn structure_terms(board: &Board, colour: usize, features: EvalFeatures) -> (i32, i32) {
    let mut mg = 0;
    let mut eg = 0;

    // Two bishops cover both colour complexes, which is worth more than the sum
    // of the two pieces.
    if popcount(board.pieces_of(colour, BISHOP)) >= 2 {
        mg += BISHOP_PAIR.0;
        eg += BISHOP_PAIR.1;
    }

    // Pawns by file, which every pawn term below reads.
    let mut per_file = [0u8; 8];
    let mut pawns = board.pieces_of(colour, PAWN);
    while pawns != 0 {
        per_file[crate::bitboard::file_of(pop_lsb(&mut pawns))] += 1;
    }

    for file in 0..8 {
        let count = per_file[file];
        if count > 1 {
            // Doubled: the pawns past the first are worth less than it is.
            let extra = (count - 1) as i32;
            mg += DOUBLED_PAWN.0 * extra;
            eg += DOUBLED_PAWN.1 * extra;
        }
        if count > 0 {
            let left = if file > 0 { per_file[file - 1] } else { 0 };
            let right = if file < 7 { per_file[file + 1] } else { 0 };
            if left == 0 && right == 0 {
                mg += ISOLATED_PAWN.0 * count as i32;
                eg += ISOLATED_PAWN.1 * count as i32;
            }
        }
    }

    let mut pawns = board.pieces_of(colour, PAWN);
    while pawns != 0 {
        let square = pop_lsb(&mut pawns);
        let (passed_mg, mut passed_eg) = passed_pawn_bonus(board, colour, square);
        if passed_mg != 0 || passed_eg != 0 {
            if features.passed_pawn_king_proximity {
                passed_eg += passed_pawn_king_proximity(board, colour, square);
            }
            mg += passed_mg;
            eg += passed_eg;
        }
    }

    let open_files = open_file_flags(board);
    let mut rooks = board.pieces_of(colour, ROOK);
    while rooks != 0 {
        let square = pop_lsb(&mut rooks);
        let file = crate::bitboard::file_of(square);
        // Bit 0 is "no pawns at all", bit 1 is "no white pawns", bit 2 is "no
        // black pawns".
        let flags = open_files[file];
        let own_missing = if colour == WHITE { flags & 2 != 0 } else { flags & 4 != 0 };
        if flags & 1 != 0 {
            mg += ROOK_OPEN_FILE.0;
            eg += ROOK_OPEN_FILE.1;
        } else if own_missing {
            mg += ROOK_SEMI_OPEN_FILE.0;
            eg += ROOK_SEMI_OPEN_FILE.1;
        }
    }

    mg += king_shield(board, colour);
    (mg, eg)
}

/// True when no enemy pawn stands on the pawn's file or either neighbour ahead of
/// it.
fn passed_pawn_bonus(board: &Board, colour: usize, square: usize) -> (i32, i32) {
    let file = crate::bitboard::file_of(square);
    let rank = crate::bitboard::rank_of(square);

    let mut files = FILE_MASKS[file];
    if file > 0 {
        files |= FILE_MASKS[file - 1];
    }
    if file < 7 {
        files |= FILE_MASKS[file + 1];
    }

    let ahead: Bitboard = if colour == WHITE {
        if rank >= 7 {
            0
        } else {
            u64::MAX << (8 * (rank + 1))
        }
    } else if rank == 0 {
        0
    } else {
        (1u64 << (8 * rank)) - 1
    };

    if board.pieces_of(1 - colour, PAWN) & files & ahead != 0 {
        return (0, 0);
    }

    // Distance from promotion, so both colours read the same ladder.
    let relative = if colour == WHITE { rank } else { 7 - rank };
    PASSED_PAWN_BONUS[relative]
}

/// A passed pawn is worth more when its own king is near the square it is heading
/// for and the enemy king is not: in the endgame that is most of what a passed pawn
/// is for.
///
/// The distance is Chebyshev (king moves), so it counts real steps towards the pawn.
fn passed_pawn_king_proximity(board: &Board, colour: usize, pawn: usize) -> i32 {
    let file = crate::bitboard::file_of(pawn);
    let promotion_rank = if colour == WHITE { 7 } else { 0 };
    let target = crate::bitboard::square_index(file, promotion_rank);

    let ours = king_distance(board.king_square(colour), target);
    let theirs = king_distance(board.king_square(1 - colour), target);
    (theirs - ours) * PASSED_PAWN_KING_STEP
}

/// King moves between two squares.
fn king_distance(from: usize, to: usize) -> i32 {
    let (from_file, from_rank) = (crate::bitboard::file_of(from), crate::bitboard::rank_of(from));
    let (to_file, to_rank) = (crate::bitboard::file_of(to), crate::bitboard::rank_of(to));
    (from_file as i32 - to_file as i32)
        .abs()
        .max((from_rank as i32 - to_rank as i32).abs())
}

/// Per file: bit 0 means no pawns at all, bit 1 means no white pawns, bit 2 means
/// no black pawns.
fn open_file_flags(board: &Board) -> [u8; 8] {
    let mut flags = [0u8; 8];
    for file in 0..8 {
        let mask = FILE_MASKS[file];
        let white = popcount(board.pieces_of(WHITE, PAWN) & mask);
        let black = popcount(board.pieces_of(BLACK, PAWN) & mask);
        let mut value = 0;
        if white == 0 && black == 0 {
            value |= 1;
        }
        if white == 0 {
            value |= 2;
        }
        if black == 0 {
            value |= 4;
        }
        flags[file] = value;
    }
    flags
}

/// A light king-safety term: one pawn in front of the king on its file or either
/// neighbour is worth a little, up to three of them.
fn king_shield(board: &Board, colour: usize) -> i32 {
    let king = board.kings[colour];
    let file = crate::bitboard::file_of(king) as i32;
    let rank = crate::bitboard::rank_of(king) as i32;
    let step = if colour == WHITE { 1 } else { -1 };
    let pawns = board.pieces_of(colour, PAWN);

    let mut shield = 0;
    for offset in -1..=1 {
        let shield_file = file + offset;
        if !(0..8).contains(&shield_file) {
            continue;
        }
        for distance in 1..=2 {
            let shield_rank = rank + step * distance;
            if !(0..8).contains(&shield_rank) {
                continue;
            }
            let square = (shield_rank * 8 + shield_file) as usize;
            if pawns & square_bb(square) != 0 {
                shield += 1;
                break;
            }
        }
    }
    KING_SHIELD_PAWN * shield.min(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors a position top to bottom and swaps the colours, which must flip the
    /// sign of the score and change nothing else.
    fn mirrored(fen: &str) -> String {
        let board = Board::from_fen(fen).expect("legal FEN");
        let mut placement = String::new();
        for rank in 0..8 {
            let mut empty = 0;
            for file in 0..8 {
                // Reading the source bottom-up does the mirroring; swapping the
                // case of every letter and the side to move swaps the colours.
                let code = board.piece_on(crate::bitboard::square_index(file, rank));
                if code == crate::board::NO_PIECE {
                    empty += 1;
                    continue;
                }
                if empty > 0 {
                    placement.push_str(&empty.to_string());
                    empty = 0;
                }
                let letter = crate::board::piece_letter(code);
                placement.push(if letter.is_ascii_uppercase() {
                    letter.to_ascii_lowercase()
                } else {
                    letter.to_ascii_uppercase()
                });
            }
            if empty > 0 {
                placement.push_str(&empty.to_string());
            }
            if rank < 7 {
                placement.push('/');
            }
        }
        let side = if board.side_to_move() == WHITE { "b" } else { "w" };
        format!("{} {} - - 0 1", placement, side)
    }

    #[test]
    fn mobility_counts_the_squares_a_piece_can_move_to() {
        // A knight on d4 has eight moves; the same knight in the corner has two.
        let central = Board::from_fen("4k3/8/8/8/3N4/8/8/4K3 w - - 0 1").unwrap();
        let corner = Board::from_fen("4k3/8/8/8/8/8/8/N3K3 w - - 0 1").unwrap();
        assert_eq!(
            mobility_terms(&central, WHITE, pawn_cover(&central, BLACK)).0,
            8 * MOBILITY[KNIGHT].0,
            "central knight"
        );
        assert_eq!(
            mobility_terms(&corner, WHITE, pawn_cover(&corner, BLACK)).0,
            2 * MOBILITY[KNIGHT].0,
            "corner knight"
        );
        // And the whole score moves with it, in the same direction, when the term is
        // switched on: compare the shipped set against the same set with mobility off.
        let without_mobility = EvalFeatures {
            mobility: false,
            ..EvalFeatures::all()
        };
        let central_gain =
            evaluate_with(&central, EvalFeatures::all()) - evaluate_with(&central, without_mobility);
        let corner_gain =
            evaluate_with(&corner, EvalFeatures::all()) - evaluate_with(&corner, without_mobility);
        assert!(
            central_gain > corner_gain,
            "central {} vs corner {}",
            central_gain,
            corner_gain
        );
        assert!(corner_gain >= 0, "a corner knight should not lose by mobility");
    }

    #[test]
    fn mobility_leaves_out_the_squares_the_enemy_pawns_cover() {
        // The knight on d4 has eight squares either way.  In the first position a black
        // pawn on c6 covers b5, so one of the eight is a square the knight cannot use;
        // in the second there is no pawn and all eight count.  This is the difference
        // between the mobility term that ships and the one that measured worse than
        // nothing.
        let covered = Board::from_fen("4k3/8/2p5/8/3N4/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(
            mobility_terms(&covered, WHITE, pawn_cover(&covered, BLACK)).0,
            7 * MOBILITY[KNIGHT].0,
            "b5 belongs to the pawn on c6"
        );

        let open = Board::from_fen("4k3/8/8/8/3N4/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(
            mobility_terms(&open, WHITE, pawn_cover(&open, BLACK)).0,
            8 * MOBILITY[KNIGHT].0
        );
        assert_eq!(pawn_cover(&open, BLACK), 0, "no black pawns to cover anything");
    }

    #[test]
    fn a_pawn_attacking_a_piece_is_worth_its_threat() {
        // The white pawn on d4 attacks c5, where the black knight stands.
        let board = Board::from_fen("4k3/8/8/2n5/3P4/8/8/4K3 w - - 0 1").unwrap();
        let (mg, eg) = threat_terms(&board, WHITE, pawn_cover(&board, WHITE));
        assert_eq!(mg, PAWN_THREAT[KNIGHT].0, "white threatens the knight");
        assert_eq!(eg, PAWN_THREAT[KNIGHT].1);
        // Black's pawns are gone, so black threatens nothing back.
        assert_eq!(threat_terms(&board, BLACK, pawn_cover(&board, BLACK)).0, 0);

        // The same position mirrored gives black the same threat, which is what keeps
        // the term from favouring a colour.
        let mirrored_fen = mirrored("4k3/8/8/2n5/3P4/8/8/4K3 w - - 0 1");
        let mirror = Board::from_fen(&mirrored_fen).unwrap();
        let (mirror_mg, mirror_eg) = threat_terms(&mirror, BLACK, pawn_cover(&mirror, BLACK));
        assert_eq!((mirror_mg, mirror_eg), (mg, eg));
    }

    #[test]
    fn king_safety_is_zero_until_something_attacks_the_king() {
        let quiet = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        assert_eq!(king_danger(&quiet, WHITE, true), 0);
        assert_eq!(king_danger(&quiet, BLACK, true), 0);

        // A queen on e2 stands on the white king's ring and hits five squares of it.
        let watched = Board::from_fen("4k3/8/8/8/8/8/4q3/4K3 w - - 0 1").unwrap();
        let units = KING_ATTACK_WEIGHT[QUEEN];
        assert_eq!(king_danger(&watched, WHITE, true), units * units / 4);
        // The white king on e1 is nowhere near the black king on e8.
        assert_eq!(king_danger(&watched, BLACK, true), 0);

        // Two attackers are more than twice as bad as one: the term is quadratic.
        let doubled = Board::from_fen("4k3/8/8/8/8/8/3qq3/4K3 w - - 0 1").unwrap();
        assert!(
            king_danger(&doubled, WHITE, true) > 2 * king_danger(&watched, WHITE, true),
            "one attacker {} vs two {}",
            king_danger(&watched, WHITE, true),
            king_danger(&doubled, WHITE, true)
        );
    }

    #[test]
    fn enemy_pawns_marching_at_the_king_are_dangerous() {
        // A black pawn on f6 is two files from the white king on g1 and past the middle
        // of the board: that is the pawn the storm term counts.
        let near = Board::from_fen("4k3/8/5p2/8/8/8/8/6K1 w - - 0 1").unwrap();
        assert_eq!(king_storm_units(&near, WHITE), KING_STORM_UNIT);

        // The same pawn still on its own half has not arrived yet.
        let far = Board::from_fen("4k3/8/8/8/5p2/8/8/6K1 w - - 0 1").unwrap();
        assert_eq!(king_storm_units(&far, WHITE), 0, "f4 has not crossed the middle");

        // A pawn on the other side of the board is not a threat to this king at all,
        // even though it is far advanced.
        let elsewhere = Board::from_fen("4k3/p7/8/8/8/8/8/6K1 w - - 0 1").unwrap();
        assert_eq!(king_storm_units(&elsewhere, WHITE), 0);

        // And the term is symmetric: the mirror position counts the same pawn for a
        // black king.
        let black_king = Board::from_fen("6k1/8/8/8/5P2/8/8/4K3 b - - 0 1").unwrap();
        assert_eq!(king_storm_units(&black_king, BLACK), KING_STORM_UNIT);
        assert_eq!(king_storm_units(&black_king, WHITE), 0, "it is white's pawn");
    }

    #[test]
    fn connected_pawns_are_worth_more_than_scattered_ones() {
        // Three white pawns either in a chain (defending each other) or spread out with
        // nothing defending anything.
        let connected = Board::from_fen("4k3/8/8/8/3P4/4P3/8/4K3 w - - 0 1").unwrap();
        let scattered = Board::from_fen("4k3/8/8/8/3P4/8/2P5/4K3 w - - 0 1").unwrap();
        let defended = |board: &Board| popcount(board.pieces_of(WHITE, PAWN) & pawn_cover(board, WHITE));
        assert_eq!(defended(&connected), 1, "e3 is defended by d4");
        assert_eq!(defended(&scattered), 0, "nothing defends anything");
    }

    #[test]
    fn a_drawn_ending_is_not_worth_its_material() {
        // Two pawns each and bishops on opposite colours: material says level, and the
        // position says nobody is winning either way.
        let opposite = Board::from_fen("8/5k2/3p4/8/2P5/1B6/5K2/6b1 w - - 0 1").unwrap();
        assert_eq!(
            square_colour(crate::bitboard::parse_square("b3").unwrap()),
            1,
            "b3 is a light square"
        );
        assert_eq!(drawishness_scale(&opposite), SCALE_DRAWN_BISHOPS);

        // The same position with the black bishop on a dark square is a normal game.
        let same = Board::from_fen("8/5k2/3p4/8/8/1B6/2P2K2/8 w - - 0 1").unwrap();
        assert_eq!(drawishness_scale(&same), SCALE_ONE, "only one bishop");

        // A knight each and no pawns anywhere: drawn whatever the pieces are.
        let minors = Board::from_fen("8/5k2/8/8/8/1N6/5K2/6n1 w - - 0 1").unwrap();
        assert_eq!(drawishness_scale(&minors), SCALE_DRAWN_MINOR);

        // A rook up with no pawns is *not* a drawn ending: material that can mate.
        let rook = Board::from_fen("8/5k2/8/8/8/R7/5K2/8 w - - 0 1").unwrap();
        assert_eq!(drawishness_scale(&rook), SCALE_ONE);
    }

    #[test]
    fn the_scale_only_ever_shrinks_a_score() {
        // A half-scaled score must stay on the same side of zero, or the scaling would
        // turn a winning position into a losing one.
        for fen in [
            "8/5k2/3p4/8/2P5/1B6/5K2/6b1 w - - 0 1",
            "8/5k2/8/8/8/1N6/5K2/6n1 w - - 0 1",
            "4k3/8/8/8/8/8/8/4K3 w - - 0 1",
        ] {
            let board = Board::from_fen(fen).unwrap();
            let scaled = evaluate_with(&board, EvalFeatures::all());
            let unscaled = evaluate_with(
                &board,
                EvalFeatures {
                    drawish_endings: false,
                    ..EvalFeatures::all()
                },
            );
            assert!(scaled.abs() <= unscaled.abs() + TEMPO, "{}: {} against {}", fen, scaled, unscaled);
            assert!(scaled.signum() == unscaled.signum() || scaled.abs() <= TEMPO);
        }
    }

    #[test]
    fn a_passed_pawn_is_worth_more_with_its_own_king_in_front_of_it() {
        // Both kings are the same distance away in one position and not in the
        // other; the promotion square is d8 and the pawn is on d5.
        let pawn_square = crate::bitboard::parse_square("d5").unwrap();
        let escort = Board::from_fen("8/8/8/3P4/3K4/8/8/4k3 w - - 0 1").unwrap();
        let distant = Board::from_fen("8/8/8/3P4/8/8/8/3Kk3 w - - 0 1").unwrap();
        assert!(
            passed_pawn_king_proximity(&escort, WHITE, pawn_square) > 0,
            "escorting king is four steps from d8, the defending king seven"
        );
        assert_eq!(
            passed_pawn_king_proximity(&distant, WHITE, pawn_square),
            0,
            "both kings are seven steps away"
        );
        // The term is a bonus for whoever owns the pawn, so the mirrored position
        // hands black exactly the same number.  That is the property that keeps the
        // whole evaluation antisymmetric when the colours swap.
        let mirrored_escort = Board::from_fen(&mirrored("8/8/8/3P4/3K4/8/8/4k3 w - - 0 1")).unwrap();
        let mirrored_pawn = crate::bitboard::parse_square("d4").unwrap();
        assert_eq!(
            passed_pawn_king_proximity(&escort, WHITE, pawn_square),
            passed_pawn_king_proximity(&mirrored_escort, BLACK, mirrored_pawn)
        );
    }

    #[test]
    fn every_term_can_be_switched_off_and_back_on() {
        let board = Board::from_fen("r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4")
            .unwrap();
        assert_eq!(evaluate(&board), evaluate_with(&board, EvalFeatures::all()));
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R b KQkq - 0 1",
        ] {
            let position = Board::from_fen(fen).unwrap();
            let mirror = Board::from_fen(&mirrored(fen)).unwrap();
            for features in [
                EvalFeatures::all(),
                EvalFeatures::none(),
                EvalFeatures {
                    mobility: true,
                    ..EvalFeatures::none()
                },
                EvalFeatures {
                    threats: true,
                    ..EvalFeatures::none()
                },
                EvalFeatures {
                    connected_pawns: true,
                    ..EvalFeatures::none()
                },
                EvalFeatures {
                    king_storm: true,
                    king_safety: true,
                    ..EvalFeatures::none()
                },
                EvalFeatures {
                    drawish_endings: true,
                    ..EvalFeatures::none()
                },
                EvalFeatures {
                    king_safety: true,
                    passed_pawn_king_proximity: true,
                    ..EvalFeatures::none()
                },
            ] {
                // The mirrored position hands the move to the other side, so both
                // readings are the mover's and have to agree.
                assert_eq!(
                    evaluate_with(&position, features),
                    evaluate_with(&mirror, features),
                    "different score for the mirrored {} with {:?}",
                    fen,
                    features
                );
            }
        }
    }

    #[test]
    fn the_orientation_of_the_score_is_the_side_to_moves() {
        // The search negates whatever a child returns, so a score has to be the
        // mover's.  This is the test that pins that down: the same imbalance has to
        // read positive for whoever is up the material, whichever colour that is.
        for (up, down) in [
            ("3Qk3/8/8/8/8/8/8/4K3 w - - 0 1", "whoever is to move has a queen"),
            ("4k3/8/8/8/8/8/8/3qK3 b - - 0 1", "black to move, black has a queen"),
        ] {
            let board = Board::from_fen(up).unwrap();
            let score = evaluate(&board);
            assert!(
                score > 700,
                "{} (mover up a queen) scored {}",
                down,
                score
            );
        }
        // And the mirror of the imbalance reads the same way round, because the mover
        // is the side that is up the material in both readings.
        let white_up = Board::from_fen("3Qk3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let black_up = Board::from_fen("4k3/8/8/8/8/8/8/3qK3 b - - 0 1").unwrap();
        assert!(evaluate(&white_up) > 700 && evaluate(&black_up) > 700);
        // A side that is *down* a queen has to read negative, which is the case the
        // first release got backwards.
        let white_down = Board::from_fen("4k3/8/8/8/8/8/8/3qK3 w - - 0 1").unwrap();
        let black_down = Board::from_fen("3Qk3/8/8/8/8/8/8/4K3 b - - 0 1").unwrap();
        assert!(evaluate(&white_down) < -700, "{}", evaluate(&white_down));
        assert!(evaluate(&black_down) < -700, "{}", evaluate(&black_down));
    }

    #[test]
    fn the_orientation_flag_reproduces_the_release_that_got_it_wrong() {
        // Two positions that differ only in whose turn it is read opposite ways with
        // the flag off and the same way — white's — with it on.
        let white_to_move = Board::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let black_to_move = Board::from_fen("4k3/8/8/8/8/8/8/3QK3 b - - 0 1").unwrap();

        assert!(evaluate(&white_to_move) > 700);
        assert!(evaluate(&black_to_move) < -700);

        let shipped = EvalFeatures::as_shipped();
        assert!(evaluate_with(&white_to_move, shipped) > 700);
        assert!(
            evaluate_with(&black_to_move, shipped) > 700,
            "the flagged reading is white's, whatever the side to move"
        );
        // Which also means the flagged reading *is* antisymmetric under mirroring,
        // the invariant the fixed reading deliberately drops.
        let startpos = Board::startpos();
        let mirrored_startpos = Board::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR b KQkq - 0 1")
            .unwrap();
        assert_eq!(
            evaluate_with(&mirrored_startpos, shipped),
            -evaluate_with(&startpos, shipped)
        );
        assert_eq!(evaluate(&mirrored_startpos), evaluate(&startpos));
    }

    #[test]
    fn the_starting_position_is_about_level() {
        let score = evaluate(&Board::startpos());
        assert!(score.abs() <= 60, "start position scored {}", score);
    }

    #[test]
    fn evaluation_is_the_same_for_the_mirrored_position() {
        // The score is the *mover's*, so mirroring a position — swapping the colours
        // and handing the move to the other side — has to give the same number: the
        // mover is exactly as well off in the mirrored world, tempo included.  (The
        // tempting opposite, `score == -mirror`, only holds for an evaluation written
        // from white's side, which is the mistake the orientation test pins down.)
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r1bqkbnr/pppp1ppp/2n5/4p3/2B1P3/5N2/PPPP1PPP/RNBQK2R w KQkq - 4 4",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R b KQkq - 0 1",
        ] {
            let board = Board::from_fen(fen).expect("legal FEN");
            let mirror = Board::from_fen(&mirrored(fen)).expect("legal mirrored FEN");
            assert_eq!(
                evaluate(&board),
                evaluate(&mirror),
                "different score for the mirrored position {}",
                fen
            );
        }
    }

    #[test]
    fn material_decides_the_score() {
        let queen_up = Board::from_fen("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").unwrap();
        let level = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").unwrap();
        let swing = evaluate(&queen_up) - evaluate(&level);
        assert!(swing > 700, "a queen was worth only {}", swing);

        // The same position with black to move and the queen: the mover is the side
        // with the queen, so the score is positive again rather than negative.
        let black_queen_to_move =
            Board::from_fen("4k3/8/8/8/8/8/8/3qK3 b - - 0 1").unwrap();
        assert!(
            evaluate(&black_queen_to_move) > 700,
            "black scored {}",
            evaluate(&black_queen_to_move)
        );
    }

    #[test]
    fn two_rooks_are_winning_but_not_double_counted() {
        let board = Board::from_fen("4k3/8/8/8/8/8/8/R3K2R w - - 0 1").unwrap();
        let score = evaluate(&board);
        assert!(score > 0, "two rooks up scored {}", score);
        assert!(score < 1200, "two rooks scored {} - double counted?", score);
    }

    #[test]
    fn a_passed_pawn_beats_a_doubled_one() {
        let passed = Board::from_fen("4k3/8/8/3P4/8/8/8/4K3 w - - 0 1").unwrap();
        let doubled = Board::from_fen("4k3/3p4/8/3P4/8/8/8/4K3 w - - 0 1").unwrap();
        assert!(
            evaluate(&passed) > evaluate(&doubled),
            "passed {} vs doubled {}",
            evaluate(&passed),
            evaluate(&doubled)
        );
    }

    #[test]
    fn the_passed_pawn_ladder_climbs_with_distance() {
        for rank in 1..6 {
            assert!(
                PASSED_PAWN_BONUS[rank].1 < PASSED_PAWN_BONUS[rank + 1].1,
                "rank {} should be worth less than rank {}",
                rank,
                rank + 1
            );
        }
        // The back and promotion ranks are not real pawn squares.
        assert_eq!(PASSED_PAWN_BONUS[0].1, 0);
        assert_eq!(PASSED_PAWN_BONUS[7].1, 0);
    }

    #[test]
    fn a_doubled_pawn_on_one_file_is_penalised_once_per_extra_pawn() {
        let doubled = Board::from_fen("4k3/8/8/8/8/3P4/3P4/4K3 w - - 0 1").unwrap();
        let split = Board::from_fen("4k3/8/8/8/8/8/3P1P2/4K3 w - - 0 1").unwrap();
        // Same material, worse structure: the doubled pair must score lower.
        assert!(
            evaluate(&doubled) < evaluate(&split),
            "doubled {} vs split {}",
            evaluate(&doubled),
            evaluate(&split)
        );
    }
}



