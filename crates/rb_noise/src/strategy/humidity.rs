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
            // Use much larger scale than erosion (0.003 vs 0.015) for broad humidity zones
            let nx = x * freq * 0.003;
            let ny = y * freq * 0.003;
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

    /// Generate humidity for a tidally locked planet.
    ///
    /// Takes into account:
    /// - Continentalness (distance from water)
    /// - Latitude (y position) - sun side is extremely dry
    /// - Noise variation for natural-looking zone boundaries
    ///
    /// # Arguments
    /// * `world_height` - Total height of the world map
    pub fn generate_tidally_locked(
        &self,
        x: f64,
        y: f64,
        detail_level: u32,
        continentalness: f64,
        world_height: f64,
    ) -> f64 {
        let base_humidity = (self.fbm(x, y, detail_level) + 1.0) * 0.5;

        // Boundary noise for irregular zone edges (same pattern as temperature)
        let boundary_noise = self.fbm(x * 0.5, y * 0.3, 0);
        let latitude_offset = boundary_noise * 0.15;

        // Latitude factor with noise offset: 0 = top (dark/frozen), 1 = bottom (sun/scorched)
        let latitude = ((y / world_height) + latitude_offset).clamp(0.0, 1.0);

        // Sun-side dryness multiplier
        // - Dark side (0-0.33): normal humidity possible
        // - Terminator (0.33-0.66): slightly reduced
        // - Sun side (0.66-1.0): extremely dry
        let latitude_multiplier = if latitude < 0.33 {
            1.0  // Dark side can have normal humidity
        } else if latitude < 0.66 {
            // Terminator: gradual reduction
            let t = (latitude - 0.33) / 0.33;
            1.0 - t * 0.3  // 1.0 to 0.7
        } else {
            // Sun side: very dry
            let t = (latitude - 0.66) / 0.34;
            0.7 - t * 0.6  // 0.7 to 0.1
        };

        // Use continentalness as proxy for water distance
        let water_factor = if continentalness < -0.025 {
            // In water - high humidity (but reduced on sun side)
            1.0
        } else if continentalness < 0.05 {
            // Coastal
            0.85 - (continentalness + 0.025) * 4.0
        } else if continentalness < 0.15 {
            // Near coast
            0.55 - (continentalness - 0.05) * 3.0
        } else {
            // Inland
            0.25 - (continentalness - 0.15) * 0.8
        };

        // Combine factors
        let combined = (base_humidity * 0.4 + water_factor.max(0.0) * 0.6) * latitude_multiplier;
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
        // Positive continentalness = land (decreasing humidity inland)
        let water_factor = if continentalness < -0.025 {
            // In water - very high humidity
            1.0
        } else if continentalness < 0.05 {
            // Coastal - high humidity, gradual decrease
            0.85 - (continentalness + 0.025) * 4.0
        } else if continentalness < 0.15 {
            // Near coast - moderate humidity
            0.55 - (continentalness - 0.05) * 3.0
        } else {
            // Inland - can get quite dry
            0.25 - (continentalness - 0.15) * 0.8
        };

        // Allow humidity to go quite low inland (no floor)
        let combined = base_humidity * 0.4 + water_factor.max(0.0) * 0.6;
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
