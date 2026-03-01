use rb_core::TileType;

/// Spline-based biome determination using all 6 noise layers.
///
/// Unlike the simple `TileType::from_climate()` which only uses continentalness
/// and temperature with hard thresholds, this evaluator combines all layers
/// to create smoother, more realistic biome transitions.
pub struct BiomeSplines {
    sea_level: f64,
}

impl BiomeSplines {
    /// Create a new spline evaluator with the given sea level threshold.
    pub fn new(sea_level: f64) -> Self {
        Self { sea_level }
    }

    /// Determine biome from all 6 noise layers using spline interpolation.
    ///
    /// # Arguments
    /// * `continentalness` - Base terrain height (-1 to 1, negative = ocean)
    /// * `temperature` - Raw temperature in degrees (-50 to 100)
    /// * `tectonic` - Distance from plate boundaries (0 = boundary, 1 = center)
    /// * `erosion` - Erosion amount (0-1)
    /// * `peaks_valleys` - Ridgeline noise (-1 = valley, 1 = peak)
    /// * `humidity` - Moisture level (0 = dry, 1 = wet)
    pub fn evaluate(
        &self,
        continentalness: f64,
        temperature: f64,
        tectonic: f64,
        erosion: f64,
        peaks_valleys: f64,
        humidity: f64,
    ) -> TileType {
        // Step 1: Compute effective elevation from multiple factors
        let elevation = self.compute_elevation(continentalness, peaks_valleys, erosion);

        // Step 2: Adjust temperature based on elevation and tectonic activity
        let adjusted_temp = self.adjust_temperature(temperature, elevation, tectonic);

        // Step 3: Adjust humidity with rain shadow effect
        let adjusted_humidity = self.adjust_humidity(humidity, elevation, continentalness);

        // Step 4: Determine biome from adjusted climate values
        self.biome_from_climate(elevation, adjusted_temp, adjusted_humidity)
    }

    /// Compute effective elevation combining continentalness, peaks, and erosion.
    fn compute_elevation(&self, cont: f64, peaks: f64, erosion: f64) -> f64 {
        // Base elevation from continentalness
        let base = cont;

        // Peaks add height, but only on land (scaled by distance from sea level)
        let land_factor = ((cont - self.sea_level) / 0.3).clamp(0.0, 1.0);
        let peak_contribution = peaks * 0.12 * land_factor;

        // Erosion reduces elevation (creates valleys and worn terrain)
        let erosion_reduction = erosion * 0.04 * land_factor;

        base + peak_contribution - erosion_reduction
    }

    /// Adjust temperature based on elevation (lapse rate) and tectonic heat.
    fn adjust_temperature(&self, temp: f64, elevation: f64, tectonic: f64) -> f64 {
        // Lapse rate: temperature decreases with altitude
        // Approximately 6.5°C per 1000m, scaled to our elevation units
        let elevation_above_sea = (elevation - self.sea_level).max(0.0);
        let lapse_rate = elevation_above_sea * 60.0; // ~60°C per 1.0 elevation unit

        // Volcanic heat near plate boundaries (tectonic = 0 means boundary)
        let boundary_proximity = 1.0 - tectonic;
        let volcanic_heat = boundary_proximity * boundary_proximity * 8.0;

        temp - lapse_rate + volcanic_heat
    }

    /// Adjust humidity with rain shadow effect at high elevations.
    fn adjust_humidity(&self, humidity: f64, elevation: f64, _cont: f64) -> f64 {
        // Mountains block moisture - rain shadow effect
        let elevation_above_sea = (elevation - self.sea_level).max(0.0);

        // Rain shadow kicks in above certain elevation
        let rain_shadow = if elevation_above_sea > 0.15 {
            ((elevation_above_sea - 0.15) * 2.5).min(0.4)
        } else {
            0.0
        };

        (humidity - rain_shadow).clamp(0.0, 1.0)
    }

