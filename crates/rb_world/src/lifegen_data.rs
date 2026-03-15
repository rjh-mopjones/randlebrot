use bevy::prelude::*;
use rb_core::TileType;
use rb_noise::BiomeMap;

use crate::definition::{CityTier, WorldDefinition};
use crate::roads::{terrain_movement_cost, RoadType};

/// Complete output of civilisation generation.
/// All grid layers are at 8192x4096 meso pixel resolution.
/// Immutable after world gen (will support mutation via political tick later).
#[derive(Resource, Debug)]
pub struct LifeGenData {
    pub width: usize,
    pub height: usize,

    // Analysis grids (continuous, derived from terrain)
    pub habitability: Vec<f32>,
    pub navigation_cost: Vec<f32>,
    pub resource_desirability: Vec<f32>,

    // Province layer
    pub province_ids: Vec<u16>,
    pub provinces: Vec<Province>,

    // Faction layer
    pub faction_ids: Vec<u32>,
    pub factions: Vec<FactionData>,

    // Settlement layer
    pub settlement_seeds: Vec<SettlementSeed>,

    // Road network
    pub road_segments: Vec<RoadSegment>,
}

#[derive(Debug, Clone)]
pub struct Province {
    pub id: u16,
    pub site: (f64, f64),
    pub biome: TileType,
    pub habitability: f32,
    pub area_px: u32,
    pub is_coastal: bool,
    pub is_river_junction: bool,
    pub elevation_mean: f32,
    pub terrain_cost: f32,
    pub political_state: PoliticalState,
}

#[derive(Debug, Clone)]
pub enum PoliticalState {
    Claimed { faction_id: u32 },
    Unclaimed,
    Uninhabited,
}

#[derive(Debug, Clone)]
pub struct FactionData {
    pub id: u32,
    pub name: String,
    pub colour: [u8; 4],
    pub capital_province: u16,
}

#[derive(Debug, Clone)]
pub struct SettlementSeed {
    pub id: u32,
    pub position: (f64, f64),
    pub province_id: u16,
    pub size_class: SizeClass,
    pub tier: SettlementTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeClass {
    Metropolis,
    City,
    Town,
    Village,
    Outpost,
    Ruins,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementTier {
    Capital,
    Major,
    Minor,
    Outpost,
    Camp,
    Ruins,
}

#[derive(Debug, Clone)]
pub struct RoadSegment {
    pub from: (f64, f64),
    pub to: (f64, f64),
    pub cost: f32,
    /// Road type: 0=Imperial, 1=Provincial, 2=Trail.
    pub road_type_id: u8,
}

impl SizeClass {
    pub fn name(&self) -> &'static str {
        match self {
            SizeClass::Metropolis => "Metropolis",
            SizeClass::City => "City",
            SizeClass::Town => "Town",
            SizeClass::Village => "Village",
            SizeClass::Outpost => "Outpost",
            SizeClass::Ruins => "Ruins",
        }
    }
}

impl SettlementTier {
    pub fn name(&self) -> &'static str {
        match self {
            SettlementTier::Capital => "Capital",
            SettlementTier::Major => "Major",
            SettlementTier::Minor => "Minor",
            SettlementTier::Outpost => "Outpost",
            SettlementTier::Camp => "Camp",
            SettlementTier::Ruins => "Ruins",
        }
    }
}

impl PoliticalState {
    pub fn name(&self) -> &'static str {
        match self {
            PoliticalState::Claimed { .. } => "Claimed",
            PoliticalState::Unclaimed => "Unclaimed",
            PoliticalState::Uninhabited => "Uninhabited",
        }
    }
}

impl LifeGenData {
    /// Look up province ID at a lifegen pixel coordinate. Returns None for out-of-bounds or ID 0.
    pub fn province_at_pixel(&self, px: usize, py: usize) -> Option<u16> {
        if px >= self.width || py >= self.height {
            return None;
        }
        let idx = py * self.width + px;
        let id = *self.province_ids.get(idx)?;
        if id == 0 { None } else { Some(id) }
    }

    /// Get a province by its 1-based ID.
    pub fn province_by_id(&self, id: u16) -> Option<&Province> {
        self.provinces.iter().find(|p| p.id == id)
    }

