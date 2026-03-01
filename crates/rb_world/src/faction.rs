//! Political factions for procedural civilization generation.
//!
//! Factions are political entities that control settlements and territories.

use crate::culture::CultureType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A political faction controlling settlements and territory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Faction {
    /// Unique identifier.
    pub id: u32,
    /// Faction name.
    pub name: String,
    /// Culture type this faction belongs to.
    pub culture: CultureType,
    /// Display color (RGBA).
    pub color: [u8; 4],
    /// Capital city ID (if any).
    pub capital_id: Option<u32>,
    /// All settlement IDs belonging to this faction.
    pub settlement_ids: Vec<u32>,
    /// Relationships with other factions: faction_id -> relationship value.
    /// Range: -1.0 (hostile) to 1.0 (allied).
    pub relations: HashMap<u32, f64>,
    /// Faction disposition affecting behavior.
    pub disposition: FactionDisposition,
}

impl Faction {
    /// Create a new faction with default disposition.
    pub fn new(id: u32, name: String, culture: CultureType) -> Self {
        Self {
            id,
            name,
            culture,
            color: culture.default_color(),
            capital_id: None,
            settlement_ids: Vec::new(),
            relations: HashMap::new(),
            disposition: FactionDisposition::default(),
        }
    }

    /// Add a settlement to this faction.
    pub fn add_settlement(&mut self, city_id: u32) {
        if !self.settlement_ids.contains(&city_id) {
            self.settlement_ids.push(city_id);
        }
    }

    /// Set the capital city.
    pub fn set_capital(&mut self, city_id: u32) {
        self.capital_id = Some(city_id);
        self.add_settlement(city_id);
    }

    /// Get relationship with another faction.
    pub fn get_relation(&self, other_faction_id: u32) -> f64 {
        self.relations.get(&other_faction_id).copied().unwrap_or(0.0)
    }

    /// Set relationship with another faction.
    pub fn set_relation(&mut self, other_faction_id: u32, value: f64) {
        self.relations.insert(other_faction_id, value.clamp(-1.0, 1.0));
    }

    /// Check if this faction is hostile to another.
    pub fn is_hostile_to(&self, other_faction_id: u32) -> bool {
        self.get_relation(other_faction_id) < -0.3
    }

    /// Check if this faction is allied with another.
    pub fn is_allied_with(&self, other_faction_id: u32) -> bool {
        self.get_relation(other_faction_id) > 0.5
    }

    /// Get the number of settlements.
    pub fn settlement_count(&self) -> usize {
        self.settlement_ids.len()
    }
}

/// Faction disposition affecting behavior and expansion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactionDisposition {
    /// How aggressive the faction is (0.0 = peaceful, 1.0 = warlike).
    pub aggressiveness: f64,
    /// How open to trade (0.0 = isolationist, 1.0 = merchant).
    pub trade_openness: f64,
    /// How isolated/insular (0.0 = expansionist, 1.0 = isolationist).
    pub isolationism: f64,
}

impl Default for FactionDisposition {
    fn default() -> Self {
        Self {
            aggressiveness: 0.5,
            trade_openness: 0.5,
            isolationism: 0.3,
        }
    }
}

impl FactionDisposition {
    /// Generate a random disposition based on culture type and seed.
    pub fn from_culture_and_seed(culture: CultureType, seed: u32) -> Self {
        // Use seed to create pseudo-random but deterministic values
        let hash = seed.wrapping_mul(2654435761);
        let r1 = ((hash >> 0) & 0xFF) as f64 / 255.0;
        let r2 = ((hash >> 8) & 0xFF) as f64 / 255.0;
        let r3 = ((hash >> 16) & 0xFF) as f64 / 255.0;

        // Base values from culture, with random variation
        let (base_agg, base_trade, base_iso) = match culture {
            CultureType::TwilightDweller => (0.4, 0.7, 0.2),
            CultureType::FrostKin => (0.6, 0.3, 0.6),
            CultureType::SunForged => (0.5, 0.5, 0.4),
            CultureType::TideWalker => (0.3, 0.9, 0.1),
            CultureType::StoneBorn => (0.5, 0.4, 0.5),
        };

        // Add variation of ±0.2
        Self {
            aggressiveness: (base_agg + (r1 - 0.5) * 0.4).clamp(0.0, 1.0),
            trade_openness: (base_trade + (r2 - 0.5) * 0.4).clamp(0.0, 1.0),
            isolationism: (base_iso + (r3 - 0.5) * 0.4).clamp(0.0, 1.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_faction_has_no_settlements() {
        let faction = Faction::new(1, "Test".into(), CultureType::TwilightDweller);
        assert_eq!(faction.settlement_count(), 0);
        assert!(faction.capital_id.is_none());
    }

    #[test]
    fn set_capital_adds_settlement() {
        let mut faction = Faction::new(1, "Test".into(), CultureType::TwilightDweller);
        faction.set_capital(42);
        assert_eq!(faction.capital_id, Some(42));
        assert!(faction.settlement_ids.contains(&42));
    }

    #[test]
    fn relations_default_to_neutral() {
        let faction = Faction::new(1, "Test".into(), CultureType::TwilightDweller);
        assert_eq!(faction.get_relation(999), 0.0);
    }

    #[test]
    fn disposition_from_seed_is_deterministic() {
        let d1 = FactionDisposition::from_culture_and_seed(CultureType::FrostKin, 42);
        let d2 = FactionDisposition::from_culture_and_seed(CultureType::FrostKin, 42);
        assert_eq!(d1.aggressiveness, d2.aggressiveness);
    }
}
