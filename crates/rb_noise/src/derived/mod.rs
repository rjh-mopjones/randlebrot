use rb_core::TileType;

/// Temperature derived from light level + elevation + humidity + continentalness.
///
/// Output range: ~[-80, +150]°C (matches existing BiomeSplines expectations).
/// - light_level: [0, 1] where 1.0 = sub-stellar point
/// - elevation: heightmap value (can be negative for ocean)
/// - humidity: [0, 1] where 1.0 = saturated
/// - continentalness: distance from coast (negative = ocean, positive = inland)
pub fn derive_temperature(light_level: f64, elevation: f64, humidity: f64, continentalness: f64) -> f64 {
    // Piecewise linear S-curve: compress cold/hot extremes, stretch habitable range.
    // Habitable (3-45°C) now spans ~34% of the light gradient (up from ~21% linear).
    //   Light 0.00–0.28 → -80 to  0°C  (cold side compressed)
    //   Light 0.28–0.62 →   0 to 45°C  (habitable zone stretched)
    //   Light 0.62–1.00 →  45 to 120°C (hot side, transition kept ≤0.625 for tests)
    let base_temp = if light_level < 0.28 {
        let t = light_level / 0.28;
        -80.0 + t * 80.0
    } else if light_level < 0.62 {
        let t = (light_level - 0.28) / 0.34;
        t * 45.0
    } else {
        let t = (light_level - 0.62) / 0.38;
        45.0 + t * 75.0
    };
    // Lapse rate: mountains are colder (only for positive elevation)
    // Capped at 30% of base_temp — on a tidally locked planet with constant
    // direct sunlight, mountains can't cool enough to become habitable.
    // At 100°C base: max drop = 30°C → 70°C minimum (still scorching)
    let max_lapse = if base_temp > 0.0 { base_temp * 0.30 } else { 25.0 };
    let lapse_rate = (elevation.max(0.0) * 55.0).min(max_lapse);
    // Moisture moderates extremes slightly
    let humidity_buffer = humidity * 5.0;
    let raw = base_temp - lapse_rate + humidity_buffer;
    // Coastal moderation: ocean proximity pulls temperature toward moderate
    let inland_factor = ((continentalness + 0.01).max(0.0) * 5.0).clamp(0.0, 1.0);
    let moderate_temp = 15.0;
    // Coastal areas get pulled toward moderate (25% effect)
    // Taper off in scorching zones so the 45°C vegetation gate isn't breached
    let heat_damper = ((55.0 - raw) / 10.0).clamp(0.0, 1.0); // full effect below 45°C, zero above 55°C
    let coastal_moderation = (moderate_temp - raw) * (1.0 - inland_factor) * 0.25 * heat_damper;
    // Deep inland areas get pushed away from moderate (continental extremity)
    let extremity = if inland_factor > 0.7 {
        let deep_inland = (inland_factor - 0.7) / 0.3; // 0..1
        (raw - moderate_temp) * deep_inland * 0.12 // push 12% further from moderate
    } else {
        0.0
    };
    raw + coastal_moderation + extremity
}

/// Heightmap from geological layers (used as elevation input for temperature).
///
/// Combines continentalness (reduced to 80% for base), broad tectonic uplift,
/// and peaks/valleys relief with coastal tapering.
pub fn derive_heightmap(continentalness: f64, _tectonic: f64, peaks_valleys: f64) -> f64 {
    let continental_base = continentalness * 0.95;
    let relief = peaks_valleys * 0.85;
    let coastal_taper = if continentalness < -0.05 {
        0.3
    } else if continentalness < 0.1 {
        ((continentalness + 0.05) / 0.15).clamp(0.0, 1.0).sqrt()
    } else {
        1.0
    };
    (continental_base + relief * coastal_taper).clamp(-1.0, 1.0)
}

/// Erosion derived from heightmap, rock hardness, and humidity.
///
/// Steep terrain erodes more, wet regions erode faster, hard rock resists.
/// - heightmap: unified elevation value
/// - rock_hardness: [0, 1] where 1.0 = very hard
/// - humidity: [0, 1] where 1.0 = saturated
pub fn derive_erosion(heightmap: f64, rock_hardness: f64, humidity: f64) -> f64 {
    let raw = (heightmap.max(0.0) * 2.5 + humidity * 0.8) * (1.0 - rock_hardness * 0.3);
    raw.sqrt().clamp(0.0, 1.0)
}

