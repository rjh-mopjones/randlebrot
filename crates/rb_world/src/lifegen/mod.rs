pub mod analysis;
pub mod factions;
pub mod provinces;
pub mod roads;
pub mod settlements;
pub mod trade;

use crate::lifegen_data::{LifeGenData, PoliticalState};
use rb_core::TerrainQuery;

/// Run the complete LifeGen pipeline.
/// Reads terrain via TerrainQuery, produces a fully populated LifeGenData.
/// Deterministic for a given civ_seed.
pub fn generate(terrain: &dyn TerrainQuery, civ_seed: u32) -> LifeGenData {
    let w = terrain.width();
    let h = terrain.height();

    println!("[LifeGen] Starting generation at {}x{} with seed {}", w, h, civ_seed);

    // Phase 1: Analysis grids (all independent)
    println!("[LifeGen] Phase 1: Computing analysis grids...");
    let habitability = analysis::compute_habitability(terrain);
    let navigation_cost = analysis::compute_navigation_cost(terrain);
    let resource_desirability = analysis::compute_resource_desirability(terrain);

    let hab_min = habitability.iter().cloned().fold(f32::MAX, f32::min);
    let hab_max = habitability.iter().cloned().fold(f32::MIN, f32::max);
    let hab_nonzero = habitability.iter().filter(|&&v| v > 0.01).count();
    println!(
        "[LifeGen]   Habitability range: {:.3}--{:.3}, {} land pixels above 0.01",
        hab_min, hab_max, hab_nonzero
    );

    // Phase 2: Provinces
    println!("[LifeGen] Phase 2: Generating provinces...");
    let seed_phase2 = civ_seed.wrapping_add(2);
    let province_seeds = provinces::seed_provinces(&habitability, w, h, seed_phase2);
    println!("[LifeGen]   Seeded {} province sites", province_seeds.len());

    let mut province_ids =
        provinces::tessellate_provinces(&province_seeds, &navigation_cost, terrain, w, h);
    provinces::snap_borders_to_rivers(&mut province_ids, terrain, w, h);

    let mut province_list = provinces::bake_province_attributes(
        &province_ids,
        &province_seeds,
        terrain,
        &habitability,
        &navigation_cost,
        w,
        h,
    );
    println!("[LifeGen]   {} provinces created", province_list.len());

    // Phase 3: Factions
    println!("[LifeGen] Phase 3: Generating factions...");
    let seed_phase3 = civ_seed.wrapping_add(3);
    let capitals = factions::place_capitals(&province_list, 0, seed_phase3);
    println!("[LifeGen]   {} faction capitals placed", capitals.len());

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
    println!("[LifeGen]   {} factions created", faction_list.len());

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
    println!("[LifeGen] Phase 4: Placing settlements...");
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
    println!("[LifeGen]   {} settlements placed", settlement_seeds.len());

    // Phase 5: Roads
    println!("[LifeGen] Phase 5: Building road network...");
    let road_segments = roads::build_roads(&settlement_seeds, &navigation_cost, w, h);
    println!("[LifeGen]   {} road segments", road_segments.len());

    // Phase 6: Trade DAG
    println!("[LifeGen] Phase 6: Building trade network...");
    let trade_edges = trade::build_trade_dag(&settlement_seeds, &road_segments, &province_list);
    println!("[LifeGen]   {} trade edges", trade_edges.len());

    // Assemble LifeGenData
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

    println!("[LifeGen] Generation complete.");
    data
}