    /// Find the nearest settlement within `max_radius` lifegen pixels of (px, py).
    pub fn nearest_settlement(&self, px: f64, py: f64, max_radius: f64) -> Option<&SettlementSeed> {
        let max_r2 = max_radius * max_radius;
        let mut best: Option<(&SettlementSeed, f64)> = None;
        for s in &self.settlement_seeds {
            let dx = s.position.0 - px;
            let dy = s.position.1 - py;
            let d2 = dx * dx + dy * dy;
            if d2 <= max_r2 {
                if best.map_or(true, |(_, bd)| d2 < bd) {
                    best = Some((s, d2));
                }
            }
        }
        best.map(|(s, _)| s)
    }

    /// Find a faction by its ID.
    pub fn faction_by_id(&self, id: u32) -> Option<&FactionData> {
        self.factions.iter().find(|f| f.id == id)
    }

    /// Create an empty LifeGenData at the given resolution.
    pub fn empty(width: usize, height: usize) -> Self {
        let total = width * height;
        Self {
            width,
            height,
            habitability: vec![0.0; total],
            navigation_cost: vec![1.0; total],
            resource_desirability: vec![0.0; total],
            province_ids: vec![0; total],
            provinces: Vec::new(),
            faction_ids: Vec::new(),
            factions: Vec::new(),
            settlement_seeds: Vec::new(),
            road_segments: Vec::new(),
        }
    }

