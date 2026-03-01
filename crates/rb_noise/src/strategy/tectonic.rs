use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Generates tectonic plate boundaries for fault lines, volcanic activity, and mineral deposits.
///
/// Output range: [0.0, 1.0] where 0 = on plate boundary, 1 = center of plate
/// Uses gradient magnitude detection to find boundaries.
pub struct TectonicPlatesStrategy {
    noise: OpenSimplex,
    octaves: u32,
    frequency: f64,
    persistence: f64,
    lacunarity: f64,
}

impl TectonicPlatesStrategy {
    pub fn new(seed: u32) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            octaves: 4,        // Large-scale features
            frequency: 0.5,    // Very large plates
            persistence: 0.4,  // Smooth transitions
            lacunarity: 2.5,
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
            let nx = x * freq * 0.005; // Very large scale
            let ny = y * freq * 0.005;
            value += self.noise.get([nx, ny]) * amplitude;
            max_amplitude += amplitude;
            amplitude *= self.persistence;
            freq *= self.lacunarity;
        }

        value / max_amplitude
    }

    /// Returns distance from nearest plate boundary.
    /// 0 = on boundary (high gradient), 1 = center of plate (low gradient)
    pub fn plate_boundary_distance(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let eps = 1.0;
        let center = self.fbm(x, y, detail_level);
        let dx = (self.fbm(x + eps, y, detail_level) - center) / eps;
        let dy = (self.fbm(x, y + eps, detail_level) - center) / eps;

        // Gradient magnitude indicates rate of change (boundaries)
        let gradient_magnitude = (dx * dx + dy * dy).sqrt();

        // Normalize: high gradient = boundary (0), low gradient = plate center (1)
        (1.0 - (gradient_magnitude * 5.0).min(1.0)).max(0.0)
    }
}

impl NoiseStrategy for TectonicPlatesStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        self.plate_boundary_distance(x, y, detail_level)
    }

    fn name(&self) -> &'static str {
        "Tectonic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tectonic_generates_valid_range() {
        let strategy = TectonicPlatesStrategy::new(42);
        for i in 0..100 {
            let x = i as f64 * 10.0;
            let y = i as f64 * 10.0;
            let val = strategy.generate(x, y, 0);
            assert!(val >= 0.0 && val <= 1.0, "Value {} out of range", val);
        }
    }

    #[test]
    fn boundary_distance_is_normalized() {
        let strategy = TectonicPlatesStrategy::new(42);
        let dist = strategy.plate_boundary_distance(100.0, 100.0, 0);
        assert!(dist >= 0.0 && dist <= 1.0);
    }
}
