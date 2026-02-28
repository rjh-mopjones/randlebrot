use noise::{NoiseFn, OpenSimplex};
use rb_core::NoiseStrategy;

/// Temperature strategy for a tidally-locked world.
///
/// The world has three zones:
/// - **West (dark side)**: Frozen, always facing away from the sun
/// - **Center (twilight zone)**: Habitable crescent along the terminator
/// - **East (sun side)**: Scorched, always facing the sun
///
/// Temperature varies by:
/// 1. Distance from the terminator (primary factor)
/// 2. Latitude within the twilight zone
/// 3. Noise variation for natural-looking irregularity
pub struct TidallyLockedTemperatureStrategy {
    noise: OpenSimplex,
    scale: f64,
    octaves: u32,
    persistence: f64,
    lacunarity: f64,
    /// X position of the terminator line (center of twilight zone)
    terminator_x: f64,
    /// Half-width of the habitable twilight zone
    twilight_width: f64,
    /// Total map width for normalization
    map_width: f64,
    /// Total map height for latitude calculations
    map_height: f64,
}

impl TidallyLockedTemperatureStrategy {
    /// Create a new tidally-locked temperature strategy.
    ///
    /// # Arguments
    /// * `seed` - Random seed for noise
    /// * `terminator_x` - X position of terminator (typically map_width / 2)
    /// * `twilight_width` - Half-width of habitable zone (e.g., 200 for 1024-wide map)
    /// * `map_width` - Total map width
    /// * `map_height` - Total map height
    pub fn new(
        seed: u32,
        terminator_x: f64,
        twilight_width: f64,
        map_width: f64,
        map_height: f64,
    ) -> Self {
        Self {
            noise: OpenSimplex::new(seed),
            scale: 150.0,
            octaves: 6,
            persistence: 0.5,
            lacunarity: 2.0,
            terminator_x,
            twilight_width,
            map_width,
            map_height,
        }
    }

    /// Create with default parameters for a 1024x512 map.
    pub fn default_for_map(seed: u32) -> Self {
        Self::new(seed, 512.0, 200.0, 1024.0, 512.0)
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

    /// Calculate base temperature from position in the tidally-locked world.
    fn base_temperature(&self, x: f64, y: f64) -> f64 {
        let dist_from_terminator = x - self.terminator_x;

        if dist_from_terminator < -self.twilight_width {
            // Dark side (frozen)
            // Gradually gets colder further from terminator
            let depth_into_dark = (-dist_from_terminator - self.twilight_width)
                / (self.terminator_x - self.twilight_width);
            -40.0 - depth_into_dark.min(1.0) * 40.0 // -40 to -80
        } else if dist_from_terminator > self.twilight_width {
            // Sun side (scorched)
            // Gradually gets hotter further from terminator
            let depth_into_sun = (dist_from_terminator - self.twilight_width)
                / (self.map_width - self.terminator_x - self.twilight_width);
            60.0 + depth_into_sun.min(1.0) * 60.0 // 60 to 120
        } else {
            // Twilight zone (habitable)
            // Temperature varies by latitude: equator warmer, poles cooler
            let normalized_y = y / self.map_height; // 0 to 1
            let latitude_factor = (normalized_y - 0.5).abs() * 2.0; // 0 at equator, 1 at poles

            // Also slight temperature variation based on proximity to sun side
            let terminator_factor = dist_from_terminator / self.twilight_width; // -1 to 1
            let sun_proximity_bonus = terminator_factor * 15.0; // -15 to +15

            // Base: 25°C at equator, -15°C at poles, adjusted by sun proximity
            25.0 - latitude_factor * 40.0 + sun_proximity_bonus
        }
    }
}

impl NoiseStrategy for TidallyLockedTemperatureStrategy {
    fn generate(&self, x: f64, y: f64, detail_level: u32) -> f64 {
        let base_temp = self.base_temperature(x, y);

        // Add noise variation (smaller in extreme zones, larger in twilight)
        let dist_from_terminator = (x - self.terminator_x).abs();
        let noise_scale = if dist_from_terminator < self.twilight_width {
            25.0 // More variation in habitable zone
        } else {
            10.0 // Less variation in extreme zones
        };

        let noise_value = self.fbm(x, y, detail_level) * noise_scale;

        (base_temp + noise_value).clamp(-100.0, 120.0)
    }

    fn name(&self) -> &'static str {
        "TidallyLockedTemperature"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_side_is_frozen() {
        let strategy = TidallyLockedTemperatureStrategy::default_for_map(42);
        let temp = strategy.generate(50.0, 256.0, 0); // Far west
        assert!(temp < -20.0, "Dark side temp {} should be frozen", temp);
    }

    #[test]
    fn sun_side_is_scorched() {
        let strategy = TidallyLockedTemperatureStrategy::default_for_map(42);
        let temp = strategy.generate(950.0, 256.0, 0); // Far east
        assert!(temp > 50.0, "Sun side temp {} should be scorched", temp);
    }

    #[test]
    fn twilight_is_habitable() {
        let strategy = TidallyLockedTemperatureStrategy::default_for_map(42);
        let temp = strategy.generate(512.0, 256.0, 0); // Center (terminator)
        assert!(
            temp > -30.0 && temp < 50.0,
            "Twilight temp {} should be habitable",
            temp
        );
    }

    #[test]
    fn equator_warmer_than_poles() {
        let strategy = TidallyLockedTemperatureStrategy::default_for_map(42);
        let equator_temp = strategy.generate(512.0, 256.0, 0);
        let pole_temp = strategy.generate(512.0, 10.0, 0);
        assert!(
            equator_temp > pole_temp,
            "Equator ({}) should be warmer than pole ({})",
            equator_temp,
            pole_temp
        );
    }
}