    /// Build LifeGenData from a completed civilization generation.
    ///
    /// `biome_map` is the macro-level BiomeMap (1024x512).
    /// `world_def` must already contain cities, factions, roads, and territory_cache
    /// (populated by `CivilizationGenerator::generate()`).
    pub fn from_world_and_terrain(biome_map: &BiomeMap, world_def: &WorldDefinition) -> Self {
        let width = biome_map.width;
        let height = biome_map.height;
        let total = width * height;

        // Analysis grids
        let mut habitability = vec![0.0f32; total];
        let mut navigation_cost = vec![1.0f32; total];
        let mut resource_desirability = vec![0.0f32; total];

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                if let Some(biome) = biome_map.get_biome(x, y) {
                    let move_cost = terrain_movement_cost(biome);
                    navigation_cost[idx] = if move_cost.is_finite() {
                        (move_cost / 10.0) as f32 // Normalize to ~[0,1]
                    } else {
                        1.0
                    };

                    // Habitability: inverse of movement cost, zero for impassable
                    habitability[idx] = if move_cost.is_finite() && move_cost < 50.0 {
                        (1.0 - move_cost / 50.0).max(0.0) as f32
                    } else {
                        0.0
                    };

                    // Resource desirability from continentalness
                    if let Some(cont) = biome_map.get_continentalness(x, y) {
                        resource_desirability[idx] = (cont.abs() * 0.5 + 0.25).min(1.0) as f32;
                    }
                }
            }
        }

        // Faction territory grid
        let mut faction_ids = vec![0u32; total];
        if let Some(ref territory) = world_def.territory_cache {
            for y in 0..height.min(territory.height) {
                for x in 0..width.min(territory.width) {
                    faction_ids[y * width + x] = territory.get_owner(x, y);
                }
            }
        }

        // Convert factions to FactionData
        let factions: Vec<FactionData> = world_def
            .factions
            .iter()
            .map(|f| {
                let h = f.id.wrapping_mul(2654435761);
                FactionData {
                    id: f.id,
                    name: f.name.clone(),
                    colour: [
                        ((h >> 0) & 0xFF) as u8 | 0x40,
                        ((h >> 8) & 0xFF) as u8 | 0x40,
                        ((h >> 16) & 0xFF) as u8 | 0x40,
                        255,
                    ],
                    capital_province: 0, // No province system yet
                }
            })
            .collect();

        // Convert cities to SettlementSeeds
        let settlement_seeds: Vec<SettlementSeed> = world_def
            .cities
            .iter()
            .map(|c| SettlementSeed {
                id: c.id,
                position: (c.position.x, c.position.y),
                province_id: 0, // No province system yet
                size_class: match c.tier {
                    CityTier::Capital => SizeClass::City,
                    CityTier::Town => SizeClass::Town,
                    CityTier::Village => SizeClass::Village,
                },
                tier: match c.tier {
                    CityTier::Capital => SettlementTier::Capital,
                    CityTier::Town => SettlementTier::Major,
                    CityTier::Village => SettlementTier::Minor,
                },
            })
            .collect();

        // Convert roads to RoadSegments
        let mut road_segments = Vec::new();
        for road in &world_def.roads {
            let type_id = match road.road_type {
                RoadType::Imperial => 0,
                RoadType::Provincial => 1,
                RoadType::Trail => 2,
            };
            for pair in road.waypoints.windows(2) {
                road_segments.push(RoadSegment {
                    from: (pair[0].x, pair[0].y),
                    to: (pair[1].x, pair[1].y),
                    cost: 1.0,
                    road_type_id: type_id,
                });
            }
        }

        Self {
            width,
            height,
            habitability,
            navigation_cost,
            resource_desirability,
            province_ids: vec![0; total], // Province system not yet implemented
            provinces: Vec::new(),
            faction_ids,
            factions,
            settlement_seeds,
            road_segments,
        }
    }

    /// Save debug layer PNGs to debug_layers/lifegen/.
    /// Each layer is rendered at full resolution then downscaled 2x.
    pub fn save_debug_layers(&self, base_path: &std::path::Path) {
        let lifegen_dir = base_path.join("lifegen");
        std::fs::create_dir_all(&lifegen_dir).ok();

        // Habitability: green (high) to red (low)
        save_gradient_layer(
            &lifegen_dir,
            "habitability.png",
            &self.habitability,
            self.width,
            self.height,
            [180, 30, 30],
            [30, 180, 30],
        );

        // Navigation cost: green (easy) to red (hard)
        save_gradient_layer(
            &lifegen_dir,
            "navigation_cost.png",
            &self.navigation_cost,
            self.width,
            self.height,
            [30, 180, 30],
            [180, 30, 30],
        );

        // Resource desirability: dark (low) to bright yellow (high)
        save_gradient_layer(
            &lifegen_dir,
            "resource_desirability.png",
            &self.resource_desirability,
            self.width,
            self.height,
            [20, 20, 20],
            [240, 220, 60],
        );

        // Province IDs: random colour per province
        save_indexed_layer(
            &lifegen_dir,
            "provinces.png",
            &self.province_ids,
            self.width,
            self.height,
        );

        // Factions: hash-coloured regions by faction_id
        save_faction_layer(
            &lifegen_dir,
            "factions.png",
            &self.faction_ids,
            self.width,
            self.height,
        );

        // Settlements: dots on dark background
        save_settlement_layer(
            &lifegen_dir,
            "settlements.png",
            &self.settlement_seeds,
            self.width,
            self.height,
        );

        // Roads: line segments on dark background
        save_road_layer(
            &lifegen_dir,
            "roads.png",
            &self.road_segments,
            self.width,
            self.height,
        );

        // Composite: factions at 40% alpha + roads + settlements
        save_composite_layer(
            &lifegen_dir,
            "composite.png",
            &self.faction_ids,
            &self.road_segments,
            &self.settlement_seeds,
            self.width,
            self.height,
        );

        println!("  Saved lifegen debug layers to {}", lifegen_dir.display());
    }

    /// Return RGBA pixel data (width x height, 4 bytes per pixel, row-major, y=0 is top)
    /// for the given layer name. Supported layers:
    /// "habitability", "navigation_cost", "resource_desirability",
    /// "factions", "settlements", "roads", "composite".
    pub fn to_layer_image(&self, layer: &str) -> Vec<u8> {
        let w = self.width;
        let h = self.height;
        let total = w * h * 4;

        match layer {
            "habitability" => {
                // Red (low) to green (high)
                let mut pixels = vec![0u8; total];
                for i in 0..w * h {
                    let t = self.habitability.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                    let off = i * 4;
                    pixels[off] = (180.0 * (1.0 - t) + 30.0 * t) as u8;
                    pixels[off + 1] = (30.0 * (1.0 - t) + 180.0 * t) as u8;
                    pixels[off + 2] = 30;
                    pixels[off + 3] = 255;
                }
                pixels
            }
            "navigation_cost" => {
                // Green (easy/low) to red (hard/high)
                let mut pixels = vec![0u8; total];
                for i in 0..w * h {
                    let t = self.navigation_cost.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                    let off = i * 4;
                    pixels[off] = (30.0 * (1.0 - t) + 180.0 * t) as u8;
                    pixels[off + 1] = (180.0 * (1.0 - t) + 30.0 * t) as u8;
                    pixels[off + 2] = 30;
                    pixels[off + 3] = 255;
                }
                pixels
            }
            "resource_desirability" => {
                // Dark to bright yellow
                let mut pixels = vec![0u8; total];
                for i in 0..w * h {
                    let t = self.resource_desirability.get(i).copied().unwrap_or(0.0).clamp(0.0, 1.0);
                    let off = i * 4;
                    pixels[off] = (20.0 * (1.0 - t) + 240.0 * t) as u8;
                    pixels[off + 1] = (20.0 * (1.0 - t) + 220.0 * t) as u8;
                    pixels[off + 2] = (20.0 * (1.0 - t) + 60.0 * t) as u8;
                    pixels[off + 3] = 255;
                }
                pixels
            }
            "factions" => {
                // Hash-coloured faction territory, alpha 180 for claimed, 0 for unclaimed
                let mut pixels = vec![0u8; total];
                for i in 0..w * h {
                    let fid = self.faction_ids.get(i).copied().unwrap_or(0);
                    let off = i * 4;
                    if fid == 0 {
                        // unclaimed: transparent
                        pixels[off] = 0;
                        pixels[off + 1] = 0;
                        pixels[off + 2] = 0;
                        pixels[off + 3] = 0;
                    } else {
                        let hv = fid.wrapping_mul(2654435761);
                        pixels[off] = ((hv >> 0) & 0xFF) as u8 | 0x40;
                        pixels[off + 1] = ((hv >> 8) & 0xFF) as u8 | 0x40;
                        pixels[off + 2] = ((hv >> 16) & 0xFF) as u8 | 0x40;
                        pixels[off + 3] = 180;
                    }
                }
                pixels
            }
            "settlements" => {
                // Transparent background with settlement dots
                let mut pixels = vec![0u8; total];
                for s in &self.settlement_seeds {
                    let (colour, radius) = settlement_dot_params(s);
                    let cx = s.position.0 as i32;
                    let cy = s.position.1 as i32;
                    draw_circle(&mut pixels, w, h, cx, cy, radius, colour);
                }
                pixels
            }
            "roads" => {
                // Transparent background with road lines
                let mut pixels = vec![0u8; total];
                for seg in &self.road_segments {
                    let (colour, thickness) = road_line_params(seg.road_type_id);
                    let x0 = seg.from.0 as i32;
                    let y0 = seg.from.1 as i32;
                    let x1 = seg.to.0 as i32;
                    let y1 = seg.to.1 as i32;
                    draw_line(&mut pixels, w, h, x0, y0, x1, y1, colour, thickness);
                }
                pixels
            }
            "composite" => {
                // Factions at alpha 120 + roads + settlements, all on transparent background
                let mut pixels = vec![0u8; total];
                // Factions at alpha 120
                for i in 0..w * h {
                    let fid = self.faction_ids.get(i).copied().unwrap_or(0);
                    if fid != 0 {
                        let off = i * 4;
                        let hv = fid.wrapping_mul(2654435761);
                        pixels[off] = ((hv >> 0) & 0xFF) as u8 | 0x40;
                        pixels[off + 1] = ((hv >> 8) & 0xFF) as u8 | 0x40;
                        pixels[off + 2] = ((hv >> 16) & 0xFF) as u8 | 0x40;
                        pixels[off + 3] = 120;
                    }
                }
                // Roads
                for seg in &self.road_segments {
                    let (colour, thickness) = road_line_params(seg.road_type_id);
                    let x0 = seg.from.0 as i32;
                    let y0 = seg.from.1 as i32;
                    let x1 = seg.to.0 as i32;
                    let y1 = seg.to.1 as i32;
                    draw_line(&mut pixels, w, h, x0, y0, x1, y1, colour, thickness);
                }
                // Settlements
                for s in &self.settlement_seeds {
                    let (colour, radius) = settlement_dot_params(s);
                    let cx = s.position.0 as i32;
                    let cy = s.position.1 as i32;
                    draw_circle(&mut pixels, w, h, cx, cy, radius, colour);
                }
                pixels
            }
            _ => {
                // Unknown layer: return transparent
                vec![0u8; total]
            }
        }
    }

    /// Build a composited overlay image from the base layer + enabled overlay flags.
    pub fn to_composited_image(
        &self,
        base_layer: &str,
        province_borders: bool,
        faction_borders: bool,
        settlement_icons: bool,
        road_network: bool,
        _trade_routes: bool,
    ) -> Vec<u8> {
        let mut pixels = self.to_layer_image(base_layer);
        let w = self.width;
        let h = self.height;

        // Province borders: draw where neighboring province IDs differ
        if province_borders && !self.province_ids.is_empty() {
            let border_colour = [255u8, 255, 255, 160];
            for y in 0..h {
                for x in 0..w {
                    let idx = y * w + x;
                    let pid = self.province_ids[idx];
                    if pid == 0 { continue; }
                    let is_border = (x + 1 < w && self.province_ids[idx + 1] != pid)
                        || (y + 1 < h && self.province_ids[idx + w] != pid)
                        || (x > 0 && self.province_ids[idx - 1] != pid)
                        || (y > 0 && self.province_ids[idx - w] != pid);
                    if is_border {
                        let off = idx * 4;
                        alpha_blend(&mut pixels, off, border_colour);
                    }
                }
            }
        }

        // Faction borders: draw where neighboring faction IDs differ (thicker)
        if faction_borders && !self.faction_ids.is_empty() {
            let border_colour = [255u8, 200, 0, 200];
            for y in 0..h {
                for x in 0..w {
                    let idx = y * w + x;
                    let fid = self.faction_ids[idx];
                    if fid == 0 { continue; }
                    let is_border = (x + 1 < w && self.faction_ids[idx + 1] != fid && self.faction_ids[idx + 1] != 0)
                        || (y + 1 < h && self.faction_ids[idx + w] != fid && self.faction_ids[idx + w] != 0)
                        || (x > 0 && self.faction_ids[idx - 1] != fid && self.faction_ids[idx - 1] != 0)
                        || (y > 0 && self.faction_ids[idx - w] != fid && self.faction_ids[idx - w] != 0);
                    if is_border {
                        // Draw 2px thick border
                        for dy in 0..2i32 {
                            for dx in 0..2i32 {
                                let bx = x as i32 + dx;
                                let by = y as i32 + dy;
                                if bx >= 0 && bx < w as i32 && by >= 0 && by < h as i32 {
                                    let off = (by as usize * w + bx as usize) * 4;
                                    alpha_blend(&mut pixels, off, border_colour);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Road network overlay
        if road_network {
            for seg in &self.road_segments {
                let (colour, thickness) = road_line_params(seg.road_type_id);
                let x0 = seg.from.0 as i32;
                let y0 = seg.from.1 as i32;
                let x1 = seg.to.0 as i32;
                let y1 = seg.to.1 as i32;
                draw_line(&mut pixels, w, h, x0, y0, x1, y1, colour, thickness);
            }
        }

        // Settlement icons overlay
        if settlement_icons {
            for s in &self.settlement_seeds {
                let (colour, radius) = settlement_dot_params(s);
                let cx = s.position.0 as i32;
                let cy = s.position.1 as i32;
                draw_circle(&mut pixels, w, h, cx, cy, radius, colour);
            }
        }

        pixels
    }
}

/// Save a continuous f32 layer as a gradient PNG, downscaled 2x.
fn save_gradient_layer(
    dir: &std::path::Path,
    filename: &str,
    data: &[f32],
    width: usize,
    height: usize,
    colour_low: [u8; 3],
    colour_high: [u8; 3],
) {
    use image::{ImageBuffer, Rgba, RgbaImage};

    let half_w = width / 2;
    let half_h = height / 2;
    let mut img: RgbaImage = ImageBuffer::new(half_w as u32, half_h as u32);

    for sy in 0..half_h {
        for sx in 0..half_w {
            let x0 = sx * 2;
            let y0 = sy * 2;
            let mut sum = 0.0f32;
            let mut count = 0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let idx = (y0 + dy) * width + (x0 + dx);
                    if idx < data.len() {
                        sum += data[idx];
                        count += 1;
                    }
                }
            }
            let t = if count > 0 {
                (sum / count as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let r = (colour_low[0] as f32 * (1.0 - t) + colour_high[0] as f32 * t) as u8;
            let g = (colour_low[1] as f32 * (1.0 - t) + colour_high[1] as f32 * t) as u8;
            let b = (colour_low[2] as f32 * (1.0 - t) + colour_high[2] as f32 * t) as u8;
            img.put_pixel(sx as u32, sy as u32, Rgba([r, g, b, 255]));
        }
    }

    img.save(dir.join(filename)).ok();
}

/// Save an indexed u16 layer as a PNG with random distinct colours per ID.
/// ID 0 is rendered as dark grey (unassigned/ocean). Downscaled 2x via nearest.
fn save_indexed_layer(
    dir: &std::path::Path,
    filename: &str,
    data: &[u16],
    width: usize,
    height: usize,
) {
    use image::{ImageBuffer, Rgba, RgbaImage};

    let half_w = width / 2;
    let half_h = height / 2;
    let mut img: RgbaImage = ImageBuffer::new(half_w as u32, half_h as u32);

    for sy in 0..half_h {
        for sx in 0..half_w {
            let idx = (sy * 2) * width + (sx * 2);
            let id = if idx < data.len() { data[idx] } else { 0 };
            let colour = if id == 0 {
                Rgba([40, 40, 40, 255])
            } else {
                let h = (id as u32).wrapping_mul(2654435761);
                let r = ((h >> 0) & 0xFF) as u8 | 0x40;
                let g = ((h >> 8) & 0xFF) as u8 | 0x40;
                let b = ((h >> 16) & 0xFF) as u8 | 0x40;
                Rgba([r, g, b, 255])
            };
            img.put_pixel(sx as u32, sy as u32, colour);
        }
    }

    img.save(dir.join(filename)).ok();
}

/// Save faction_ids as a coloured region PNG. ID 0 = dark grey (unclaimed). Downscaled 2x.
fn save_faction_layer(
    dir: &std::path::Path,
    filename: &str,
    data: &[u32],
    width: usize,
    height: usize,
) {
    use image::{ImageBuffer, Rgba, RgbaImage};

    let half_w = width / 2;
    let half_h = height / 2;
    let mut img: RgbaImage = ImageBuffer::new(half_w as u32, half_h as u32);

    for sy in 0..half_h {
        for sx in 0..half_w {
            let idx = (sy * 2) * width + (sx * 2);
            let id = if idx < data.len() { data[idx] } else { 0 };
            let colour = if id == 0 {
                Rgba([40, 40, 40, 255])
            } else {
                let h = id.wrapping_mul(2654435761);
                let r = ((h >> 0) & 0xFF) as u8 | 0x40;
                let g = ((h >> 8) & 0xFF) as u8 | 0x40;
                let b = ((h >> 16) & 0xFF) as u8 | 0x40;
                Rgba([r, g, b, 255])
            };
            img.put_pixel(sx as u32, sy as u32, colour);
        }
    }

    img.save(dir.join(filename)).ok();
}

/// Save settlement positions as dots on a dark background. Downscaled 2x.
fn save_settlement_layer(
    dir: &std::path::Path,
    filename: &str,
    settlements: &[SettlementSeed],
    width: usize,
    height: usize,
) {
    use image::{ImageBuffer, Rgba, RgbaImage};

    let half_w = width / 2;
    let half_h = height / 2;
    let mut img: RgbaImage = ImageBuffer::new(half_w as u32, half_h as u32);

    // Fill dark background
    for pixel in img.pixels_mut() {
        *pixel = Rgba([20, 20, 20, 255]);
    }

    for s in settlements {
        let (colour, radius) = settlement_dot_params(s);
        // Scale coordinates to half-resolution
        let cx = (s.position.0 / 2.0) as i32;
        let cy = (s.position.1 / 2.0) as i32;
        let rgba = Rgba(colour);
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= radius * radius {
                    let px = cx + dx;
                    let py = cy + dy;
                    if px >= 0 && px < half_w as i32 && py >= 0 && py < half_h as i32 {
                        img.put_pixel(px as u32, py as u32, rgba);
                    }
                }
            }
        }
    }

    img.save(dir.join(filename)).ok();
}

/// Save road segments as lines on a dark background. Downscaled 2x.
fn save_road_layer(
    dir: &std::path::Path,
    filename: &str,
    road_segments: &[RoadSegment],
    width: usize,
    height: usize,
) {
    use image::{ImageBuffer, Rgba, RgbaImage};

    let half_w = width / 2;
    let half_h = height / 2;
    let mut img: RgbaImage = ImageBuffer::new(half_w as u32, half_h as u32);

    // Fill dark background
    for pixel in img.pixels_mut() {
        *pixel = Rgba([20, 20, 20, 255]);
    }

    for seg in road_segments {
        let (colour, thickness) = road_line_params(seg.road_type_id);
        // Scale coordinates to half-resolution
        let x0 = (seg.from.0 / 2.0) as i32;
        let y0 = (seg.from.1 / 2.0) as i32;
        let x1 = (seg.to.0 / 2.0) as i32;
        let y1 = (seg.to.1 / 2.0) as i32;
        let rgba = Rgba(colour);
        bresenham_thick(&mut img, half_w as i32, half_h as i32, x0, y0, x1, y1, rgba, thickness);
    }

    img.save(dir.join(filename)).ok();
}

/// Save composite layer: dark background + factions at 40% alpha + roads + settlements. Downscaled 2x.
fn save_composite_layer(
    dir: &std::path::Path,
    filename: &str,
    faction_ids: &[u32],
    road_segments: &[RoadSegment],
    settlements: &[SettlementSeed],
    width: usize,
    height: usize,
) {
    use image::{ImageBuffer, Rgba, RgbaImage};

    let half_w = width / 2;
    let half_h = height / 2;
    let mut img: RgbaImage = ImageBuffer::new(half_w as u32, half_h as u32);

    // Base: dark background with faction overlay at 40% alpha
    for sy in 0..half_h {
        for sx in 0..half_w {
            let idx = (sy * 2) * width + (sx * 2);
            let fid = if idx < faction_ids.len() { faction_ids[idx] } else { 0 };
            let colour = if fid == 0 {
                Rgba([20, 20, 20, 255])
            } else {
                let hv = fid.wrapping_mul(2654435761);
                let fr = ((hv >> 0) & 0xFF) as u8 | 0x40;
                let fg = ((hv >> 8) & 0xFF) as u8 | 0x40;
                let fb = ((hv >> 16) & 0xFF) as u8 | 0x40;
                // Blend faction colour at 40% over dark background (20,20,20)
                let alpha = 0.4f32;
                let r = (20.0 * (1.0 - alpha) + fr as f32 * alpha) as u8;
                let g = (20.0 * (1.0 - alpha) + fg as f32 * alpha) as u8;
                let b = (20.0 * (1.0 - alpha) + fb as f32 * alpha) as u8;
                Rgba([r, g, b, 255])
            };
            img.put_pixel(sx as u32, sy as u32, colour);
        }
    }

    // Roads overlay
    for seg in road_segments {
        let (colour, thickness) = road_line_params(seg.road_type_id);
        let x0 = (seg.from.0 / 2.0) as i32;
        let y0 = (seg.from.1 / 2.0) as i32;
        let x1 = (seg.to.0 / 2.0) as i32;
        let y1 = (seg.to.1 / 2.0) as i32;
        let rgba = Rgba(colour);
        bresenham_thick(&mut img, half_w as i32, half_h as i32, x0, y0, x1, y1, rgba, thickness);
    }

    // Settlements overlay
    for s in settlements {
        let (colour, radius) = settlement_dot_params(s);
        let cx = (s.position.0 / 2.0) as i32;
        let cy = (s.position.1 / 2.0) as i32;
        let rgba = Rgba(colour);
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= radius * radius {
                    let px = cx + dx;
                    let py = cy + dy;
                    if px >= 0 && px < half_w as i32 && py >= 0 && py < half_h as i32 {
                        img.put_pixel(px as u32, py as u32, rgba);
                    }
                }
            }
        }
    }

    img.save(dir.join(filename)).ok();
}

/// Return the colour and radius for a settlement dot based on its tier.
fn settlement_dot_params(s: &SettlementSeed) -> ([u8; 4], i32) {
    match s.tier {
        SettlementTier::Capital => ([255, 60, 60, 255], 5),
        SettlementTier::Major => ([255, 220, 60, 255], 3),
        SettlementTier::Minor => ([255, 255, 255, 255], 2),
        SettlementTier::Outpost | SettlementTier::Camp => ([160, 160, 160, 255], 1),
        SettlementTier::Ruins => ([120, 120, 120, 255], 1),
    }
}

/// Return the colour and thickness for a road segment based on its type_id.
fn road_line_params(road_type_id: u8) -> ([u8; 4], i32) {
    match road_type_id {
        0 => ([220, 180, 80, 255], 3),  // Imperial: gold
        1 => ([180, 180, 180, 255], 2), // Provincial: silver
        _ => ([140, 110, 80, 255], 1),  // Trail: brown
    }
}

/// Alpha-blend a colour onto an existing pixel in RGBA buffer.
fn alpha_blend(pixels: &mut [u8], off: usize, src: [u8; 4]) {
    let sa = src[3] as f32 / 255.0;
    let da = pixels[off + 3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a > 0.0 {
        for c in 0..3 {
            let sc = src[c] as f32 / 255.0;
            let dc = pixels[off + c] as f32 / 255.0;
            pixels[off + c] = ((sc * sa + dc * da * (1.0 - sa)) / out_a * 255.0) as u8;
        }
        pixels[off + 3] = (out_a * 255.0) as u8;
    }
}

/// Draw a filled circle into raw RGBA pixel data.
fn draw_circle(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    cx: i32,
    cy: i32,
    radius: i32,
    colour: [u8; 4],
) {
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy <= radius * radius {
                let px = cx + dx;
                let py = cy + dy;
                if px >= 0 && (px as usize) < width && py >= 0 && (py as usize) < height {
                    let off = (py as usize * width + px as usize) * 4;
                    if off + 3 < pixels.len() {
                        pixels[off] = colour[0];
                        pixels[off + 1] = colour[1];
                        pixels[off + 2] = colour[2];
                        pixels[off + 3] = colour[3];
                    }
                }
            }
        }
    }
}

/// Draw a thick line using Bresenham's algorithm into raw RGBA pixel data.
fn draw_line(
    pixels: &mut [u8],
    width: usize,
    height: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    colour: [u8; 4],
    thickness: i32,
) {
    let half_t = thickness / 2;
    let mut plot = |px: i32, py: i32| {
        for dy in -half_t..=half_t {
            for dx in -half_t..=half_t {
                let fx = px + dx;
                let fy = py + dy;
                if fx >= 0 && (fx as usize) < width && fy >= 0 && (fy as usize) < height {
                    let off = (fy as usize * width + fx as usize) * 4;
                    if off + 3 < pixels.len() {
                        pixels[off] = colour[0];
                        pixels[off + 1] = colour[1];
                        pixels[off + 2] = colour[2];
                        pixels[off + 3] = colour[3];
                    }
                }
            }
        }
    };
    bresenham_core(x0, y0, x1, y1, &mut plot);
}

/// Draw a thick line using Bresenham's algorithm into an image::RgbaImage.
fn bresenham_thick(
    img: &mut image::RgbaImage,
    img_w: i32,
    img_h: i32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    colour: image::Rgba<u8>,
    thickness: i32,
) {
    let half_t = thickness / 2;
    let mut plot = |px: i32, py: i32| {
        for dy in -half_t..=half_t {
            for dx in -half_t..=half_t {
                let fx = px + dx;
                let fy = py + dy;
                if fx >= 0 && fx < img_w && fy >= 0 && fy < img_h {
                    img.put_pixel(fx as u32, fy as u32, colour);
                }
            }
        }
    };
    bresenham_core(x0, y0, x1, y1, &mut plot);
}

/// Core Bresenham line algorithm that calls `plot(x, y)` for each pixel along the line.
fn bresenham_core(x0: i32, y0: i32, x1: i32, y1: i32, plot: &mut dyn FnMut(i32, i32)) {
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut cx = x0;
    let mut cy = y0;

    loop {
        plot(cx, cy);
        if cx == x1 && cy == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            if cx == x1 {
                break;
            }
            err += dy;
            cx += sx;
        }
        if e2 <= dx {
            if cy == y1 {
                break;
            }
            err += dx;
            cy += sy;
        }
    }
}
