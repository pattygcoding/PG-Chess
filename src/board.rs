//! Board state: the piece sets, the FEN reader/writer, and make/unmake.
//!
//! The board keeps three views of the same position, all updated incrementally
//! by `make_move` and restored exactly by `unmake_move`:
//!
//! * `pieces` and `colour` — the bitboards the move generator and evaluator read.
//! * `mailbox` — one piece code per square, so "what is on d5" is an array index
//!   instead of a set intersection.  This is what SAN generation uses.
//! * `hash` — the Zobrist key the transposition table and repetition checker use.
//!
//! `make_move` is the legality gate: it plays the move and then asks whether the
//! mover's king is attacked, rolling the move back if it is.  That is what lets
//! the generator be pseudo-legal without ever letting an illegal move into the
//! tree.  A test at the bottom of this file checks the incremental hash against a
//! from-scratch recomputation after every move of a long game, which is the
//! cheapest way to catch a missed update in any of the three views.

use crate::bitboard::{
    bishop_attacks, king_attacks, knight_attacks, pawn_attackers, rook_attacks, square_bb,
    square_index, square_name, Bitboard,
};
use crate::moves::{self, Move};
use crate::tt::ZOBRIST;

pub const WHITE: usize = 0;
pub const BLACK: usize = 1;

pub const PAWN: usize = 0;
pub const KNIGHT: usize = 1;
pub const BISHOP: usize = 2;
pub const ROOK: usize = 3;
pub const QUEEN: usize = 4;
pub const KING: usize = 5;

/// The mailbox's empty marker.  A `u8` rather than an `Option` so the array
/// stays a plain byte buffer.
pub const NO_PIECE: u8 = 255;

/// Castling rights, one bit each.
pub const CASTLE_WK: u8 = 1;
pub const CASTLE_WQ: u8 = 2;
pub const CASTLE_BK: u8 = 4;
pub const CASTLE_BQ: u8 = 8;

/// `ep` holds 0-63 for a real en passant target and this for "none".
pub const NO_EP: u8 = 64;

/// A piece code packs colour and type, so the mailbox is one byte per square.
#[inline]
pub const fn piece_code(colour: usize, kind: usize) -> u8 {
    (colour * 6 + kind) as u8
}

#[inline]
pub const fn code_colour(code: u8) -> usize {
    (code / 6) as usize
}

#[inline]
pub const fn code_kind(code: u8) -> usize {
    (code % 6) as usize
}

/// What `unmake_move` needs to put the position back exactly.
#[derive(Clone, Copy, Default)]
pub struct Undo {
    pub mv: Move,
    /// The captured piece's code, or `NO_PIECE`.
    pub captured: u8,
    /// Where it was taken from.  For an en passant capture that is not the
    /// destination square, which is why it is stored rather than derived.
    pub capture_square: u8,
    pub castling: u8,
    pub ep: u8,
    pub halfmove: u16,
    /// The key of the position before the move.  Restoring it is what makes the
    /// incremental hash exactly reversible; the tests check it against a
    /// from-scratch recomputation.
    pub hash: u64,
}

const CASTLE_RIGHTS_MASK: [u8; 64] = build_castle_rights_mask();

/// Moving a king or a rook off its home square, or capturing one there, retires
/// that castling right.  ANDing the masks for a move's two squares gives the
/// rights that survive it.
const fn build_castle_rights_mask() -> [u8; 64] {
    let mut table = [0b1111u8; 64];
    const ALL: u8 = 0b1111;
    // A king leaving e1 or e8 retires both rights for its colour; a rook leaving
    // a1, h1, a8 or h8 retires the one on its wing.
    table[square_index(4, 0)] &= ALL & !(CASTLE_WK | CASTLE_WQ);
    table[square_index(0, 0)] &= ALL & !CASTLE_WQ;
    table[square_index(7, 0)] &= ALL & !CASTLE_WK;
    table[square_index(4, 7)] &= ALL & !(CASTLE_BK | CASTLE_BQ);
    table[square_index(0, 7)] &= ALL & !CASTLE_BQ;
    table[square_index(7, 7)] &= ALL & !CASTLE_BK;
    table
}

