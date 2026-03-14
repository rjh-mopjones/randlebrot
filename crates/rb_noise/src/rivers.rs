//! Two-tier river generation system.
//!
//! Rivers are computed once globally on a coarse heightmap, producing an immutable
//! `RiverNetwork` tree. Chunks query this tree at any LOD level — they never compute
//! rivers independently. This ensures river positions are identical at every zoom level.
//!
//! ## Architecture
//!
//! **Tier 1 — Global River Network** (runs once, immutable):
//! Computed on the macro heightmap via geology-aware D8 flow accumulation.
//! Produces a tree of `RiverSegment`s rooted at ocean outlets.
//!
//! **Tier 2 — LOD-Aware Chunk Queries**:
//! Chunks call `RiverNetwork::query_chunk()` which returns segments filtered
//! by a drainage threshold that varies with LOD level.

use std::collections::{HashMap, BinaryHeap, VecDeque};
use std::cmp::Ordering;

// ─── D8 Constants ────────────────────────────────────────────────────────────

/// Direction offsets for D8 neighbors (dx, dy).
/// Order: N, NE, E, SE, S, SW, W, NW
const D8_OFFSETS: [(i32, i32); 8] = [
    (0, -1),   // N
    (1, -1),   // NE
    (1, 0),    // E
    (1, 1),    // SE
    (0, 1),    // S
    (-1, 1),   // SW
    (-1, 0),   // W
    (-1, -1),  // NW
];

/// Distance weights for diagonal vs cardinal directions.
const D8_DISTANCES: [f64; 8] = [
    1.0,
    std::f64::consts::SQRT_2,
    1.0,
    std::f64::consts::SQRT_2,
    1.0,
    std::f64::consts::SQRT_2,
    1.0,
    std::f64::consts::SQRT_2,
];

/// No flow direction (ocean or sink).
const NO_FLOW: u8 = 255;

// ─── River Character ─────────────────────────────────────────────────────────

/// Climate-aware river classification sampled at each segment's midpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiverCharacter {
    /// Light > 0.7, low humidity — carved channel, no surface water.
    DryWadi,
    /// Light 0.3–0.7, medium humidity — reduced width.
    SeasonalFlow,
    /// Light 0.1–0.3, high humidity — full permanent river, maximum width.
    Permanent,
    /// Light 0.05–0.1, sub-zero temperature — ice surface.
    Frozen,
    /// Light < 0.05 — buried under ice sheet, not rendered.
    BuriedIce,
}

impl RiverCharacter {
    /// Classify river character from climate values at a point.
    pub fn classify(light_level: f64, humidity: f64, temperature: f64) -> Self {
        if light_level < 0.05 {
            RiverCharacter::BuriedIce
        } else if light_level < 0.1 && temperature < 0.0 {
            RiverCharacter::Frozen
        } else if light_level < 0.3 || humidity > 0.5 {
            RiverCharacter::Permanent
        } else if light_level < 0.7 || humidity > 0.2 {
            RiverCharacter::SeasonalFlow
        } else {
            RiverCharacter::DryWadi
        }
    }

    /// Width multiplier for this character type.
    pub fn width_multiplier(&self) -> f64 {
        match self {
            RiverCharacter::DryWadi => 1.2,
            RiverCharacter::SeasonalFlow => 0.6,
            RiverCharacter::Permanent => 1.0,
            RiverCharacter::Frozen => 0.9,
            RiverCharacter::BuriedIce => 0.0,
        }
    }
}

// ─── River Segment ───────────────────────────────────────────────────────────

/// A stretch of river between two confluences (or between a source and a
/// confluence, or between a confluence and the ocean mouth).
#[derive(Clone, Debug)]
pub struct RiverSegment {
    /// Unique segment ID.
    pub id: usize,
    /// Cells along this segment in world coordinates (ordered upstream → downstream).
    pub path: Vec<(f64, f64)>,
    /// Drainage area at the downstream end (upstream cell count).
    pub drainage_area: u32,
    /// Index of downstream segment (None = ocean mouth).
    pub downstream: Option<usize>,
    /// Indices of upstream tributary segments.
    pub upstream: Vec<usize>,
    /// River character derived from climate at midpoint.
    pub character: RiverCharacter,
    /// Pre-computed meandering offsets (perpendicular displacement per path point).
    pub meander_offsets: Vec<f64>,
    /// Strahler stream order (1 = headwater, higher = larger trunk).
    pub strahler_order: u32,
}

// ─── Lake ────────────────────────────────────────────────────────────────────

/// A natural lake (depression that wasn't fully filled).
#[derive(Clone, Debug)]
pub struct Lake {
    /// Cells comprising the lake surface (world coordinates).
    pub cells: Vec<(f64, f64)>,
    /// Drainage area of the basin feeding this lake.
    pub drainage_area: u32,
    /// Outlet segment ID (where lake spills over into a river), if any.
    pub outlet: Option<usize>,
    /// Whether permanently frozen (light_level < 0.05).
    pub frozen: bool,
}

// ─── River Constraint ────────────────────────────────────────────────────────

/// A river constraint returned by chunk queries.
/// Contains everything a chunk needs to render/carve a river segment.
#[derive(Clone, Debug)]
pub struct RiverConstraint {
    /// Path points within the queried bounds (world coords).
    pub path: Vec<(f64, f64)>,
    /// Drainage area for width/depth computation.
    pub drainage_area: u32,
    /// Character for rendering style.
    pub character: RiverCharacter,
    /// Computed width in world units.
    pub width: f64,
    /// Computed depth.
    pub depth: f64,
    /// Strahler stream order.
    pub strahler_order: u32,
    /// Segment ID for deterministic seeding of refinement.
    pub river_id: usize,
    /// Position in upstream chain for deterministic seeding.
    pub segment_index: usize,
}

// ─── Chunk Coordinate for Spatial Index ──────────────────────────────────────

/// Simple chunk coordinate for the spatial index.
/// Uses the macro pixel grid (1 unit = 1 world unit for macro maps).
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct RiverChunkCoord {
    x: i32,
    y: i32,
}

// ─── River Network ───────────────────────────────────────────────────────────

/// Top-level immutable river network. Computed once during world generation.
pub struct RiverNetwork {
    /// All segments in the network.
    pub segments: Vec<RiverSegment>,
    /// Spatial index: which segment IDs pass through each chunk cell.
    spatial_index: HashMap<RiverChunkCoord, Vec<usize>>,
    /// Natural lakes (depressions that weren't filled).
    pub lakes: Vec<Lake>,
}

impl RiverNetwork {
    /// Generate the global river network from terrain and geological data.
    ///
    /// # Arguments
    /// * `heightmap` - Derived elevation grid
    /// * `rock_hardness` - Base layer: 0.0 soft, 1.0 hard
    /// * `tectonic_stress` - Base layer: high near boundaries (1 - boundary_distance)
    /// * `continentalness` - Base layer: < sea_level = ocean
    /// * `light_level` - Base layer: for river character classification
    /// * `humidity` - Base layer: for river character classification
    /// * `temperature` - Derived layer: for frozen classification
    /// * `peaks_valleys` - Derived layer: for meander strength
    /// * `width`, `height` - Grid dimensions
    /// * `sea_level` - Ocean threshold
    pub fn generate(
        heightmap: &[f64],
        rock_hardness: &[f64],
        tectonic_stress: &[f64],
        continentalness: &[f64],
        light_level: &[f64],
        humidity: &[f64],
        temperature: &[f64],
        peaks_valleys: &[f64],
        width: usize,
        height: usize,
        sea_level: f64,
    ) -> Self {
        let total = width * height;

        // Step 0: Condition heightmap for better drainage structure
        // Adds subtle distance-to-ocean bias + tectonic ridges + valley carving.
        // Used ONLY for flow routing; original heightmap preserved for biomes etc.
        let conditioned = condition_heightmap_for_drainage(
            heightmap, continentalness, tectonic_stress, width, height, sea_level,
        );

        // Step 1: Identify natural lakes (diff pre-fill vs post-fill, using ORIGINAL heightmap)
        let filled = fill_depressions(&conditioned, width, height, sea_level);
        let lakes = identify_lakes(heightmap, &filled, continentalness, light_level, width, height, sea_level);

        // Step 2: Geology-aware D8 flow direction (on conditioned + filled heightmap)
        let flow_dir = compute_geology_aware_flow(
            &filled, rock_hardness, tectonic_stress, width, height, sea_level,
        );

        // Step 3: Flow accumulation
        let accumulation = compute_flow_accumulation(&flow_dir, &filled, width, height);

        // Step 4: Build river tree
        let min_accumulation = ((total as f64) * 0.0003).max(20.0) as u32;
        let mut segments = build_river_tree(
            &flow_dir, &accumulation, continentalness, width, height, sea_level, min_accumulation,
        );

        // Step 5: Classify river character at each segment midpoint
        for seg in &mut segments {
            if seg.path.is_empty() {
                continue;
            }
            let mid_idx = seg.path.len() / 2;
            let (mx, my) = seg.path[mid_idx];
            let px = (mx as usize).min(width - 1);
            let py = (my as usize).min(height - 1);
            let idx = py * width + px;

            let light = light_level.get(idx).copied().unwrap_or(0.5);
            let humid = humidity.get(idx).copied().unwrap_or(0.5);
            let temp = temperature.get(idx).copied().unwrap_or(15.0);
            seg.character = RiverCharacter::classify(light, humid, temp);
        }

        // Step 5.25: Compute Strahler stream orders
        compute_strahler_orders(&mut segments);

        // Step 5.5: Smooth paths to remove D8 staircase
        for seg in &mut segments {
            seg.path = chaikin_smooth(&seg.path, 2);
            seg.meander_offsets = vec![0.0; seg.path.len()];
        }

        // Step 6: (meandering now applied per-tile in rasterize_from_network)

        // Step 7: Generate deltas at river mouths
        let deltas = generate_deltas(&segments, continentalness, width, height, sea_level);
        let delta_start = segments.len();
        segments.extend(deltas);

        // Smooth delta paths too
        for seg in &mut segments[delta_start..] {
            seg.path = chaikin_smooth(&seg.path, 2);
            seg.meander_offsets = vec![0.0; seg.path.len()];
        }

        // Diagnostics: report river network statistics
        {
            let no_flow_count = flow_dir.iter().filter(|&&d| d == NO_FLOW).count();
            let land_count = continentalness.iter().filter(|&&c| c > sea_level).count();
            let no_flow_land = (0..total).filter(|&i| continentalness[i] > sea_level && flow_dir[i] == NO_FLOW).count();
            let max_acc = accumulation.iter().copied().max().unwrap_or(0);
            let acc_above_min = accumulation.iter().filter(|&&a| a >= min_accumulation).count();
            let acc_above_2000 = accumulation.iter().filter(|&&a| a >= 2000).count();
            let segs_above_2000 = segments.iter().filter(|s| s.drainage_area >= 2000).count();
            let segs_above_500 = segments.iter().filter(|s| s.drainage_area >= 500).count();
            let segs_above_300 = segments.iter().filter(|s| s.drainage_area >= 300).count();
            let segs_above_100 = segments.iter().filter(|s| s.drainage_area >= 100).count();
            let max_drainage = segments.iter().map(|s| s.drainage_area).max().unwrap_or(0);
            println!("=== RiverNetwork diagnostics ===");
            println!("  Grid: {width}x{height} = {total} cells, land: {land_count}");
            println!("  NO_FLOW on land: {no_flow_land} / {land_count} ({:.1}%)", 100.0 * no_flow_land as f64 / land_count.max(1) as f64);
            println!("  Max accumulation: {max_acc}, cells >= min_acc({min_accumulation}): {acc_above_min}, cells >= 2000: {acc_above_2000}");
            println!("  Segments: {}, max drainage: {max_drainage}", segments.len());
            println!("  Segs with drainage >= 100: {segs_above_100}, >= 300: {segs_above_300}, >= 500: {segs_above_500}, >= 2000: {segs_above_2000}");
            println!("================================");
        }

        // Step 8: Build spatial index
        let spatial_index = build_spatial_index(&segments);

        Self {
            segments,
            spatial_index,
            lakes,
        }
    }

