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
    Coastal,  // < 0.04 above sea level
    Lowland,  // 0.04 to 0.12
    Upland,   // 0.12 to 0.25
    Highland, // 0.25 to 0.38
    Alpine,   // > 0.38
}

impl ElevationClass {
    pub fn from_elevation(above_sea: f64) -> Self {
        if above_sea < 0.04 {
            Self::Coastal
        } else if above_sea < 0.12 {
            Self::Lowland
        } else if above_sea < 0.25 {
            Self::Upland
        } else if above_sea < 0.38 {
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
    /// * `aridity` - Aridity level (0 = lush, 1 = hyper-arid)
    pub fn evaluate(
        &self,
        continentalness: f64,
        temperature: f64,
        tectonic: f64,
        erosion: f64,
        peaks_valleys: f64,
        humidity: f64,
        aridity: f64,
        rock_hardness: f64,
    ) -> TileType {
        self.evaluate_with_light(continentalness, temperature, tectonic, erosion, peaks_valleys, humidity, aridity, rock_hardness, 0.3)
    }

    /// Evaluate biome with light level for sun-side gate.
    pub fn evaluate_with_light(
        &self,
        continentalness: f64,
        temperature: f64,
        tectonic: f64,
        erosion: f64,
        peaks_valleys: f64,
        humidity: f64,
        aridity: f64,
        rock_hardness: f64,
        light_level: f64,
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
        let mut moisture = MoistureClass::from_humidity(adjusted_humidity);
        let above_sea = elevation - self.sea_level;
        let elev_class = ElevationClass::from_elevation(above_sea);
        let terrain = TerrainClass::from_erosion(erosion);

        // Step 5b: Temperature gate — above ~45°C, no vegetation.
        // Gate varies ±3°C with rock_hardness for a fuzzy transition.
        let gate_temp = 45.0 + (rock_hardness - 0.5) * 6.0; // 42-48°C
        if temperature > gate_temp {
            moisture = MoistureClass::Arid;
        } else if aridity > 0.75 {
            moisture = MoistureClass::Arid;
        } else if aridity > 0.6 {
            if matches!(moisture, MoistureClass::Moderate | MoistureClass::Humid | MoistureClass::Saturated) {
                moisture = MoistureClass::Dry;
            }
        }

        // Step 6: Check for special cases (volcanic, beach, coastal detail)
        // Coastal zone
        if above_sea < 0.02 {
            return match climate {
                ClimateClass::Frozen => TileType::Glacier,
                ClimateClass::Cold => TileType::Snow,
                ClimateClass::Scorching => TileType::SaltFlat,
                ClimateClass::Warm | ClimateClass::Hot
                    if matches!(moisture, MoistureClass::Humid | MoistureClass::Saturated) =>
                {
                    TileType::Mangrove
                }
                _ if rock_hardness > 0.7 && peaks_valleys.abs() > 0.2 => TileType::SeaCliff,
                _ if peaks_valleys.abs() > 0.3 || rock_hardness > 0.6 => TileType::RockyCoast,
                _ => TileType::Beach,
            };
        }

        // Sea cliff: rugged terrain just above coastal zone
        if above_sea >= 0.02 && above_sea < 0.05 && terrain == TerrainClass::Rugged {
            return TileType::SeaCliff;
        }

        // Step 7: Land biome selection
        self.land_biome(climate, moisture, elev_class, terrain, rock_hardness)
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
        let _boundary_proximity = 1.0 - tectonic; // reserved for future use

        // Erosion dampens peaks (high erosion = worn mountains)
        let erosion_damp = 1.0 - erosion * 0.7;

        // Peak contribution only on land
        let peak_height = if is_land {
            pv.max(0.0) * 0.25 * erosion_damp
        } else {
            0.0
        };

        // Valleys carve into terrain
        let valley_depth = if is_land {
            pv.min(0.0).abs() * 0.12
        } else {
            0.0
        };

        let trench = 0.0;

        cont + peak_height - valley_depth - trench
    }

    /// Determine ocean biome based on depth, temperature, and tectonic activity.
    fn ocean_biome(&self, elevation: f64, temp: f64, tectonic: f64) -> TileType {
        // Temperature extremes take priority - frozen or evaporated ocean
        if temp < -15.0 {
            return TileType::White; // Frozen ocean
        }
        if temp > 80.0 {
            return TileType::SaltFlat; // Evaporated - salt flats
        }

        let depth = self.sea_level - elevation;

        // Coral reef: warm shallow water away from plate boundaries
        if depth < 0.05 && temp > 20.0 && temp < 35.0 && tectonic > 0.5 {
            return TileType::CoralReef;
        }

        // Ocean trench at plate boundaries with significant depth
        if tectonic < 0.2 && depth > 0.3 {
            return TileType::OceanTrench;
        }
        // Mid-ocean ridge near divergent boundaries
        if tectonic < 0.3 && depth > 0.1 {
            return TileType::OceanRidge;
        }

        // Depth-based classification
        if depth < 0.05 {
            TileType::ShallowSea
        } else if depth < 0.15 {
            TileType::ContinentalShelf
        } else if depth > 0.25 {
            TileType::DeepOcean
        } else {
            TileType::Sea
        }
    }

    /// Adjust temperature based on elevation and tectonic heat.
    /// NOTE: lapse rate is already applied in derive_temperature. This only applies
    /// minor adjustments that don't duplicate the primary lapse rate.
    fn adjust_temperature(&self, temp: f64, _elevation: f64, _tectonic: f64) -> f64 {
        // No additional lapse rate — derive_temperature already handles it.
        // This prevents double-cooling that lets hot sun-side mountains appear temperate.
        temp
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

    /// Biome evaluation with spatial dithering at biome boundaries.
    ///
    /// Produces fuzzy transitions between biomes by perturbing inputs near
    /// classification boundaries and using a position-based hash as threshold.
    pub fn evaluate_dithered(
        &self,
        continentalness: f64,
        temperature: f64,
        tectonic: f64,
        erosion: f64,
        peaks_valleys: f64,
        humidity: f64,
        aridity: f64,
        rock_hardness: f64,
        px: usize,
        py: usize,
    ) -> TileType {
        self.evaluate_dithered_with_light(continentalness, temperature, tectonic, erosion, peaks_valleys, humidity, aridity, rock_hardness, px, py, 0.3)
    }

    pub fn evaluate_dithered_with_light(
        &self,
        continentalness: f64,
        temperature: f64,
        tectonic: f64,
        erosion: f64,
        peaks_valleys: f64,
        humidity: f64,
        aridity: f64,
        rock_hardness: f64,
        px: usize,
        py: usize,
        light_level: f64,
    ) -> TileType {
        // Perturb temperature using rock_hardness as a smooth noise source
        // to break up climate band boundaries. Rock hardness is an independent
        // fBm field that varies at a different scale than temperature.
        // Don't perturb near the 45°C vegetation gate to preserve that hard stop.
        let temp_perturb = (rock_hardness - 0.5) * 12.0; // ±6°C
        let biome_temp = if temperature > 40.0 {
            temperature // preserve 45°C gate zone
        } else {
            temperature + temp_perturb
        };

        // Perturb humidity using peaks_valleys as a smooth noise source
        // to break up moisture class boundaries (forest→savanna, steppe→desert)
        let humid_perturb = peaks_valleys * 0.08; // ±8% humidity shift
        let biome_humidity = (humidity + humid_perturb).clamp(0.0, 1.0);

        let base = self.evaluate_with_light(continentalness, biome_temp, tectonic, erosion, peaks_valleys, biome_humidity, aridity, rock_hardness, light_level);

        // Position hash for deterministic spatial noise
        let hash = (((px.wrapping_mul(374761393)) ^ (py.wrapping_mul(668265263))) & 0xFFFF) as f64 / 65535.0;

        // Try small perturbations to detect boundary proximity
        let alt = self.evaluate(
            continentalness + (hash - 0.5) * 0.02,
            biome_temp + (hash - 0.5) * 4.0,
            tectonic,
            erosion,
            peaks_valleys,
            biome_humidity + (hash - 0.5) * 0.06,
            aridity,
            rock_hardness,
        );

        if alt != base && hash > 0.5 {
            alt
        } else {
            base
        }
    }

    /// Multi-axis land biome selection using Whittaker-style classification.
    fn land_biome(
        &self,
        climate: ClimateClass,
        moisture: MoistureClass,
        elevation: ElevationClass,
        terrain: TerrainClass,
        rock_hardness: f64,
    ) -> TileType {
        use ClimateClass::*;
        use ElevationClass::*;
        use MoistureClass::*;
        use TerrainClass::*;

        match climate {
            // Frozen zone (dark side of tidally locked planet)
            Frozen => match (moisture, elevation) {
                (_, Alpine) => TileType::Glacier,
                (Arid | Dry, Highland) => TileType::Glacier,
                (Arid | Dry, _) => TileType::IceSheet,
                (Saturated, Lowland | Coastal) => TileType::FrozenBog,
                (_, _) => TileType::Snow,
            },

            // Cold zone (transition from dark side)
            Cold => match (moisture, elevation, terrain) {
                (_, Alpine, _) => TileType::Snow,
                (_, Highland, Rugged) => TileType::Mountain,
                (_, Highland, _) => TileType::AlpineMeadow,
                (Arid | Dry, _, _) => TileType::Tundra,
                (Saturated, Lowland | Coastal, _) => TileType::FrozenBog,
                (Humid | Saturated, _, _) => TileType::Taiga,
                (Moderate, _, _) => TileType::Tundra,
            },

            // Temperate zone (habitable terminator band)
            Temperate => match (moisture, elevation, terrain) {
                (_, Alpine, _) => TileType::Mountain,
                (_, Highland, Rugged) => TileType::Mountain,
                (Humid | Saturated, Highland, _) => TileType::Plateau,
                (_, Highland, _) => TileType::Plateau,
                (Arid, _, Rugged) => TileType::Scrubland,
                (Arid, _, _) => TileType::Steppe,
                (Dry, _, Rugged) => TileType::Scrubland,
                (Dry, Upland, _) => TileType::Woodland,
                (Dry, _, _) => TileType::Steppe,
                (Saturated, Lowland | Coastal, _) => TileType::Marsh,
                (Saturated, _, _) => TileType::TemperateRainforest,
                (Humid, Lowland, Flat) => TileType::Meadow,
                (Humid, Lowland | Coastal, _) => TileType::DeciduousForest,
                (Humid, _, _) => TileType::DeciduousForest,
                (Moderate, Lowland, Flat) => TileType::Meadow,
                (Moderate, Lowland, _) => TileType::Plains,
                (Moderate, Upland, _) => TileType::Woodland,
                (Moderate, _, _) => TileType::Plains,
            },

            // Warm zone (transition toward sun side)
            Warm => match (moisture, elevation, terrain) {
                (_, Alpine, _) => TileType::Mountain,
                (_, Highland, Rugged) => TileType::Mountain,
                (Humid | Saturated, Highland, _) => TileType::CloudForest,
                (Moderate | Dry, Highland, _) => TileType::HighlandSavanna,
                (Arid, Highland, _) => TileType::Badlands,
                (Arid, _, Rugged) => TileType::Badlands,
                (Arid, _, _) => TileType::Desert,
                (Dry, _, Rugged) => TileType::Thornland,
                (Dry, Upland, _) => TileType::DryWoodland,
                (Dry, _, _) => TileType::Savanna,
                (Saturated, Lowland | Coastal, _) => TileType::Marsh,
                (Saturated, _, _) => TileType::SubtropicalForest,
                (Humid, Lowland | Coastal, _) => TileType::SubtropicalForest,
                (Humid, _, _) => TileType::SubtropicalForest,
                (Moderate, Upland, _) => TileType::DryWoodland,
                (Moderate, _, _) => TileType::Savanna,
            },

            // Hot zone (approaching sun side)
            // Note: 45°C gate forces moisture to Arid for all Hot temps (55-80°C)
            Hot => match (moisture, elevation, terrain) {
                (Humid | Saturated, Highland | Alpine, _) => TileType::CloudForest,
                (Moderate, Highland | Alpine, _) => TileType::HighlandSavanna,
                (_, Alpine, _) => TileType::Mountain,
                (Arid, Highland, Rugged) => TileType::Badlands,
                (Arid, _, Rugged) => if rock_hardness > 0.6 { TileType::ScorchedRock } else { TileType::Badlands },
                (Arid, _, Flat) => if rock_hardness > 0.6 { TileType::Hamada } else { TileType::Erg },
                (Arid, _, _) => if rock_hardness > 0.6 { TileType::Hamada } else { TileType::Sahara },
                (Dry, _, Rugged) => TileType::Hamada,
                (Dry, _, _) => TileType::Desert,
                (Moderate, _, _) => TileType::Savanna,
                (Humid | Saturated, _, _) => TileType::Jungle,
            },

            // Scorching zone (sun side of tidally locked planet)
            // Note: 45°C gate forces moisture to Arid for all Scorching temps (>80°C)
            Scorching => match (moisture, elevation, terrain) {
                (_, Alpine | Highland, _) => TileType::ScorchedRock,
                (Arid, _, Flat) => if rock_hardness < 0.4 { TileType::Erg } else { TileType::SaltFlat },
                (Arid, _, Rugged) => if rock_hardness > 0.6 { TileType::ScorchedRock } else { TileType::MoltenWaste },
                (Arid, _, _) => if rock_hardness > 0.6 { TileType::Hamada } else { TileType::Sahara },
                (Dry, _, Rugged) => TileType::Hamada,
                (Dry, _, _) => TileType::Erg,
                (_, _, _) => TileType::Desert,
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
        // Medium depth (0.175), far from plate boundary → regular Sea
        let biome = s.evaluate(-0.2, 20.0, 0.5, 0.5, 0.0, 0.5, 0.3, 0.5);
        assert_eq!(biome, TileType::Sea);
    }

    #[test]
    fn continental_shelf_at_moderate_depth() {
        let s = splines();
        // Depth 0.125 (between 0.05 and 0.15) → ContinentalShelf
        let biome = s.evaluate(-0.15, 20.0, 0.5, 0.5, 0.0, 0.5, 0.3, 0.5);
        assert_eq!(biome, TileType::ContinentalShelf);
    }

    #[test]
    fn frozen_ocean_is_white() {
        let s = splines();
        let biome = s.evaluate(-0.15, -30.0, 0.5, 0.5, 0.0, 0.5, 0.3, 0.5);
        assert_eq!(biome, TileType::White);
    }

    #[test]
    fn deep_ocean_at_boundary_is_trench() {
        let s = splines();
        let biome = s.evaluate(-0.6, 20.0, 0.0, 0.5, 0.0, 0.5, 0.3, 0.5);
        assert_eq!(biome, TileType::OceanTrench);
    }

    #[test]
    fn coastal_is_beach() {
        let s = splines();
        // Low rock_hardness + low peaks = beach
        let biome = s.evaluate(-0.01, 25.0, 0.5, 0.5, 0.0, 0.5, 0.3, 0.3);
        assert_eq!(biome, TileType::Beach);
    }

    #[test]
    fn frozen_land_is_ice_sheet_or_snow() {
        let s = splines();
        // Frozen + dry = ice sheet (not alpine)
        let biome = s.evaluate(0.1, -40.0, 0.5, 0.5, 0.0, 0.1, 0.3, 0.5);
        assert_eq!(biome, TileType::IceSheet);
        // Frozen + humid = snow
        let biome2 = s.evaluate(0.1, -40.0, 0.5, 0.5, 0.0, 0.7, 0.3, 0.5);
        assert_eq!(biome2, TileType::Snow);
    }

    #[test]
    fn cold_dry_is_tundra() {
        let s = splines();
        let biome = s.evaluate(0.1, -10.0, 0.5, 0.5, 0.0, 0.1, 0.3, 0.5);
        assert_eq!(biome, TileType::Tundra);
    }

    #[test]
    fn cold_wet_is_taiga() {
        let s = splines();
        let biome = s.evaluate(0.1, -10.0, 0.5, 0.5, 0.0, 0.6, 0.3, 0.5);
        assert_eq!(biome, TileType::Taiga);
    }

    #[test]
    fn temperate_dry_is_steppe() {
        let s = splines();
        let biome = s.evaluate(0.1, 20.0, 0.5, 0.5, 0.0, 0.15, 0.3, 0.5);
        assert_eq!(biome, TileType::Steppe);
    }

    #[test]
    fn temperate_wet_lowland_is_marsh() {
        let s = splines();
        let biome = s.evaluate(0.02, 20.0, 0.5, 0.5, 0.0, 0.9, 0.3, 0.5);
        assert_eq!(biome, TileType::Marsh);
    }

    #[test]
    fn hot_dry_rugged_is_desert() {
        let s = splines();
        // Hot (65°C) + rugged — above 45°C gate, forced to arid desert biome
        let biome = s.evaluate(0.1, 65.0, 0.5, 0.1, 0.0, 0.1, 0.3, 0.5);
        assert!(matches!(biome, TileType::Sahara | TileType::Desert | TileType::Badlands | TileType::Hamada | TileType::ScorchedRock),
            "At 65°C should be desert-type, got {:?}", biome);
    }

    #[test]
    fn hot_humid_is_NOT_jungle() {
        let s = splines();
        // 65°C is above the 45°C hard gate — even with humidity, no green biomes
        let biome = s.evaluate(0.1, 65.0, 0.5, 0.5, 0.0, 0.7, 0.3, 0.5);
        assert!(!matches!(biome, TileType::Jungle | TileType::Forest | TileType::Plains
            | TileType::DeciduousForest | TileType::TemperateRainforest | TileType::SubtropicalForest),
            "At 65°C nothing should be green, got {:?}", biome);
    }

    #[test]
    fn warm_moderate_lowland_is_savanna() {
        let s = splines();
        // Lower continentalness for lowland
        let biome = s.evaluate(0.05, 45.0, 0.5, 0.5, 0.0, 0.5, 0.3, 0.5);
        assert_eq!(biome, TileType::Savanna);
    }

    #[test]
    fn scorching_is_sahara_or_desert() {
        let s = splines();
        // Scorching + arid = sahara (rolling terrain)
        let biome = s.evaluate(0.1, 100.0, 0.5, 0.5, 0.0, 0.1, 0.3, 0.5);
        assert_eq!(biome, TileType::Sahara);
        // Scorching + humidity forced arid by 45°C gate = still desert
        let biome2 = s.evaluate(0.1, 100.0, 0.5, 0.5, 0.0, 0.4, 0.3, 0.5);
        // 45°C gate forces Arid, so same result as above
        assert_eq!(biome2, TileType::Sahara);
    }

    #[test]
    fn mountains_at_high_peaks() {
        let s = splines();
        // High peaks with moderate erosion should create mountains
        let biome = s.evaluate(0.2, 50.0, 0.5, 0.2, 0.8, 0.5, 0.3, 0.5);
        assert_eq!(biome, TileType::Mountain);
    }

    #[test]
    fn high_aridity_forces_arid() {
        let s = splines();
        // Temperate + moderate humidity but very high aridity => should force arid moisture class
        let biome = s.evaluate(0.1, 20.0, 0.5, 0.5, 0.0, 0.5, 0.9, 0.5);
        assert_eq!(biome, TileType::Steppe, "High aridity should override moisture to Arid");
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

    // --- New biome variant tests ---

    #[test]
    fn frozen_dry_lowland_is_ice_sheet() {
        let s = splines();
        let biome = s.evaluate(0.05, -40.0, 0.5, 0.5, 0.0, 0.15, 0.3, 0.5);
        assert_eq!(biome, TileType::IceSheet);
    }

    #[test]
    fn frozen_saturated_lowland_is_frozen_bog() {
        let s = splines();
        let biome = s.evaluate(0.02, -40.0, 0.5, 0.5, 0.0, 0.9, 0.3, 0.5);
        assert_eq!(biome, TileType::FrozenBog);
    }

    #[test]
    fn cold_highland_is_alpine_meadow() {
        let s = splines();
        // Cold zone (-20 to 3°C), highland, rolling terrain
        let biome = s.evaluate(0.25, -5.0, 0.5, 0.5, 0.0, 0.5, 0.3, 0.5);
        assert_eq!(biome, TileType::AlpineMeadow);
    }

    #[test]
    fn temperate_humid_lowland_flat_is_meadow() {
        let s = splines();
        // Temperate, humid, lowland, flat (high erosion)
        let biome = s.evaluate(0.05, 20.0, 0.5, 0.8, 0.0, 0.7, 0.3, 0.5);
        assert_eq!(biome, TileType::Meadow);
    }

    #[test]
    fn temperate_humid_is_deciduous_forest() {
        let s = splines();
        // Temperate, humid, lowland, rolling
        let biome = s.evaluate(0.05, 20.0, 0.5, 0.5, 0.0, 0.7, 0.3, 0.5);
        assert_eq!(biome, TileType::DeciduousForest);
    }

    #[test]
    fn temperate_saturated_upland_is_rainforest() {
        let s = splines();
        let biome = s.evaluate(0.12, 20.0, 0.5, 0.5, 0.0, 0.9, 0.3, 0.5);
        assert_eq!(biome, TileType::TemperateRainforest);
    }

    #[test]
    fn temperate_dry_upland_is_woodland() {
        let s = splines();
        let biome = s.evaluate(0.12, 20.0, 0.5, 0.5, 0.0, 0.3, 0.3, 0.5);
        assert_eq!(biome, TileType::Woodland);
    }

    #[test]
    fn temperate_arid_rugged_is_scrubland() {
        let s = splines();
        let biome = s.evaluate(0.1, 20.0, 0.5, 0.1, 0.0, 0.1, 0.3, 0.5);
        assert_eq!(biome, TileType::Scrubland);
    }

    #[test]
    fn warm_humid_highland_not_green_above_45c() {
        let s = splines();
        // 55°C is above the 45°C gate — even with max humidity, no green
        let biome = s.evaluate(0.25, 55.0, 0.5, 0.5, 0.0, 0.95, 0.3, 0.5);
        assert!(!matches!(biome, TileType::CloudForest | TileType::Forest | TileType::Jungle),
            "At 55°C nothing should be green, got {:?}", biome);
    }

    #[test]
    fn warm_humid_highland_below_45c_is_cloud_forest() {
        let s = splines();
        // 35°C is well below the 45°C gate — green biomes should be possible
        let biome = s.evaluate(0.1, 35.0, 0.5, 0.5, 0.0, 0.8, 0.2, 0.5);
        assert!(matches!(biome, TileType::Forest | TileType::Jungle | TileType::SubtropicalForest
            | TileType::TemperateRainforest | TileType::Woodland | TileType::DryWoodland
            | TileType::Plains | TileType::Meadow | TileType::Savanna | TileType::Marsh),
            "Below 45°C with moisture should allow green biomes, got {:?}", biome);
    }

    #[test]
    fn warm_dry_highland_not_green_above_45c() {
        let s = splines();
        // 55°C above gate — forced to arid
        let biome = s.evaluate(0.25, 55.0, 0.5, 0.5, 0.0, 0.6, 0.3, 0.5);
        assert!(!matches!(biome, TileType::HighlandSavanna | TileType::Savanna | TileType::Forest),
            "At 55°C nothing should be green, got {:?}", biome);
    }

    #[test]
    fn warm_humid_is_subtropical_forest() {
        let s = splines();
        let biome = s.evaluate(0.05, 45.0, 0.5, 0.5, 0.0, 0.7, 0.3, 0.5);
        assert_eq!(biome, TileType::SubtropicalForest);
    }

    #[test]
    fn warm_dry_rugged_is_thornland() {
        let s = splines();
        let biome = s.evaluate(0.1, 45.0, 0.5, 0.1, 0.0, 0.3, 0.3, 0.5);
        assert_eq!(biome, TileType::Thornland);
    }

    #[test]
    fn warm_dry_upland_is_dry_woodland() {
        let s = splines();
        let biome = s.evaluate(0.12, 45.0, 0.5, 0.5, 0.0, 0.3, 0.3, 0.5);
        assert_eq!(biome, TileType::DryWoodland);
    }

    #[test]
    fn hot_arid_flat_is_erg() {
        let s = splines();
        // Hot, arid, flat (high erosion)
        let biome = s.evaluate(0.1, 65.0, 0.5, 0.8, 0.0, 0.1, 0.3, 0.5);
        assert_eq!(biome, TileType::Erg);
    }

    #[test]
    fn hot_dry_rugged_is_desert_type() {
        let s = splines();
        // 65°C above gate — must be desert/scorched, not green
        let biome = s.evaluate(0.1, 65.0, 0.5, 0.1, 0.0, 0.3, 0.3, 0.5);
        assert!(matches!(biome, TileType::Sahara | TileType::Desert | TileType::Badlands
            | TileType::Hamada | TileType::ScorchedRock | TileType::Erg),
            "At 65°C should be desert-type, got {:?}", biome);
    }

    #[test]
    fn scorching_highland_is_scorched_rock() {
        let s = splines();
        let biome = s.evaluate(0.25, 100.0, 0.5, 0.5, 0.0, 0.1, 0.3, 0.5);
        assert_eq!(biome, TileType::ScorchedRock);
    }

    #[test]
    fn scorching_arid_flat_is_salt_flat() {
        let s = splines();
        let biome = s.evaluate(0.1, 100.0, 0.5, 0.8, 0.0, 0.1, 0.3, 0.5);
        assert_eq!(biome, TileType::SaltFlat);
    }

    #[test]
    fn scorching_arid_rugged_is_molten_waste() {
        let s = splines();
        let biome = s.evaluate(0.1, 100.0, 0.5, 0.1, 0.0, 0.1, 0.3, 0.5);
        assert_eq!(biome, TileType::MoltenWaste);
    }

    #[test]
    fn scorching_dry_is_desert_type() {
        let s = splines();
        let biome = s.evaluate(0.1, 100.0, 0.5, 0.5, 0.0, 0.3, 0.3, 0.5);
        assert!(matches!(biome, TileType::Sahara | TileType::Desert | TileType::Erg
            | TileType::ScorchedRock | TileType::MoltenWaste | TileType::SaltFlat),
            "At 100°C should be scorched/desert, got {:?}", biome);
    }

    #[test]
    fn nothing_green_above_45c() {
        let s = splines();
        let green_biomes = [
            TileType::Forest, TileType::Jungle, TileType::Plains, TileType::Meadow,
            TileType::DeciduousForest, TileType::TemperateRainforest, TileType::SubtropicalForest,
            TileType::CloudForest, TileType::Woodland, TileType::DryWoodland, TileType::Taiga,
            TileType::AlpineMeadow, TileType::Marsh, TileType::Mangrove, TileType::Oasis,
        ];
        // Test across a range of temperatures above 45°C, various humidity/elevation combos
        for temp in [46.0, 55.0, 65.0, 80.0, 100.0, 120.0] {
            for humidity in [0.1, 0.3, 0.5, 0.7, 0.9] {
                for cont in [0.05, 0.15, 0.3] {
                    let biome = s.evaluate(cont, temp, 0.5, 0.5, 0.0, humidity, 0.3, 0.5);
                    assert!(!green_biomes.contains(&biome),
                        "At {}°C, humidity={}, cont={}: got green biome {:?}",
                        temp, humidity, cont, biome);
                }
            }
        }
    }

    #[test]
    fn ocean_hot_is_salt_flat() {
        let s = splines();
        let biome = s.evaluate(-0.5, 90.0, 0.5, 0.5, 0.0, 0.5, 0.3, 0.5);
        assert_eq!(biome, TileType::SaltFlat);
    }

    #[test]
    fn coastal_hard_rock_is_rocky_coast() {
        let s = splines();
        // Coastal, temperate, hard rock
        let biome = s.evaluate(-0.01, 25.0, 0.5, 0.5, 0.0, 0.5, 0.3, 0.8);
        assert_eq!(biome, TileType::RockyCoast);
    }

    #[test]
    fn many_distinct_biomes_reachable() {
        let s = splines();
        let mut biomes = std::collections::HashSet::new();
        // Sweep through parameter space
        for &cont in &[-0.5, -0.01, 0.02, 0.05, 0.1, 0.12, 0.2, 0.25, 0.35] {
            for &temp in &[-40.0, -10.0, 10.0, 20.0, 45.0, 65.0, 100.0] {
                for &humid in &[0.1, 0.3, 0.5, 0.7, 0.9] {
                    for &eros in &[0.1, 0.5, 0.8] {
                        let biome = s.evaluate(cont, temp, 0.5, eros, 0.0, humid, 0.3, 0.5);
                        biomes.insert(biome);
                    }
                }
            }
        }
        assert!(
            biomes.len() >= 30,
            "Expected 30+ distinct biomes, got {}: {:?}",
            biomes.len(),
            biomes
        );
    }
}
