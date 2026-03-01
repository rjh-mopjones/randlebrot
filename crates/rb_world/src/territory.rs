//! Territory system for faction land ownership.
//!
//! Territories are generated via flood-fill expansion from settlements,
//! with natural boundaries at mountains, coasts, and other terrain features.

use rb_core::TileType;
use serde::{Deserialize, Serialize};

/// Influence decay rate for different terrain types.
/// Lower values mean terrain acts as a stronger boundary.
pub fn terrain_influence_decay(biome: TileType) -> f64 {
    match biome {
        TileType::Sea | TileType::White => 0.0, // Complete barrier
        TileType::Mountain => 0.3,               // Strong barrier
        TileType::Plateau => 0.5,
        TileType::Snow => 0.6,
        TileType::Desert | TileType::Sahara => 0.7,
        TileType::Forest => 0.8,
        TileType::Beach => 0.9,
        TileType::Plains => 0.95, // Easy expansion
    }
}

/// Map of faction territory ownership.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerritoryMap {
    pub width: usize,
    pub height: usize,
    /// Faction ID for each pixel (0 = unclaimed).
    pub ownership: Vec<u32>,
    /// Influence strength [0.0, 1.0] for each pixel.
    pub influence: Vec<f64>,
}

impl TerritoryMap {
    /// Create a new empty territory map.
    pub fn new(width: usize, height: usize) -> Self {
        let size = width * height;
        Self {
            width,
            height,
            ownership: vec![0; size],
            influence: vec![0.0; size],
        }
    }

    /// Get the index for a position.
    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.width + x
    }

    /// Check if coordinates are in bounds.
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height
    }

    /// Get ownership at a position.
    pub fn get_owner(&self, x: usize, y: usize) -> u32 {
        if x < self.width && y < self.height {
            self.ownership[self.idx(x, y)]
        } else {
            0
        }
    }

    /// Get influence at a position.
    pub fn get_influence(&self, x: usize, y: usize) -> f64 {
        if x < self.width && y < self.height {
            self.influence[self.idx(x, y)]
        } else {
            0.0
        }
    }

    /// Set ownership and influence at a position.
    pub fn set(&mut self, x: usize, y: usize, faction_id: u32, influence: f64) {
        if x < self.width && y < self.height {
            let idx = self.idx(x, y);
            self.ownership[idx] = faction_id;
            self.influence[idx] = influence;
        }
    }

    /// Check if a position is claimed by any faction.
    pub fn is_claimed(&self, x: usize, y: usize) -> bool {
        self.get_owner(x, y) != 0
    }

    /// Get neighbors of a position (4-connected).
    pub fn neighbors(&self, x: usize, y: usize) -> Vec<(usize, usize)> {
        let mut result = Vec::with_capacity(4);
        if x > 0 {
            result.push((x - 1, y));
        }
        if x + 1 < self.width {
            result.push((x + 1, y));
        }
        if y > 0 {
            result.push((x, y - 1));
        }
        if y + 1 < self.height {
            result.push((x, y + 1));
        }
        result
    }

    /// Convert territory data to RGBA image bytes for visualization.
    /// Uses faction colors with influence-based alpha.
    pub fn to_image(&self, faction_colors: &[(u32, [u8; 4])]) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.width * self.height * 4);

        for y in 0..self.height {
            for x in 0..self.width {
                let idx = self.idx(x, y);
                let faction_id = self.ownership[idx];
                let influence = self.influence[idx];

                let color = if faction_id == 0 {
                    [0, 0, 0, 0] // Transparent for unclaimed
                } else {
                    // Find faction color
                    faction_colors
                        .iter()
                        .find(|(id, _)| *id == faction_id)
                        .map(|(_, color)| {
                            let [r, g, b, base_a] = *color;
                            let alpha = (influence * base_a as f64) as u8;
                            [r, g, b, alpha]
                        })
                        .unwrap_or([128, 128, 128, (influence * 128.0) as u8])
                };

                data.extend_from_slice(&color);
            }
        }

        data
    }

    /// Count pixels owned by each faction.
    pub fn count_by_faction(&self) -> std::collections::HashMap<u32, usize> {
        let mut counts = std::collections::HashMap::new();
        for &faction_id in &self.ownership {
            if faction_id != 0 {
                *counts.entry(faction_id).or_insert(0) += 1;
            }
        }
        counts
    }

    /// Get total claimed area (non-zero ownership).
    pub fn total_claimed_area(&self) -> usize {
        self.ownership.iter().filter(|&&id| id != 0).count()
    }
}

/// Types of natural boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryType {
    MountainRange,
    Coastline,
    River, // Future - from moisture noise
    ForestEdge,
    DesertEdge,
}

/// A natural boundary line.
#[derive(Debug, Clone)]
pub struct Boundary {
    pub boundary_type: BoundaryType,
    pub points: Vec<(usize, usize)>,
    /// How much this boundary impedes expansion [0.0, 1.0].
    pub strength: f64,
}

impl Boundary {
    /// Create a new boundary.
    pub fn new(boundary_type: BoundaryType, strength: f64) -> Self {
        Self {
            boundary_type,
            points: Vec::new(),
            strength,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_territory_is_unclaimed() {
        let map = TerritoryMap::new(10, 10);
        assert_eq!(map.get_owner(5, 5), 0);
        assert_eq!(map.get_influence(5, 5), 0.0);
    }

    #[test]
    fn set_and_get_territory() {
        let mut map = TerritoryMap::new(10, 10);
        map.set(5, 5, 1, 0.8);
        assert_eq!(map.get_owner(5, 5), 1);
        assert_eq!(map.get_influence(5, 5), 0.8);
    }

    #[test]
    fn neighbors_in_center() {
        let map = TerritoryMap::new(10, 10);
        let neighbors = map.neighbors(5, 5);
        assert_eq!(neighbors.len(), 4);
    }

    #[test]
    fn neighbors_in_corner() {
        let map = TerritoryMap::new(10, 10);
        let neighbors = map.neighbors(0, 0);
        assert_eq!(neighbors.len(), 2);
    }

    #[test]
    fn sea_blocks_expansion() {
        assert_eq!(terrain_influence_decay(TileType::Sea), 0.0);
    }

    #[test]
    fn plains_allow_expansion() {
        assert!(terrain_influence_decay(TileType::Plains) > 0.9);
    }
}
