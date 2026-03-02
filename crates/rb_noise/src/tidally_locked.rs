use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Temperature strategy for a tidally locked planet.
///
/// The world has three distinct zones:
/// - **North (dark side)**: Frozen wasteland, perpetually facing away from the star
/// - **Middle (terminator)**: Habitable twilight band where civilization thrives
/// - **South (sun side)**: Scorching desert, perpetually facing the star
///
/// Temperature uses a non-linear curve with reduced noise at extremes
/// (frozen and scorched regions are more uniformly hostile).
pub struct LatitudeTemperatureStrategy {
    noise: OpenSimplex,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
    map_height: f64,
}

impl LatitudeTemperatureStrategy {
    pub fn new(seed: u32, map_height: f64) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
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
        // Get noise for boundary variation (use different coordinates for variety)
        let boundary_noise = self.fbm(x * 0.5, y * 0.3, 0);

        // Offset the effective latitude by noise to create wavy zone boundaries
        // The noise shifts the boundary up/down by up to ~15% of map height
        let latitude_offset = boundary_noise * 0.15;

        // Normalized position with noise offset: 0 = top (dark side), 1 = bottom (sun side)
        let t = ((y / self.map_height) + latitude_offset).clamp(0.0, 1.0);

        // Non-linear temperature curve for tidally locked planet:
        // - Top third (0-0.33): Frozen, -80°C to -20°C
        // - Middle third (0.33-0.66): Habitable, -10°C to +60°C
        // - Bottom third (0.66-1.0): Scorching, +80°C to +150°C
        let base_temp = if t < 0.33 {
            // Dark side: frozen
            let local_t = t / 0.33;
            -80.0 + local_t * 60.0  // -80 to -20
        } else if t < 0.66 {
            // Terminator: habitable band
            let local_t = (t - 0.33) / 0.33;
            -10.0 + local_t * 70.0  // -10 to +60
        } else {
            // Sun side: scorching
            let local_t = (t - 0.66) / 0.34;
            80.0 + local_t * 70.0   // +80 to +150
        };

        // Local noise variation for terrain detail
        let local_noise = self.fbm(x, y, detail_level);
        let noise_scale = if t < 0.2 || t > 0.8 {
            // Extreme zones: less variation
            25.0
        } else if t < 0.33 || t > 0.66 {
            // Transition zones: moderate variation
            40.0
        } else {
            // Habitable zone: most variation
            50.0
        };

        base_temp + local_noise * noise_scale
    }

    fn name(&self) -> &'static str {
        "TidallyLockedTemperature"
    }
}

// Alias for backwards compatibility
pub type TidallyLockedTemperatureStrategy = LatitudeTemperatureStrategy;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_side_is_frozen() {
        let strategy = LatitudeTemperatureStrategy::default_for_map(42);
        // Top of map (dark side) - should be very cold
        let temp = strategy.generate(512.0, 0.0, 0);
        assert!(temp < -20.0, "Dark side temp {} should be frozen", temp);
    }

    #[test]
    fn sun_side_is_scorching() {
        let strategy = LatitudeTemperatureStrategy::default_for_map(42);
        // Bottom of map (sun side) - should be very hot
        let temp = strategy.generate(512.0, 512.0, 0);
        assert!(temp > 100.0, "Sun side temp {} should be scorching", temp);
    }

    #[test]
    fn terminator_is_habitable() {
        let strategy = LatitudeTemperatureStrategy::default_for_map(42);
        // Middle of map (terminator) - should be habitable range
        let temp = strategy.generate(512.0, 256.0, 0);
        assert!(temp > -30.0 && temp < 80.0, "Terminator temp {} should be habitable", temp);
    }
}
