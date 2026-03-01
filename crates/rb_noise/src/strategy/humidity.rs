use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates humidity values that naturally decay with distance from water.
///
/// Output range: [0.0, 1.0] where 1 = very humid (near water)
pub struct HumidityStrategy {
    noise: OpenSimplex,
    octaves: u32,
    frequency: f64,
    persistence: f64,
    lacunarity: f64,
}

impl HumidityStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 5,
            frequency: 1.0,
            persistence: 0.5,
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

    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = self.frequency;
        let mut max_amplitude = 0.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            let nx = x * freq * 0.008;
            let ny = y * freq * 0.008;
            value += self.noise.get([nx, ny]) * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        value / max_amplitude
    }

    /// Generate humidity considering distance from water.
    ///
    /// # Arguments
    /// * `water_distance_factor` - 0 = on water, 1 = far from water
    pub fn generate_with_water_distance(
        &self,
        x: f64,
        y: f64,
        detail_level: u32,
        water_distance_factor: f64,
    ) -> f64 {
        let base_humidity = (self.fbm(x, y, detail_level) + 1.0) * 0.5;

        // Humidity decays exponentially with distance from water
        // Near coast: high humidity
        // Far inland: low humidity (modified by noise for local variations)
        let decay = (-water_distance_factor * 3.0).exp();

        // Base humidity provides local variation
        // Decay provides global gradient from coast
        let combined = base_humidity * 0.4 + decay * 0.6;

        combined.clamp(0.0, 1.0)
    }

    /// Generate humidity based on continentalness (proxy for water distance).
    /// Useful when water distance isn't precomputed.
    pub fn generate_with_continentalness(
        &self,
        x: f64,
        y: f64,
        detail_level: u32,
        continentalness: f64,
    ) -> f64 {
        let base_humidity = (self.fbm(x, y, detail_level) + 1.0) * 0.5;

        // Use continentalness as proxy for water distance
        // Negative continentalness = water (high humidity)
        // Positive continentalness = land (decreasing humidity with elevation)
        let water_factor = if continentalness < -0.025 {
            // In water - very high humidity
            1.0
        } else if continentalness < 0.1 {
            // Near coast - high humidity
            0.9 - (continentalness + 0.025) * 2.0
        } else {
            // Inland - decreasing humidity
            0.5 - (continentalness - 0.1) * 0.5
        };

        let combined = base_humidity * 0.3 + water_factor.max(0.1) * 0.7;
        combined.clamp(0.0, 1.0)
    }
}

impl NoiseStrategy for HumidityStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        // Without water distance context, return base humidity noise
        (self.fbm(x, y, detail_level) + 1.0) * 0.5
    }

    fn name(&self) -> &'static str {
        "Humidity"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humidity_generates_valid_range() {
        let strategy = HumidityStrategy::new(42);
        for i in 0..100 {
            let x = i as f64 * 10.0;
            let y = i as f64 * 10.0;
            let val = strategy.generate(x, y, 0);
            assert!(val >= 0.0 && val <= 1.0, "Value {} out of range", val);
        }
    }

    #[test]
    fn humidity_decreases_from_water() {
        let strategy = HumidityStrategy::new(42);

        let near_water = strategy.generate_with_water_distance(100.0, 100.0, 0, 0.0);
        let far_water = strategy.generate_with_water_distance(100.0, 100.0, 0, 1.0);

        assert!(
            near_water > far_water,
            "Near water ({}) should be more humid than far ({})",
            near_water,
            far_water
        );
    }

    #[test]
    fn humidity_with_continentalness() {
        let strategy = HumidityStrategy::new(42);

        let ocean = strategy.generate_with_continentalness(100.0, 100.0, 0, -0.5);
        let inland = strategy.generate_with_continentalness(100.0, 100.0, 0, 0.5);

        assert!(
            ocean > inland,
            "Ocean ({}) should be more humid than inland ({})",
            ocean,
            inland
        );
    }
}