/// A complete chess position.
#[derive(Clone)]
pub struct Board {
    /// Piece sets by type, both colours together.
    pub pieces: [Bitboard; 6],
    /// Piece sets by colour.
    pub colour: [Bitboard; 2],
    /// One code per square (`piece_code`), or `NO_PIECE`.
    pub mailbox: [u8; 64],
    /// `WHITE` or `BLACK`: the side to move.
    pub side: usize,
    /// The `CASTLE_*` bits still available.
    pub castling: u8,
    /// The en passant target square, or `NO_EP`.
    pub ep: u8,
    /// Plies since the last pawn move or capture (the fifty-move counter).
    pub halfmove: u16,
    /// The move number, starting at 1.
    pub fullmove: u16,
    /// The Zobrist key of this position.
    pub hash: u64,
    /// Where each king stands, so the attack check never has to search for it.
    pub kings: [usize; 2],
    /// Keys of every position since the root, for repetition detection.
    pub history: Vec<u64>,
    /// One record per move played, for `unmake_move`.
    undo: Vec<Undo>,
}

impl Board {
    /// The standard starting position.
    pub fn startpos() -> Self {
        Self::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1")
            .expect("the starting position is a legal FEN")
    }

    /// Reads a FEN string.  The last two fields are optional, because plenty of
    /// published positions leave them off.
    pub fn from_fen(fen: &str) -> Option<Self> {
        let mut board = Board {
            pieces: [0; 6],
            colour: [0; 2],
            mailbox: [NO_PIECE; 64],
            side: WHITE,
            castling: 0,
            ep: NO_EP,
            halfmove: 0,
            fullmove: 1,
            hash: 0,
            kings: [0, 0],
            history: Vec::with_capacity(1024),
            undo: Vec::with_capacity(1024),
        };

        let fields: Vec<&str> = fen.split_whitespace().collect();
        if fields.is_empty() {
            return None;
        }

        // Field 1: the ranks, eighth down to first.
        let mut rank = 7i32;
        let mut file = 0i32;
        for ch in fields[0].chars() {
            match ch {
                '/' => {
                    if file != 8 {
                        return None;
                    }
                    rank -= 1;
                    file = 0;
                }
                '1'..='8' => file += ch as i32 - '0' as i32,
                _ => {
                    let (colour, kind) = match ch {
                        'P' => (WHITE, PAWN),
                        'N' => (WHITE, KNIGHT),
                        'B' => (WHITE, BISHOP),
                        'R' => (WHITE, ROOK),
                        'Q' => (WHITE, QUEEN),
                        'K' => (WHITE, KING),
                        'p' => (BLACK, PAWN),
                        'n' => (BLACK, KNIGHT),
                        'b' => (BLACK, BISHOP),
                        'r' => (BLACK, ROOK),
                        'q' => (BLACK, QUEEN),
                        'k' => (BLACK, KING),
                        _ => return None,
                    };
                    if rank < 0 || file > 7 {
                        return None;
                    }
                    board.put_piece(colour, kind, square_index(file as usize, rank as usize));
                    file += 1;
                }
            }
        }
        if rank != 0 || file != 8 {
            return None;
        }

        if let Some(side) = fields.get(1) {
            board.side = match *side {
                "w" => WHITE,
                "b" => BLACK,
                _ => return None,
            };
        }

        if let Some(rights) = fields.get(2).copied() {
            if rights != "-" {
                for ch in rights.chars() {
                    board.castling |= match ch {
                        'K' => CASTLE_WK,
                        'Q' => CASTLE_WQ,
                        'k' => CASTLE_BK,
                        'q' => CASTLE_BQ,
                        _ => return None,
                    };
                }
            }
        }

        if let Some(ep) = fields.get(3).copied() {
            if ep != "-" {
                board.ep = crate::bitboard::parse_square(ep)? as u8;
            }
        }

        if let Some(halfmove) = fields.get(4) {
            board.halfmove = halfmove.parse().ok()?;
        }
        if let Some(fullmove) = fields.get(5) {
            board.fullmove = fullmove.parse().ok()?;
        }

        board.hash = board.compute_hash();
        board.history.push(board.hash);
        Some(board)
    }

