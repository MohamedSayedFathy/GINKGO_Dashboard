use rand::SeedableRng;
use rand_chacha::ChaCha20Rng;

/// Create a deterministic [`ChaCha20Rng`] from a u64 seed.
pub fn rng_from_seed(seed: u64) -> ChaCha20Rng {
    ChaCha20Rng::seed_from_u64(seed)
}