/// Peaks amplified by tectonic stress, sustained by hard rock.
///
/// Plate interiors get 25% amplitude (visible rolling terrain),
/// boundaries get full amplitude for dramatic mountain ranges.
///
/// - base_pv: raw peaks/valleys noise [-1, 1]
/// - tectonic: boundary distance [0, 1] where 0 = boundary
/// - rock_hardness: [0, 1] where 1.0 = very hard
pub fn derive_peaks_valleys(base_pv: f64, tectonic: f64, rock_hardness: f64) -> f64 {
    let stress = 1.0 - tectonic;
    // Cubic envelope: boundaries get full amplitude, interiors get gentle rolling hills
    let stress_envelope = stress * stress * stress;
    let amplitude = 0.08 + stress_envelope * 0.92;
    let hardness_factor = 0.7 + rock_hardness * 0.3;
    (base_pv * amplitude * hardness_factor).clamp(-1.0, 1.0)
}

/// Aridity from temperature and humidity.
///
/// This is where the tidal-lock drying effect lives. High temperature + low humidity = arid.
/// - temperature: degrees C
/// - humidity: [0, 1]
/// Output: [0, 1] where 1.0 = hyper-arid
pub fn derive_aridity(temperature: f64, humidity: f64) -> f64 {
    // Steeper temperature ramp: anything above 55°C is fully arid from heat alone
    let temp_factor = ((temperature - 10.0) / 45.0).clamp(0.0, 1.0);
    // Temperature dominates: 65% temp, 35% dryness
    (temp_factor * 0.65 + (1.0 - humidity) * 0.35).clamp(0.0, 1.0)
}

/// Precipitation type from temperature, humidity, and elevation.
///
/// Output: [-1, 1] where -1 = snow, 0 = rain, +1 = no precipitation.
pub fn derive_precipitation_type(temperature: f64, humidity: f64, heightmap: f64) -> f64 {
    if humidity < 0.15 {
        return 1.0; // Too dry for any precip
    }
    let snow_factor = ((-temperature + 5.0) / 25.0).clamp(0.0, 1.0);
    let altitude_bonus = (heightmap - 0.1).max(0.0) * 2.0; // Higher = more snow
    let snow = (snow_factor + altitude_bonus).min(1.0);
    let humid_capped = humidity.min(0.8);
    -snow * humid_capped + (1.0 - humid_capped) * (1.0 - snow)
}

/// Snowpack — persistent snow accumulation.
///
/// Combines temperature-based snow and altitude-based snow.
/// - precipitation_type: [-1, 1] where -1 = snow
/// - temperature: degrees C
/// - heightmap: elevation value
/// - light_level: [0, 1] where 1.0 = sub-stellar point
/// Output: [0, 1]
pub fn derive_snowpack(precipitation_type: f64, temperature: f64, heightmap: f64, light_level: f64) -> f64 {
    let cold_factor = ((3.0 - temperature) / 40.0).clamp(0.0, 1.0);
    let snow_precip = (-precipitation_type).max(0.0);
    let temperature_snow = cold_factor * snow_precip;

    // Altitude snow: peaks above threshold accumulate snow, but NOT in hot zones
    // No altitude snow if temperature > 30°C (too warm for snow to persist)
    // Snow line varies with light level (higher near sub-stellar, lower in twilight)
    // Dark side has very low snow line (permanent ice at any elevation) but less moisture
    let temp_gate = ((30.0 - temperature) / 20.0).clamp(0.0, 1.0); // 1.0 below 10°C, 0.0 above 30°C
    let snow_altitude = if light_level < 0.5 {
        light_level * 0.1 // low snow line on dark side, rising toward terminator
    } else {
        0.05 + (light_level - 0.5) * 0.2 // higher snow line on sun side
    };
    let moisture_availability = (light_level * 3.0).clamp(0.2, 1.0);
    let altitude_snow = if heightmap > snow_altitude {
        ((heightmap - snow_altitude) * 12.0).min(1.0) * temp_gate * moisture_availability
    } else { 0.0 };

    temperature_snow.max(altitude_snow).clamp(0.0, 1.0)
}

