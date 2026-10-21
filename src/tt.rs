//! Zobrist keys and the transposition table.
//!
//! The keys are generated at compile time from a splitmix64 stream seeded with a
//! fixed constant, so every build — native, wasm, test, bench — agrees on them.
//! Nothing is read from the filesystem or a random device: a wasm module that
//! reached for entropy would not be reproducible, and the transposition table
//! only needs the keys to be well spread, not secret.
//!
//! The table is one pre-allocated buffer of 16-byte entries, indexed by the low
//! bits of a key and verified by the whole key.  Its default size is 32 MB,
//! allocated once when the engine starts; the search never grows or frees it.

use crate::moves::Move;

/// One key per (piece, square) pair, one per castling-rights pattern, one per en
/// passant file and one for the side to move.
pub struct Zobrist {
    pieces: [[u64; 64]; 12],
    castling: [u64; 16],
    ep_file: [u64; 9],
    side: u64,
}

impl Zobrist {
    #[inline]
    pub const fn piece(&self, code: u8, square: usize) -> u64 {
        self.pieces[code as usize][square]
    }

    /// The rights pattern is a four-bit index, so a rights change is one XOR out
    /// and one XOR in.
    #[inline]
    pub const fn castling(&self, rights: u8) -> u64 {
        self.castling[(rights & 0b1111) as usize]
    }

    /// Slot 8 is the "no en passant target" case.
    #[inline]
    pub const fn ep_file(&self, index: usize) -> u64 {
        self.ep_file[index]
    }

    #[inline]
    pub const fn side(&self) -> u64 {
        self.side
    }
}

