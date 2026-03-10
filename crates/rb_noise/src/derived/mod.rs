use rb_core::TileType;

/// Temperature derived from light level + elevation + humidity + continentalness.
///
/// Output range: ~[-80, +150]°C (matches existing BiomeSplines expectations).
/// - light_level: [0, 1] where 1.0 = sub-stellar point
/// - elevation: heightmap value (can be negative for ocean)
/// - humidity: [0, 1] where 1.0 = saturated
/// - continentalness: distance from coast (negative = ocean, positive = inland)
pub fn derive_temperature(light_level: f64, elevation: f64, humidity: f64, continentalness: f64) -> f64 {
    // Map light [0,1] to temp [-80, +150]
    let base_temp = light_level * 230.0 - 80.0;
    // Lapse rate: mountains are colder (only for positive elevation)
    let lapse_rate = elevation.max(0.0) * 60.0;
    // Moisture moderates extremes slightly
    let humidity_buffer = humidity * 5.0;
    let raw = base_temp - lapse_rate + humidity_buffer;
    // Coastal moderation: ocean proximity pulls temperature toward moderate
    let inland_factor = ((continentalness + 0.025).max(0.0) * 5.0).clamp(0.0, 1.0);
    let moderate_temp = 15.0;
    raw + (moderate_temp - raw) * (1.0 - inland_factor) * 0.3
}

/// Heightmap from geological layers (used as elevation input for temperature).
///
/// Combines continentalness + tectonic boundary effects + peaks into unified elevation.
pub fn derive_heightmap(continentalness: f64, tectonic: f64, peaks_valleys: f64) -> f64 {
    // Tectonic boundaries (low tectonic value) push up elevation slightly
    let tectonic_uplift = (1.0 - tectonic) * 0.05;
    // Peaks add height, valleys subtract
    let peak_contribution = peaks_valleys * 0.15;
    continentalness + tectonic_uplift + peak_contribution
}

/// Erosion derived from heightmap, rock hardness, and humidity.
///
/// Steep terrain erodes more, wet regions erode faster, hard rock resists.
/// - heightmap: unified elevation value
/// - rock_hardness: [0, 1] where 1.0 = very hard
/// - humidity: [0, 1] where 1.0 = saturated
pub fn derive_erosion(heightmap: f64, rock_hardness: f64, humidity: f64) -> f64 {
    ((heightmap.max(0.0) * 0.5 + humidity * 0.5) * (1.0 - rock_hardness * 0.6)).clamp(0.0, 1.0)
}

/// Peaks amplified by tectonic stress, sustained by hard rock.
///
/// - base_pv: raw peaks/valleys noise [-1, 1]
/// - tectonic: boundary distance [0, 1] where 0 = boundary
/// - rock_hardness: [0, 1] where 1.0 = very hard
pub fn derive_peaks_valleys(base_pv: f64, tectonic: f64, rock_hardness: f64) -> f64 {
    // Near tectonic boundaries = taller peaks
    let tectonic_amp = 1.0 + (1.0 - tectonic) * 0.5;
    // Hard rock sustains sharper peaks
    let hardness_sustain = 0.7 + rock_hardness * 0.3;
    (base_pv * tectonic_amp * hardness_sustain).clamp(-1.0, 1.0)
}

/// Aridity from temperature and humidity.
///
/// This is where the tidal-lock drying effect lives. High temperature + low humidity = arid.
/// - temperature: degrees C
/// - humidity: [0, 1]
/// Output: [0, 1] where 1.0 = hyper-arid
pub fn derive_aridity(temperature: f64, humidity: f64) -> f64 {
    let temp_factor = ((temperature - 10.0) / 80.0).clamp(0.0, 1.0);
    (temp_factor * 0.5 + (1.0 - humidity) * 0.5).clamp(0.0, 1.0)
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

    // Altitude snow: peaks above threshold accumulate snow even in warmer zones
    // Snow line varies with light level (higher near sub-stellar, lower in twilight)
    let snow_altitude = if light_level < 0.2 { 0.0 }
        else if light_level < 0.5 { 0.12 + (light_level - 0.2) * 0.6 }
        else { 0.30 + (light_level - 0.5) * 0.4 };
    let altitude_snow = if heightmap > snow_altitude {
        ((heightmap - snow_altitude) * 5.0).min(1.0)
    } else { 0.0 };

    temperature_snow.max(altitude_snow).clamp(0.0, 1.0)
}

/// River moisture — how much moisture rivers contribute to surrounding area.
///
/// Simple amplification of flow.
/// - river_flow: [0, 1]
/// Output: [0, 1]
pub fn derive_river_moisture(river_flow: f64) -> f64 {
    (river_flow * 3.0).min(1.0)
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

/// Vegetation density from biome type and river moisture.
///
/// Each biome has base vegetation. River moisture boosts it.
/// Output: [0, 1]
pub fn derive_vegetation_density(biome: TileType, river_moisture: f64) -> f64 {
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
    (base + river_moisture * 0.3).clamp(0.0, 1.0)
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
    }

    #[test]
    fn heightmap_includes_tectonic_uplift() {
        // At tectonic boundary (tectonic=0) vs center (tectonic=1)
        let at_boundary = derive_heightmap(0.1, 0.0, 0.0);
        let at_center = derive_heightmap(0.1, 1.0, 0.0);
        assert!(at_boundary > at_center, "Boundary ({}) should have more uplift than center ({})", at_boundary, at_center);
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
    fn river_moisture_amplifies() {
        let low = derive_river_moisture(0.1);
        let high = derive_river_moisture(0.5);
        assert!(high > low, "More flow ({}) should give more moisture than less ({})", high, low);
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
