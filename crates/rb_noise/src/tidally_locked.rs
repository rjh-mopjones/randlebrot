use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Latitude-based temperature strategy.
///
/// Temperature varies primarily by Y position (latitude):
/// - **Top (y=0)**: Cold polar region
/// - **Bottom (y=max)**: Hot equatorial/tropical region
///
/// This creates a gradual north-to-south temperature gradient
/// with noise variation for natural irregularity.
pub struct LatitudeTemperatureStrategy {
    noise: OpenSimplex,
    scale: f64,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
    /// Total map height for normalization
    map_height: f64,
    /// Minimum temperature (at top/cold pole)
    min_temp: f64,
    /// Maximum temperature (at bottom/hot equator)
    max_temp: f64,
    /// How much noise affects temperature (0.0 to 1.0)
    noise_influence: f64,
}

impl LatitudeTemperatureStrategy {
    /// Create a new latitude-based temperature strategy.
    pub fn new(seed: u32, map_height: f64) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            scale: 150.0,
            octaves: 6,
            persistence: 0.5,
            lacunarity: 2.0,
            map_height,
            min_temp: -50.0,  // Cold at top
            max_temp: 100.0,  // Hot at bottom
            noise_influence: 0.3, // 30% noise, 70% latitude
        }
    }

    /// Create with default parameters for a 1024x512 map.
    pub fn default_for_map(seed: u32) -> Self {
        Self::new(seed, 512.0)
    }

    /// Generate fBm noise for variation.
    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut max_amplitude = 0.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            let nx = x * frequency / self.scale;
            let ny = y * frequency / self.scale;
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
        // Latitude factor: 0.0 at top (cold), 1.0 at bottom (hot)
        let latitude_factor = (y / self.map_height).clamp(0.0, 1.0);

        // Base temperature from latitude
        let latitude_temp = self.min_temp + latitude_factor * (self.max_temp - self.min_temp);

        // Add noise variation
        let noise_value = self.fbm(x, y, detail_level); // -1 to 1
        let noise_temp = noise_value * 50.0; // ±50 degrees variation

        // Blend latitude and noise
        let temp = latitude_temp * (1.0 - self.noise_influence)
                 + (latitude_temp + noise_temp) * self.noise_influence;

        temp.clamp(-100.0, 120.0)
    }

    fn name(&self) -> &'static str {
        "LatitudeTemperature"
    }
}

// Keep the old strategy for reference but rename it
pub use LatitudeTemperatureStrategy as TidallyLockedTemperatureStrategy;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_is_cold() {
        let strategy = LatitudeTemperatureStrategy::default_for_map(42);
        let temp = strategy.generate(512.0, 10.0, 0); // Near top
        assert!(temp < 0.0, "Top temp {} should be cold", temp);
    }

    #[test]
    fn bottom_is_hot() {
        let strategy = LatitudeTemperatureStrategy::default_for_map(42);
        let temp = strategy.generate(512.0, 500.0, 0); // Near bottom
        assert!(temp > 50.0, "Bottom temp {} should be hot", temp);
    }

    #[test]
    fn gradual_change() {
        let strategy = LatitudeTemperatureStrategy::default_for_map(42);
        let top = strategy.generate(512.0, 50.0, 0);
        let middle = strategy.generate(512.0, 256.0, 0);
        let bottom = strategy.generate(512.0, 450.0, 0);

        assert!(
            top < middle && middle < bottom,
            "Temperature should increase from top ({}) to middle ({}) to bottom ({})",
            top, middle, bottom
        );
    }
}
