use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates continentalness values using 16-octave fBm.
/// Matches fungal-jungle parameters.
///
/// Output range: approximately [-1.0, 1.0]
/// Higher values = more continental (land), lower values = more oceanic (water)
pub struct ContinentalnessStrategy {
    noise: OpenSimplex,
    octaves: u32,
    frequency: f64,
    lacunarity: f64,
    persistence: f64,
    /// World width for horizontal wrapping. 0 = no wrapping.
    world_width: f64,
}

impl ContinentalnessStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 16,
            frequency: 1.0,
            lacunarity: 2.0,
            persistence: 0.59,
            world_width: 0.0,
        }
    }

    pub fn new_wrapping(seed: u32, world_width: f64) -> Self {
        let mut s = Self::new(seed);
        s.world_width = world_width;
        s
    }

    pub fn with_params(
        seed: u32,
        octaves: u32,
        frequency: f64,
        lacunarity: f64,
        persistence: f64,
    ) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves,
            frequency,
            lacunarity,
            persistence,
            world_width: 0.0,
        }
    }

    /// Generate fBm (fractal Brownian motion) noise.
    /// Uses 0.01 scale factor like fungal-jungle.
    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = self.frequency;
        let mut max_amplitude = 0.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            let sample = if self.world_width > 0.0 {
                let [cx, cz, cy] = crate::wrap::cylindrical_noise_coords(x, y, freq, 0.01, self.world_width);
                self.noise.get([cx, cz, cy])
            } else {
                let nx = x * freq * 0.01;
                let ny = y * freq * 0.01;
                self.noise.get([nx, ny])
            };
            value += sample * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        // Normalize to [-1, 1]
        value / max_amplitude
    }
}

impl NoiseStrategy for ContinentalnessStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        self.fbm(x, y, detail_level)
    }

    fn name(&self) -> &'static str {
        "Continentalness"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continentalness_generates_values() {
        let strategy = ContinentalnessStrategy::new(42);
        let value = strategy.generate(0.0, 0.0, 0);
        assert!(value >= -1.0 && value <= 1.0, "Value {} out of range", value);
    }

    #[test]
    fn continentalness_is_deterministic() {
        let strategy1 = ContinentalnessStrategy::new(42);
        let strategy2 = ContinentalnessStrategy::new(42);
        let val1 = strategy1.generate(100.0, 200.0, 0);
        let val2 = strategy2.generate(100.0, 200.0, 0);
        assert_eq!(val1, val2);
    }
}
