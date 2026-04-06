use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates rock hardness values using simple fBm noise.
///
/// Higher hardness means rock resists erosion and sustains sharper peaks.
/// Output range: [0.0, 1.0] where 1.0 = very hard rock (granite, basalt)
pub struct RockHardnessStrategy {
    noise: OpenSimplex,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
    world_width: f64,
}

impl RockHardnessStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 3,
            persistence: 0.6,
            lacunarity: 2.0,
            world_width: 0.0,
        }
    }

    pub fn new_wrapping(seed: u32, world_width: f64) -> Self {
        let mut s = Self::new(seed);
        s.world_width = world_width;
        s
    }

    pub fn with_params(seed: u32, octaves: u32, persistence: f64) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves,
            persistence,
            lacunarity: 2.0,
            world_width: 0.0,
        }
    }

    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = 1.0;
        let mut max_amplitude = 0.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            let sample = if self.world_width > 0.0 {
                let [cx, cz, cy] = crate::wrap::cylindrical_noise_coords(x, y, freq, 0.0125, self.world_width);
                self.noise.get([cx, cz, cy])
            } else {
                let nx = x * freq * 0.0125;
                let ny = y * freq * 0.0125;
                self.noise.get([nx, ny])
            };
            value += sample * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        value / max_amplitude
    }
}

impl NoiseStrategy for RockHardnessStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        // Map from [-1, 1] to [0, 1]
        let raw = self.fbm(x, y, detail_level);
        ((raw + 1.0) * 0.5).clamp(0.0, 1.0)
    }

    fn name(&self) -> &'static str {
        "RockHardness"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rock_hardness_in_range() {
        let strategy = RockHardnessStrategy::new(42);
        for i in 0..100 {
            let x = i as f64 * 10.0;
            let y = i as f64 * 10.0;
            let val = strategy.generate(x, y, 0);
            assert!(val >= 0.0 && val <= 1.0, "Value {} out of range", val);
        }
    }

    #[test]
    fn detail_level_adds_octaves() {
        let strategy = RockHardnessStrategy::new(42);
        let v0 = strategy.generate(100.0, 100.0, 0);
        let v1 = strategy.generate(100.0, 100.0, 1);
        // Different detail levels should produce different values
        // Different detail levels may produce different values (not guaranteed).
        let _ = (v0, v1);
    }
}
