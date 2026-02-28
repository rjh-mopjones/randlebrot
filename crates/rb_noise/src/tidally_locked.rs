use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Temperature strategy matching fungal-jungle's approach.
///
/// Temperature is calculated as:
/// - Latitude component: (y / map_height) * 150 - 50 (range: -50 to +100)
/// - Noise variation: 100 * noise.sample([x, y]) (adds variation)
/// - Combined: latitude_temp + noise_variation
///
/// This creates cold at top (y=0), hot at bottom (y=max), with noise variation.
pub struct LatitudeTemperatureStrategy {
    noise: OpenSimplex,
    scale: f64,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
    map_height: f64,
}

impl LatitudeTemperatureStrategy {
    pub fn new(seed: u32, map_height: f64) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            scale: 100.0,  // Scale for noise sampling
            octaves: 8,
            persistence: 0.59,
            lacunarity: 2.0,
            map_height,
        }
    }

    pub fn default_for_map(seed: u32) -> Self {
        Self::new(seed, 512.0)
    }

    /// Generate fBm noise.
    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut max_amplitude = 0.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            // Scale coordinates like fungal-jungle (0.01 scale factor)
            let nx = x * frequency * 0.01;
            let ny = y * frequency * 0.01;
            value += self.noise.get([nx, ny]) * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            frequency *= self.lacunarity;
        }

        value / max_amplitude
    }
}

impl NoiseStrategy for LatitudeTemperatureStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        // Latitude-based temperature: -50 at top, +100 at bottom
        let latitude_temp = (y / self.map_height) * 150.0 - 50.0;

        // Noise variation: ±100 degrees
        let noise_value = self.fbm(x, y, detail_level);
        let noise_variation = 100.0 * noise_value;

        // Combined temperature
        latitude_temp + noise_variation
    }

    fn name(&self) -> &'static str {
        "LatitudeTemperature"
    }
}

// Alias for backwards compatibility
pub type TidallyLockedTemperatureStrategy = LatitudeTemperatureStrategy;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_is_cold() {
        let strategy = LatitudeTemperatureStrategy::default_for_map(42);
        let temp = strategy.generate(512.0, 0.0, 0);
        // At y=0, latitude_temp = -50, plus noise
        assert!(temp < 50.0, "Top temp {} should be cold", temp);
    }

    #[test]
    fn bottom_is_hot() {
        let strategy = LatitudeTemperatureStrategy::default_for_map(42);
        let temp = strategy.generate(512.0, 512.0, 0);
        // At y=512, latitude_temp = +100, plus noise
        assert!(temp > 0.0, "Bottom temp {} should be hot", temp);
    }

    #[test]
    fn middle_is_moderate() {
        let strategy = LatitudeTemperatureStrategy::default_for_map(42);
        let temp = strategy.generate(512.0, 256.0, 0);
        // At y=256, latitude_temp = +25, plus noise
        // Should be in moderate range
        assert!(temp > -50.0 && temp < 150.0, "Middle temp {} should be moderate", temp);
    }
}