/// Topographic Wetness Index — physically grounded measure of how wet a location is.
///
/// TWI = ln(A / tan(slope)), normalized to [0, 1].
/// High values = flat areas with large drainage catchments (valleys, floodplains).
/// Low values = steep slopes with small catchments (ridges, mountain sides).
///
/// - drainage_area: upstream contributing area (number of cells)
/// - slope: local terrain gradient (radians or unitless)
/// Output: [0, 1] where 1.0 = saturated ground, 0.0 = bone dry.
pub fn derive_twi(drainage_area: f64, slope: f64) -> f64 {
    let safe_slope = slope.max(0.001);
    let safe_area = drainage_area.max(1.0);
    (safe_area / safe_slope.tan()).ln().clamp(0.0, 15.0) / 15.0
}

/// Water table depth — combines multiple moisture sources into a single groundwater metric.
///
/// Inputs: river_flow [0,1], humidity [0,1], heightmap (elevation), precipitation_type [-1,1], continentalness.
/// Output: [0, 1] where 1.0 = saturated ground, 0.0 = bone dry.
pub fn derive_water_table(
    river_flow: f64, humidity: f64, heightmap: f64,
    precipitation_type: f64, continentalness: f64,
) -> f64 {
    let humidity_base = humidity * 0.3;
    let river_boost = (river_flow * 4.0).min(1.0) * 0.45;
    let elevation_boost = (1.0 - heightmap.max(0.0) * 2.0).max(0.0) * 0.2;
    let precip_boost = (-precipitation_type).max(0.0) * 0.1;
    let sea_level = -0.01_f64;
    let coastal_boost = (1.0 - (continentalness - sea_level).max(0.0) * 10.0).max(0.0) * 0.1;
    (humidity_base + river_boost + elevation_boost + precip_boost + coastal_boost).clamp(0.0, 1.0)
}

/// Resource richness from geological factors.
///
/// Near plate boundaries = more minerals. Hard rock = ore. Erosion exposes veins.
/// - tectonic: boundary distance [0, 1] where 0 = boundary
/// - rock_hardness: [0, 1]
/// - erosion: [0, 1]
/// Output: [0, 1]
pub fn derive_resource_richness(tectonic: f64, rock_hardness: f64, erosion: f64) -> f64 {
    let boundary = (1.0 - tectonic).powf(1.5);
    (boundary * 0.5 + rock_hardness * 0.3 + erosion * 0.2).clamp(0.0, 1.0)
}

/// Vegetation density from biome type and water table.
///
/// Each biome has base vegetation. Water table boosts it.
/// Output: [0, 1]
pub fn derive_vegetation_density(biome: TileType, water_table: f64) -> f64 {
    let base = match biome {
        TileType::Jungle => 0.95,
        TileType::TemperateRainforest | TileType::SubtropicalForest => 0.85,
        TileType::Forest | TileType::DeciduousForest | TileType::CloudForest => 0.8,
        TileType::Marsh => 0.7,
        TileType::Woodland | TileType::DryWoodland => 0.65,
        TileType::Taiga | TileType::Mangrove => 0.6,
        TileType::Oasis => 0.55,
        TileType::Plains | TileType::Meadow => 0.5,
        TileType::Savanna | TileType::HighlandSavanna => 0.4,
        TileType::AlpineMeadow => 0.35,
        TileType::Steppe | TileType::Thornland => 0.25,
        TileType::Scrubland => 0.2,
        TileType::Plateau => 0.2,
        TileType::Mountain => 0.15,
        TileType::Tundra | TileType::FrozenBog => 0.1,
        TileType::Beach => 0.1,
        TileType::SeaCliff => 0.1,
        TileType::Badlands | TileType::Hamada => 0.08,
        TileType::Desert | TileType::Sahara | TileType::Erg => 0.05,
        TileType::RockyCoast => 0.05,
        TileType::SaltFlat | TileType::ScorchedRock => 0.02,
        TileType::Volcanic | TileType::LavaField | TileType::MoltenWaste => 0.02,
        TileType::Snow | TileType::Glacier | TileType::White | TileType::IceSheet => 0.0,
        TileType::Sea | TileType::ShallowSea | TileType::ContinentalShelf
        | TileType::DeepOcean | TileType::OceanTrench | TileType::OceanRidge
        | TileType::CoralReef | TileType::River => 0.0,
    };
    (base + water_table * 0.3).clamp(0.0, 1.0)
}

