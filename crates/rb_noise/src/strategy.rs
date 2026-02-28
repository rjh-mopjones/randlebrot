use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates continentalness values using 6-octave fBm.
///
/// Output range: approximately [-1.0, 1.0]
/// Higher values = more continental (land), lower values = more oceanic (water)
pub struct ContinentalnessStrategy {
    noise: OpenSimplex,
    scale: f64,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
}

impl ContinentalnessStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            scale: 100.0,
            octaves: 8,
            persistence: 0.59,
            lacunarity: 2.0,
        }
    }

    pub fn with_params(
        seed: u32,
        scale: f64,
        octaves: u32,
        persistence: f64,
        lacunarity: f64,
    ) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            scale,
            octaves,
            persistence,
            lacunarity,
        }
    }

    /// Generate fBm (fractal Brownian motion) noise.
    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut max_amplitude = 0.0;

        // Add extra octaves based on detail level for finer detail at higher zoom
        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            let nx = x * frequency / self.scale;
            let ny = y * frequency / self.scale;
            value += self.noise.get([nx, ny]) * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            frequency *= self.lacunarity;
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

/// Generates temperature values using 4-octave fBm blended with latitude.
///
/// Output range: [-100.0, 100.0] (Celsius-like scale)
/// Blends 30% noise + 70% latitude factor
pub struct TemperatureStrategy {
    noise: OpenSimplex,
    scale: f64,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
    /// Weight for noise component (0.0 to 1.0)
    noise_weight: f64,
    /// Weight for latitude component (0.0 to 1.0)
    latitude_weight: f64,
}

impl TemperatureStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            scale: 150.0,
            octaves: 4,
            persistence: 0.5,
            lacunarity: 2.0,
            noise_weight: 0.3,
            latitude_weight: 0.7,
        }
    }

    pub fn with_params(
        seed: u32,
        scale: f64,
        octaves: u32,
        persistence: f64,
        lacunarity: f64,
        noise_weight: f64,
    ) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            scale,
            octaves,
            persistence,
            lacunarity,
            noise_weight,
            latitude_weight: 1.0 - noise_weight,
        }
    }

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

    /// Calculate latitude factor based on Y coordinate.
    /// Assumes Y=0 is equator, positive Y is north, negative Y is south.
    /// Returns value in [-1, 1] where 0 = equator (hottest), ±1 = poles (coldest).
    fn latitude_factor(&self, y: f64) -> f64 {
        // Normalize Y to a latitude factor
        // Using a sigmoid-like curve for more realistic temperature distribution
        let normalized = (y / 1000.0).tanh();
        normalized.abs()
    }
}

impl NoiseStrategy for TemperatureStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        // Get noise component in [-1, 1]
        let noise_value = self.fbm(x, y, detail_level);

        // Get latitude factor in [0, 1] where 0 = equator, 1 = poles
        let lat_factor = self.latitude_factor(y);

        // Blend: equator is hot (100), poles are cold (-100)
        // Latitude contribution: (1 - lat_factor) maps poles to 0, equator to 1
        // Then scale to [-100, 100]: (value * 2 - 1) * 100
        let latitude_temp = (1.0 - lat_factor) * 2.0 - 1.0; // [-1, 1]

        // Combine noise and latitude
        let combined = noise_value * self.noise_weight + latitude_temp * self.latitude_weight;

        // Scale to temperature range [-100, 100]
        combined * 100.0
    }

    fn name(&self) -> &'static str {
        "Temperature"
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
    fn temperature_generates_values() {
        let strategy = TemperatureStrategy::new(42);
        let value = strategy.generate(0.0, 0.0, 0);
        assert!(
            value >= -100.0 && value <= 100.0,
            "Value {} out of range",
            value
        );
    }

    #[test]
    fn temperature_varies_with_latitude() {
        let strategy = TemperatureStrategy::new(42);
        let equator = strategy.generate(0.0, 0.0, 0);
        let north = strategy.generate(0.0, 2000.0, 0);
        // Equator should generally be warmer than poles
        assert!(
            equator > north,
            "Equator ({}) should be warmer than north ({})",
            equator,
            north
        );
    }
}