    /// Writes the position back out in full six-field FEN form.
    pub fn to_fen(&self) -> String {
        let mut out = String::with_capacity(90);
        for rank in (0..8).rev() {
            let mut empty = 0;
            for file in 0..8 {
                let code = self.mailbox[square_index(file, rank)];
                if code == NO_PIECE {
                    empty += 1;
                    continue;
                }
                if empty > 0 {
                    out.push_str(&empty.to_string());
                    empty = 0;
                }
                out.push(piece_letter(code));
            }
            if empty > 0 {
                out.push_str(&empty.to_string());
            }
            if rank > 0 {
                out.push('/');
            }
        }

        out.push(' ');
        out.push(if self.side == WHITE { 'w' } else { 'b' });

        out.push(' ');
        if self.castling == 0 {
            out.push('-');
        } else {
            if self.castling & CASTLE_WK != 0 {
                out.push('K');
            }
            if self.castling & CASTLE_WQ != 0 {
                out.push('Q');
            }
            if self.castling & CASTLE_BK != 0 {
                out.push('k');
            }
            if self.castling & CASTLE_BQ != 0 {
                out.push('q');
            }
        }

        out.push(' ');
        if self.ep == NO_EP {
            out.push('-');
        } else {
            out.push_str(&square_name(self.ep as usize));
        }

        out.push(' ');
        out.push_str(&self.halfmove.to_string());
        out.push(' ');
        out.push_str(&self.fullmove.to_string());
        out
    }

    /// Adds a piece.  Only the FEN reader and `apply` call this, because it does
    /// not touch the hash: the FEN reader recomputes it, and `apply` XORs the
    /// same key in itself.
    fn put_piece(&mut self, colour: usize, kind: usize, sq: usize) {
        self.pieces[kind] |= square_bb(sq);
        self.colour[colour] |= square_bb(sq);
        self.mailbox[sq] = piece_code(colour, kind);
        if kind == KING {
            self.kings[colour] = sq;
        }
    }

    /// Removes whatever stands on `sq`.
    fn remove_piece(&mut self, sq: usize) {
        let code = self.mailbox[sq];
        if code == NO_PIECE {
            return;
        }
        let colour = code_colour(code);
        let kind = code_kind(code);
        self.pieces[kind] &= !square_bb(sq);
        self.colour[colour] &= !square_bb(sq);
        self.mailbox[sq] = NO_PIECE;
    }

    #[inline]
    pub fn occupied(&self) -> Bitboard {
        self.colour[WHITE] | self.colour[BLACK]
    }

    /// One colour's set of one piece type, the shape the evaluator wants.
    #[inline]
    pub fn pieces_of(&self, colour: usize, kind: usize) -> Bitboard {
        self.pieces[kind] & self.colour[colour]
    }

    #[inline]
    pub fn piece_on(&self, sq: usize) -> u8 {
        self.mailbox[sq]
    }

    #[inline]
    pub fn side_to_move(&self) -> usize {
        self.side
    }

    /// Plies played since this board was read from a FEN (or the last reset).
    #[inline]
    pub fn ply_count(&self) -> usize {
        self.undo.len()
    }

    /// The move that produced the current position, if any.
    pub fn last_move(&self) -> Option<Move> {
        self.undo.last().map(|undo| undo.mv)
    }

    /// True when the side to move's king is attacked.
    #[inline]
    pub fn in_check(&self) -> bool {
        self.is_attacked(self.kings[self.side], 1 - self.side)
    }

    /// True when `sq` is attacked by any piece of colour `by`.
    ///
    /// Every test is a reverse lookup: instead of generating the enemy's attacks
    /// and intersecting, it asks whether an enemy piece of each type stands on a
    /// square that reaches `sq`.
    pub fn is_attacked(&self, sq: usize, by: usize) -> bool {
        let occupied = self.occupied();
        let theirs = self.colour[by];
        if pawn_attackers(sq, by) & self.pieces[PAWN] & theirs != 0 {
            return true;
        }
        if knight_attacks(sq) & self.pieces[KNIGHT] & theirs != 0 {
            return true;
        }
        if king_attacks(sq) & self.pieces[KING] & theirs != 0 {
            return true;
        }
        let queens = self.pieces[QUEEN] & theirs;
        if rook_attacks(sq, occupied) & (self.pieces[ROOK] | queens) & theirs != 0 {
            return true;
        }
        if bishop_attacks(sq, occupied) & (self.pieces[BISHOP] | queens) & theirs != 0 {
            return true;
        }
        false
    }

