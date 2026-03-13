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

    /// Generate humidity for a tidally locked planet using light level.
    ///
    /// Replaces latitude-based drying with light-level-based drying:
    /// - High light (> 0.7): extremely dry (sun side)
    /// - Medium light (0.3-0.7): gradual reduction
    /// - Low light (< 0.3): normal humidity (dark side, cold trap)
    pub fn generate_with_light_level(
        &self,
        x: f64,
        y: f64,
        detail_level: u32,
        continentalness: f64,
        light_level: f64,
        _world_height: f64,
    ) -> f64 {
        let base_humidity = (self.fbm(x, y, detail_level) + 1.0) * 0.5;

        // Light-level-based dryness multiplier
        let light_multiplier = if light_level < 0.3 {
            1.0 // Dark side: normal humidity
        } else if light_level < 0.7 {
            // Transition zone: gradual reduction
            let t = (light_level - 0.3) / 0.4;
            1.0 - t * 0.3 // 1.0 to 0.7
        } else {
            // Sun side: very dry
            let t = (light_level - 0.7) / 0.3;
            0.7 - t * 0.6 // 0.7 to 0.1
        };

        // Use continentalness as proxy for water distance
        let water_factor = if continentalness < -0.025 {
            1.0
        } else if continentalness < 0.05 {
            0.85 - (continentalness + 0.025) * 4.0
        } else if continentalness < 0.15 {
            0.55 - (continentalness - 0.05) * 3.0
        } else {
            0.25 - (continentalness - 0.15) * 0.8
        };

        let combined = (base_humidity * 0.4 + water_factor.max(0.0) * 0.6) * light_multiplier;
        combined.clamp(0.0, 1.0)
    }

    /// Generate humidity using the terminator ring model for tidally locked planets.
    ///
    /// Physics-motivated atmospheric circulation:
    /// - Gaussian peak at the terminator ring (light_level ≈ 0.2) where warm and cold air collide
    /// - Day-side drying from intense solar radiation (light_level > 0.4)
    /// - Night-side cold trap reduces moisture capacity (light_level < 0.05)
    /// - Continental moisture decay from coast to inland
    /// - Local variation from fBm noise
    pub fn generate_terminator_model(
        &self,
        x: f64,
        y: f64,
        detail_level: u32,
        continentalness: f64,
        light_level: f64,
    ) -> f64 {
        let base_noise = (self.fbm(x, y, detail_level) + 1.0) * 0.5;

        // Terminator ring: Gaussian peak at light_level ≈ 0.2 with width 0.15
        let terminator_center = 0.2;
        let terminator_width = 0.25;
        let terminator_peak = (-(light_level - terminator_center).powi(2)
            / (2.0 * terminator_width * terminator_width))
            .exp();

        // Day-side drying: quadratic reduction for light_level > 0.4
        let day_drying = if light_level > 0.4 {
            let t = (light_level - 0.4) / 0.6; // 0 at 0.4, 1 at 1.0
            1.0 - t * t * 0.8 // up to 0.8 reduction
        } else {
            1.0
        };

        // Night-side cold trap: reduced moisture capacity for light_level < 0.05
        let night_trap = if light_level < 0.05 {
            let t = light_level / 0.05; // 0 at 0, 1 at 0.05
            0.3 + t * 0.7 // 0.3 minimum (up to 0.7 reduction at edge)
        } else {
            1.0
        };

        // Continental moisture decay: ocean=1.0, coastal→0.5, inland→0.1
        let moisture_source = if continentalness < -0.025 {
            1.0 // ocean
        } else if continentalness < 0.05 {
            let t = (continentalness + 0.025) / 0.075; // 0 at -0.025, 1 at 0.05
            1.0 - t * 0.5 // 1.0 to 0.5
        } else if continentalness < 0.2 {
            let t = (continentalness - 0.05) / 0.15; // 0 at 0.05, 1 at 0.2
            0.5 - t * 0.3 // 0.5 to 0.2
        } else {
            let t = ((continentalness - 0.2) / 0.3).min(1.0); // 0 at 0.2, 1 at 0.5
            0.2 - t * 0.1 // 0.2 to 0.1
        };

        // Combine: terminator ring enhances base, day/night modifiers reduce
        let atmospheric = terminator_peak * day_drying * night_trap;
        let scaled_moisture = moisture_source * (0.3 + terminator_peak * 0.7);
        let combined = base_noise * 0.2 + scaled_moisture * 0.3 + atmospheric * 0.5;

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
    fn terminator_peak_humidity() {
        let strategy = HumidityStrategy::new(42);
        // At the terminator ring (light ≈ 0.2), humidity should be higher than day side
        let terminator = strategy.generate_terminator_model(100.0, 100.0, 0, -0.5, 0.2);
        let day_side = strategy.generate_terminator_model(100.0, 100.0, 0, -0.5, 0.9);
        assert!(
            terminator > day_side,
            "Terminator ({}) should be more humid than day side ({})",
            terminator, day_side
        );
    }

    #[test]
    fn day_side_drying() {
        let strategy = HumidityStrategy::new(42);
        // High light level on land should produce low humidity
        let bright = strategy.generate_terminator_model(100.0, 100.0, 0, 0.3, 0.95);
        assert!(bright < 0.5, "Day side inland humidity ({}) should be reduced", bright);
    }

    #[test]
    fn night_side_cold_trap() {
        let strategy = HumidityStrategy::new(42);
        // Very low light should reduce humidity vs terminator
        let night = strategy.generate_terminator_model(100.0, 100.0, 0, -0.5, 0.01);
        let terminator = strategy.generate_terminator_model(100.0, 100.0, 0, -0.5, 0.2);
        assert!(
            night < terminator,
            "Night side ({}) should be less humid than terminator ({})",
            night, terminator
        );
    }

    #[test]
    fn ocean_vs_inland_terminator() {
        let strategy = HumidityStrategy::new(42);
        let ocean = strategy.generate_terminator_model(100.0, 100.0, 0, -0.5, 0.2);
        let inland = strategy.generate_terminator_model(100.0, 100.0, 0, 0.4, 0.2);
        assert!(
            ocean > inland,
            "Ocean terminator ({}) should be more humid than inland terminator ({})",
            ocean, inland
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
