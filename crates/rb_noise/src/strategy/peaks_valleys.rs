use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates peaks and valleys using ridged multifractal noise.
/// Creates distinct mountain ridgelines and valley networks.
///
/// Output range: [-1.0, 1.0] where -1 = deep valley, +1 = sharp ridge
pub struct PeaksAndValleysStrategy {
    noise: OpenSimplex,
    octaves: u32,
    frequency: f64,
    persistence: f64,
    lacunarity: f64,
}

impl PeaksAndValleysStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 8,
            frequency: 1.5,
            persistence: 0.6,
            lacunarity: 2.0,
        }
    }

    pub fn with_params(
        seed: u32,
        octaves: u32,
        frequency: f64,
        persistence: f64,
        lacunarity: f64,
    ) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves,
            frequency,
            persistence,
            lacunarity,
        }
    }

    /// Ridged multifractal noise produces sharp ridges.
    /// Based on the ridged multifractal algorithm from Musgrave et al.
    fn ridged_fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = self.frequency;
        let mut weight = 1.0;
        let mut max_value = 0.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            let nx = x * freq * 0.01;
            let ny = y * freq * 0.01;

            // Get absolute value of noise and invert it
            // This creates sharp ridges where the noise crosses zero
            let signal = 1.0 - self.noise.get([nx, ny]).abs();

            // Square to make ridges sharper
            let signal = signal * signal;

            // Weight successive octaves by previous signal
            let signal = signal * weight;
            weight = (signal * 2.0).clamp(0.0, 1.0);

            value += signal * amplitude;
            max_value += amplitude;

            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        // Normalize and shift to [-1, 1]
        (value / max_value) * 2.0 - 1.0
    }

    /// Alternative: standard valleys (inverted peaks)
    fn valleys(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = self.frequency;
        let mut max_amplitude = 0.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            let nx = x * freq * 0.01;
            let ny = y * freq * 0.01;

            // Get noise and take absolute value for valleys
            let n = self.noise.get([nx, ny]).abs();
            value += (1.0 - n) * amplitude;

            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        // Invert so valleys are negative, ridges positive
        (value / max_amplitude) * 2.0 - 1.0
    }
}

impl NoiseStrategy for PeaksAndValleysStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        self.ridged_fbm(x, y, detail_level)
    }

    fn name(&self) -> &'static str {
        "PeaksValleys"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_valleys_generates_valid_range() {
        let strategy = PeaksAndValleysStrategy::new(42);
        for i in 0..100 {
            let x = i as f64 * 10.0;
            let y = i as f64 * 10.0;
            let val = strategy.generate(x, y, 0);
            assert!(
                val >= -1.0 && val <= 1.0,
                "Value {} out of range at ({}, {})",
                val,
                x,
                y
            );
        }
    }

    #[test]
    fn ridges_are_prominent() {
        let strategy = PeaksAndValleysStrategy::new(42);

        // Sample many points and check we get both positive (ridges) and negative (valleys)
        let mut has_ridge = false;
        let mut has_valley = false;

        for i in 0..1000 {
            let x = i as f64 * 3.0;
            let y = i as f64 * 7.0;
            let val = strategy.generate(x, y, 0);

            if val > 0.3 {
                has_ridge = true;
            }
            if val < -0.3 {
                has_valley = true;
            }
        }

        assert!(has_ridge, "Should have prominent ridges");
        assert!(has_valley, "Should have prominent valleys");
    }
}