    /// Plays `mv` if it is legal, and says whether it was.
    ///
    /// This is the legality gate: the move generator is pseudo-legal, and the
    /// only reason an illegal move can never reach the tree is that playing it
    /// leaves the mover's own king attacked, which this rolls back.
    pub fn make_move(&mut self, mv: Move) -> bool {
        let us = self.side;
        self.apply(mv);
        if self.is_attacked(self.kings[us], self.side) {
            self.unmake_move();
            false
        } else {
            true
        }
    }

    /// Plays `mv` without checking legality.
    fn apply(&mut self, mv: Move) {
        let us = self.side;
        let from = moves::from_square(mv);
        let to = moves::to_square(mv);
        let moving = self.mailbox[from];
        let kind = code_kind(moving);

        // A capture is usually on the destination, but an en passant capture
        // takes a pawn that stands beside it.
        let mut captured = self.mailbox[to];
        let mut capture_square = to as u8;
        if kind == PAWN && captured == NO_PIECE && moves::is_diagonal(mv) {
            let behind = if us == WHITE { to - 8 } else { to + 8 };
            if self.mailbox[behind] != NO_PIECE {
                captured = self.mailbox[behind];
                capture_square = behind as u8;
            }
        }

        self.undo.push(Undo {
            mv,
            captured,
            capture_square,
            castling: self.castling,
            ep: self.ep,
            halfmove: self.halfmove,
            hash: self.hash,
        });

        let mut hash = self.hash;

        // Retire the castling rights the move leaves behind.
        let rights = self.castling & CASTLE_RIGHTS_MASK[from] & CASTLE_RIGHTS_MASK[to];
        hash ^= ZOBRIST.castling(self.castling) ^ ZOBRIST.castling(rights);
        self.castling = rights;

        // Clear the old en passant target and lift the captured piece.
        hash ^= ZOBRIST.ep_file(self.ep_hash_index());
        self.ep = NO_EP;
        if captured != NO_PIECE {
            hash ^= ZOBRIST.piece(captured, capture_square as usize);
            self.remove_piece(capture_square as usize);
        }

        // Move the piece, promoting on arrival if the move says so.
        self.remove_piece(from);
        hash ^= ZOBRIST.piece(moving, from);
        if moves::is_promotion(mv) {
            let promoted = piece_code(us, moves::promotion_piece(mv));
            hash ^= ZOBRIST.piece(promoted, to);
            self.put_piece(us, moves::promotion_piece(mv), to);
        } else {
            hash ^= ZOBRIST.piece(moving, to);
            self.put_piece(us, kind, to);
        }

        // A castle moves a rook as well as the king.
        if kind == KING && moves::is_castle(mv) {
            let (rook_from, rook_to) = rook_squares(from, to);
            let rook = self.mailbox[rook_from];
            self.remove_piece(rook_from);
            self.put_piece(us, ROOK, rook_to);
            hash ^= ZOBRIST.piece(rook, rook_from) ^ ZOBRIST.piece(rook, rook_to);
        }

        // The fifty-move clock resets on a pawn move or a capture.
        if kind == PAWN || captured != NO_PIECE {
            self.halfmove = 0;
        } else {
            self.halfmove += 1;
        }

        // A double push offers the square it skipped.  Whether that offer counts
        // depends on the side to move, so its key is added once the side is set.
        if kind == PAWN && moves::is_double_push(mv) {
            self.ep = ((from + to) / 2) as u8;
        }

        if us == BLACK {
            self.fullmove += 1;
        }
        self.side = 1 - us;
        hash ^= ZOBRIST.side();
        hash ^= ZOBRIST.ep_file(self.ep_hash_index());

        self.hash = hash;
        self.history.push(hash);
    }

