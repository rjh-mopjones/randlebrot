use rb_core::TileType;

/// Climate classification for temperature-based biome selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClimateClass {
    Frozen,    // < -20°C (dark side)
    Cold,      // -20 to 3°C
    Temperate, // 3 to 35°C
    Warm,      // 35 to 55°C
    Hot,       // 55 to 80°C
    Scorching, // > 80°C (sun side)
}

impl ClimateClass {
    pub fn from_temperature(temp: f64) -> Self {
        if temp < -20.0 {
            Self::Frozen
        } else if temp < 3.0 {
            Self::Cold
        } else if temp < 35.0 {
            Self::Temperate
        } else if temp < 55.0 {
            Self::Warm
        } else if temp < 80.0 {
            Self::Hot
        } else {
            Self::Scorching
        }
    }
}

/// Moisture classification for humidity-based biome selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoistureClass {
    Arid,      // < 0.2
    Dry,       // 0.2 to 0.4
    Moderate,  // 0.4 to 0.6
    Humid,     // 0.6 to 0.8
    Saturated, // > 0.8
}

impl MoistureClass {
    pub fn from_humidity(humidity: f64) -> Self {
        if humidity < 0.2 {
            Self::Arid
        } else if humidity < 0.4 {
            Self::Dry
        } else if humidity < 0.6 {
            Self::Moderate
        } else if humidity < 0.8 {
            Self::Humid
        } else {
            Self::Saturated
        }
    }
}

/// Elevation classification for altitude-based biome selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElevationClass {
    Coastal,  // < 0.02 above sea level
    Lowland,  // 0.02 to 0.08
    Upland,   // 0.08 to 0.18
    Highland, // 0.18 to 0.28
    Alpine,   // > 0.28
}

impl ElevationClass {
    pub fn from_elevation(above_sea: f64) -> Self {
        if above_sea < 0.02 {
            Self::Coastal
        } else if above_sea < 0.08 {
            Self::Lowland
        } else if above_sea < 0.18 {
            Self::Upland
        } else if above_sea < 0.28 {
            Self::Highland
        } else {
            Self::Alpine
        }
    }
}

/// Terrain ruggedness classification based on erosion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainClass {
    Flat,    // erosion < 0.3 (heavily eroded = flat)
    Rolling, // 0.3 to 0.7
    Rugged,  // > 0.7 (low erosion = rugged peaks)
}

impl TerrainClass {
    /// Note: High erosion = flat terrain, low erosion = rugged terrain
    pub fn from_erosion(erosion: f64) -> Self {
        if erosion < 0.3 {
            Self::Rugged // Low erosion = jagged peaks preserved
        } else if erosion < 0.7 {
            Self::Rolling
        } else {
            Self::Flat // High erosion = worn down
        }
    }
}

/// Multi-axis biome determination using all noise layers.
///
/// Uses Whittaker diagram-style classification with:
/// - Temperature → ClimateClass
/// - Humidity → MoistureClass
/// - Elevation → ElevationClass
/// - Erosion → TerrainClass
/// - Tectonic → Mountain amplification & volcanic biomes
pub struct BiomeSplines {
    sea_level: f64,
}

impl BiomeSplines {
    /// Create a new spline evaluator with the given sea level threshold.
    pub fn new(sea_level: f64) -> Self {
        Self { sea_level }
    }

    /// Determine biome from all noise layers using multi-axis classification.
    ///
    /// # Arguments
    /// * `continentalness` - Base terrain height (-1 to 1, negative = ocean)
    /// * `temperature` - Raw temperature in degrees (-80 to 150)
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
        // Step 1: Compute effective elevation with tectonic amplification
        let elevation = self.compute_elevation(continentalness, peaks_valleys, erosion, tectonic);

        // Step 2: Check for ocean biomes first
        if elevation < self.sea_level {
            return self.ocean_biome(elevation, temperature, tectonic);
        }

        // Step 3: Adjust temperature based on elevation (lapse rate) and tectonic heat
        let adjusted_temp = self.adjust_temperature(temperature, elevation, tectonic);

        // Step 4: Adjust humidity with rain shadow effect
        let adjusted_humidity = self.adjust_humidity(humidity, elevation);

        // Step 5: Classify climate parameters
        let climate = ClimateClass::from_temperature(adjusted_temp);
        let moisture = MoistureClass::from_humidity(adjusted_humidity);
        let above_sea = elevation - self.sea_level;
        let elev_class = ElevationClass::from_elevation(above_sea);
        let terrain = TerrainClass::from_erosion(erosion);

        // Step 6: Check for special cases (volcanic, beach)
        // Volcanic only at very close plate boundaries with high heat
        let boundary_proximity = 1.0 - tectonic;
        if boundary_proximity > 0.9 && adjusted_temp > 50.0 && above_sea > 0.08 {
            return TileType::Volcanic;
        }

        // Coastal beach check
        if above_sea < 0.02 {
            return match climate {
                ClimateClass::Frozen => TileType::Glacier,
                ClimateClass::Cold => TileType::Snow,
                ClimateClass::Scorching => TileType::Sahara,
                _ => TileType::Beach,
            };
        }

