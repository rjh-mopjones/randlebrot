use rb_core::TileType;

/// Calculate the political/settlement suitability score for a location.
///
/// # Arguments
/// * `biome` - The biome type at this location
/// * `temperature` - Temperature value (-100 to 100)
/// * `humidity` - Humidity value (0 to 1)
/// * `continentalness` - Continentalness value (-1 to 1)
/// * `water_distance` - Normalized distance to nearest water (0 = on water, 1 = far)
/// * `nearby_mountain_factor` - How many mountains are nearby (0 = none, 1 = many)
/// * `resource_value` - Value of resources at this location (0 to 1)
///
/// # Returns
/// Settlement suitability score in [0, 1] where 1 = ideal location
pub fn calculate_political_score(
    biome: TileType,
    temperature: f64,
    humidity: f64,
    continentalness: f64,
    water_distance: f64,
    nearby_mountain_factor: f64,
    resource_value: f64,
) -> f64 {
    // Can't settle in water
    if matches!(biome, TileType::Sea | TileType::White) {
        return 0.0;
    }

    // Fertility score (35%) - based on biome
    let fertility = match biome {
        TileType::Plains => 1.0,
        TileType::Forest => 0.7,
        TileType::Beach => 0.5,
        TileType::Plateau => 0.3,
        TileType::Desert => 0.2,
        TileType::Sahara => 0.1,
        TileType::Mountain => 0.15,
        TileType::Snow => 0.2,
        TileType::Sea | TileType::White => 0.0,
    };

    // Climate comfort (25%) - prefer 10-30°C, humidity 0.3-0.7
    let temp_comfort = 1.0 - ((temperature - 20.0) / 50.0).abs().min(1.0);
    let humid_comfort = 1.0 - ((humidity - 0.5) / 0.5).abs().min(1.0);
    let climate_score = temp_comfort * 0.6 + humid_comfort * 0.4;

    // Water access (20%) - closer to water is better (but not in water)
    let water_score = if continentalness < -0.025 {
        0.0 // In water
    } else {
        (1.0 - water_distance).max(0.0)
    };

    // Defensibility (10%) - nearby mountains help defense
    let defense_score = nearby_mountain_factor;

    // Resources (10%)
    let resource_score = resource_value;

    // Combine scores
    let score = 0.35 * fertility
        + 0.25 * climate_score
        + 0.20 * water_score
        + 0.10 * defense_score
        + 0.10 * resource_score;

    score.clamp(0.0, 1.0)
}

/// Simplified political score calculation using just biome map data.
/// Used when full context isn't available.
pub fn calculate_political_score_simple(
    biome: TileType,
    temperature: f64,
    humidity: f64,
) -> f64 {
    // Can't settle in water
    if matches!(biome, TileType::Sea | TileType::White) {
        return 0.0;
    }

    // Fertility score (50%)
    let fertility = match biome {
        TileType::Plains => 1.0,
        TileType::Forest => 0.7,
        TileType::Beach => 0.6,
        TileType::Plateau => 0.3,
        TileType::Desert => 0.2,
        TileType::Sahara => 0.1,
        TileType::Mountain => 0.15,
        TileType::Snow => 0.2,
        TileType::Sea | TileType::White => 0.0,
    };

    // Climate comfort (50%)
    let temp_comfort = 1.0 - ((temperature - 20.0) / 50.0).abs().min(1.0);
    let humid_comfort = 1.0 - ((humidity - 0.5) / 0.5).abs().min(1.0);
    let climate_score = temp_comfort * 0.6 + humid_comfort * 0.4;

    (0.5 * fertility + 0.5 * climate_score).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_has_zero_score() {
        let score = calculate_political_score(
            TileType::Sea,
            25.0,
            0.5,
            -0.5,
            0.0,
            0.0,
            0.0,
        );
        assert_eq!(score, 0.0);
    }

    #[test]
    fn plains_are_good_for_settlement() {
        let plains = calculate_political_score(
            TileType::Plains,
            25.0,
            0.5,
            0.1,
            0.2,
            0.3,
            0.2,
        );
        let mountain = calculate_political_score(
            TileType::Mountain,
            25.0,
            0.5,
            0.3,
            0.5,
            0.8,
            0.5,
        );

        assert!(plains > mountain, "Plains ({}) should be better than mountains ({})", plains, mountain);
    }

    #[test]
    fn extreme_temperature_reduces_score() {
        let comfortable = calculate_political_score_simple(TileType::Plains, 25.0, 0.5);
        let hot = calculate_political_score_simple(TileType::Plains, 80.0, 0.5);
        let cold = calculate_political_score_simple(TileType::Plains, -30.0, 0.5);

        assert!(comfortable > hot);
        assert!(comfortable > cold);
    }
}