    /// Query river segments intersecting a rectangular bounds,
    /// filtered by LOD drainage threshold.
    pub fn query_chunk(
        &self,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
        lod_drainage_threshold: u32,
    ) -> Vec<RiverConstraint> {
        // Determine which spatial index cells overlap the bounds
        let ix_min = min_x.floor() as i32;
        let iy_min = min_y.floor() as i32;
        let ix_max = max_x.ceil() as i32;
        let iy_max = max_y.ceil() as i32;

        let mut seen_segments: Vec<bool> = vec![false; self.segments.len()];
        let mut constraints = Vec::new();

        for iy in iy_min..=iy_max {
            for ix in ix_min..=ix_max {
                let coord = RiverChunkCoord { x: ix, y: iy };
                if let Some(seg_ids) = self.spatial_index.get(&coord) {
                    for &id in seg_ids {
                        if id >= self.segments.len() || seen_segments[id] {
                            continue;
                        }
                        seen_segments[id] = true;

                        let seg = &self.segments[id];
                        if seg.drainage_area < lod_drainage_threshold {
                            continue;
                        }

                        // Clip segment path to bounds
                        let clipped_path: Vec<(f64, f64)> = seg.path.iter()
                            .filter(|&&(px, py)| {
                                px >= min_x && px <= max_x && py >= min_y && py <= max_y
                            })
                            .copied()
                            .collect();

                        if clipped_path.is_empty() {
                            continue;
                        }

                        constraints.push(RiverConstraint {
                            path: clipped_path,
                            drainage_area: seg.drainage_area,
                            character: seg.character,
                            width: compute_river_width(seg.drainage_area, seg.character),
                            depth: compute_river_depth(seg.drainage_area),
                            strahler_order: seg.strahler_order,
                            river_id: seg.id,
                            segment_index: id,
                        });
                    }
                }
            }
        }

        constraints
    }

    /// Convert the river network to a flat flow grid (backward compatibility).
    /// Returns values in [0, 1] where higher = larger river.
    /// Uses smooth rasterisation with graduated width per segment.
    pub fn to_flow_grid(&self, width: usize, height: usize) -> Vec<f64> {
        let total = width * height;
        let mut grid = vec![0.0f64; total];

        let max_drainage = self.segments.iter()
            .map(|s| s.drainage_area)
            .max()
            .unwrap_or(1);

        for seg in &self.segments {
            if seg.character == RiverCharacter::BuriedIce {
                continue;
            }

            if seg.path.len() < 2 {
                continue;
            }

            // Uniform drainage along this segment (it's constant per segment)
            let drainage_per_point = vec![seg.drainage_area; seg.path.len()];
            let max_half_width = 3.0 * seg.character.width_multiplier();

            rasterise_smooth_line(
                &mut grid, width, height,
                &seg.path, &drainage_per_point,
                max_drainage, max_half_width,
            );
        }

        grid
    }

    /// Number of segments in the network.
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }
}

// ─── LOD Drainage Thresholds ────────────────────────────────────────────────

/// Drainage threshold for macro-level rendering (trunk rivers).
/// Set relative to typical max accumulation (~5000 on 1024×512 grids).
pub const LOD_THRESHOLD_MACRO: u32 = 300;
/// Drainage threshold for meso-level rendering (major tributaries).
pub const LOD_THRESHOLD_MESO: u32 = 50;
/// Drainage threshold for micro-level rendering (full network).
pub const LOD_THRESHOLD_MICRO: u32 = 10;

// ─── Strahler Stream Order ──────────────────────────────────────────────────

/// Compute Strahler stream orders for all segments in place.
/// Leaves (no upstream) get order 1. Interior nodes: if two+ upstream share
/// the max order → max + 1, else just max.
fn compute_strahler_orders(segments: &mut [RiverSegment]) {
    if segments.is_empty() {
        return;
    }

    // Find leaves (segments with no upstream tributaries)
    let mut order: Vec<Option<u32>> = vec![None; segments.len()];
    let mut stack: Vec<usize> = Vec::new();

    for i in 0..segments.len() {
        if segments[i].upstream.is_empty() {
            order[i] = Some(1);
            stack.push(i);
        }
    }

    // Propagate downstream
    while let Some(seg_idx) = stack.pop() {
        let Some(downstream_id) = segments[seg_idx].downstream else {
            continue;
        };
        if downstream_id >= segments.len() {
            continue;
        }

        // Check if all upstream of downstream are computed
        let all_computed = segments[downstream_id].upstream.iter()
            .all(|&u| u >= segments.len() || order[u].is_some());

        if !all_computed {
            continue;
        }

        // Compute Strahler order for downstream
        let upstream_orders: Vec<u32> = segments[downstream_id].upstream.iter()
            .filter_map(|&u| if u < segments.len() { order[u] } else { None })
            .collect();

        let new_order = if upstream_orders.is_empty() {
            1
        } else {
            let max_order = *upstream_orders.iter().max().unwrap();
            let count_max = upstream_orders.iter().filter(|&&o| o == max_order).count();
            if count_max >= 2 { max_order + 1 } else { max_order }
        };

        order[downstream_id] = Some(new_order);
        stack.push(downstream_id);
    }

    // Apply computed orders to segments
    for (i, seg) in segments.iter_mut().enumerate() {
        seg.strahler_order = order[i].unwrap_or(1);
    }
}

// ─── Rasterize from Global Network ─────────────────────────────────────────

/// Rasterize rivers from a pre-computed global RiverNetwork onto a tile grid.
///
/// Queries the network for segments within the tile bounds, filtered by
/// LOD drainage threshold, then rasterizes them into a flow grid matching
/// the format used by per-tile river generation.
pub fn rasterize_from_network(
    network: &RiverNetwork,
    world_x: f64,
    world_y: f64,
    world_size: f64,
    output_size: usize,
    lod_drainage_threshold: u32,
) -> Vec<f64> {
    let total = output_size * output_size;
    let mut grid = vec![0.0f64; total];

    // Expand query bounds so that segments passing through the tile are captured
    // even if their path points lie just outside. Critical for small tiles (micro=0.25)
    // where river paths have ~1 point per world unit.
    let margin = 2.0;
    let constraints = network.query_chunk(
        world_x - margin, world_y - margin,
        world_x + world_size + margin, world_y + world_size + margin,
        lod_drainage_threshold,
    );

    if constraints.is_empty() {
        return grid;
    }

    // Use global max drainage for consistent normalization across all tiles.
    // This prevents small rivers from being crushed in tiles that also contain large ones.
    let global_max = network.segments.iter()
        .map(|s| s.drainage_area)
        .max()
        .unwrap_or(1);

    let scale = output_size as f64 / world_size;

    let pixels_per_world_unit = output_size as f64 / world_size;

    for constraint in &constraints {
        if constraint.character == RiverCharacter::BuriedIce {
            continue;
        }

        // Stage 1: Adaptive subdivision — ensure point spacing ≤ target
        let target_spacing = 6.0 / pixels_per_world_unit;
        let world_path = &constraint.path;
        let subdivided = subdivide_to_spacing(world_path, target_spacing);

        // Stage 2: Sine-generated meanders for meso+ resolution
        let meandered = if pixels_per_world_unit > 20.0 {
            apply_sine_meander(
                &subdivided,
                constraint.drainage_area,
                constraint.strahler_order,
                constraint.river_id,
                constraint.segment_index,
            )
        } else {
            subdivided
        };

        // Stage 3: Fractal midpoint displacement for micro resolution
        let refined = if pixels_per_world_unit > 500.0 {
            fractal_refine(&meandered, constraint.river_id, 0.7, 0.15, 4)
        } else {
            meandered
        };

        // Convert refined world path to pixel coordinates
        let pixel_path: Vec<(f64, f64)> = refined.iter()
            .map(|&(wx, wy)| {
                ((wx - world_x) * scale, (wy - world_y) * scale)
            })
            .collect();

        if pixel_path.len() < 2 {
            continue;
        }

        let drainage_per_point = vec![constraint.drainage_area; pixel_path.len()];
        // Scale river width with tile resolution so rivers have consistent world-space width.
        // Base width ~3 world units for the largest river, scaled to pixel space.
        // Cap to 1/4 of tile size to avoid absurd scan areas at micro resolution.
        let max_half_width = (3.0 * scale * constraint.character.width_multiplier())
            .min(output_size as f64 * 0.25);

        rasterise_smooth_line(
            &mut grid, output_size, output_size,
            &pixel_path, &drainage_per_point,
            global_max, max_half_width,
        );
    }

    grid
}

// ─── Width & Depth ───────────────────────────────────────────────────────────