/// Soil type from biome, erosion, and rock hardness.
///
/// 0 = bare rock, 0.5 = loam, 1.0 = rich organic.
pub fn derive_soil_type(biome: TileType, erosion: f64, rock_hardness: f64) -> f64 {
    let base = match biome {
        TileType::Forest | TileType::Jungle | TileType::DeciduousForest
        | TileType::TemperateRainforest | TileType::SubtropicalForest
        | TileType::CloudForest => 0.8,
        TileType::Plains | TileType::Marsh | TileType::Meadow | TileType::Oasis => 0.7,
        TileType::Woodland | TileType::DryWoodland => 0.6,
        TileType::Savanna | TileType::HighlandSavanna => 0.5,
        TileType::Taiga | TileType::Mangrove => 0.4,
        TileType::AlpineMeadow => 0.35,
        TileType::Steppe | TileType::Thornland | TileType::Scrubland => 0.3,
        TileType::FrozenBog | TileType::Tundra => 0.2,
        TileType::Beach => 0.15,
        TileType::Desert | TileType::Sahara | TileType::Badlands
        | TileType::Erg | TileType::Hamada => 0.1,
        TileType::SaltFlat | TileType::ScorchedRock => 0.05,
        TileType::RockyCoast | TileType::SeaCliff => 0.05,
        TileType::Mountain | TileType::Glacier | TileType::Plateau => 0.05,
        TileType::Volcanic | TileType::LavaField | TileType::MoltenWaste => 0.08,
        TileType::Snow | TileType::White | TileType::IceSheet => 0.03,
        TileType::Sea | TileType::ShallowSea | TileType::ContinentalShelf
        | TileType::DeepOcean | TileType::OceanTrench | TileType::OceanRidge
        | TileType::CoralReef | TileType::River => 0.0,
    };
    (base + erosion * 0.3 - rock_hardness * 0.2).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_at_sub_stellar() {
        // Full light, flat terrain, moderate humidity, inland
        let temp = derive_temperature(1.0, 0.0, 0.5, 0.2);
        assert!(temp > 80.0, "Sub-stellar temp {} should be very hot", temp);
    }

    #[test]
    fn temperature_at_dark_side() {
        // No light, inland
        let temp = derive_temperature(0.0, 0.0, 0.0, 0.2);
        assert!(temp < -50.0, "Dark side temp {} should be very cold", temp);
    }

    #[test]
    fn temperature_lapse_rate() {
        // Same light but higher elevation = colder, inland
        let low = derive_temperature(0.5, 0.0, 0.0, 0.2);
        let high = derive_temperature(0.5, 0.5, 0.0, 0.2);
        assert!(low > high, "Lower elevation ({}) should be warmer than higher ({})", low, high);
    }

    #[test]
    fn temperature_coastal_moderation() {
        // Coastal (cont=-0.01) vs inland (cont=0.3) at same light level
        let coastal = derive_temperature(0.8, 0.0, 0.5, -0.01);
        let inland = derive_temperature(0.8, 0.0, 0.5, 0.3);
        // Coastal should be more moderate (closer to 15°C)
        assert!((coastal - 15.0).abs() < (inland - 15.0).abs(),
            "Coastal temp ({}) should be more moderate than inland ({})", coastal, inland);
    }

    #[test]
    fn erosion_varies_with_inputs() {
        // High elevation + wet + soft rock = high erosion
        let high_eros = derive_erosion(0.5, 0.0, 0.8);
        // Low elevation + dry + hard rock = low erosion
        let low_eros = derive_erosion(0.0, 1.0, 0.1);
        assert!(high_eros > low_eros, "Steep wet soft ({}) should erode more than flat dry hard ({})", high_eros, low_eros);
    }

    #[test]
    fn erosion_hard_rock_resists() {
        let soft = derive_erosion(0.3, 0.0, 0.5);
        let hard = derive_erosion(0.3, 1.0, 0.5);
        assert!(soft > hard, "Soft rock ({}) should erode more than hard rock ({})", soft, hard);
    }

    #[test]
    fn peaks_amplified_at_boundaries() {
        // At boundary (tectonic=0) vs plate center (tectonic=1)
        let at_boundary = derive_peaks_valleys(0.5, 0.0, 0.5);
        let at_center = derive_peaks_valleys(0.5, 1.0, 0.5);
        assert!(at_boundary > at_center, "Boundary peaks ({}) should be taller than center ({})", at_boundary, at_center);
        assert!(at_boundary > at_center * 5.0, "Boundary/center ratio ({:.1}x) should be > 5x", at_boundary / at_center);
    }

    #[test]
    fn heightmap_ignores_tectonic() {
        // Tectonic no longer affects heightmap (uplift removed to eliminate Voronoi lines)
        let at_boundary = derive_heightmap(0.1, 0.0, 0.3);
        let at_center = derive_heightmap(0.1, 1.0, 0.3);
        assert!((at_boundary - at_center).abs() < f64::EPSILON,
            "Heightmap should be identical regardless of tectonic ({} vs {})", at_boundary, at_center);
    }

    #[test]
    fn aridity_hot_dry() {
        let arid = derive_aridity(80.0, 0.1); // hot + dry
        let lush = derive_aridity(15.0, 0.8); // cool + humid
        assert!(arid > lush, "Hot dry ({}) should be more arid than cool humid ({})", arid, lush);
    }

    #[test]
    fn precipitation_type_cold_is_snow() {
        let precip = derive_precipitation_type(-20.0, 0.5, 0.0);
        assert!(precip < 0.0, "Cold precip ({}) should be snow (negative)", precip);
    }

    #[test]
    fn precipitation_type_dry_is_none() {
        let precip = derive_precipitation_type(20.0, 0.05, 0.0);
        assert!((precip - 1.0).abs() < 0.01, "Very dry precip ({}) should be +1 (no precip)", precip);
    }

    #[test]
    fn snowpack_cold_snowy() {
        let snow = derive_snowpack(-0.5, -20.0, 0.0, 0.3); // snow precip, very cold
        assert!(snow > 0.0, "Cold snowy conditions should produce snowpack ({})", snow);
    }

    #[test]
    fn snowpack_warm_lowland_is_zero() {
        let snow = derive_snowpack(-0.5, 10.0, 0.0, 0.5); // snow precip type but warm, low elevation
        assert!((snow - 0.0).abs() < 0.01, "Warm lowland should have no snowpack ({})", snow);
    }

    #[test]
    fn snowpack_high_altitude() {
        // High peak in moderate light zone should get altitude snow
        let snow = derive_snowpack(0.0, 20.0, 0.5, 0.6);
        assert!(snow > 0.0, "High altitude peak should have snowpack even when warm ({})", snow);
    }

    #[test]
    fn water_table_varies_with_inputs() {
        // River flow boosts water table
        let low = derive_water_table(0.1, 0.3, 0.1, 0.0, 0.1);
        let high = derive_water_table(0.5, 0.3, 0.1, 0.0, 0.1);
        assert!(high > low, "More flow ({}) should give higher water table than less ({})", high, low);
        // Humidity boosts water table
        let dry = derive_water_table(0.0, 0.1, 0.1, 0.0, 0.1);
        let wet = derive_water_table(0.0, 0.9, 0.1, 0.0, 0.1);
        assert!(wet > dry, "Higher humidity ({}) should give higher water table than lower ({})", wet, dry);
    }

    #[test]
    fn resource_richness_at_boundaries() {
        let boundary = derive_resource_richness(0.0, 0.5, 0.5); // at plate boundary
        let center = derive_resource_richness(1.0, 0.5, 0.5);   // plate center
        assert!(boundary > center, "Boundary resources ({}) should exceed center ({})", boundary, center);
    }

    #[test]
    fn vegetation_forest_high() {
        let forest = derive_vegetation_density(TileType::Forest, 0.0);
        let desert = derive_vegetation_density(TileType::Desert, 0.0);
        assert!(forest > desert, "Forest veg ({}) should exceed desert ({})", forest, desert);
    }

    #[test]
    fn vegetation_river_boost() {
        let dry = derive_vegetation_density(TileType::Plains, 0.0);
        let wet = derive_vegetation_density(TileType::Plains, 1.0);
        assert!(wet > dry, "River moisture should boost vegetation ({} > {})", wet, dry);
    }

    #[test]
    fn soil_type_range() {
        let forest = derive_soil_type(TileType::Forest, 0.5, 0.3);
        let mountain = derive_soil_type(TileType::Mountain, 0.1, 0.9);
        assert!(forest > mountain, "Forest soil ({}) should be richer than mountain ({})", forest, mountain);
    }
}
