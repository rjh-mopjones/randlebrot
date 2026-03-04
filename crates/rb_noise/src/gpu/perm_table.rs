//! Permutation table generation matching the noise crate's OpenSimplex.
//!
//! This generates the exact same permutation table as the `noise` crate,
//! ensuring GPU and CPU noise output is identical.

use rand::{seq::SliceRandom, SeedableRng};
use rand_xorshift::XorShiftRng;

const TABLE_SIZE: usize = 256;

/// Generate a permutation table identical to the noise crate's PermutationTable::new().
///
/// The noise crate uses XorShiftRng seeded in a specific way, then shuffles [0..255].
pub fn generate_permutation_table(seed: u32) -> [u8; TABLE_SIZE] {
    // Replicate the exact seeding from noise crate's PermutationTable::new()
    let mut real = [0u8; 16];
    real[0] = 1;
    for i in 1..4 {
        real[i * 4] = seed as u8;
        real[(i * 4) + 1] = (seed >> 8) as u8;
        real[(i * 4) + 2] = (seed >> 16) as u8;
        real[(i * 4) + 3] = (seed >> 24) as u8;
    }

    let mut rng: XorShiftRng = SeedableRng::from_seed(real);

    // Initialize with [0, 1, 2, ..., 255]
    let mut table = [0u8; TABLE_SIZE];
    for (i, val) in table.iter_mut().enumerate() {
        *val = i as u8;
    }

    // Shuffle using Fisher-Yates (same as SliceRandom::shuffle)
    table.shuffle(&mut rng);

    table
}

/// Convert permutation table to u32 array for GPU buffer.
/// Pads each u8 value to u32 for simpler GPU access.
pub fn permutation_table_to_u32(table: &[u8; TABLE_SIZE]) -> [u32; TABLE_SIZE] {
    let mut result = [0u32; TABLE_SIZE];
    for (i, &val) in table.iter().enumerate() {
        result[i] = val as u32;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use noise::{NoiseFn, OpenSimplex};

    #[test]
    fn permutation_table_is_valid() {
        let table = generate_permutation_table(42);

        // Check it's a valid permutation (contains 0-255 exactly once each)
        let mut counts = [0u32; 256];
        for &val in &table {
            counts[val as usize] += 1;
        }
        for count in &counts {
            assert_eq!(*count, 1, "Each value 0-255 should appear exactly once");
        }
    }

    #[test]
    fn different_seeds_produce_different_tables() {
        let table1 = generate_permutation_table(42);
        let table2 = generate_permutation_table(123);

        // Tables should be different
        assert_ne!(table1, table2);
    }

    #[test]
    fn same_seed_produces_same_table() {
        let table1 = generate_permutation_table(42);
        let table2 = generate_permutation_table(42);

        assert_eq!(table1, table2);
    }
}
