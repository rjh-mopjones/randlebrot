pub mod analysis;
pub mod factions;
pub mod provinces;
pub mod roads;
pub mod settlements;
pub mod trade;

use crate::lifegen_data::{LifeGenData, PoliticalState};
use rb_core::TerrainQuery;
use std::sync::{Arc, Mutex};

/// Progress tracker for the LifeGen pipeline.
/// Updated by generate() as it progresses through phases.
/// Read by the UI system each frame to show progress.
#[derive(Debug, Clone)]
pub struct LifeGenProgress {
    /// Current phase (1-6), 0 = not started, 7 = complete.
    pub phase: u8,
    /// Human-readable label for the current phase.
    pub phase_label: String,
    /// Detail text (e.g. "970 provinces created").
    pub detail: String,
    /// Overall progress [0.0, 1.0].
    pub progress: f32,
}

impl Default for LifeGenProgress {
    fn default() -> Self {
        Self {
            phase: 0,
            phase_label: "Starting...".into(),
            detail: String::new(),
            progress: 0.0,
        }
    }
}

/// Shared progress handle passed into generate().
pub type ProgressHandle = Arc<Mutex<LifeGenProgress>>;

/// Create a new progress handle.
pub fn new_progress() -> ProgressHandle {
    Arc::new(Mutex::new(LifeGenProgress::default()))
}

fn update_progress(handle: &Option<ProgressHandle>, phase: u8, label: &str, detail: &str, progress: f32) {
    println!("[LifeGen] Phase {}: {} {}", phase, label, detail);
    if let Some(h) = handle {
        if let Ok(mut p) = h.lock() {
            p.phase = phase;
            p.phase_label = label.to_string();
            p.detail = detail.to_string();
            p.progress = progress;
        }
    }
}

/// Run the complete LifeGen pipeline.
/// Reads terrain via TerrainQuery, produces a fully populated LifeGenData.
/// Deterministic for a given civ_seed.
///
/// If `progress` is Some, it will be updated as generation proceeds.
pub fn generate(terrain: &dyn TerrainQuery, civ_seed: u32) -> LifeGenData {
    generate_with_progress(terrain, civ_seed, None)
}

/// Run the complete LifeGen pipeline with progress tracking.
pub fn generate_with_progress(
    terrain: &dyn TerrainQuery,
    civ_seed: u32,
    progress: Option<ProgressHandle>,
) -> LifeGenData {
    let w = terrain.width();
    let h = terrain.height();

    update_progress(&progress, 1, "Computing analysis grids...", "", 0.0);

    // Phase 1: Analysis grids (all independent)
    let habitability = analysis::compute_habitability(terrain);
    let navigation_cost = analysis::compute_navigation_cost(terrain);
    let resource_desirability = analysis::compute_resource_desirability(terrain);

    let hab_nonzero = habitability.iter().filter(|&&v| v > 0.01).count();
    update_progress(
        &progress, 1, "Analysis grids",
        &format!("{} habitable pixels", hab_nonzero),
        0.15,
    );

    // Phase 2: Provinces
    update_progress(&progress, 2, "Seeding provinces...", "", 0.15);
    let seed_phase2 = civ_seed.wrapping_add(2);
    let province_seeds = provinces::seed_provinces(&habitability, terrain, w, h, seed_phase2);

    update_progress(
        &progress, 2, "Tessellating provinces...",
        &format!("{} seeds", province_seeds.len()),
        0.20,
    );
    let mut province_ids =
        provinces::tessellate_provinces(&province_seeds, &navigation_cost, terrain, w, h);

    update_progress(&progress, 2, "Snapping borders to rivers...", "", 0.30);
    provinces::snap_borders_to_rivers(&mut province_ids, terrain, w, h);

    update_progress(&progress, 2, "Baking province attributes...", "", 0.33);
    let mut province_list = provinces::bake_province_attributes(
        &province_ids,
        &province_seeds,
        terrain,
        &habitability,
        &navigation_cost,
        w,
        h,
    );
    update_progress(
        &progress, 2, "Provinces",
        &format!("{} provinces created", province_list.len()),
        0.40,
    );

    // Phase 3: Factions
    update_progress(&progress, 3, "Placing capitals...", "", 0.40);
    let seed_phase3 = civ_seed.wrapping_add(3);
    let capitals = factions::place_capitals(&province_list, 0, seed_phase3);

    update_progress(
        &progress, 3, "Growing faction territories...",
        &format!("{} capitals", capitals.len()),
        0.45,
    );
    factions::grow_factions(
        &mut province_list,
        &province_ids,
        &navigation_cost,
        &capitals,
        w,
        h,
        seed_phase3,
    );
    let faction_list = factions::build_faction_data(&province_list, &capitals, seed_phase3);
    update_progress(
        &progress, 3, "Factions",
        &format!("{} factions created", faction_list.len()),
        0.55,
    );

    // Build per-pixel faction_ids grid from province assignments
    let mut faction_ids_grid = vec![0u32; w * h];
    for (idx, &pid) in province_ids.iter().enumerate() {
        if pid == 0 {
            continue;
        }
        let pi = (pid - 1) as usize;
        if pi < province_list.len() {
            if let PoliticalState::Claimed { faction_id } = province_list[pi].political_state {
                faction_ids_grid[idx] = faction_id;
            }
        }
    }

    // Phase 4: Settlements
    update_progress(&progress, 4, "Placing settlements...", "", 0.55);
    let seed_phase4 = civ_seed.wrapping_add(4);
    let settlement_seeds = settlements::place_settlements(
        &province_list,
        &province_ids,
        terrain,
        &habitability,
        w,
        h,
        seed_phase4,
        &capitals,
    );
    update_progress(
        &progress, 4, "Settlements",
        &format!("{} settlements placed", settlement_seeds.len()),
        0.65,
    );

    // Phase 5: Roads
    update_progress(&progress, 5, "Building road network...", "", 0.65);
    let road_segments = roads::build_roads(&settlement_seeds, &navigation_cost, w, h);
    update_progress(
        &progress, 5, "Roads",
        &format!("{} road segments", road_segments.len()),
        0.90,
    );

    // Phase 6: Trade DAG
    update_progress(&progress, 6, "Building trade network...", "", 0.90);
    let trade_edges = trade::build_trade_dag(&settlement_seeds, &road_segments, &province_list);
    update_progress(
        &progress, 6, "Trade",
        &format!("{} trade edges", trade_edges.len()),
        0.95,
    );

    // Assemble LifeGenData
    update_progress(&progress, 7, "Complete", "", 1.0);
    let mut data = LifeGenData::empty(w, h);
    data.habitability = habitability;
    data.navigation_cost = navigation_cost;
    data.resource_desirability = resource_desirability;
    data.province_ids = province_ids;
    data.provinces = province_list;
    data.faction_ids = faction_ids_grid;
    data.factions = faction_list;
    data.settlement_seeds = settlement_seeds;
    data.road_segments = road_segments;

    data
}