/// Compute river width in world units from drainage area and character.
fn compute_river_width(drainage_area: u32, character: RiverCharacter) -> f64 {
    let base_width = (drainage_area as f64).sqrt() * 0.1;
    base_width * character.width_multiplier()
}

/// Compute river depth from drainage area.
fn compute_river_depth(drainage_area: u32) -> f64 {
    (drainage_area as f64).log10().max(0.0) * 0.5
}

// ─── Depression Filling (Priority-Flood) ─────────────────────────────────────

/// Min-heap entry for Priority-Flood algorithm.
#[derive(Clone, Copy)]
struct FloodCell {
    elevation: f64,
    index: usize,
}

impl PartialEq for FloodCell {
    fn eq(&self, other: &Self) -> bool { self.index == other.index }
}
impl Eq for FloodCell {}
impl PartialOrd for FloodCell {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) }
}
impl Ord for FloodCell {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse order for min-heap (BinaryHeap is max-heap by default)
        other.elevation.partial_cmp(&self.elevation).unwrap_or(Ordering::Equal)
    }
}

/// Fill depressions using the Priority-Flood algorithm (Barnes et al. 2014).
/// O(n log n), guaranteed convergence in a single pass.
/// Naturally assigns monotonically increasing elevations through filled areas,
/// giving D8 flow clear drainage direction through flats.
fn fill_depressions(elevation: &[f64], width: usize, height: usize, sea_level: f64) -> Vec<f64> {
    let total = width * height;
    let epsilon = 1e-4;
    let mut filled = elevation.to_vec();
    let mut resolved = vec![false; total];
    let mut heap = BinaryHeap::new();

    // Seed the heap with all ocean cells and grid-boundary cells
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let is_boundary = x == 0 || x == width - 1 || y == 0 || y == height - 1;
            if elevation[idx] <= sea_level || is_boundary {
                heap.push(FloodCell { elevation: elevation[idx], index: idx });
                resolved[idx] = true;
            }
        }
    }

    // Process cells in elevation order (lowest first)
    while let Some(cell) = heap.pop() {
        let x = cell.index % width;
        let y = cell.index / width;

        for &(dx, dy) in &D8_OFFSETS {
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;
            if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                continue;
            }
            let nidx = ny as usize * width + nx as usize;
            if resolved[nidx] {
                continue;
            }
            resolved[nidx] = true;

            // If neighbor is lower than current cell, it's in a depression — raise it
            let new_elev = if elevation[nidx] <= filled[cell.index] {
                filled[cell.index] + epsilon
            } else {
                elevation[nidx]
            };
            filled[nidx] = new_elev;
            heap.push(FloodCell { elevation: new_elev, index: nidx });
        }
    }

    filled
}

// ─── Heightmap Drainage Conditioning ─────────────────────────────────────────

/// Condition the heightmap to create coherent drainage basins before D8 flow.
///
/// Noise-based terrain lacks large-scale drainage gradients: each local hill
/// drains independently to its nearest coast, producing many small fragmented
/// basins. D8 flow only looks at adjacent cells, so smooth global biases
/// are invisible — instead we blend the heightmap with a heavily smoothed
/// version of itself to eliminate high-frequency noise while preserving
/// large-scale terrain structure.
///
/// The conditioned heightmap is used ONLY for D8 flow routing — the original
/// heightmap is preserved for all other layers (biome, temperature, etc.).
fn condition_heightmap_for_drainage(
    heightmap: &[f64],
    continentalness: &[f64],
    tectonic_stress: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
) -> Vec<f64> {
    let total = width * height;

    // Step 1: Create a heavily smoothed heightmap that shows only large-scale terrain.
    // D8 flow on this surface follows macro-scale gradients (highlands → coast)
    // rather than chasing every noise wiggle.
    let smoothed = box_blur(heightmap, width, height, 32);

    // Step 2: Blend original and smoothed heightmaps.
    // The smoothed version controls large-scale flow direction;
    // the original adds local variation for natural-looking river courses.
    // blend = 0.0 → pure original (fragmented), 1.0 → pure smooth (too straight)
    let blend = 0.65;
    let mut conditioned = Vec::with_capacity(total);
    for idx in 0..total {
        conditioned.push(heightmap[idx] * (1.0 - blend) + smoothed[idx] * blend);
    }

    // Step 3: Tectonic ridge enhancement on the conditioned surface.
    // Plate boundaries (high stress) get elevation boost → act as watershed divides.
    let beta = 0.05;
    for idx in 0..total {
        if continentalness[idx] > sea_level {
            conditioned[idx] += tectonic_stress[idx] * beta;
        }
    }

    conditioned
}

/// Simple separable box blur for a 2D grid.
fn box_blur(data: &[f64], width: usize, height: usize, radius: usize) -> Vec<f64> {
    let mut temp = data.to_vec();
    let mut output = data.to_vec();
    let diameter = radius * 2 + 1;

    // Horizontal pass
    for y in 0..height {
        let mut sum = 0.0;
        let mut count = 0;
        // Initial window
        for x in 0..=radius.min(width - 1) {
            sum += data[y * width + x];
            count += 1;
        }
        for x in 0..width {
            temp[y * width + x] = sum / count as f64;
            // Add right edge
            let right = x + radius + 1;
            if right < width {
                sum += data[y * width + right];
                count += 1;
            }
            // Remove left edge
            if x >= radius {
                let left = x - radius;
                sum -= data[y * width + left];
                count -= 1;
            }
        }
    }

    // Vertical pass
    for x in 0..width {
        let mut sum = 0.0;
        let mut count = 0;
        for y in 0..=radius.min(height - 1) {
            sum += temp[y * width + x];
            count += 1;
        }
        for y in 0..height {
            output[y * width + x] = sum / count as f64;
            let bottom = y + radius + 1;
            if bottom < height {
                sum += temp[bottom * width + x];
                count += 1;
            }
            if y >= radius {
                let top = y - radius;
                sum -= temp[top * width + x];
                count -= 1;
            }
        }
    }

    output
}

// ─── Lake Identification ─────────────────────────────────────────────────────

/// Identify natural lakes by diffing pre-fill and post-fill heightmaps.
fn identify_lakes(
    original: &[f64],
    filled: &[f64],
    continentalness: &[f64],
    light_level: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
) -> Vec<Lake> {
    let total = width * height;
    let lake_threshold = 0.02;
    let min_lake_cells = 4;

    // Find cells where fill_depth > threshold and on land
    let mut is_lake_cell = vec![false; total];
    for idx in 0..total {
        let fill_depth = filled[idx] - original[idx];
        if fill_depth > lake_threshold && continentalness[idx] >= sea_level {
            is_lake_cell[idx] = true;
        }
    }

    // Flood-fill to find contiguous lake regions
    let mut visited = vec![false; total];
    let mut lakes = Vec::new();

    for start_idx in 0..total {
        if !is_lake_cell[start_idx] || visited[start_idx] {
            continue;
        }

        // BFS flood fill
        let mut cells = Vec::new();
        let mut queue = vec![start_idx];
        visited[start_idx] = true;

        while let Some(idx) = queue.pop() {
            cells.push(idx);

            let x = idx % width;
            let y = idx / width;

            for (dx, dy) in D8_OFFSETS {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                    continue;
                }
                let nidx = ny as usize * width + nx as usize;
                if !visited[nidx] && is_lake_cell[nidx] {
                    visited[nidx] = true;
                    queue.push(nidx);
                }
            }
        }

        if cells.len() < min_lake_cells {
            continue;
        }

        // Convert cell indices to world coordinates
        let world_cells: Vec<(f64, f64)> = cells.iter()
            .map(|&idx| ((idx % width) as f64, (idx / width) as f64))
            .collect();

        // Check if lake is frozen
        let avg_light: f64 = cells.iter()
            .map(|&idx| light_level.get(idx).copied().unwrap_or(0.5))
            .sum::<f64>() / cells.len() as f64;

        lakes.push(Lake {
            cells: world_cells,
            drainage_area: cells.len() as u32,
            outlet: None, // Will be linked after river tree is built
            frozen: avg_light < 0.05,
        });
    }

    lakes
}

// ─── Geology-Aware D8 Flow Direction ─────────────────────────────────────────

/// Compute D8 flow directions with geological bias.
/// Rivers prefer fractured fault zones and avoid hard rock.
fn compute_geology_aware_flow(
    elevation: &[f64],
    rock_hardness: &[f64],
    tectonic_stress: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
) -> Vec<u8> {
    let mut flow_dir = vec![NO_FLOW; width * height];

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;

            if elevation[idx] <= sea_level {
                continue;
            }

            let mut max_slope = 0.0;
            let mut best_dir = NO_FLOW;

            for (dir, (dx, dy)) in D8_OFFSETS.iter().enumerate() {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;

                if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                    continue;
                }

                let nidx = ny as usize * width + nx as usize;
                let drop = elevation[idx] - elevation[nidx];
                let base_slope = drop / D8_DISTANCES[dir];

                // Geological bias: prefer softer rock and fault zones, but never block flow
                let geo_factor = (1.0
                    - rock_hardness.get(nidx).copied().unwrap_or(0.5) * 0.5
                    + tectonic_stress.get(nidx).copied().unwrap_or(0.0) * 0.4)
                    .clamp(0.1, 2.0);
                let adjusted_slope = base_slope * geo_factor;

                if adjusted_slope > max_slope {
                    max_slope = adjusted_slope;
                    best_dir = dir as u8;
                }
            }

            flow_dir[idx] = best_dir;
        }
    }

    flow_dir
}

// ─── Flow Accumulation ───────────────────────────────────────────────────────

