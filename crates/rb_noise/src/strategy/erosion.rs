use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates erosion patterns. Valleys erode more than peaks.
///
/// Output range: [0.0, 1.0] where 1 = heavily eroded
/// Depends on continentalness - lower elevations erode more.
pub struct ErosionStrategy {
    noise: OpenSimplex,
    octaves: u32,
    frequency: f64,
    persistence: f64,
    lacunarity: f64,
}

impl ErosionStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 6,
            frequency: 2.0, // Fine-grained detail
            persistence: 0.55,
            lacunarity: 2.2,
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

    /// Standard fBm for base erosion patterns.
    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = self.frequency;
        let mut max_amplitude = 0.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            let nx = x * freq * 0.01;
            let ny = y * freq * 0.01;
            value += self.noise.get([nx, ny]) * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        value / max_amplitude
    }

    /// Ridged noise creates sharp valleys/channels for erosion patterns.
    /// Different from smooth fBm - produces distinct erosion features.
    fn ridged_fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = self.frequency;
        let mut weight = 1.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            // Use different scale than humidity (0.015 vs 0.008)
            let nx = x * freq * 0.015;
            let ny = y * freq * 0.015;

            // Ridged noise: 1 - |noise| creates sharp valleys
            let signal = 1.0 - self.noise.get([nx, ny]).abs();
            // Square for sharper ridges
            let signal = signal * signal;
            // Weight by previous value for more natural look
            let weighted = signal * weight;
            weight = signal.clamp(0.0, 1.0);

            value += weighted * amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        // Normalize to 0-1 range
        (value * 0.5).clamp(0.0, 1.0)
    }

    /// Generate erosion value that depends on continentalness.
    /// Lower elevations (valleys) erode more than peaks.
    pub fn generate_with_continentalness(
        &self,
        x: f64,
        y: f64,
        detail_level: u32,
        continentalness: f64,
    ) -> f64 {
        let base_erosion = (self.fbm(x, y, detail_level) + 1.0) * 0.5; // Normalize to [0, 1]

        // Lower continentalness (valleys/ocean) = more erosion
        // continentalness typically [-1, 1], where -1 = deep ocean, +1 = high mountains
        // We want valleys (low positive values near sea level) to erode most
        let elevation_factor = if continentalness < -0.025 {
            // Underwater - moderate erosion
            0.5
        } else if continentalness < 0.2 {
            // Low elevation - high erosion (water flows through)
            0.8 + 0.2 * (1.0 - (continentalness + 0.025) / 0.225)
        } else {
            // High elevation - lower erosion (harder rock, less water)
            0.3 + 0.5 * (1.0 - (continentalness - 0.2) / 0.8).max(0.0)
        };

        // Combine base erosion noise with elevation factor
        (base_erosion * 0.4 + elevation_factor * 0.6).clamp(0.0, 1.0)
    }
}

impl NoiseStrategy for ErosionStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        // Use ridged noise for sharp erosion valleys (distinct from humidity)
        self.ridged_fbm(x, y, detail_level)
    }

    fn name(&self) -> &'static str {
        "Erosion"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erosion_generates_valid_range() {
        let strategy = ErosionStrategy::new(42);
        for i in 0..100 {
            let x = i as f64 * 10.0;
            let y = i as f64 * 10.0;
            let val = strategy.generate(x, y, 0);
            assert!(val >= 0.0 && val <= 1.0, "Value {} out of range", val);
        }
    }

    #[test]
    fn erosion_with_continentalness() {
        let strategy = ErosionStrategy::new(42);

        // Low elevation should have more erosion
        let low_elev = strategy.generate_with_continentalness(100.0, 100.0, 0, 0.0);
        let high_elev = strategy.generate_with_continentalness(100.0, 100.0, 0, 0.5);

        assert!(
            low_elev >= high_elev,
            "Low elevation ({}) should have >= erosion than high ({})",
            low_elev,
            high_elev
        );
    }
}
