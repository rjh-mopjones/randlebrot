use rb_core::TileType;

/// Calculate the travel/trade cost for traversing a tile.
///
/// # Arguments
/// * `biome` - The biome type at this location
/// * `erosion` - Erosion value (0 to 1) - eroded terrain is easier to traverse
///
/// # Returns
/// Travel cost where:
/// - 1.0 = easy (plains, roads)
/// - Higher values = harder
/// - f64::INFINITY = impassable
pub fn calculate_trade_cost(biome: TileType, erosion: f64) -> f64 {
    let base_cost = match biome {
        TileType::Sea | TileType::White => f64::INFINITY, // Impassable by land
        TileType::Mountain => 10.0,
        TileType::Plateau => 6.0,
        TileType::Snow => 8.0,
        TileType::Desert => 5.0,
        TileType::Sahara => 6.0,
        TileType::Forest => 3.0,
        TileType::Beach => 1.5,
        TileType::Plains => 1.0,
    };

    if base_cost.is_infinite() {
        return base_cost;
    }

    // Eroded terrain is slightly easier to traverse (natural paths, worn ground)
    let erosion_modifier = 1.0 - erosion * 0.2;

    base_cost * erosion_modifier
}

/// Calculate trade cost without erosion data.
pub fn calculate_trade_cost_simple(biome: TileType) -> f64 {
    calculate_trade_cost(biome, 0.0)
}

/// Check if a tile is passable for land travel.
pub fn is_passable(biome: TileType) -> bool {
    !matches!(biome, TileType::Sea | TileType::White)
}

/// Get terrain difficulty rating as a descriptive string.
pub fn terrain_difficulty(biome: TileType) -> &'static str {
    match biome {
        TileType::Plains => "Easy",
        TileType::Beach => "Easy",
        TileType::Forest => "Moderate",
        TileType::Desert => "Hard",
        TileType::Sahara => "Hard",
        TileType::Plateau => "Difficult",
        TileType::Snow => "Difficult",
        TileType::Mountain => "Very Difficult",
        TileType::Sea | TileType::White => "Impassable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn water_is_impassable() {
        assert!(calculate_trade_cost(TileType::Sea, 0.0).is_infinite());
        assert!(calculate_trade_cost(TileType::White, 0.0).is_infinite());
    }

    #[test]
    fn plains_are_cheapest() {
        let plains = calculate_trade_cost(TileType::Plains, 0.0);
        let forest = calculate_trade_cost(TileType::Forest, 0.0);
        let mountain = calculate_trade_cost(TileType::Mountain, 0.0);

        assert!(plains < forest);
        assert!(forest < mountain);
    }

    #[test]
    fn erosion_reduces_cost() {
        let no_erosion = calculate_trade_cost(TileType::Forest, 0.0);
        let eroded = calculate_trade_cost(TileType::Forest, 1.0);

        assert!(eroded < no_erosion);
    }

    #[test]
    fn is_passable_works() {
        assert!(is_passable(TileType::Plains));
        assert!(is_passable(TileType::Mountain));
        assert!(!is_passable(TileType::Sea));
        assert!(!is_passable(TileType::White));
    }
}