    /// Determine biome from adjusted elevation, temperature, and humidity.
    fn biome_from_climate(&self, elevation: f64, temp: f64, humidity: f64) -> TileType {
        // Ocean check
        if elevation < self.sea_level {
            return if temp < -15.0 {
                TileType::White // Frozen ocean / ice
            } else {
                TileType::Sea
            };
        }

        let above_sea = elevation - self.sea_level;

        // Coastal zone (just above sea level)
        if above_sea < 0.02 {
            return if temp > 3.0 {
                TileType::Beach
            } else {
                TileType::Snow
            };
        }

        // Cold regions (frozen)
        if temp < 3.0 {
            return TileType::Snow;
        }

        // High elevation (mountains and plateaus)
        if above_sea > 0.22 {
            return if temp > 65.0 {
                TileType::Plateau // Hot highlands
            } else {
                TileType::Mountain
            };
        }

        // Hot and dry = desert variants
        if temp > 55.0 && humidity < 0.3 {
            return if temp > 70.0 && humidity < 0.15 {
                TileType::Sahara // Extreme desert
            } else {
                TileType::Desert
            };
        }

        // Mid-high elevation (hills/uplands)
        if above_sea > 0.12 {
            // More nuanced based on humidity
            return if humidity > 0.55 {
                TileType::Forest // Wet highlands = forest
            } else if humidity < 0.25 && temp > 40.0 {
                TileType::Desert // Dry warm highlands
            } else {
                TileType::Mountain // Default to mountainous terrain
            };
        }

        // Lowlands with humidity-based biome selection
        if above_sea > 0.04 {
            return if humidity > 0.6 {
                TileType::Forest // Wet lowlands = forest
            } else if humidity > 0.35 {
                TileType::Plains // Moderate humidity = grassland
            } else if temp > 45.0 {
                TileType::Desert // Hot and dry lowlands
            } else {
                TileType::Plains
            };
        }

        // Very low elevation (near coast)
        if humidity > 0.5 {
            TileType::Forest
        } else {
            TileType::Plains
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn splines() -> BiomeSplines {
        BiomeSplines::new(-0.025) // Default sea level
    }

    #[test]
    fn ocean_is_sea() {
        let s = splines();
        let biome = s.evaluate(-0.5, 20.0, 0.5, 0.5, 0.0, 0.5);
        assert_eq!(biome, TileType::Sea);
    }

    #[test]
    fn frozen_ocean_is_white() {
        let s = splines();
        let biome = s.evaluate(-0.5, -30.0, 0.5, 0.5, 0.0, 0.5);
        assert_eq!(biome, TileType::White);
    }

    #[test]
    fn coastal_is_beach() {
        let s = splines();
        let biome = s.evaluate(-0.01, 25.0, 0.5, 0.5, 0.0, 0.5);
        assert_eq!(biome, TileType::Beach);
    }

    #[test]
    fn cold_is_snow() {
        let s = splines();
        let biome = s.evaluate(0.1, -10.0, 0.5, 0.5, 0.0, 0.5);
        assert_eq!(biome, TileType::Snow);
    }

    #[test]
    fn high_elevation_is_mountain() {
        let s = splines();
        // High continentalness + peaks
        let biome = s.evaluate(0.3, 30.0, 0.5, 0.2, 0.5, 0.5);
        assert_eq!(biome, TileType::Mountain);
    }

    #[test]
    fn hot_dry_is_desert() {
        let s = splines();
        let biome = s.evaluate(0.1, 60.0, 0.5, 0.5, 0.0, 0.1);
        assert_eq!(biome, TileType::Desert);
    }

    #[test]
    fn wet_lowland_is_forest() {
        let s = splines();
        let biome = s.evaluate(0.08, 25.0, 0.5, 0.5, 0.0, 0.7);
        assert_eq!(biome, TileType::Forest);
    }

    #[test]
    fn moderate_is_plains() {
        let s = splines();
        let biome = s.evaluate(0.08, 25.0, 0.5, 0.5, 0.0, 0.4);
        assert_eq!(biome, TileType::Plains);
    }

    #[test]
    fn volcanic_heat_affects_temperature() {
        let s = splines();
        // Near plate boundary (tectonic = 0) should be warmer
        let adjusted_temp_boundary = s.adjust_temperature(10.0, 0.0, 0.0);
        let adjusted_temp_center = s.adjust_temperature(10.0, 0.0, 1.0);
        assert!(
            adjusted_temp_boundary > adjusted_temp_center,
            "Boundary {} should be warmer than center {}",
            adjusted_temp_boundary,
            adjusted_temp_center
        );
    }

    #[test]
    fn rain_shadow_reduces_humidity() {
        let s = splines();
        // High elevation should have reduced humidity
        let humid_low = s.adjust_humidity(0.8, 0.0, 0.0);
        let humid_high = s.adjust_humidity(0.8, 0.3, 0.3);
        assert!(
            humid_high < humid_low,
            "High elevation humidity {} should be less than low {}",
            humid_high,
            humid_low
        );
    }
}