        // Step 7: Land biome selection
        self.land_biome(climate, moisture, elev_class, terrain)
    }

    /// Compute effective elevation with tectonic mountain chain amplification.
    fn compute_elevation(
        &self,
        cont: f64,
        pv: f64,
        erosion: f64,
        tectonic: f64,
    ) -> f64 {
        let is_land = cont >= self.sea_level;
        let boundary_proximity = 1.0 - tectonic; // 1 at boundary, 0 at plate center

        // Mountains form along plate boundaries
        let tectonic_amp = 1.0 + boundary_proximity * boundary_proximity * 2.0;

        // Erosion dampens peaks (high erosion = worn mountains)
        let erosion_damp = 1.0 - erosion * 0.7;

        // Peak contribution only on land
        let peak_height = if is_land {
            pv.max(0.0) * 0.15 * tectonic_amp * erosion_damp
        } else {
            0.0
        };

        // Valleys carve into terrain
        let valley_depth = if is_land {
            pv.min(0.0).abs() * 0.08
        } else {
            0.0
        };

        // Ocean trenches at convergent plate boundaries
        let trench = if !is_land && boundary_proximity > 0.7 {
            (boundary_proximity - 0.7) * 0.5
        } else {
            0.0
        };

        cont + peak_height - valley_depth - trench
    }

    /// Determine ocean biome based on temperature and tectonic activity.
    fn ocean_biome(&self, elevation: f64, temp: f64, tectonic: f64) -> TileType {
        // Temperature extremes take priority - frozen or evaporated ocean
        if temp < -15.0 {
            return TileType::White; // Frozen ocean
        }
        if temp > 80.0 {
            return TileType::Sahara; // Evaporated - salt flats
        }

        // Ocean trenches only in temperate water at plate boundaries
        let boundary_proximity = 1.0 - tectonic;
        if boundary_proximity > 0.75 && elevation < self.sea_level - 0.2 {
            return TileType::OceanTrench;
        }

        TileType::Sea
    }

    /// Adjust temperature based on elevation (lapse rate) and tectonic heat.
    fn adjust_temperature(&self, temp: f64, elevation: f64, tectonic: f64) -> f64 {
        // Lapse rate: temperature decreases with altitude
        let elevation_above_sea = (elevation - self.sea_level).max(0.0);
        let lapse_rate = elevation_above_sea * 60.0; // ~60°C per 1.0 elevation unit

        // Volcanic heat near plate boundaries
        let boundary_proximity = 1.0 - tectonic;
        let volcanic_heat = boundary_proximity * boundary_proximity * 8.0;

        temp - lapse_rate + volcanic_heat
    }

    /// Adjust humidity with rain shadow effect at high elevations.
    fn adjust_humidity(&self, humidity: f64, elevation: f64) -> f64 {
        let elevation_above_sea = (elevation - self.sea_level).max(0.0);

        // Rain shadow kicks in above certain elevation
        let rain_shadow = if elevation_above_sea > 0.15 {
            ((elevation_above_sea - 0.15) * 2.5).min(0.4)
        } else {
            0.0
        };

        (humidity - rain_shadow).clamp(0.0, 1.0)
    }

    /// Multi-axis land biome selection using Whittaker-style classification.
    fn land_biome(
        &self,
        climate: ClimateClass,
        moisture: MoistureClass,
        elevation: ElevationClass,
        terrain: TerrainClass,
    ) -> TileType {
        use ClimateClass::*;
        use ElevationClass::*;
        use MoistureClass::*;
        use TerrainClass::*;

        match climate {
            // Frozen zone (dark side of tidally locked planet)
            Frozen => match moisture {
                Arid | Dry => TileType::Glacier,
                _ => TileType::Snow,
            },

            // Cold zone (transition from dark side)
            Cold => match (moisture, elevation) {
                (Arid | Dry, _) => TileType::Tundra,
                (_, Highland | Alpine) => TileType::Snow,
                (Moderate | Humid | Saturated, _) => TileType::Taiga,
            },

            // Temperate zone (habitable terminator band)
            Temperate => match (moisture, elevation, terrain) {
                (_, Alpine, _) => TileType::Mountain,
                (_, Highland, Rugged) => TileType::Mountain,
                (_, Highland, _) => TileType::Plateau,
                (Arid, _, _) => TileType::Steppe,
                (Dry, _, _) => TileType::Steppe,
                (Saturated, Lowland | Coastal, _) => TileType::Marsh,
                (Humid | Saturated, _, _) => TileType::Forest,
                (Moderate, _, _) => TileType::Plains,
            },

            // Warm zone (transition toward sun side)
            Warm => match (moisture, elevation, terrain) {
                (_, Alpine | Highland, _) => TileType::Mountain,
                (Arid, _, Rugged) => TileType::Badlands,
                (Arid, _, _) => TileType::Desert,
                (Dry, _, _) => TileType::Savanna,
                (Saturated, Lowland | Coastal, _) => TileType::Marsh,
                (Humid | Saturated, _, _) => TileType::Forest,
                (Moderate, _, _) => TileType::Savanna,
            },

            // Hot zone (approaching sun side)
            Hot => match (moisture, terrain) {
                (Arid, Rugged) => TileType::Badlands,
                (Arid, _) => TileType::Sahara,
                (Dry, _) => TileType::Desert,
                (Moderate, _) => TileType::Savanna,
                (Humid | Saturated, _) => TileType::Jungle,
            },

            // Scorching zone (sun side of tidally locked planet)
            Scorching => match moisture {
                Arid => TileType::Sahara,
                _ => TileType::Desert,
            },
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
    fn ocean_trench_at_boundary() {
        let s = splines();
        // Deep ocean at plate boundary (tectonic = 0) in temperate water
        // Note: Needs to be deep enough (elevation < sea_level - 0.2)
        let biome = s.evaluate(-0.6, 20.0, 0.0, 0.5, 0.0, 0.5);
        assert_eq!(biome, TileType::OceanTrench);
    }

    #[test]
    fn coastal_is_beach() {
        let s = splines();
        let biome = s.evaluate(-0.01, 25.0, 0.5, 0.5, 0.0, 0.5);
        assert_eq!(biome, TileType::Beach);
    }

    #[test]
    fn frozen_land_is_glacier_or_snow() {
        let s = splines();
        // Frozen + dry = glacier
        let biome = s.evaluate(0.1, -40.0, 0.5, 0.5, 0.0, 0.1);
        assert_eq!(biome, TileType::Glacier);
        // Frozen + humid = snow
        let biome2 = s.evaluate(0.1, -40.0, 0.5, 0.5, 0.0, 0.7);
        assert_eq!(biome2, TileType::Snow);
    }

    #[test]
    fn cold_dry_is_tundra() {
        let s = splines();
        let biome = s.evaluate(0.1, -10.0, 0.5, 0.5, 0.0, 0.1);
        assert_eq!(biome, TileType::Tundra);
    }

    #[test]
    fn cold_wet_is_taiga() {
        let s = splines();
        let biome = s.evaluate(0.1, -10.0, 0.5, 0.5, 0.0, 0.6);
        assert_eq!(biome, TileType::Taiga);
    }

    #[test]
    fn temperate_dry_is_steppe() {
        let s = splines();
        let biome = s.evaluate(0.1, 20.0, 0.5, 0.5, 0.0, 0.15);
        assert_eq!(biome, TileType::Steppe);
    }

    #[test]
    fn temperate_wet_lowland_is_marsh() {
        let s = splines();
        let biome = s.evaluate(0.02, 20.0, 0.5, 0.5, 0.0, 0.9);
        assert_eq!(biome, TileType::Marsh);
    }

    #[test]
    fn hot_dry_rugged_is_badlands() {
        let s = splines();
        // Hot + arid + rugged terrain (low erosion)
        let biome = s.evaluate(0.1, 65.0, 0.5, 0.1, 0.0, 0.1);
        assert_eq!(biome, TileType::Badlands);
    }

    #[test]
    fn hot_humid_is_jungle() {
        let s = splines();
        let biome = s.evaluate(0.1, 65.0, 0.5, 0.5, 0.0, 0.7);
        assert_eq!(biome, TileType::Jungle);
    }

    #[test]
    fn warm_moderate_is_savanna() {
        let s = splines();
        let biome = s.evaluate(0.1, 45.0, 0.5, 0.5, 0.0, 0.5);
        assert_eq!(biome, TileType::Savanna);
    }

    #[test]
    fn scorching_is_sahara_or_desert() {
        let s = splines();
        // Scorching + arid = sahara
        let biome = s.evaluate(0.1, 100.0, 0.5, 0.5, 0.0, 0.1);
        assert_eq!(biome, TileType::Sahara);
        // Scorching + some moisture = desert
        let biome2 = s.evaluate(0.1, 100.0, 0.5, 0.5, 0.0, 0.4);
        assert_eq!(biome2, TileType::Desert);
    }

    #[test]
    fn volcanic_near_boundary() {
        let s = splines();
        // Very close to plate boundary (tectonic < 0.1) with hot temperature
        // Need higher elevation (above_sea > 0.08) and temp > 50 after adjustment
        let biome = s.evaluate(0.15, 60.0, 0.05, 0.5, 0.0, 0.5);
        assert_eq!(biome, TileType::Volcanic);
    }

    #[test]
    fn mountains_at_plate_boundaries() {
        let s = splines();
        // High peaks at plate boundary should create mountains
        // Use 50°C base temp because high elevation causes ~30°C cooling from lapse rate
        let biome = s.evaluate(0.2, 50.0, 0.1, 0.2, 0.8, 0.5);
        assert_eq!(biome, TileType::Mountain);
    }

    #[test]
    fn volcanic_heat_affects_temperature() {
        let s = splines();
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
        let humid_low = s.adjust_humidity(0.8, 0.0);
        let humid_high = s.adjust_humidity(0.8, 0.3);
        assert!(
            humid_high < humid_low,
            "High elevation humidity {} should be less than low {}",
            humid_high,
            humid_low
        );
    }
}
