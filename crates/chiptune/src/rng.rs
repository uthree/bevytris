//! A tiny deterministic PRNG. The composer must be reproducible from a
//! seed forever (the "now playing" toast prints it, and the tests assert
//! byte-identical renders), so it cannot depend on `rand`'s algorithm
//! staying put across versions.

/// xorshift64*, seeded so that 0 is not a fixed point.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Uniform in `0..n`. Panics on an empty range, which is always a bug
    /// in the caller rather than something to paper over.
    pub fn below(&mut self, n: usize) -> usize {
        assert!(n > 0, "empty range");
        (self.next_u64() % n as u64) as usize
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as usize) as i32
    }

    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1 << 24) as f32
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// Index into `weights` proportionally. Weights must be non-negative
    /// and not all zero.
    pub fn weighted(&mut self, weights: &[f32]) -> usize {
        let total: f32 = weights.iter().sum();
        let mut t = self.unit() * total;
        for (i, w) in weights.iter().enumerate() {
            t -= w;
            if t <= 0.0 {
                return i;
            }
        }
        weights.len() - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_stream() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..64 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn ranges_stay_in_bounds() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let v = r.range(-3, 5);
            assert!((-3..=5).contains(&v));
            assert!(r.below(4) < 4);
            let u = r.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn weighted_never_picks_a_zero_weight() {
        let mut r = Rng::new(99);
        for _ in 0..2_000 {
            assert_ne!(r.weighted(&[1.0, 0.0, 1.0]), 1);
        }
    }
}