    /// Takes the last move back.  Returns false when there is nothing to undo.
    pub fn unmake_move(&mut self) -> bool {
        let undo = match self.undo.pop() {
            Some(undo) => undo,
            None => return false,
        };
        self.history.pop();

        let mv = undo.mv;
        let from = moves::from_square(mv);
        let to = moves::to_square(mv);
        // The side that made the move is the one *not* to move now.
        let us = 1 - self.side;
        let moving = self.mailbox[to];
        let kind = code_kind(moving);

        // Bring the piece home, turning a promoted piece back into a pawn.
        if moves::is_promotion(mv) {
            self.remove_piece(to);
            self.put_piece(us, PAWN, from);
        } else {
            self.remove_piece(to);
            self.put_piece(us, kind, from);
        }

        if kind == KING && moves::is_castle(mv) {
            let (rook_from, rook_to) = rook_squares(from, to);
            self.remove_piece(rook_to);
            self.put_piece(us, ROOK, rook_from);
        }

        if undo.captured != NO_PIECE {
            self.put_piece(
                code_colour(undo.captured),
                code_kind(undo.captured),
                undo.capture_square as usize,
            );
        }

        self.castling = undo.castling;
        self.ep = undo.ep;
        self.halfmove = undo.halfmove;
        if us == BLACK {
            self.fullmove -= 1;
        }
        self.side = us;
        self.hash = undo.hash;
        true
    }

    /// Passes the move.  The search uses this for null-move pruning, where the
    /// question is "how would this stand if it were the opponent's turn".
    ///
    /// A null record is marked by move 0, which is a1a1 and therefore never a real
    /// move, so `unmake_null_move` can tell the two kinds of record apart.
    pub fn make_null_move(&mut self) {
        self.undo.push(Undo {
            mv: 0,
            captured: NO_PIECE,
            capture_square: 0,
            castling: self.castling,
            ep: self.ep,
            halfmove: self.halfmove,
            hash: self.hash,
        });
        let mut hash = self.hash;
        hash ^= ZOBRIST.ep_file(self.ep_hash_index());
        // Passing forfeits the en passant target: on the next ply it is nobody's
        // turn to capture there.
        self.ep = NO_EP;
        self.side = 1 - self.side;
        hash ^= ZOBRIST.side();
        self.hash = hash;
        self.history.push(hash);
    }

    /// Takes a null move back.
    pub fn unmake_null_move(&mut self) {
        if let Some(undo) = self.undo.pop() {
            self.history.pop();
            self.side = 1 - self.side;
            self.ep = undo.ep;
            self.halfmove = undo.halfmove;
            self.hash = undo.hash;
        }
    }

    /// The Zobrist key of the current position, recomputed from the pieces.
    ///
    /// `apply` maintains `self.hash` incrementally; this is the independent
    /// version the tests compare that against, and what `compute_hash` is for.
    pub fn compute_hash(&self) -> u64 {
        let mut hash = 0u64;
        for sq in 0..64 {
            let code = self.mailbox[sq];
            if code != NO_PIECE {
                hash ^= ZOBRIST.piece(code, sq);
            }
        }
        hash ^= ZOBRIST.castling(self.castling);
        hash ^= ZOBRIST.ep_file(self.ep_hash_index());
        if self.side == BLACK {
            hash ^= ZOBRIST.side();
        }
        hash
    }

    /// The Zobrist slot for the en passant file.
    ///
    /// A target square only changes the key when a pawn can actually capture on
    /// it, so two positions that differ only by an unreachable en passant target
    /// still hash as one position, which is what FIDE's repetition rule says.
    /// Slot 8 means "no target".
    pub fn ep_hash_index(&self) -> usize {
        if self.ep == NO_EP {
            return 8;
        }
        let ep = self.ep as usize;
        // The square the double-pushed pawn now stands on: the target is always
        // one square behind it, in the direction it was travelling.
        let pushed = if self.side == WHITE { ep + 8 } else { ep - 8 };
        if pawn_attackers(pushed, self.side) & self.pieces_of(self.side, PAWN) != 0 {
            crate::bitboard::file_of(ep)
        } else {
            8
        }
    }