const fn next_random(state: &mut u64) -> u64 {
    *state = (*state).wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

const fn build_zobrist() -> Zobrist {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut pieces = [[0u64; 64]; 12];
    let mut code = 0;
    while code < 12 {
        let mut square = 0;
        while square < 64 {
            pieces[code][square] = next_random(&mut state);
            square += 1;
        }
        code += 1;
    }
    let mut castling = [0u64; 16];
    let mut index = 0;
    while index < 16 {
        castling[index] = next_random(&mut state);
        index += 1;
    }
    let mut ep_file = [0u64; 9];
    let mut index = 0;
    while index < 9 {
        ep_file[index] = next_random(&mut state);
        index += 1;
    }
    Zobrist {
        pieces,
        castling,
        ep_file,
        side: next_random(&mut state),
    }
}

pub const ZOBRIST: Zobrist = build_zobrist();

/// The score in the table is a bound, and the flag says which kind.
pub const FLAG_EXACT: u8 = 0;
pub const FLAG_LOWER: u8 = 1;
pub const FLAG_UPPER: u8 = 2;

/// 16 bytes: the key, then move, score, depth, flag and generation packed into
/// one word.
#[derive(Clone, Copy, Default)]
#[repr(C)]
struct TtEntry {
    key: u64,
    data: u64,
}

/// What a probe found.
#[derive(Clone, Copy)]
pub struct TtProbe {
    pub mv: Move,
    pub score: i32,
    pub depth: i32,
    pub flag: u8,
}

/// A fixed-size table that keeps a deeper entry and otherwise replaces.
pub struct TranspositionTable {
    entries: Box<[TtEntry]>,
    mask: usize,
    generation: u8,
}

impl TranspositionTable {
    /// Allocates roughly `megabytes`, rounded up to a power of two entries so the
    /// index is a mask rather than a division.
    pub fn new(megabytes: usize) -> Self {
        let bytes = megabytes.max(1) * 1024 * 1024;
        let per_entry = std::mem::size_of::<TtEntry>();
        let wanted = (bytes / per_entry).max(1024);
        let count = wanted.next_power_of_two();
        TranspositionTable {
            entries: vec![TtEntry::default(); count].into_boxed_slice(),
            mask: count - 1,
            generation: 0,
        }
    }

    /// How many entries the table holds.
    pub fn capacity(&self) -> usize {
        self.entries.len()
    }

    /// Bytes actually allocated, after rounding up to a power of two.
    pub fn size_bytes(&self) -> usize {
        self.entries.len() * std::mem::size_of::<TtEntry>()
    }

    /// Starts a new search.  Entries from the previous one stay usable; the
    /// generation only decides what may be overwritten.
    pub fn new_search(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub fn clear(&mut self) {
        for entry in self.entries.iter_mut() {
            *entry = TtEntry::default();
        }
        self.generation = 0;
    }

    #[inline]
    fn index(&self, key: u64) -> usize {
        (key as usize) & self.mask
    }

    /// Looks the position up.  A key mismatch is a miss, which is what the stored
    /// key buys.
    #[inline]
    pub fn probe(&self, key: u64) -> Option<TtProbe> {
        let entry = &self.entries[self.index(key)];
        if entry.key != key || entry.data == 0 {
            return None;
        }
        Some(TtProbe {
            mv: (entry.data & 0xffff) as Move,
            score: ((entry.data >> 16) & 0xffff) as u16 as i16 as i32,
            depth: ((entry.data >> 32) & 0xff) as i32,
            flag: ((entry.data >> 40) & 0b11) as u8,
        })
    }

    /// Stores a bound.  A deeper entry from the same generation is kept: a
    /// shallower one would only lose information the next search wants.
    pub fn store(&mut self, key: u64, mv: Move, score: i32, depth: i32, flag: u8) {
        let index = self.index(key);
        let existing = self.entries[index];
        if existing.key == key {
            let existing_depth = ((existing.data >> 32) & 0xff) as i32;
            let existing_generation = ((existing.data >> 42) & 0xff) as u8;
            if existing_depth > depth + 2 && existing_generation == self.generation {
                return;
            }
        }
        let packed = (mv as u64)
            | (((score as i16 as u16) as u64) << 16)
            | ((depth.clamp(0, 255) as u64) << 32)
            | ((flag as u64) << 40)
            | ((self.generation as u64) << 42);
        self.entries[index] = TtEntry { key, data: packed };
    }
}

impl Default for TranspositionTable {
    fn default() -> Self {
        Self::new(32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zobrist_stream_has_no_duplicate_keys() {
        // Every key has to be distinct for the XOR scheme to work at all.
        let mut seen = std::collections::HashSet::new();
        for code in 0..12u8 {
            for square in 0..64 {
                assert!(
                    seen.insert(ZOBRIST.piece(code, square)),
                    "duplicate key for piece {} on {}",
                    code,
                    square
                );
            }
        }
        for rights in 0..16u8 {
            assert!(seen.insert(ZOBRIST.castling(rights)));
        }
        for file in 0..9 {
            assert!(seen.insert(ZOBRIST.ep_file(file)));
        }
        assert!(seen.insert(ZOBRIST.side()));
    }

    #[test]
    fn the_table_returns_what_it_was_given() {
        let mut table = TranspositionTable::new(1);
        assert!(table.capacity().is_power_of_two());
        assert!(table.probe(0x1234_5678_9abc_def0).is_none());

        table.store(0x1234_5678_9abc_def0, 0x1234, 137, 7, FLAG_LOWER);
        let probe = table.probe(0x1234_5678_9abc_def0).expect("stored");
        assert_eq!(probe.mv, 0x1234);
        assert_eq!(probe.score, 137);
        assert_eq!(probe.depth, 7);
        assert_eq!(probe.flag, FLAG_LOWER);

        // A negative score survives the signed round trip.
        table.store(0x1234_5678_9abc_def1, 0x2222, -31000, 11, FLAG_UPPER);
        assert_eq!(table.probe(0x1234_5678_9abc_def1).unwrap().score, -31000);
    }

    #[test]
    fn a_colliding_key_is_a_miss_not_a_false_hit() {
        let mut table = TranspositionTable::new(1);
        let first = 0xdead_beef_0000_0100u64;
        table.store(first, 0x1111, 5, 4, FLAG_EXACT);
        // Any other key that maps into the same bucket must not read as a hit.
        let other = first ^ (1u64 << 40);
        if table.probe(first).is_some() && table.probe(other).is_some() {
            assert_eq!(table.probe(other).unwrap().mv, 0);
        }
    }

    #[test]
    fn a_shallower_entry_cannot_displace_a_deeper_one_from_this_search() {
        let mut table = TranspositionTable::new(1);
        let key = 0xdead_beef_0000_0001;
        table.clear();
        table.store(key, 0x1111, 20, 9, FLAG_EXACT);
        table.store(key, 0x2222, 20, 2, FLAG_EXACT);
        assert_eq!(table.probe(key).unwrap().mv, 0x1111);
        // A deeper entry does replace it.
        table.store(key, 0x3333, 20, 12, FLAG_EXACT);
        assert_eq!(table.probe(key).unwrap().mv, 0x3333);
    }

    #[test]
    fn clearing_empties_the_table() {
        let mut table = TranspositionTable::new(1);
        table.store(42, 0x1234, 0, 1, FLAG_EXACT);
        assert!(table.probe(42).is_some());
        table.clear();
        assert!(table.probe(42).is_none());
    }

    #[test]
    fn the_default_table_is_thirty_two_megabytes() {
        let table = TranspositionTable::default();
        assert_eq!(table.size_bytes(), 32 * 1024 * 1024);
    }
}