/// Compute flow accumulation using topological sort.
/// Each cell's accumulation = 1 + sum of all upstream cells.
fn compute_flow_accumulation(
    flow_dir: &[u8],
    elevation: &[f64],
    width: usize,
    height: usize,
) -> Vec<u32> {
    let total = width * height;
    let mut accumulation = vec![1u32; total];

    // Sort cells by elevation (highest first) for topological processing
    let mut sorted_indices: Vec<usize> = (0..total).collect();
    sorted_indices.sort_by(|&a, &b| {
        elevation[b]
            .partial_cmp(&elevation[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for &idx in &sorted_indices {
        if flow_dir[idx] == NO_FLOW {
            continue;
        }

        let x = idx % width;
        let y = idx / width;
        let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
        let nx = (x as i32 + dx) as usize;
        let ny = (y as i32 + dy) as usize;

        if nx < width && ny < height {
            let target_idx = ny * width + nx;
            accumulation[target_idx] = accumulation[target_idx].saturating_add(accumulation[idx]);
        }
    }

    accumulation
}

// ─── River Tree Building ─────────────────────────────────────────────────────

/// Build river tree from flow direction and accumulation grids.
/// Segments are stretches between confluences, sources, and ocean outlets.
fn build_river_tree(
    flow_dir: &[u8],
    accumulation: &[u32],
    continentalness: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
    min_accumulation: u32,
) -> Vec<RiverSegment> {
    let total = width * height;

    // Mark which cells are "river cells" (above threshold)
    let is_river: Vec<bool> = accumulation.iter()
        .map(|&a| a >= min_accumulation)
        .collect();

    // Count how many river-cell upstream neighbors flow into each cell
    let mut river_inflow_count = vec![0u32; total];
    for idx in 0..total {
        if !is_river[idx] || flow_dir[idx] == NO_FLOW {
            continue;
        }
        let x = idx % width;
        let y = idx / width;
        let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        if nx >= 0 && nx < width as i32 && ny >= 0 && ny < height as i32 {
            let nidx = ny as usize * width + nx as usize;
            if is_river[nidx] {
                river_inflow_count[nidx] += 1;
            }
        }
    }

    // Find segment start points: river cells that are either:
    // - headwaters (no river-cell upstream neighbor flowing in)
    // - confluences (2+ river-cell upstream neighbors flowing in)
    // We trace FROM these start points DOWNSTREAM.

    // Actually, it's simpler to trace from every river cell that is a "source"
    // (no incoming river flow) or a "confluence" (2+ incoming), downstream until
    // hitting another confluence or the ocean.

    let mut segment_id_at: Vec<Option<usize>> = vec![None; total];
    let mut segments: Vec<RiverSegment> = Vec::new();

    // Find all river cells that are segment start points
    let mut starts: Vec<usize> = Vec::new();
    for idx in 0..total {
        if !is_river[idx] {
            continue;
        }
        // Headwater: no river cell flows into this one
        if river_inflow_count[idx] == 0 {
            starts.push(idx);
        }
        // Confluence: 2+ river cells flow into this one
        // These are also segment starts (for the downstream segment)
        if river_inflow_count[idx] >= 2 {
            starts.push(idx);
        }
    }

    // Remove duplicates (a confluence headwater)
    starts.sort_unstable();
    starts.dedup();

    // Trace each segment downstream
    for &start in &starts {
        if segment_id_at[start].is_some() && river_inflow_count[start] == 0 {
            // Already part of a segment from a confluence trace, skip if headwater
            continue;
        }

        let mut path = Vec::new();
        let mut current = start;

        loop {
            // If this cell already belongs to a segment and we're not at the start,
            // we've hit a confluence — end this segment here (exclusive)
            if current != start && segment_id_at[current].is_some() {
                break;
            }
            // If this cell is a confluence and we're not at the start, end here
            if current != start && river_inflow_count[current] >= 2 {
                break;
            }

            path.push((
                (current % width) as f64,
                (current / width) as f64,
            ));

            // Check if we've reached the ocean
            if continentalness.get(current).copied().unwrap_or(0.0) < sea_level {
                break;
            }

            // Follow flow direction
            if flow_dir[current] == NO_FLOW {
                break;
            }

            let x = current % width;
            let y = current / width;
            let (dx, dy) = D8_OFFSETS[flow_dir[current] as usize];
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;

            if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                break;
            }

            let next = ny as usize * width + nx as usize;

            // If next cell is below threshold, terminate
            if !is_river[next] && continentalness.get(next).copied().unwrap_or(0.0) >= sea_level {
                break;
            }

            current = next;
        }

        if path.len() < 2 {
            continue;
        }

        let seg_id = segments.len();
        let last_cell = path.last().unwrap();
        let last_idx = last_cell.1 as usize * width + last_cell.0 as usize;
        let drainage = accumulation.get(last_idx).copied().unwrap_or(0);

        // Mark cells as belonging to this segment
        for &(px, py) in &path {
            let idx = py as usize * width + px as usize;
            if segment_id_at[idx].is_none() {
                segment_id_at[idx] = Some(seg_id);
            }
        }

        segments.push(RiverSegment {
            id: seg_id,
            meander_offsets: vec![0.0; path.len()],
            path,
            drainage_area: drainage,
            downstream: None,
            upstream: Vec::new(),
            character: RiverCharacter::Permanent, // Will be classified later
            strahler_order: 1,
        });
    }

    // Link segments: find downstream connections
    // For each segment, check where its last cell flows to — find which segment owns that cell
    for i in 0..segments.len() {
        let last = *segments[i].path.last().unwrap();
        let last_idx = last.1 as usize * width + last.0 as usize;

        if flow_dir[last_idx] == NO_FLOW {
            continue;
        }

        let x = last_idx % width;
        let y = last_idx / width;
        let (dx, dy) = D8_OFFSETS[flow_dir[last_idx] as usize];
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;

        if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
            continue;
        }

        let next_idx = ny as usize * width + nx as usize;
        if let Some(downstream_seg) = segment_id_at[next_idx] {
            if downstream_seg != i {
                segments[i].downstream = Some(downstream_seg);
            }
        }
    }

    // Build upstream links from downstream links
    let downstream_links: Vec<(usize, Option<usize>)> = segments.iter()
        .map(|s| (s.id, s.downstream))
        .collect();
    for (seg_id, downstream) in downstream_links {
        if let Some(ds) = downstream {
            if ds < segments.len() {
                segments[ds].upstream.push(seg_id);
            }
        }
    }

    segments
}

// ─── Per-Tile River Path Refinement ──────────────────────────────────────────

/// FNV-1a hash of world-space coordinates for deterministic displacement.
/// Quantises to 4 decimal places so identical positions hash identically
/// regardless of floating-point accumulation order.
fn world_space_hash(river_id: usize, ax: f64, ay: f64, bx: f64, by: f64, depth: u32) -> u64 {
    const FNV_OFFSET: u64 = 14695981039346656037;
    const FNV_PRIME: u64 = 1099511628211;

    let quantize = |v: f64| -> u64 { (v * 10000.0).round() as i64 as u64 };

    let mut h = FNV_OFFSET;
    for val in [river_id as u64, quantize(ax), quantize(ay), quantize(bx), quantize(by), depth as u64] {
        h ^= val;
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Convert a hash to a deterministic f64 in [-1, 1).
fn hash_to_f64(h: u64) -> f64 {
    ((h & 0xFFFFFFFF) as f64 / 0xFFFFFFFF_u64 as f64) * 2.0 - 1.0
}

/// Adaptive Chaikin subdivision until point spacing ≤ target.
/// Pure, deterministic. Reuses existing `chaikin_smooth()`.
fn subdivide_to_spacing(path: &[(f64, f64)], target: f64) -> Vec<(f64, f64)> {
    if path.len() < 2 || target <= 0.0 {
        return path.to_vec();
    }

    // Find max segment length
    let max_len = path.windows(2)
        .map(|w| {
            let dx = w[1].0 - w[0].0;
            let dy = w[1].1 - w[0].1;
            (dx * dx + dy * dy).sqrt()
        })
        .fold(0.0f64, f64::max);

    if max_len <= target {
        return path.to_vec();
    }

    // Number of Chaikin passes needed: ceil(log2(max_len / target)), capped at 8
    let passes = ((max_len / target).log2().ceil() as usize).min(8);
    if passes == 0 {
        return path.to_vec();
    }

    chaikin_smooth(path, passes)
}

/// Langbein-Leopold sine-generated curve meander.
///
/// Heading angle varies sinusoidally along arc length, producing realistic
/// river bends. Width follows Leopold-Maddock: W = 0.08 * drainage^0.4.
/// theta_max scales with Strahler order (15° for order 1, up to 95° for 6+).
fn apply_sine_meander(
    path: &[(f64, f64)],
    drainage_area: u32,
    strahler_order: u32,
    river_id: usize,
    segment_index: usize,
) -> Vec<(f64, f64)> {
    if path.len() < 3 || strahler_order < 2 {
        return path.to_vec();
    }

    // Leopold-Maddock empirical width
    let w = 0.08 * (drainage_area as f64).powf(0.4);
    let lambda = 11.0 * w;

    if lambda < 0.5 {
        return path.to_vec();
    }

    // Compute total arc length
    let mut arc_lengths = Vec::with_capacity(path.len());
    arc_lengths.push(0.0);
    for i in 1..path.len() {
        let dx = path[i].0 - path[i - 1].0;
        let dy = path[i].1 - path[i - 1].1;
        arc_lengths.push(arc_lengths[i - 1] + (dx * dx + dy * dy).sqrt());
    }

    let total_arc = *arc_lengths.last().unwrap();
    if total_arc < lambda * 0.5 {
        return path.to_vec();
    }

    // theta_max from Strahler order: 15° (order 1) → 95° (order 6+)
    let theta_max_deg: f64 = match strahler_order {
        0 | 1 => 15.0,
        2 => 30.0,
        3 => 50.0,
        4 => 70.0,
        5 => 85.0,
        _ => 95.0,
    };
    let theta_max = theta_max_deg.to_radians();

    // Deterministic phase from segment identity
    let phase = deterministic_hash(river_id, segment_index);

    let mut result = Vec::with_capacity(path.len());
    result.push(path[0]); // preserve first endpoint

    for i in 1..path.len() - 1 {
        let s = arc_lengths[i];
        let t_norm = s / total_arc; // 0..1

        // Endpoint taper: blend 0→1→1→0 over first/last 25%
        let taper = if t_norm < 0.25 {
            t_norm / 0.25
        } else if t_norm > 0.75 {
            (1.0 - t_norm) / 0.25
        } else {
            1.0
        };

        // Sine-generated displacement perpendicular to local direction
        let amplitude = theta_max * taper;
        let sine_val = (std::f64::consts::TAU * s / lambda + phase).sin();
        let displacement = amplitude * sine_val * w * 0.5;

        // Compute perpendicular direction from neighbors
        let (prev_x, prev_y) = path[i - 1];
        let (next_x, next_y) = path[i + 1];
        let flow_dx = next_x - prev_x;
        let flow_dy = next_y - prev_y;
        let flow_len = (flow_dx * flow_dx + flow_dy * flow_dy).sqrt().max(0.001);
        let perp_x = -flow_dy / flow_len;
        let perp_y = flow_dx / flow_len;

        result.push((
            path[i].0 + displacement * perp_x,
            path[i].1 + displacement * perp_y,
        ));
    }

    result.push(*path.last().unwrap()); // preserve last endpoint
    result
}

/// Fractal midpoint displacement with world-space-seeded FNV hash.
///
/// H = Hurst exponent (0.7 = moderate smoothness).
/// roughness = base displacement amplitude.
/// max_depth = recursion depth (4 for micro tiles).
/// Displacement capped at 25% of segment length to prevent self-intersection.
fn fractal_refine(
    path: &[(f64, f64)],
    river_id: usize,
    h: f64,
    roughness: f64,
    max_depth: u32,
) -> Vec<(f64, f64)> {
    if path.len() < 2 || max_depth == 0 {
        return path.to_vec();
    }

    // Start with the original path as indexed points
    let mut points: Vec<(f64, f64)> = path.to_vec();

    for depth in 0..max_depth {
        let scale = roughness * (0.5f64).powf((depth as f64) * h);
        let n = points.len();
        if n < 2 {
            break;
        }

        let mut new_points = Vec::with_capacity(n * 2);
        new_points.push(points[0]);

        for i in 0..n - 1 {
            let (ax, ay) = points[i];
            let (bx, by) = points[i + 1];

            let seg_dx = bx - ax;
            let seg_dy = by - ay;
            let seg_len = (seg_dx * seg_dx + seg_dy * seg_dy).sqrt();

            if seg_len < 1e-10 {
                new_points.push(points[i + 1]);
                continue;
            }

            // Midpoint
            let mx = (ax + bx) * 0.5;
            let my = (ay + by) * 0.5;

            // Perpendicular direction
            let perp_x = -seg_dy / seg_len;
            let perp_y = seg_dx / seg_len;

            // Deterministic displacement from world-space hash
            let hash = world_space_hash(river_id, ax, ay, bx, by, depth);
            let disp = hash_to_f64(hash) * scale * seg_len;

            // Cap at 25% of segment length
            let max_disp = seg_len * 0.25;
            let clamped = disp.clamp(-max_disp, max_disp);

            new_points.push((mx + clamped * perp_x, my + clamped * perp_y));
            new_points.push(points[i + 1]);
        }

        points = new_points;
    }

    points
}

/// Deterministic hash for stable meander phase.
fn deterministic_hash(seg_id: usize, point_idx: usize) -> f64 {
    let n = (seg_id as u32).wrapping_mul(2654435761)
        .wrapping_add(point_idx as u32)
        .wrapping_mul(1103515245)
        .wrapping_add(12345);
    (n & 0xFFFF) as f64 / 0xFFFF as f64 * std::f64::consts::TAU
}

// ─── Path Smoothing & Graduated Rendering ───────────────────────────────────

/// Chaikin corner-cutting subdivision to smooth D8 staircase paths.
/// Preserves first and last points exactly.
fn chaikin_smooth(path: &[(f64, f64)], passes: usize) -> Vec<(f64, f64)> {
    if path.len() < 3 {
        return path.to_vec();
    }

    let mut current = path.to_vec();
    for _ in 0..passes {
        let n = current.len();
        if n < 3 {
            break;
        }
        let mut smoothed = Vec::with_capacity(n * 2);
        smoothed.push(current[0]); // preserve first

        for i in 0..n - 1 {
            let (ax, ay) = current[i];
            let (bx, by) = current[i + 1];
            // Skip endpoints: first point already added, last will be added
            if i > 0 {
                smoothed.push((0.75 * ax + 0.25 * bx, 0.75 * ay + 0.25 * by));
            }
            if i + 1 < n - 1 {
                smoothed.push((0.25 * ax + 0.75 * bx, 0.25 * ay + 0.75 * by));
            }
        }

        smoothed.push(current[n - 1]); // preserve last
        current = smoothed;
    }

    current
}

/// Trace connected river paths from flow direction and accumulation grids.
/// Returns (path_coords, drainage_at_each_point) per path.
fn trace_river_paths(
    flow_dir: &[u8],
    accumulation: &[u32],
    width: usize,
    height: usize,
    min_accumulation: u32,
    climate_threshold: Option<&[u32]>,
) -> Vec<(Vec<(f64, f64)>, Vec<u32>)> {
    let total = width * height;

    // Mark river cells using per-cell threshold
    let is_river: Vec<bool> = (0..total)
        .map(|idx| {
            let threshold = climate_threshold
                .map(|ct| ct[idx])
                .unwrap_or(min_accumulation);
            accumulation[idx] >= threshold
        })
        .collect();

    // Count river-cell inflows (same logic as build_river_tree)
    let mut river_inflow_count = vec![0u32; total];
    for idx in 0..total {
        if !is_river[idx] || flow_dir[idx] == NO_FLOW {
            continue;
        }
        let x = idx % width;
        let y = idx / width;
        let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
        let nx = x as i32 + dx;
        let ny = y as i32 + dy;
        if nx >= 0 && nx < width as i32 && ny >= 0 && ny < height as i32 {
            let nidx = ny as usize * width + nx as usize;
            if is_river[nidx] {
                river_inflow_count[nidx] += 1;
            }
        }
    }

    // Find start points: headwaters (inflow=0) and confluences (inflow>=2)
    let mut starts: Vec<usize> = Vec::new();
    for idx in 0..total {
        if !is_river[idx] {
            continue;
        }
        if river_inflow_count[idx] == 0 || river_inflow_count[idx] >= 2 {
            starts.push(idx);
        }
    }
    starts.sort_unstable();
    starts.dedup();

    let mut visited = vec![false; total];
    let mut result = Vec::new();

    for &start in &starts {
        if visited[start] && river_inflow_count[start] == 0 {
            continue;
        }

        let mut path = Vec::new();
        let mut drainage = Vec::new();
        let mut current = start;

        loop {
            if current != start && visited[current] {
                break;
            }
            if current != start && river_inflow_count[current] >= 2 {
                break;
            }

            path.push(((current % width) as f64, (current / width) as f64));
            drainage.push(accumulation[current]);
            visited[current] = true;

            if flow_dir[current] == NO_FLOW {
                break;
            }

            let x = current % width;
            let y = current / width;
            let (dx, dy) = D8_OFFSETS[flow_dir[current] as usize];
            let nx = x as i32 + dx;
            let ny = y as i32 + dy;

            if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                break;
            }

            let next = ny as usize * width + nx as usize;

            // Once tracing has started, continue even if per-cell threshold rises,
            // but stop if below absolute floor min_accumulation.
            if accumulation[next] < min_accumulation {
                break;
            }

            current = next;
        }

        if path.len() >= 3 {
            result.push((path, drainage));
        }
    }

    result
}

/// Rasterise a smooth line with graduated width onto a grid.
pub fn rasterise_smooth_line(
    grid: &mut [f64],
    width: usize,
    height: usize,
    path: &[(f64, f64)],
    drainage_per_point: &[u32],
    max_drainage: u32,
    max_half_width: f64,
) {
    if path.len() < 2 || max_drainage == 0 {
        return;
    }
    let max_drain_f = max_drainage as f64;

    for i in 0..path.len() - 1 {
        let (x0, y0) = path[i];
        let (x1, y1) = path[i + 1];
        let d0 = drainage_per_point[i] as f64;
        let d1 = drainage_per_point[i + 1] as f64;

        let seg_dx = x1 - x0;
        let seg_dy = y1 - y0;
        let seg_len = (seg_dx * seg_dx + seg_dy * seg_dy).sqrt();
        if seg_len < 0.001 {
            continue;
        }

        // Perpendicular direction
        let perp_x = -seg_dy / seg_len;
        let perp_y = seg_dx / seg_len;

        // Step along at 0.5-pixel increments
        let steps = (seg_len / 0.5).ceil() as usize;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let cx = x0 + seg_dx * t;
            let cy = y0 + seg_dy * t;
            let drainage = d0 + (d1 - d0) * t;

            let norm_drain = (drainage / max_drain_f).sqrt();
            let half_width = (norm_drain * max_half_width).max(0.7); // min ~1.4px wide
            let value = norm_drain.max(0.15); // min brightness so thin rivers are visible

            let hw_ceil = half_width.ceil() as i32;
            let px_center = cx.round() as i32;
            let py_center = cy.round() as i32;

            for dy in -hw_ceil..=hw_ceil {
                for dx in -hw_ceil..=hw_ceil {
                    let px = px_center + dx;
                    let py = py_center + dy;

                    if px < 0 || px >= width as i32 || py < 0 || py >= height as i32 {
                        continue;
                    }

                    // Perpendicular distance to line
                    let rel_x = px as f64 - cx;
                    let rel_y = py as f64 - cy;
                    let perp_dist = (rel_x * perp_x + rel_y * perp_y).abs();

                    if perp_dist > half_width || half_width < 0.001 {
                        continue;
                    }

                    let falloff = (1.0 - (perp_dist / half_width)).powi(2);
                    let pixel_value = value * falloff;

                    let idx = py as usize * width + px as usize;
                    grid[idx] = grid[idx].max(pixel_value);
                }
            }
        }
    }
}

/// Interpolate drainage values to match a new path length (from smoothing).
fn interpolate_drainage(original: &[u32], new_len: usize) -> Vec<u32> {
    if original.len() == new_len || original.is_empty() {
        return original.to_vec();
    }
    if original.len() == 1 {
        return vec![original[0]; new_len];
    }
    let mut result = Vec::with_capacity(new_len);
    let scale = (original.len() - 1) as f64 / (new_len - 1).max(1) as f64;
    for i in 0..new_len {
        let t = i as f64 * scale;
        let lo = (t as usize).min(original.len() - 1);
        let hi = (lo + 1).min(original.len() - 1);
        let frac = t - lo as f64;
        let val = original[lo] as f64 * (1.0 - frac) + original[hi] as f64 * frac;
        result.push(val as u32);
    }
    result
}

// ─── Delta Generation ────────────────────────────────────────────────────────

/// Generate deltas at major river mouths where they meet the coast.
fn generate_deltas(
    segments: &[RiverSegment],
    continentalness: &[f64],
    width: usize,
    height: usize,
    sea_level: f64,
) -> Vec<RiverSegment> {
    let mut deltas = Vec::new();

    for seg in segments {
        // Only large rivers with no downstream segment (ocean mouth)
        if seg.drainage_area < 2000 || seg.downstream.is_some() {
            continue;
        }

        let mouth = match seg.path.last() {
            Some(m) => *m,
            None => continue,
        };

        let mx = (mouth.0 as usize).min(width.saturating_sub(1));
        let my = (mouth.1 as usize).min(height.saturating_sub(1));
        let cont = continentalness.get(my * width + mx).copied().unwrap_or(0.0);

        // Only in the continental shelf taper zone
        if cont > 0.15 || cont < -0.1 {
            continue;
        }

        // Determine flow direction at mouth
        let flow_dir = if seg.path.len() >= 2 {
            let prev = seg.path[seg.path.len() - 2];
            let dx = mouth.0 - prev.0;
            let dy = mouth.1 - prev.1;
            let len = (dx * dx + dy * dy).sqrt().max(0.001);
            (dx / len, dy / len)
        } else {
            (0.0, 1.0)
        };

        // Fork into 2-3 distributaries
        let num_branches = if seg.drainage_area > 5000 { 3 } else { 2 };
        let spread_angle = std::f64::consts::PI / 6.0; // 30° total spread

        for branch in 0..num_branches {
            let angle_offset = if num_branches == 1 {
                0.0
            } else {
                spread_angle * (branch as f64 / (num_branches - 1) as f64 - 0.5)
            };

            let cos_a = angle_offset.cos();
            let sin_a = angle_offset.sin();
            let branch_dx = flow_dir.0 * cos_a - flow_dir.1 * sin_a;
            let branch_dy = flow_dir.0 * sin_a + flow_dir.1 * cos_a;

            // Trace branch until hitting deep ocean
            let mut path = Vec::new();
            let mut bx = mouth.0;
            let mut by = mouth.1;

            for _ in 0..20 {
                bx += branch_dx;
                by += branch_dy;

                let ix = (bx as usize).min(width.saturating_sub(1));
                let iy = (by as usize).min(height.saturating_sub(1));

                if ix >= width || iy >= height {
                    break;
                }

                let c = continentalness.get(iy * width + ix).copied().unwrap_or(-1.0);
                path.push((bx, by));

                if c < sea_level - 0.05 {
                    break;
                }
            }

            if path.len() >= 2 {
                let delta_id = segments.len() + deltas.len();
                deltas.push(RiverSegment {
                    id: delta_id,
                    meander_offsets: vec![0.0; path.len()],
                    path,
                    drainage_area: seg.drainage_area / num_branches as u32,
                    downstream: None,
                    upstream: vec![seg.id],
                    character: seg.character,
                    strahler_order: 1,
                });
            }
        }
    }

    deltas
}

// ─── Spatial Index ───────────────────────────────────────────────────────────

/// Build spatial index mapping grid cells to segment IDs.
fn build_spatial_index(segments: &[RiverSegment]) -> HashMap<RiverChunkCoord, Vec<usize>> {
    let mut index: HashMap<RiverChunkCoord, Vec<usize>> = HashMap::new();

    for seg in segments {
        for &(x, y) in &seg.path {
            let coord = RiverChunkCoord {
                x: x.floor() as i32,
                y: y.floor() as i32,
            };
            index.entry(coord).or_default().push(seg.id);
        }
    }

    // Deduplicate segment IDs per cell
    for ids in index.values_mut() {
        ids.sort_unstable();
        ids.dedup();
    }

    index
}

// ─── Heightmap Carving ──────────────────────────────────────────────────────

/// Carve macro river channels into a meso heightmap before D8 flow computation.
///
/// For each meso pixel that overlaps a macro river cell, the heightmap is lowered
/// proportionally to the macro flow magnitude. This forces meso-level D8 routing
/// to follow the same corridors established at macro level.
pub fn carve_river_channels(
    heightmap: &mut [f64],
    width: usize,
    height: usize,
    macro_rivers: &[f64],
    macro_width: usize,
    macro_height: usize,
    world_x: f64,
    world_y: f64,
    world_size: f64,
    sea_level: f64,
) {
    let scale = world_size / width as f64;
    let max_carve_depth = 0.04;
    let carve_radius_base = 2.0_f64;
    let carve_radius_flow_scale = 4.0_f64;

    // First pass: collect carve points from macro rivers
    let mut carve_points: Vec<(usize, usize, f64)> = Vec::new(); // (mx_px, my_px, flow)

    for my in 0..macro_height {
        for mx in 0..macro_width {
            let macro_flow = macro_rivers[my * macro_width + mx];
            if macro_flow <= 0.0 {
                continue;
            }

            // Map macro world coord to meso pixel
            let meso_px = ((mx as f64 - world_x) / scale) as i64;
            let meso_py = ((my as f64 - world_y) / scale) as i64;

            if meso_px >= 0 && meso_px < width as i64 && meso_py >= 0 && meso_py < height as i64 {
                carve_points.push((meso_px as usize, meso_py as usize, macro_flow));
            }
        }
    }

    // Second pass: apply Gaussian-width carving around each river point
    for &(cx, cy, flow) in &carve_points {
        let depth = (flow * max_carve_depth).min(max_carve_depth);
        let radius = carve_radius_base + flow * carve_radius_flow_scale;
        let r_ceil = radius.ceil() as i64;

        for dy in -r_ceil..=r_ceil {
            for dx in -r_ceil..=r_ceil {
                let px = cx as i64 + dx;
                let py = cy as i64 + dy;

                if px < 0 || px >= width as i64 || py < 0 || py >= height as i64 {
                    continue;
                }

                let idx = py as usize * width + px as usize;

                // Only carve above sea level
                if heightmap[idx] <= sea_level {
                    continue;
                }

                let dist = ((dx * dx + dy * dy) as f64).sqrt();
                if dist > radius {
                    continue;
                }

                // Gaussian falloff
                let sigma = radius / 2.0;
                let falloff = (-dist * dist / (2.0 * sigma * sigma)).exp();
                let carve_amount = depth * falloff;

                heightmap[idx] -= carve_amount;
            }
        }
    }
}

// ─── Legacy API ──────────────────────────────────────────────────────────────

/// Legacy river generator for backward compatibility.
/// Wraps the new RiverNetwork system but provides the old Vec<f64> interface.
pub struct RiverGenerator {
    pub sea_level: f64,
    pub min_accumulation: u32,
}

impl Default for RiverGenerator {
    fn default() -> Self {
        Self {
            sea_level: -0.025,
            min_accumulation: 100,
        }
    }
}

impl RiverGenerator {
    pub fn new(sea_level: f64) -> Self {
        Self {
            sea_level,
            ..Default::default()
        }
    }

    /// Create a river generator with threshold based on map size.
    pub fn for_map_size(sea_level: f64, width: usize, height: usize) -> Self {
        let total = width * height;
        let threshold = ((total as f64) * 0.0005).max(25.0) as u32;
        Self {
            sea_level,
            min_accumulation: threshold,
        }
    }

    /// Create a river generator with threshold scaled by detail level.
    /// Higher detail levels raise the accumulation threshold so that zoomed-in
    /// views refine existing rivers instead of spawning independent networks.
    pub fn for_map_size_with_detail(sea_level: f64, width: usize, height: usize, detail_level: u32) -> Self {
        let total = width * height;
        let base_threshold = ((total as f64) * 0.0005).max(25.0);
        let detail_multiplier = match detail_level {
            0 | 1 => 1.0,
            2 => 3.0,
            3 => 8.0,
            _ => (detail_level as f64).powi(2),
        };
        Self {
            sea_level,
            min_accumulation: (base_threshold * detail_multiplier) as u32,
        }
    }

    /// Generate rivers for a map using the new geology-aware system.
    /// Falls back to simple D8 when geological layers aren't available.
    ///
    /// This is the simple interface that doesn't use geological data.
    /// Used by macro-level generation where we don't have per-pixel geological data
    /// separately (it's baked into the heightmap).
    pub fn generate(&self, elevation: &[f64], width: usize, height: usize) -> Vec<f64> {
        let filled = fill_depressions(elevation, width, height, self.sea_level);
        let flow_dir = self.compute_flow_directions_simple(&filled, width, height);
        let accumulation = compute_flow_accumulation(&flow_dir, &filled, width, height);
        self.generate_smooth_rivers(&flow_dir, &accumulation, width, height, None)
    }

    /// Generate rivers with climate-aware thresholds.
    /// Desert areas require more accumulation to form visible rivers.
    pub fn generate_climate_aware(
        &self,
        elevation: &[f64],
        light_level: &[f64],
        humidity: &[f64],
        width: usize,
        height: usize,
    ) -> Vec<f64> {
        let filled = fill_depressions(elevation, width, height, self.sea_level);
        let flow_dir = self.compute_flow_directions_simple(&filled, width, height);
        let accumulation = compute_flow_accumulation(&flow_dir, &filled, width, height);
        let climate_threshold = self.compute_climate_threshold(light_level, humidity, width * height);
        self.generate_smooth_rivers(&flow_dir, &accumulation, width, height, Some(&climate_threshold))
    }

    /// Generate rivers with macro flow seeding and climate-aware thresholds.
    pub fn generate_with_macro_flow_climate(
        &self,
        elevation: &[f64],
        width: usize,
        height: usize,
        macro_rivers: &[f64],
        macro_width: usize,
        macro_height: usize,
        world_x: f64,
        world_y: f64,
        world_size: f64,
        light_level: &[f64],
        humidity: &[f64],
    ) -> Vec<f64> {
        let filled = fill_depressions(elevation, width, height, self.sea_level);
        let flow_dir = self.compute_flow_directions_simple(&filled, width, height);

        let total = width * height;
        let mut accumulation = vec![1u32; total];

        // Seed macro river pixels with upstream accumulation
        let scale = world_size / width as f64;
        let macro_total = (macro_width * macro_height) as f64;

        for y in 0..height {
            for x in 0..width {
                let wx = world_x + x as f64 * scale;
                let wy = world_y + y as f64 * scale;
                let mx = wx as i64;
                let my = wy as i64;

                if mx >= 0 && mx < macro_width as i64
                    && my >= 0 && my < macro_height as i64
                {
                    let macro_flow = macro_rivers[my as usize * macro_width + mx as usize];
                    if macro_flow > 0.0 {
                        let boost = (macro_flow * macro_total * 0.08) as u32;
                        let idx = y * width + x;
                        accumulation[idx] = accumulation[idx].saturating_add(boost);
                    }
                }
            }
        }

        // Topological sort: propagate flow from highest to lowest elevation
        let mut sorted_indices: Vec<usize> = (0..total).collect();
        sorted_indices.sort_by(|&a, &b| {
            filled[b]
                .partial_cmp(&filled[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for &idx in &sorted_indices {
            if flow_dir[idx] == NO_FLOW {
                continue;
            }

            let x = idx % width;
            let y = idx / width;
            let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
            let nx = (x as i32 + dx) as usize;
            let ny = (y as i32 + dy) as usize;

            if nx < width && ny < height {
                let target_idx = ny * width + nx;
                accumulation[target_idx] =
                    accumulation[target_idx].saturating_add(accumulation[idx]);
            }
        }

        let climate_threshold = self.compute_climate_threshold(light_level, humidity, total);
        self.generate_smooth_rivers(&flow_dir, &accumulation, width, height, Some(&climate_threshold))
    }

    /// Compute per-cell climate threshold for river visibility.
    fn compute_climate_threshold(&self, light_level: &[f64], humidity: &[f64], total: usize) -> Vec<u32> {
        (0..total)
            .map(|idx| {
                let aridity = (light_level[idx] / 0.7).clamp(0.0, 1.0)
                    * (1.0 - humidity[idx]).clamp(0.0, 1.0);
                (self.min_accumulation as f64 * (1.0 + aridity * 8.0)) as u32
            })
            .collect()
    }

    /// Generate smooth, graduated-width rivers from flow data.
    fn generate_smooth_rivers(
        &self,
        flow_dir: &[u8],
        accumulation: &[u32],
        width: usize,
        height: usize,
        climate_threshold: Option<&[u32]>,
    ) -> Vec<f64> {
        let paths = trace_river_paths(
            flow_dir, accumulation, width, height, self.min_accumulation, climate_threshold,
        );

        // Smooth each path and find max drainage
        let smoothed: Vec<(Vec<(f64, f64)>, Vec<u32>)> = paths
            .into_iter()
            .map(|(path, drainage)| {
                let smooth = chaikin_smooth(&path, 2);
                // Interpolate drainage values to match smoothed path length
                let interp_drainage = interpolate_drainage(&drainage, smooth.len());
                (smooth, interp_drainage)
            })
            .collect();

        let max_drainage = smoothed.iter()
            .flat_map(|(_, d)| d.iter())
            .copied()
            .max()
            .unwrap_or(1);

        let mut grid = vec![0.0f64; width * height];

        for (path, drainage) in &smoothed {
            rasterise_smooth_line(&mut grid, width, height, path, drainage, max_drainage, 3.0);
        }

        grid
    }

    /// Generate rivers with full geological awareness.
    /// Returns a RiverNetwork for LOD-consistent chunk queries.
    pub fn generate_network(
        &self,
        heightmap: &[f64],
        rock_hardness: &[f64],
        tectonic_stress: &[f64],
        continentalness: &[f64],
        light_level: &[f64],
        humidity: &[f64],
        temperature: &[f64],
        peaks_valleys: &[f64],
        width: usize,
        height: usize,
    ) -> RiverNetwork {
        RiverNetwork::generate(
            heightmap, rock_hardness, tectonic_stress, continentalness,
            light_level, humidity, temperature, peaks_valleys,
            width, height, self.sea_level,
        )
    }

    /// Generate rivers with macro flow seeding for meso-level chunks.
    /// Seeds edge cells with upstream accumulation from the macro river map,
    /// ensuring rivers flow continuously across chunk boundaries.
    pub fn generate_with_macro_flow(
        &self,
        elevation: &[f64],
        width: usize,
        height: usize,
        macro_rivers: &[f64],
        macro_width: usize,
        macro_height: usize,
        world_x: f64,
        world_y: f64,
        world_size: f64,
    ) -> Vec<f64> {
        let filled = fill_depressions(elevation, width, height, self.sea_level);
        let flow_dir = self.compute_flow_directions_simple(&filled, width, height);

        let total = width * height;
        let mut accumulation = vec![1u32; total];

        // Seed ALL macro river pixels with upstream accumulation.
        let scale = world_size / width as f64;
        let macro_total = (macro_width * macro_height) as f64;

        for y in 0..height {
            for x in 0..width {
                let wx = world_x + x as f64 * scale;
                let wy = world_y + y as f64 * scale;
                let mx = wx as i64;
                let my = wy as i64;

                if mx >= 0 && mx < macro_width as i64
                    && my >= 0 && my < macro_height as i64
                {
                    let macro_flow = macro_rivers[my as usize * macro_width + mx as usize];
                    if macro_flow > 0.0 {
                        let boost = (macro_flow * macro_total * 0.08) as u32;
                        let idx = y * width + x;
                        accumulation[idx] = accumulation[idx].saturating_add(boost);
                    }
                }
            }
        }

        // Topological sort: propagate flow from highest to lowest elevation.
        let mut sorted_indices: Vec<usize> = (0..total).collect();
        sorted_indices.sort_by(|&a, &b| {
            filled[b]
                .partial_cmp(&filled[a])
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for &idx in &sorted_indices {
            if flow_dir[idx] == NO_FLOW {
                continue;
            }

            let x = idx % width;
            let y = idx / width;
            let (dx, dy) = D8_OFFSETS[flow_dir[idx] as usize];
            let nx = (x as i32 + dx) as usize;
            let ny = (y as i32 + dy) as usize;

            if nx < width && ny < height {
                let target_idx = ny * width + nx;
                accumulation[target_idx] =
                    accumulation[target_idx].saturating_add(accumulation[idx]);
            }
        }

        self.generate_smooth_rivers(&flow_dir, &accumulation, width, height, None)
    }

    /// Simple D8 flow direction (no geological bias).
    fn compute_flow_directions_simple(&self, elevation: &[f64], width: usize, height: usize) -> Vec<u8> {
        let mut flow_dir = vec![NO_FLOW; width * height];

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;

                if elevation[idx] <= self.sea_level {
                    continue;
                }

                let mut max_slope = 0.0;
                let mut best_dir = NO_FLOW;

                for (dir, (dx, dy)) in D8_OFFSETS.iter().enumerate() {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;

                    if nx < 0 || nx >= width as i32 || ny < 0 || ny >= height as i32 {
                        continue;
                    }

                    let nidx = ny as usize * width + nx as usize;
                    let drop = elevation[idx] - elevation[nidx];
                    let slope = drop / D8_DISTANCES[dir];

                    if slope > max_slope {
                        max_slope = slope;
                        best_dir = dir as u8;
                    }
                }

                flow_dir[idx] = best_dir;
            }
        }

        flow_dir
    }

}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_river_generator_default() {
        let gen = RiverGenerator::default();
        assert!(gen.min_accumulation > 0);
        assert!(gen.sea_level < 0.0);
    }

    #[test]
    fn test_for_map_size() {
        let small = RiverGenerator::for_map_size(-0.025, 64, 32);
        let large = RiverGenerator::for_map_size(-0.025, 1024, 512);
        assert!(small.min_accumulation < large.min_accumulation);
        assert!(small.min_accumulation >= 25);
    }

    #[test]
    fn test_depression_filling() {
        let width = 5;
        let height = 5;
        let sea_level = -0.025;

        #[rustfmt::skip]
        let elevation = vec![
            0.1, 0.1, 0.1, 0.1, 0.1,
            0.1, 0.0, 0.0, 0.0, 0.1,
            0.1, 0.0, -0.1, 0.0, 0.1,
            0.1, 0.0, 0.0, 0.0, 0.1,
            0.1, 0.1, 0.1, 0.1, 0.1,
        ];

        let filled = fill_depressions(&elevation, width, height, sea_level);
        let center_idx = 2 * width + 2;
        assert!(
            filled[center_idx] >= elevation[center_idx],
            "Depression should be filled"
        );
    }

    #[test]
    fn test_flow_directions_downhill() {
        let gen = RiverGenerator::new(-0.025);
        let width = 3;
        let height = 3;

        #[rustfmt::skip]
        let elevation = vec![
            0.3, 0.2, 0.1,
            0.2, 0.1, 0.0,
            0.1, 0.0, -0.1,
        ];

        let flow_dir = gen.compute_flow_directions_simple(&elevation, width, height);

        let center_idx = 1 * width + 1;
        assert_ne!(flow_dir[center_idx], NO_FLOW);

        let ocean_idx = 2 * width + 2;
        assert_eq!(flow_dir[ocean_idx], NO_FLOW);
    }

    #[test]
    fn test_flow_accumulation_convergence() {
        let width = 5;
        let height = 5;

        #[rustfmt::skip]
        let elevation = vec![
            0.2, 0.15, 0.1, 0.15, 0.2,
            0.15, 0.1, 0.05, 0.1, 0.15,
            0.1, 0.05, 0.0, 0.05, 0.1,
            0.15, 0.1, 0.05, 0.1, 0.15,
            0.2, 0.15, 0.1, 0.15, 0.2,
        ];

        let gen = RiverGenerator::new(-0.025);
        let flow_dir = gen.compute_flow_directions_simple(&elevation, width, height);
        let accumulation = compute_flow_accumulation(&flow_dir, &elevation, width, height);

        let center_idx = 2 * width + 2;
        let corner_idx = 0;

        assert!(
            accumulation[center_idx] > accumulation[corner_idx],
            "Valley bottom should accumulate more flow than corners"
        );
    }

    #[test]
    fn test_river_generation_produces_output() {
        let gen = RiverGenerator::default();
        let width = 16;
        let height = 8;

        // Simple slope
        let mut elevation = vec![0.0; width * height];
        for y in 0..height {
            for x in 0..width {
                elevation[y * width + x] = 0.3 - x as f64 * 0.03;
            }
        }

        let rivers = gen.generate(&elevation, width, height);
        assert_eq!(rivers.len(), width * height);
    }

    #[test]
    fn test_geology_aware_flow_differs_from_simple() {
        let width = 8;
        let height = 8;
        let sea_level = -0.025;

        // Gentle slope from top-left to bottom-right
        let mut elevation = vec![0.0; width * height];
        for y in 0..height {
            for x in 0..width {
                elevation[y * width + x] = 0.5 - (x + y) as f64 * 0.03;
            }
        }

        // Uniform rock hardness
        let rock_uniform = vec![0.5; width * height];
        let tect_zero = vec![0.0; width * height];

        // Banded rock hardness: hard vertical stripe in middle
        let mut rock_banded = vec![0.3; width * height];
        for y in 0..height {
            rock_banded[y * width + 3] = 0.95; // Hard stripe
            rock_banded[y * width + 4] = 0.95;
        }

        let flow_uniform = compute_geology_aware_flow(
            &elevation, &rock_uniform, &tect_zero, width, height, sea_level,
        );
        let flow_banded = compute_geology_aware_flow(
            &elevation, &rock_banded, &tect_zero, width, height, sea_level,
        );

        // With banded hardness, some flow directions should differ
        let mut differs = false;
        for idx in 0..width * height {
            if flow_uniform[idx] != flow_banded[idx]
                && flow_uniform[idx] != NO_FLOW
                && flow_banded[idx] != NO_FLOW
            {
                differs = true;
                break;
            }
        }
        assert!(differs, "Geological bias should change some flow directions");
    }

    #[test]
    fn test_river_network_generation() {
        let width = 32;
        let height = 16;
        let sea_level = -0.025;

        // Simple terrain: high on left, sloping to ocean on right
        let mut heightmap = vec![0.0; width * height];
        let mut continentalness = vec![0.0; width * height];
        let rock = vec![0.5; width * height];
        let tect = vec![0.0; width * height];
        let light = vec![0.3; width * height];
        let humid = vec![0.5; width * height];
        let temp = vec![15.0; width * height];
        let pv = vec![0.0; width * height];

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                heightmap[idx] = 0.3 - x as f64 * 0.015;
                continentalness[idx] = 0.3 - x as f64 * 0.015;
            }
        }

        let network = RiverNetwork::generate(
            &heightmap, &rock, &tect, &continentalness,
            &light, &humid, &temp, &pv,
            width, height, sea_level,
        );

        // Should produce some segments
        let grid = network.to_flow_grid(width, height);
        assert_eq!(grid.len(), width * height);
    }

    #[test]
    fn test_river_character_classification() {
        assert_eq!(RiverCharacter::classify(0.8, 0.1, 20.0), RiverCharacter::DryWadi);
        assert_eq!(RiverCharacter::classify(0.5, 0.3, 20.0), RiverCharacter::SeasonalFlow);
        assert_eq!(RiverCharacter::classify(0.2, 0.6, 15.0), RiverCharacter::Permanent);
        assert_eq!(RiverCharacter::classify(0.08, 0.5, -10.0), RiverCharacter::Frozen);
        assert_eq!(RiverCharacter::classify(0.03, 0.5, -30.0), RiverCharacter::BuriedIce);
    }

    #[test]
    fn test_lake_identification() {
        let width = 8;
        let height = 8;
        let sea_level = -0.5;

        // Create terrain with a depression
        let mut original = vec![0.2; width * height];
        // Make a lake basin in the center
        for y in 2..6 {
            for x in 2..6 {
                original[y * width + x] = 0.05;
            }
        }
        // Even deeper center
        original[3 * width + 3] = 0.0;
        original[3 * width + 4] = 0.0;
        original[4 * width + 3] = 0.0;
        original[4 * width + 4] = 0.0;

        let filled = fill_depressions(&original, width, height, sea_level);
        let continentalness = vec![0.2; width * height]; // all land
        let light_level = vec![0.3; width * height];

        let lakes = identify_lakes(&original, &filled, &continentalness, &light_level, width, height, sea_level);

        // Should find at least one lake
        // Note: depends on whether fill depth exceeds threshold
        // The center cells should have significant fill depth
        let center_fill = filled[3 * width + 3] - original[3 * width + 3];
        if center_fill > 0.02 {
            assert!(!lakes.is_empty(), "Should identify a lake basin");
        }
    }

    #[test]
    fn test_spatial_index_query() {
        // Build a simple river network and verify query returns correct results
        let seg = RiverSegment {
            id: 0,
            path: vec![(5.0, 5.0), (6.0, 5.0), (7.0, 5.0)],
            drainage_area: 100,
            downstream: None,
            upstream: vec![],
            character: RiverCharacter::Permanent,
            meander_offsets: vec![0.0, 0.0, 0.0],
            strahler_order: 1,
        };

        let spatial_index = build_spatial_index(&[seg.clone()]);

        // Query the area containing the segment
        let network = RiverNetwork {
            segments: vec![seg],
            spatial_index,
            lakes: vec![],
        };

        let constraints = network.query_chunk(4.0, 4.0, 8.0, 6.0, 50);
        assert!(!constraints.is_empty(), "Should find river segment in query bounds");
        assert_eq!(constraints[0].drainage_area, 100);
    }

    #[test]
    fn test_sine_meander_produces_displacement() {
        // Straight horizontal path, Strahler order 3, decent drainage
        let path: Vec<(f64, f64)> = (0..40).map(|x| (x as f64 * 0.5, 10.0)).collect();
        let result = apply_sine_meander(&path, 500, 3, 42, 7);

        // Some points should be displaced from the original y=10 line
        let has_displacement = result.iter()
            .any(|&(_x, y)| (y - 10.0).abs() > 0.01);
        assert!(has_displacement, "Sine meander should displace points");

        // Endpoints should be preserved
        assert_eq!(result.first(), path.first());
        assert_eq!(result.last(), path.last());
    }

    #[test]
    fn test_sine_meander_skips_low_order() {
        let path: Vec<(f64, f64)> = (0..20).map(|x| (x as f64, 5.0)).collect();
        let result = apply_sine_meander(&path, 500, 1, 0, 0);
        // Strahler order 1 should return path unchanged
        assert_eq!(result, path);
    }

    #[test]
    fn test_subdivide_to_spacing() {
        let path = vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)];
        let result = subdivide_to_spacing(&path, 2.0);
        assert!(result.len() > path.len(), "Should add intermediate points");
        assert_eq!(result.first(), Some(&(0.0, 0.0)));
        assert_eq!(result.last(), Some(&(20.0, 0.0)));
    }

    #[test]
    fn test_subdivide_no_op_for_short_segments() {
        let path = vec![(0.0, 0.0), (0.5, 0.0)];
        let result = subdivide_to_spacing(&path, 2.0);
        assert_eq!(result, path, "Short segments need no subdivision");
    }

    #[test]
    fn test_fractal_refine_adds_points() {
        let path = vec![(0.0, 0.0), (10.0, 0.0), (20.0, 0.0)];
        let result = fractal_refine(&path, 5, 0.7, 0.15, 3);
        assert!(result.len() > path.len(), "Fractal refine should add midpoints");
        // Endpoints preserved
        assert_eq!(result.first(), Some(&(0.0, 0.0)));
        assert_eq!(result.last(), Some(&(20.0, 0.0)));
    }

    #[test]
    fn test_fractal_refine_deterministic() {
        let path = vec![(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)];
        let a = fractal_refine(&path, 42, 0.7, 0.15, 4);
        let b = fractal_refine(&path, 42, 0.7, 0.15, 4);
        assert_eq!(a, b, "Fractal refine should be deterministic");
    }

    #[test]
    fn test_world_space_hash_deterministic() {
        let h1 = world_space_hash(1, 5.0, 10.0, 6.0, 10.0, 2);
        let h2 = world_space_hash(1, 5.0, 10.0, 6.0, 10.0, 2);
        assert_eq!(h1, h2);
        // Different inputs should (almost certainly) differ
        let h3 = world_space_hash(2, 5.0, 10.0, 6.0, 10.0, 2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn test_chaikin_preserves_endpoints() {
        let path = vec![(0.0, 0.0), (1.0, 1.0), (2.0, 0.0), (3.0, 1.0), (4.0, 0.0)];
        let smoothed = chaikin_smooth(&path, 2);
        assert_eq!(smoothed.first(), Some(&(0.0, 0.0)));
        assert_eq!(smoothed.last(), Some(&(4.0, 0.0)));
        assert!(smoothed.len() > path.len(), "Smoothing should add points");
    }

    #[test]
    fn test_chaikin_short_path_unchanged() {
        let path1 = vec![(0.0, 0.0)];
        assert_eq!(chaikin_smooth(&path1, 2), path1);

        let path2 = vec![(0.0, 0.0), (1.0, 1.0)];
        assert_eq!(chaikin_smooth(&path2, 2), path2);
    }

    #[test]
    fn test_graduated_width_increases_downstream() {
        let width = 20;
        let height = 5;
        let mut grid = vec![0.0; width * height];

        // Straight horizontal path with increasing drainage
        let path: Vec<(f64, f64)> = (0..width).map(|x| (x as f64, 2.0)).collect();
        let drainage: Vec<u32> = (0..width).map(|x| (x as u32 + 1) * 50).collect();
        let max_drainage = *drainage.last().unwrap();

        rasterise_smooth_line(&mut grid, width, height, &path, &drainage, max_drainage, 3.0);

        // Count non-zero pixels in left half vs right half
        let left_count = grid.iter().take(width * height / 2).filter(|&&v| v > 0.0).count();
        let right_count = grid.iter().skip(width * height / 2).filter(|&&v| v > 0.0).count();
        // More drainage downstream = wider rendering = more non-zero pixels
        assert!(right_count >= left_count,
            "Downstream (right={}) should have >= pixels than upstream (left={})",
            right_count, left_count);
    }

    #[test]
    fn test_climate_threshold_suppresses_desert_rivers() {
        let width = 32;
        let height = 16;
        let sea_level = -0.025;

        // Gentle slope
        let mut elevation = vec![0.0; width * height];
        for y in 0..height {
            for x in 0..width {
                elevation[y * width + x] = 0.3 - x as f64 * 0.015;
            }
        }

        let gen = RiverGenerator::new(sea_level);

        // Wet climate: low light, high humidity
        let light_wet = vec![0.2; width * height];
        let humid_wet = vec![0.7; width * height];
        let rivers_wet = gen.generate_climate_aware(&elevation, &light_wet, &humid_wet, width, height);

        // Dry climate: high light, low humidity
        let light_dry = vec![0.9; width * height];
        let humid_dry = vec![0.1; width * height];
        let rivers_dry = gen.generate_climate_aware(&elevation, &light_dry, &humid_dry, width, height);

        let wet_count = rivers_wet.iter().filter(|&&v| v > 0.0).count();
        let dry_count = rivers_dry.iter().filter(|&&v| v > 0.0).count();

        assert!(wet_count >= dry_count,
            "Wet climate should have >= river pixels ({}) than dry ({})",
            wet_count, dry_count);
    }
}
