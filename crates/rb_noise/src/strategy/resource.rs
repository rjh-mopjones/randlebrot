use noise::{NoiseFn, OpenSimplex};
use rb_core::{NoiseStrategy, ResourceType, TileType};

/// Generates noise for resource deposits with terrain-aware biasing.
/// Each resource type gets a unique noise pattern biased by terrain.
pub struct ResourceNoiseStrategy {
    noise: OpenSimplex,
    resource_type: ResourceType,
    octaves: u32,
    frequency: f64,
    persistence: f64,
}

impl ResourceNoiseStrategy {
    pub fn new(seed: u32, resource_type: ResourceType) -> Self {
        Self {
            noise: OpenSimplex::new(seed.wrapping_add(resource_type.seed_offset())),
            resource_type,
            octaves: 4,
            frequency: 2.0, // Higher frequency = more localized deposits
            persistence: 0.5,
        }
    }

    pub fn with_params(
        seed: u32,
        resource_type: ResourceType,
        octaves: u32,
        frequency: f64,
        persistence: f64,
    ) -> Self {
        Self {
            noise: OpenSimplex::new(seed.wrapping_add(resource_type.seed_offset())),
            resource_type,
            octaves,
            frequency,
            persistence,
        }
    }

    pub fn resource_type(&self) -> ResourceType {
        self.resource_type
    }

    fn fbm(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let mut value = 0.0;
        let mut amplitude = 1.0;
        let mut freq = self.frequency;
        let mut max_amplitude = 0.0;

        let total_octaves = self.octaves + detail_level;

        for _ in 0..total_octaves {
            let nx = x * freq * 0.015;
            let ny = y * freq * 0.015;
            value += self.noise.get([nx, ny]) * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.frequency;
        }

        value / max_amplitude
    }

    /// Generate resource abundance with terrain biasing.
    pub fn generate_with_context(&self, x: f64, y: f64, detail_level: u32, context: &ResourceContext) -> f64 {
        let base_value = (self.fbm(x, y, detail_level) + 1.0) * 0.5;

        // Apply terrain bias
        let bias_multiplier = self.resource_type.terrain_bias().calculate(
            context.continentalness,
            context.tectonic_boundary_distance,
            context.water_distance,
            context.biome,
        );

        // Resources are sparse - use threshold to create distinct deposits
        let threshold = 0.55;
        let biased_value = base_value * bias_multiplier;

        if biased_value > threshold {
            // Normalize above-threshold values to [0, 1]
            ((biased_value - threshold) / (1.0 - threshold)).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

impl NoiseStrategy for ResourceNoiseStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        // Without context, just return base noise (not very useful for resources)
        (self.fbm(x, y, detail_level) + 1.0) * 0.5
    }

    fn name(&self) -> &'static str {
        self.resource_type.name()
    }
}

/// Context data needed for resource generation with terrain biasing.
#[derive(Clone, Copy, Debug)]
pub struct ResourceContext {
    pub continentalness: f64,
    pub tectonic_boundary_distance: f64,
    pub water_distance: f64,
    pub biome: TileType,
}

impl Default for ResourceContext {
    fn default() -> Self {
        Self {
            continentalness: 0.0,
            tectonic_boundary_distance: 0.5,
            water_distance: 0.5,
            biome: TileType::Plains,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_generates_valid_range() {
        let strategy = ResourceNoiseStrategy::new(42, ResourceType::Iron);
        for i in 0..100 {
            let x = i as f64 * 10.0;
            let y = i as f64 * 10.0;
            let val = strategy.generate(x, y, 0);
            assert!(val >= 0.0 && val <= 1.0, "Value {} out of range", val);
        }
    }

    #[test]
    fn different_resources_have_different_patterns() {
        let iron = ResourceNoiseStrategy::new(42, ResourceType::Iron);
        let gold = ResourceNoiseStrategy::new(42, ResourceType::Gold);

        let iron_val = iron.generate(100.0, 100.0, 0);
        let gold_val = gold.generate(100.0, 100.0, 0);

        // Different seed offsets should produce different values
        assert_ne!(iron_val, gold_val);
    }

    #[test]
    fn mountain_bias_affects_iron() {
        let strategy = ResourceNoiseStrategy::new(42, ResourceType::Iron);

        let low_context = ResourceContext {
            continentalness: -0.5,
            tectonic_boundary_distance: 0.5,
            water_distance: 0.5,
            biome: TileType::Plains,
        };

        let high_context = ResourceContext {
            continentalness: 0.5,
            tectonic_boundary_distance: 0.5,
            water_distance: 0.5,
            biome: TileType::Mountain,
        };

        // Sample multiple points to find one with resources
        let mut found_higher_in_mountains = false;
        for i in 0..100 {
            let x = i as f64 * 7.0;
            let y = i as f64 * 13.0;

            let low_val = strategy.generate_with_context(x, y, 0, &low_context);
            let high_val = strategy.generate_with_context(x, y, 0, &high_context);

            if high_val > 0.0 && high_val > low_val {
                found_higher_in_mountains = true;
                break;
            }
        }

        assert!(
            found_higher_in_mountains,
            "Iron should be more common in mountains"
        );
    }

    #[test]
    fn coastal_bias_affects_fish() {
        let strategy = ResourceNoiseStrategy::new(42, ResourceType::Fish);

        let inland_context = ResourceContext {
            continentalness: 0.3,
            tectonic_boundary_distance: 0.5,
            water_distance: 1.0,
            biome: TileType::Plains,
        };

        let coastal_context = ResourceContext {
            continentalness: -0.01,
            tectonic_boundary_distance: 0.5,
            water_distance: 0.0,
            biome: TileType::Beach,
        };

        // Sample multiple points
        let mut found_higher_on_coast = false;
        for i in 0..100 {
            let x = i as f64 * 7.0;
            let y = i as f64 * 13.0;

            let inland_val = strategy.generate_with_context(x, y, 0, &inland_context);
            let coastal_val = strategy.generate_with_context(x, y, 0, &coastal_context);

            if coastal_val > 0.0 && coastal_val > inland_val {
                found_higher_on_coast = true;
                break;
            }
        }

        assert!(found_higher_on_coast, "Fish should be more common on coast");
    }
}