    /// How many times the current position has occurred in this game.
    ///
    /// Positions repeat with the same side to move, so the walk steps back two
    /// plies at a time, and only as far as the fifty-move counter reaches: no
    /// position before the last capture or pawn move can come back.
    pub fn repetition_count(&self) -> u32 {
        let len = self.history.len();
        if len < 3 {
            return 1;
        }
        let earliest = len.saturating_sub(1 + self.halfmove as usize);
        let mut count = 1;
        let mut index = len - 3;
        loop {
            if self.history[index] == self.hash {
                count += 1;
            }
            if index <= earliest + 1 || index < 2 {
                break;
            }
            index -= 2;
        }
        count
    }

    /// True when the position has been seen at least once before.  The search
    /// wants this (a second occurrence is enough to score it as a draw), while
    /// the UI's game-over check wants the third.
    pub fn is_repetition(&self) -> bool {
        self.repetition_count() > 1
    }

    /// True when a player may claim a draw under the fifty-move rule.
    pub fn is_fifty_move_draw(&self) -> bool {
        self.halfmove >= 100
    }

    /// True when neither side can possibly mate.
    ///
    /// The cases are king against king, king and a single minor against king,
    /// and king and bishop against king and bishop with both bishops on the same
    /// colour complex — plus the same-coloured-bishop pairs a heavy exchange
    /// leaves behind.
    pub fn is_insufficient_material(&self) -> bool {
        let pawns = self.pieces[PAWN];
        let rooks = self.pieces[ROOK];
        let queens = self.pieces[QUEEN];
        if pawns != 0 || rooks != 0 || queens != 0 {
            return false;
        }
        let knights = crate::bitboard::popcount(self.pieces[KNIGHT]);
        let bishops = crate::bitboard::popcount(self.pieces[BISHOP]);
        match (knights, bishops) {
            // King against king, or with one minor piece.
            (0, 0) | (1, 0) | (0, 1) => true,
            // Two knights cannot force mate, but they *can* mate a careless
            // king, so this is deliberately not called a dead draw.
            _ => {
                // Any number of bishops, all on one colour complex: the only
                // case that is unquestionably dead is bishop against bishop on
                // the same squares' colour.
                if knights == 0 && bishops == 2 {
                    let light = 0x55aa_55aa_55aa_55aau64;
                    let (mut only_light, mut only_dark) = (true, true);
                    for colour in [WHITE, BLACK] {
                        let mut pieces = self.pieces_of(colour, BISHOP);
                        while pieces != 0 {
                            let sq = crate::bitboard::pop_lsb(&mut pieces);
                            if light & square_bb(sq) != 0 {
                                only_dark = false;
                            } else {
                                only_light = false;
                            }
                        }
                    }
                    only_light || only_dark
                } else {
                    false
                }
            }
        }
    }
}

/// The second half of the board's interface: the reads the UI and the tests make,
/// kept in their own block so the file reads as "state, then queries".
impl Board {
    /// Every legal move in the position.
    ///
    /// The search calls the generator directly so that it can keep the
    /// pseudo-legal fast path; this filtered version is what the UI, the
    /// game-over check and the SAN writer use.
    pub fn legal_moves(&self) -> crate::moves::MoveList {
        let mut list = crate::moves::MoveList::new();
        crate::movegen::generate_legal(self, &mut list);
        list
    }

    /// How the game stands, from the perspective of the side to move.
    pub fn outcome(&self) -> Outcome {
        let moves = self.legal_moves();
        if moves.is_empty() {
            return if self.in_check() {
                Outcome::Checkmate
            } else {
                Outcome::Stalemate
            };
        }
        if self.is_fifty_move_draw() {
            return Outcome::DrawFiftyMove;
        }
        if self.repetition_count() >= 3 {
            return Outcome::DrawRepetition;
        }
        if self.is_insufficient_material() {
            return Outcome::DrawMaterial;
        }
        Outcome::Playing
    }

    /// Writes a move in Standard Algebraic Notation.
    ///
    /// The `+` / `#` suffix is the reason this plays the move on a copy: whether
    /// a move gives check, and whether the reply has any legal answers, is only
    /// knowable from the position after it.
    pub fn move_san(&self, mv: Move) -> String {
        let from = crate::moves::from_square(mv);
        let to = crate::moves::to_square(mv);
        let moving = self.mailbox[from];
        let kind = code_kind(moving);
        let captured = self.mailbox[to] != NO_PIECE || self.en_passant_capture(mv);

        let mut san = String::with_capacity(8);
        if kind == KING && crate::moves::is_castle(mv) {
            san.push_str(if to > from { "O-O" } else { "O-O-O" });
        } else {
            if kind != PAWN {
                san.push(piece_letter(piece_code(WHITE, kind)));
            }
            if kind != PAWN {
                san.push_str(&self.san_disambiguation(mv, kind, to));
            } else if captured {
                // A pawn capture always names the file it left.
                san.push((b'a' + crate::bitboard::file_of(from) as u8) as char);
            }
            if captured {
                san.push('x');
            }
            san.push_str(&square_name(to));
            if crate::moves::is_promotion(mv) {
                san.push('=');
                san.push(piece_letter(piece_code(WHITE, crate::moves::promotion_piece(mv))));
            }
        }

        let mut probe = self.clone();
        if probe.make_move(mv) && probe.in_check() {
            san.push(if probe.legal_moves().is_empty() { '#' } else { '+' });
        }
        san
    }

    /// The file, rank or both, needed when another identical piece could also
    /// reach `to`.
    fn san_disambiguation(&self, mv: Move, kind: usize, to: usize) -> String {
        let from = crate::moves::from_square(mv);
        let mut same_file = false;
        let mut same_rank = false;
        let mut ambiguous = false;
        for other in &self.legal_moves() {
            if other == mv || crate::moves::to_square(other) != to {
                continue;
            }
            let other_from = crate::moves::from_square(other);
            if self.mailbox[other_from] != self.mailbox[from] {
                continue;
            }
            if code_kind(self.mailbox[other_from]) != kind {
                continue;
            }
            ambiguous = true;
            if crate::bitboard::file_of(other_from) == crate::bitboard::file_of(from) {
                same_file = true;
            }
            if crate::bitboard::rank_of(other_from) == crate::bitboard::rank_of(from) {
                same_rank = true;
            }
        }
        if !ambiguous {
            return String::new();
        }
        let mut out = String::with_capacity(2);
        if !same_file {
            out.push((b'a' + crate::bitboard::file_of(from) as u8) as char);
        }
        if !same_rank {
            out.push((b'1' + crate::bitboard::rank_of(from) as u8) as char);
        }
        if out.is_empty() {
            out.push((b'a' + crate::bitboard::file_of(from) as u8) as char);
            out.push((b'1' + crate::bitboard::rank_of(from) as u8) as char);
        }
        out
    }

    /// True when `mv` is a pawn moving diagonally onto an empty en passant
    /// target, which is the only capture that does not land on its victim.
    fn en_passant_capture(&self, mv: Move) -> bool {
        let from = crate::moves::from_square(mv);
        let to = crate::moves::to_square(mv);
        code_kind(self.mailbox[from]) == PAWN
            && crate::moves::is_diagonal(mv)
            && self.ep != NO_EP
            && to == self.ep as usize
            && self.mailbox[to] == NO_PIECE
    }
}

/// How a game stands, once no further moves are needed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Playing,
    Checkmate,
    Stalemate,
    DrawFiftyMove,
    DrawRepetition,
    DrawMaterial,
}

impl Outcome {
    /// The slug the UI switches on.
    pub fn slug(self) -> &'static str {
        match self {
            Outcome::Playing => "playing",
            Outcome::Checkmate => "checkmate",
            Outcome::Stalemate => "stalemate",
            Outcome::DrawFiftyMove => "fifty-move",
            Outcome::DrawRepetition => "repetition",
            Outcome::DrawMaterial => "insufficient-material",
        }
    }

    /// True when the game is over.
    pub fn is_over(self) -> bool {
        self != Outcome::Playing
    }
}

/// The rook's two squares for a castle, given the king's.
#[inline]
fn rook_squares(from: usize, to: usize) -> (usize, usize) {
    if to > from {
        (from + 3, from + 1)
    } else {
        (from - 4, from - 1)
    }
}

/// The FEN letter for a piece code, upper case for white.
pub const fn piece_letter(code: u8) -> char {
    let letters = ['p', 'n', 'b', 'r', 'q', 'k'];
    let letter = letters[code_kind(code)];
    if code_colour(code) == WHITE {
        letter.to_ascii_uppercase()
    } else {
        letter
    }
}

