use bevy::image::{ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};
use bevy_egui::{egui, EguiContexts};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rb_core::{AppMode, ModeTransitionEvent, PlayableLevel, SelectedChunk, SelectedMesoTile, SelectedMicroTile, TerrainQuery, WorldPos, handle_mode_shortcuts};
use rb_editor::{CurrentLayer, CurrentLifeGenLayer, GenerateMesoRequest, GeneratorUiState, LaunchLevelRequest, LauncherPhase, LifeGenLayer, OpenArtifactRequest, RegenerateLifeGenRequest, RegenerationRequest, SaveAsArtifactRequest, SaveLevelRequest, SaveLevelUiState, StartPlayRequest};
use rb_noise::{BiomeMap, MesoTerrainView, NoiseBackend, NormalizationHints};
use rb_player::Player;
use rb_tilemap::{LevelChunk, LoadedChunks};
use rb_world::{LifeGenData, PoliticalState, WorldDefinition};
use bevy::window::PrimaryWindow;
use clap::{Parser, Subcommand, ValueEnum, Args};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

mod cli;
mod commands;

// ─── CLI ────────────────────────────────────────────────────────────────────

/// Randlebrot — procedural world engine for Margin's Grip
#[derive(Parser, Debug)]
#[command(name = "randlebrot", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Launch the full Bevy editor (default when no subcommand given)
    Gui {
        /// Open an existing layer artifact by tag
        layers_tag: Option<String>,
    },

    /// Generate world data (layers or levels)
    Generate {
        #[command(subcommand)]
        target: GenerateTarget,
    },

    /// View and inspect generated artifacts
    View {
        #[command(subcommand)]
        target: ViewTarget,
    },

    /// Launch a playable level from a previously generated level artifact
    Launch {
        /// Tag of the level artifact to launch
        level_tag: String,
    },
}

#[derive(Subcommand, Debug)]
enum GenerateTarget {
    /// Generate terrain + civilisation layers for a world seed
    Layers {
        /// Terrain seed
        seed: u32,

        /// Tag name for the generated artifact
        tag: String,

        /// Civilisation seed (defaults to terrain seed if omitted)
        #[arg(long)]
        civ_seed: Option<u32>,

        /// Compute backend
        #[arg(long, value_enum, default_value_t = Backend::Gpu)]
        backend: Backend,

        /// Overwrite existing artifact with the same tag
        #[arg(long)]
        force: bool,
    },

    /// Generate a playable micro-level from a layers artifact or raw seed
    Level {
        #[command(flatten)]
        source: LevelSource,

        /// Chunk coordinate as x,y (comma-separated)
        #[arg(value_parser = parse_coordinate)]
        coord: (i32, i32),

        /// Tag name for the generated level artifact
        tag: String,

        /// Compute backend
        #[arg(long, value_enum, default_value_t = Backend::Gpu)]
        backend: Backend,

        /// Overwrite existing artifact with the same tag
        #[arg(long)]
        force: bool,
    },
}

/// Source for level generation — either a layers tag or a raw seed.
/// Exactly one of `layers_tag` or `--seed` must be provided.
#[derive(Args, Debug)]
#[group(required = true, multiple = false)]
struct LevelSource {
    /// Use a previously generated layers artifact
    #[arg(group = "source")]
    layers_tag: Option<String>,

    /// Generate from a raw terrain seed instead
    #[arg(long, group = "source")]
    seed: Option<u32>,
}

#[derive(Subcommand, Debug)]
enum ViewTarget {
    /// View layer artifacts
    Layers {
        /// Tag of a specific layers artifact to inspect (lists all if omitted)
        tag: Option<String>,
    },

    /// View level artifacts
    Levels {
        /// Tag of a specific level artifact to inspect (lists all if omitted)
        tag: Option<String>,
    },
}

#[derive(ValueEnum, Clone, Debug)]
enum Backend {
    Gpu,
    Cpu,
}

/// Parse a "x,y" coordinate string into an (i32, i32) pair.
fn parse_coordinate(s: &str) -> Result<(i32, i32), String> {
    let parts: Vec<&str> = s.splitn(2, ',').collect();
    if parts.len() != 2 {
        return Err(format!("expected x,y format, got '{s}'"));
    }
    let x = parts[0]
        .trim()
        .parse::<i32>()
        .map_err(|e| format!("invalid x coordinate '{}': {e}", parts[0].trim()))?;
    let y = parts[1]
        .trim()
        .parse::<i32>()
        .map_err(|e| format!("invalid y coordinate '{}': {e}", parts[1].trim()))?;
    Ok((x, y))
}

// ─── View Commands (headless, stdout only) ─────────────────────────────────

/// Recursively sum the size (in bytes) of every file under `path`.
/// Returns 0 on any I/O error so listing never fails for a single broken entry.
fn dir_size_bytes(path: &std::path::Path) -> u64 {
    let mut total: u64 = 0;
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        match entry.file_type() {
            Ok(ft) if ft.is_dir() => total += dir_size_bytes(&entry_path),
            Ok(ft) if ft.is_file() => {
                if let Ok(meta) = entry.metadata() {
                    total += meta.len();
                }
            }
            _ => {}
        }
    }
    total
}

/// Format a byte count as a human-readable string (B / KB / MB / GB).
/// GB and MB use one decimal place; KB and B use integer precision.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{} KB", bytes / KB)
    } else {
        format!("{} B", bytes)
    }
}

/// Convert an ISO 8601 timestamp like `2026-04-04T14:23:01Z` to
/// a human-friendly `2026-04-04 14:23:01`. Falls back to the original
/// string on parse failures so no information is lost.
fn format_timestamp(iso: &str) -> String {
    let trimmed = iso.trim_end_matches('Z');
    // Replace the T separator between date and time. Also strip any fractional
    // seconds component ("2026-04-04T14:23:01.123") for compactness.
    let without_fraction = match trimmed.find('.') {
        Some(idx) => &trimmed[..idx],
        None => trimmed,
    };
    without_fraction.replacen('T', " ", 1)
}

/// Pad a string to exactly `width` characters with trailing spaces.
/// If the string is already at or over `width`, no padding is added
/// (callers pass `max_cell_length + 2` so this is effectively unreachable,
/// but `saturating_sub` keeps it safe regardless).
fn pad(s: &str, width: usize) -> String {
    format!("{s}{}", " ".repeat(width.saturating_sub(s.len())))
}

/// `randlebrot view layers` — print a formatted table of all layer artifacts.
fn view_layers_list() -> Result<(), String> {
    let store = rb_artifacts::ArtifactStore::new().map_err(|e| e.to_string())?;
    let mut entries = store.list_layers().map_err(|e| e.to_string())?;

    if entries.is_empty() {
        println!("No layers generated yet. Run: randlebrot generate layers <seed> <tag>");
        return Ok(());
    }

    // Sort newest first. ISO 8601 is lexicographically chronological.
    entries.sort_by(|a, b| b.1.created.cmp(&a.1.created));

    // Gather rows: (tag, seed, civ_seed, created, layers, size)
    let rows: Vec<[String; 6]> = entries
        .iter()
        .map(|(tag, m)| {
            let dir = store.base_path().join("layers").join(tag);
            let size = format_bytes(dir_size_bytes(&dir));
            [
                tag.clone(),
                m.seed.to_string(),
                m.civ_seed.to_string(),
                format_timestamp(&m.created),
                m.layer_images.len().to_string(),
                size,
            ]
        })
        .collect();

    let headers = ["TAG", "SEED", "CIV_SEED", "CREATED", "LAYERS", "SIZE"];
    let mut widths = headers.map(|h| h.len());
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }

    // Header row
    let mut header_line = String::new();
    for (i, h) in headers.iter().enumerate() {
        header_line.push_str(&pad(h, widths[i] + 2));
    }
    println!("{}", header_line.trim_end());

    // Data rows
    for row in &rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            line.push_str(&pad(cell, widths[i] + 2));
        }
        println!("{}", line.trim_end());
    }

    Ok(())
}

/// `randlebrot view levels` — print a formatted table of all level artifacts.
fn view_levels_list() -> Result<(), String> {
    let store = rb_artifacts::ArtifactStore::new().map_err(|e| e.to_string())?;
    let mut entries = store.list_levels().map_err(|e| e.to_string())?;

    if entries.is_empty() {
        println!(
            "No levels generated yet. Run: randlebrot generate level <layers-tag|--seed N> <x,y> <tag>"
        );
        return Ok(());
    }

    // Sort newest first.
    entries.sort_by(|a, b| b.1.created.cmp(&a.1.created));

    let rows: Vec<[String; 4]> = entries
        .iter()
        .map(|(tag, m)| {
            let source = match &m.parent_layers_tag {
                Some(parent) => parent.clone(),
                None => format!("--seed {}", m.seed),
            };
            let coord = format!("({},{})", m.chunk_coord.0, m.chunk_coord.1);
            [
                tag.clone(),
                source,
                coord,
                format_timestamp(&m.created),
            ]
        })
        .collect();

    let headers = ["TAG", "SOURCE", "COORD", "CREATED"];
    let mut widths = headers.map(|h| h.len());
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if cell.len() > widths[i] {
                widths[i] = cell.len();
            }
        }
    }

    let mut header_line = String::new();
    for (i, h) in headers.iter().enumerate() {
        header_line.push_str(&pad(h, widths[i] + 2));
    }
    println!("{}", header_line.trim_end());

    for row in &rows {
        let mut line = String::new();
        for (i, cell) in row.iter().enumerate() {
            line.push_str(&pad(cell, widths[i] + 2));
        }
        println!("{}", line.trim_end());
    }

    Ok(())
}

/// `randlebrot view levels <tag>` — print detailed metadata for one level.
fn view_level_detail(tag: &str) -> Result<(), String> {
    let store = rb_artifacts::ArtifactStore::new().map_err(|e| e.to_string())?;

    // `rb_artifacts::ArtifactStore` currently only exposes a full `load_level`
    // (which deserializes the micro BiomeMap) and no `load_level_manifest`
    // helper (`load_layer_manifest` exists, but only for layer artifacts).
    // Listing all levels and filtering by tag lets us fetch just the manifest
    // cheaply without adding a new public method to rb_artifacts (keeps this
    // PR scoped to src/main.rs). A future `load_level_manifest` could make
    // this a single call.
    let entries = store.list_levels().map_err(|e| e.to_string())?;
    let manifest = entries
        .iter()
        .find(|(t, _)| t == tag)
        .map(|(_, m)| m.clone())
        .ok_or_else(|| {
            if entries.is_empty() {
                format!(
                    "level artifact '{tag}' not found (no levels exist — run \
                     `randlebrot generate level <layers-tag|--seed N> <x,y> <tag>`)"
                )
            } else {
                let available: Vec<&str> =
                    entries.iter().map(|(t, _)| t.as_str()).collect();
                format!(
                    "level artifact '{tag}' not found. Available: {}",
                    available.join(", ")
                )
            }
        })?;

    let dir = store.base_path().join("levels").join(tag);
    let size = format_bytes(dir_size_bytes(&dir));

    let source = match &manifest.parent_layers_tag {
        Some(parent) => format!(
            "{parent} (seed={}, civ_seed={})",
            manifest.seed, manifest.civ_seed
        ),
        None => format!("--seed {} (civ_seed={})", manifest.seed, manifest.civ_seed),
    };

    // `LevelManifest.chunk_coord` is a **global** CLI chunk coordinate —
    // see `cli::coords` for the canonical convention. `(cx, cy)` indexes
    // the 1024×512 global chunk grid, so the tile's world-space top-left
    // is `(cx * CHUNK_WORLD_SIZE, cy * CHUNK_WORLD_SIZE)`. Resolve via the
    // shared helper so every CLI surface (generate, view, future launch)
    // agrees on what a chunk coordinate means.
    let (cx, cy) = manifest.chunk_coord;
    let (world_x, world_y) = cli::coords::chunk_coord_to_world_pos((cx, cy));

    println!("Tag:            {tag}");
    println!("Source:         {source}");
    println!("Chunk Coord:    ({cx}, {cy})");
    println!("World Position: ({world_x:.1}, {world_y:.1})");
    println!("Created:        {}", format_timestamp(&manifest.created));
    println!("Size:           {size}");

    Ok(())
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        // No subcommand → default to GUI editor
        None => launch_gui(None),

        Some(Command::Gui { layers_tag }) => launch_gui(layers_tag),

        Some(Command::Generate { target }) => match target {
            GenerateTarget::Layers {
                seed,
                tag,
                civ_seed,
                backend,
                force,
            } => {
                let noise_backend = match backend {
                    Backend::Gpu => NoiseBackend::Gpu,
                    Backend::Cpu => NoiseBackend::Cpu,
                };
                if let Err(err) =
                    commands::generate_layers::run(seed, tag, civ_seed, noise_backend, force)
                {
                    eprintln!("error: {err}");
                    std::process::exit(1);
                }
            }
            GenerateTarget::Level {
                source,
                coord,
                tag,
                backend,
                force,
            } => {
                let noise_backend = match backend {
                    Backend::Gpu => NoiseBackend::Gpu,
                    Backend::Cpu => NoiseBackend::Cpu,
                };
                // clap's `#[group(required = true, multiple = false)]` on
                // `LevelSource` guarantees exactly one of these is `Some`.
                let result = if let Some(layers_tag) = source.layers_tag {
                    commands::generate_level::run_from_layers(
                        layers_tag,
                        coord,
                        tag,
                        noise_backend,
                        force,
                    )
                } else {
                    let seed = source
                        .seed
                        .expect("LevelSource::seed must be Some when layers_tag is None");
                    commands::generate_level::run_from_seed(
                        seed,
                        coord,
                        tag,
                        noise_backend,
                        force,
                    )
                };
                if let Err(err) = result {
                    eprintln!("error: {err}");
                    std::process::exit(1);
                }
            }
        },

        Some(Command::View { target }) => match target {
            ViewTarget::Layers { tag: None } => {
                if let Err(e) = view_layers_list() {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            ViewTarget::Layers { tag: Some(tag) } => {
                if let Err(e) = commands::view_layers::run(tag) {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            ViewTarget::Levels { tag: None } => {
                if let Err(e) = view_levels_list() {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
            ViewTarget::Levels { tag: Some(tag) } => {
                if let Err(e) = view_level_detail(&tag) {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        },

        Some(Command::Launch { level_tag }) => {
            if let Err(e) = commands::launch::run(level_tag) {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }
}

// ─── GUI Entrypoint ─────────────────────────────────────────────────────────

const MAP_WIDTH: usize = 1024;
const MAP_HEIGHT: usize = 512;
const CHUNK_SIZE_I: usize = 64;

/// Launch the full Bevy editor GUI. This is the default entrypoint.
fn launch_gui(layers_tag: Option<String>) {
    // If a layers tag was provided, validate it exists before launching Bevy.
    if let Some(ref tag) = layers_tag {
        match rb_artifacts::ArtifactStore::new() {
            Ok(store) => {
                if !store.exists(rb_artifacts::ArtifactKind::Layers, tag) {
                    // List available tags for a helpful error message.
                    let available = store.list_layers().unwrap_or_default();
                    if available.is_empty() {
                        eprintln!(
                            "error: layer artifact '{tag}' not found (no layers exist \
                             — run `randlebrot generate layers <seed> <tag>` first)"
                        );
                    } else {
                        let tags: Vec<&str> = available.iter().map(|(t, _)| t.as_str()).collect();
                        eprintln!(
                            "error: layer artifact '{tag}' not found. Available: {}",
                            tags.join(", ")
                        );
                    }
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("error: failed to initialise artifact store: {e}");
                std::process::exit(1);
            }
        }
    }

    let loading_from_artifact = layers_tag.is_some();

    // On macOS, force the process to register as a foreground GUI app.
    // Without this, terminal-launched binaries don't get keyboard focus.
    #[cfg(target_os = "macos")]
    {
        unsafe {
            use std::ffi::CStr;
            let cls = objc2::runtime::AnyClass::get(
                CStr::from_bytes_with_nul(b"NSApplication\0").unwrap()
            );
            if let Some(cls) = cls {
                let shared_app: *mut objc2::runtime::AnyObject =
                    objc2::msg_send![cls, sharedApplication];
                if !shared_app.is_null() {
                    let _: () = objc2::msg_send![shared_app, setActivationPolicy: 0i64];
                    let _: () = objc2::msg_send![shared_app, activateIgnoringOtherApps: true];
                }
            }
        }
    }

    let mut app = App::new();

    app
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Randlebrot - World Editor".into(),
                resolution: bevy::window::WindowResolution::new(MAP_WIDTH as u32, MAP_HEIGHT as u32),
                ..default()
            }),
            ..default()
        }))
        // State and events
        .init_state::<AppMode>();

    // When loading from artifact, start in LoadingArtifact instead of Config.
    if loading_from_artifact {
        app.insert_state(AppPhase::LoadingArtifact);
    } else {
        app.init_state::<AppPhase>();
    }

    app
        .add_message::<ModeTransitionEvent>()
        .init_resource::<CurrentLayer>()
        .init_resource::<GeneratorParams>()
        .init_resource::<CursorWorldPos>()
        .init_resource::<VisibleChunkRange>()
        .init_resource::<HighlightInfo>()
        .init_resource::<LifeGenOverlayState>()
        .init_resource::<HoveredProvince>()
        .init_resource::<HoveredSettlement>()
        // Plugins
        .add_plugins((
            rb_core::RbCorePlugin,
            rb_noise::RbNoisePlugin,
            rb_world::RbWorldPlugin,
            rb_tilemap::RbTilemapPlugin,
            rb_entity_spawn::RbEntitySpawnPlugin,
            rb_editor::RbEditorPlugin,
            rb_player::RbPlayerPlugin,
            rb_persistence::RbPersistencePlugin,
        ))
        // Startup - just spawn camera
        .add_systems(Startup, setup_camera)
        // Config phase - show config UI
        .add_systems(Update, config_ui.run_if(in_state(AppPhase::Config)))
        // Generating phase - generate macro + transition to ready
        .add_systems(Update,
            start_generation
                .run_if(resource_added::<GenerationStarted>)
                .run_if(in_state(AppPhase::Generating)),
        )
        // GeneratingMacro phase - pre-generate all 128 macro tiles
        .add_systems(Update, (
            dispatch_macro_pregen,
            poll_macro_pregen,
            macro_pregen_progress_ui,
        ).run_if(in_state(AppPhase::GeneratingMacro)))
        // GeneratingLifeGen phase - async civilisation generation
        .add_systems(Update, (
            poll_lifegen_task,
            lifegen_progress_ui,
        ).run_if(in_state(AppPhase::GeneratingLifeGen)))
        .init_resource::<LevelChunkQueue>()
        // Ready phase - main game systems
        .add_systems(Update, (
            handle_mode_shortcuts,
            handle_layer_change.run_if(in_state(AppMode::WorldGenerator)),
            regenerate_world.run_if(in_state(AppMode::WorldGenerator)),
            regenerate_lifegen,
            log_mode_transition,
        ).run_if(in_state(AppPhase::Ready)))
        // Click on world map to select a chunk (no mode switch)
        .add_systems(Update,
            click_to_select_chunk
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::WorldGenerator).or(in_state(AppMode::CivGenerator)))
                    .and(not(resource_exists::<PlayableLevel>))),
        )
        // World map systems shared by WorldGenerator and CivGenerator
        .add_systems(Update, (
            camera_zoom,
            camera_pan,
            calculate_visible_chunks,
            enqueue_and_dispatch_tiles,
            poll_tile_results,
            manage_tile_sprites,
            update_cursor_world_pos,
            update_chunk_selection_highlight,
            highlight_info_ui,
        ).run_if(in_state(AppPhase::Ready).and(
            in_state(AppMode::WorldGenerator).or(in_state(AppMode::CivGenerator))
        )))
        // WorldGenerator-only: macro chunk highlight
        .add_systems(Update, update_chunk_highlight
            .run_if(in_state(AppPhase::Ready).and(in_state(AppMode::WorldGenerator))))
        // CivGenerator-only: province + settlement highlight
        .add_systems(Update, update_civ_highlight
            .run_if(in_state(AppPhase::Ready).and(in_state(AppMode::CivGenerator))))
        // Lifegen overlay: render civilization data on world map
        .add_systems(Update, manage_lifegen_overlay
            .run_if(in_state(AppPhase::Ready).and(in_state(AppMode::CivGenerator))))
        .add_systems(Update, hide_lifegen_overlay
            .run_if(in_state(AppPhase::Ready).and(not(in_state(AppMode::CivGenerator)))))
        .add_systems(OnEnter(AppMode::CivGenerator), show_lifegen_overlay)
        .add_systems(OnExit(AppMode::CivGenerator), hide_civ_highlights)
        // Launcher: enter/exit
        .add_systems(OnEnter(AppMode::LevelLauncher), enter_launcher_macro_view)
        .add_systems(OnExit(AppMode::LevelLauncher), cleanup_launcher_entities)
        // Launcher: "Generate Mesomap" button
        .add_systems(Update,
            handle_generate_meso_request
                .run_if(in_state(AppPhase::Ready)
                    .and(resource_exists::<GenerateMesoRequest>)),
        )
        // Launcher: meso generation + progress UI
        .add_systems(Update, (
            dispatch_meso_pregen,
            poll_meso_pregen,
            meso_pregen_progress_ui,
        ).run_if(in_state(AppPhase::Ready)
            .and(resource_exists::<MesoPregenState>)))
        // Launcher: camera pan (all launcher phases)
        .add_systems(Update,
            camera_pan
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::LevelLauncher))),
        )
        // Launcher: meso view interactions
        .add_systems(Update, (
            launcher_camera_zoom,
            click_to_select_meso_tile,
            update_meso_highlight,
        ).run_if(in_state(AppPhase::Ready)
            .and(in_state(AppMode::LevelLauncher))
            .and(|phase: Option<Res<LauncherPhase>>| phase.map_or(false, |p| *p == LauncherPhase::MesoView))))
        // Launcher: re-show meso grid when ESC returns from Playing
        .add_systems(Update,
            reshow_meso_on_return
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::LevelLauncher))),
        )
        // Launcher: "Launch Level" button (starts micro generation)
        .add_systems(Update,
            handle_launch_level_request
                .run_if(in_state(AppPhase::Ready)
                    .and(resource_exists::<LaunchLevelRequest>)),
        )
        // Launcher: micro generation + progress UI
        .add_systems(Update, (
            dispatch_micro_pregen,
            poll_micro_pregen,
            micro_pregen_progress_ui,
        ).run_if(in_state(AppPhase::Ready)
            .and(resource_exists::<MicroPregenState>)))
        // Launcher: micro view interactions
        .add_systems(Update, (
            launcher_camera_zoom,
            click_to_select_micro_tile,
            update_micro_highlight,
        ).run_if(in_state(AppPhase::Ready)
            .and(in_state(AppMode::LevelLauncher))
            .and(|phase: Option<Res<LauncherPhase>>| phase.map_or(false, |p| *p == LauncherPhase::MicroView))))
        // Launcher: ESC from MicroView → MesoView
        .add_systems(Update,
            escape_micro_view
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::LevelLauncher))),
        )
        // Launcher: "Play" button
        .add_systems(Update,
            handle_start_play_request
                .run_if(in_state(AppPhase::Ready)
                    .and(resource_exists::<StartPlayRequest>)),
        )
        // Launcher: save level artifact
        .add_systems(Update,
            handle_save_level_request
                .run_if(in_state(AppPhase::Ready)
                    .and(resource_exists::<SaveLevelRequest>)),
        )
        // Launcher: re-show micro grid when ESC returns from Playing
        .add_systems(Update,
            reshow_micro_on_return
                .run_if(in_state(AppPhase::Ready)
                    .and(in_state(AppMode::LevelLauncher))),
        )
        // Hide world map when play mode starts
        .add_systems(Update,
            hide_world_map_on_play
                .run_if(in_state(AppPhase::Ready).and(resource_added::<PlayableLevel>)),
        )
        // Level chunk streaming systems (only during play mode)
        .add_systems(Update, (
            level_chunk_load_system,
            level_chunk_poll_system,
            level_chunk_unload_system,
        ).run_if(in_state(AppPhase::Ready).and(resource_exists::<PlayableLevel>)))
        // SavingArtifact phase - prompt for tag and save
        .add_systems(Update,
            artifact_save_ui
                .run_if(in_state(AppPhase::SavingArtifact)),
        )
        // LoadingArtifact phase - load from disk and populate resources
        .add_systems(Update,
            artifact_load_system
                .run_if(in_state(AppPhase::LoadingArtifact)),
        )
        // Open Artifact request (from editor UI) — transition to LoadingArtifact
        .add_systems(Update,
            handle_open_artifact_request
                .run_if(resource_exists::<OpenArtifactRequest>),
        )
        // Save As Artifact request (from editor UI) — save current state
        .add_systems(Update,
            handle_save_as_artifact_request
                .run_if(in_state(AppPhase::Ready)
                    .and(resource_exists::<SaveAsArtifactRequest>)),
        )
        // Sync world_ready flag on GeneratorUiState
        .add_systems(Update, sync_world_ready_flag);

    // If a layers tag was provided, insert it as a resource so the
    // LoadingArtifact system can read it.
    if let Some(tag) = layers_tag {
        app.insert_resource(LoadLayersTag(tag));
    }

    app.run();
}

// ─── Constants ───────────────────────────────────────────────────────────────

/// Size of macro chunks in pixels (for highlighting grid).
const CHUNK_SIZE: f32 = 64.0;

/// Size of tile maps in pixels (per tile).
const TILE_MAP_SIZE: usize = 512;

/// Number of pre-spawned macro pool sprites.
const MACRO_POOL_SIZE: usize = 160;

/// Max cached macro tiles — sized to hold all 128 pre-generated tiles.
const MACRO_CACHE_MAX: usize = 128;

/// Max concurrent async tile generation tasks (streaming).
const MAX_CONCURRENT_TILES: usize = 16;

/// Meso tile world size (8×8 world units).
const MESO_WORLD_SIZE: f64 = 8.0;

/// Helper: get the orthographic scale from a Projection enum.
fn ortho_scale(projection: &Projection) -> f32 {
    match projection {
        Projection::Orthographic(o) => o.scale,
        _ => 1.0,
    }
}

/// Helper: set the orthographic scale on a Projection enum.
fn set_ortho_scale(projection: &mut Projection, scale: f32) {
    if let Projection::Orthographic(ref mut o) = *projection {
        o.scale = scale;
    }
}

/// Number of meso tiles per macro chunk edge (64/8 = 8).
const MESO_GRID_SIZE: i32 = 8;

/// Display size of each meso tile in the launcher grid.
const MESO_TILE_DISPLAY_PX: f32 = 64.0;

/// Max concurrent async tile generation tasks during macro pre-generation.
const MACRO_PREGEN_CONCURRENCY: usize = 4;

/// Max tile completions to process per frame.
const POLL_BUDGET: usize = 16;


/// Micro tile covers this many world units (1.0×1.0 area at 512×512 pixels).
const MICRO_WORLD_SIZE: f64 = 1.0;

/// Number of micro tiles per meso tile edge (8.0/1.0 = 8).
const MICRO_GRID_SIZE: i32 = 8;

/// Display size of each micro tile in the launcher grid.
const MICRO_TILE_DISPLAY_PX: f32 = 16.0;

// ─── Resources ───────────────────────────────────────────────────────────────

/// Marker resource to trigger generation start.
#[derive(Resource)]
struct GenerationStarted;

/// Layers tag passed via CLI (`randlebrot gui <tag>`) for load-from-artifact.
#[derive(Resource)]
struct LoadLayersTag(String);

/// State for the post-generation artifact save dialog.
#[derive(Resource)]
struct ArtifactSaveState {
    /// Tag name input by the user.
    tag_input: String,
    /// Error message to display (validation/save failure).
    error: Option<String>,
    /// Whether the save is in progress (async task running).
    saving: bool,
}

impl Default for ArtifactSaveState {
    fn default() -> Self {
        Self {
            tag_input: String::new(),
            error: None,
            saving: false,
        }
    }
}

/// Global heightmap normalization hints computed after macro pregen.
/// Ensures all tiles use the same heightmap range for consistent shading.
#[derive(Resource, Clone)]
struct GlobalNormHints(NormalizationHints);

/// Parameters for world generation (editable in UI).
#[derive(Resource, Debug, Clone)]
pub struct GeneratorParams {
    pub seed: u32,
    pub needs_regenerate: bool,
}

impl Default for GeneratorParams {
    fn default() -> Self {
        Self {
            seed: 42,
            needs_regenerate: false,
        }
    }
}

/// Stores the macro BiomeMap used as parent data for meso/micro generation.
#[derive(Resource)]
struct MacroBiomeData {
    biome_map: Arc<BiomeMap>,
}

/// Global river network computed once from the macro BiomeMap.
/// All tile generations query this for consistent rivers across zoom levels.
#[derive(Resource)]
struct GlobalRiverNetwork {
    network: Arc<rb_noise::RiverNetwork>,
}

/// Marker component for the chunk highlight overlay (follows cursor).
#[derive(Component)]
struct ChunkHighlight;

/// Marker component for the persistent selection overlay (shows selected chunk).
#[derive(Component)]
struct ChunkSelectionHighlight;

/// Tracks which province the cursor is over in CivGenerator mode.
#[derive(Resource, Default)]
struct HoveredProvince {
    province_id: Option<u16>,
}

/// Tracks which settlement is near the cursor in CivGenerator mode.
#[derive(Resource, Default)]
struct HoveredSettlement {
    settlement_id: Option<u32>,
}

/// Marker for the province highlight overlay sprite.
#[derive(Component)]
struct ProvinceHighlightSprite;

/// Marker for the settlement proximity marker sprite.
#[derive(Component)]
struct SettlementHighlightSprite;

/// Marker for the lifegen overlay sprite.
#[derive(Component)]
struct LifeGenOverlay;

/// Stored MesoTerrainView for LifeGen regeneration.
/// Holds Arc clones of the 128 macro BiomeMap tiles at full resolution.
#[derive(Resource)]
struct StoredTerrainView(MesoTerrainView);

/// Tracks async LifeGen generation task and progress.
#[derive(Resource)]
struct LifeGenTask {
    task: Task<LifeGenData>,
    progress: rb_world::lifegen::ProgressHandle,
}

/// Tracks which layer was last rendered to detect changes.
#[derive(Resource)]
struct LifeGenOverlayState {
    current_layer: String,
    data_generation: u64,
    overlay_hash: u64,
}

impl Default for LifeGenOverlayState {
    fn default() -> Self {
        Self {
            current_layer: String::new(),
            data_generation: 0,
            overlay_hash: 0,
        }
    }
}

/// Info about the currently highlighted tile, displayed in the UI.
#[derive(Resource, Default)]
struct HighlightInfo {
    /// Whether the cursor is over a valid tile.
    active: bool,
    /// Tile grid coordinates.
    tile_coord: (i32, i32),
    /// World-space position (top-left of tile).
    world_pos: (f32, f32),
    /// Current detail tier name.
    tier: &'static str,
    /// Which domain is active ("Terrain" / "Civilization" / "Scene").
    domain: &'static str,
}

/// Resource tracking cursor position in world space.
#[derive(Resource, Default)]
struct CursorWorldPos(Vec2);


/// Camera viewport in chunk coordinates.
#[derive(Resource, Default)]
struct VisibleChunkRange {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
}

/// Marker component for pool slot sprites.
#[derive(Component)]
struct PoolSlot;

/// Pre-spawned sprite pool — no spawn/despawn during gameplay.
#[derive(Resource)]
struct SpritePool {
    macro_sprites: Vec<Entity>,
    macro_assigned: Vec<Option<(i32, i32)>>,
}

/// A cached tile with BiomeMap data and texture.
struct CachedTile {
    biome_map: Arc<BiomeMap>,
    texture: Handle<Image>,
    last_used_frame: u64,
}

/// Tile cache for macro tiles.
#[derive(Resource)]
struct TileCache {
    macro_tiles: HashMap<(i32, i32), CachedTile>,
    macro_max: usize,
    frame: u64,
}

impl Default for TileCache {
    fn default() -> Self {
        Self {
            macro_tiles: HashMap::new(),
            macro_max: MACRO_CACHE_MAX,
            frame: 0,
        }
    }
}

impl TileCache {
    fn insert_macro(&mut self, coord: (i32, i32), tile: CachedTile) {
        if self.macro_tiles.len() >= self.macro_max {
            if let Some((&evict_coord, _)) = self.macro_tiles.iter()
                .min_by_key(|(_, t)| t.last_used_frame)
            {
                self.macro_tiles.remove(&evict_coord);
            }
        }
        self.macro_tiles.insert(coord, tile);
    }

}

/// An in-flight async tile generation task.
struct InFlightTile {
    coord: (i32, i32),
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

/// Queue of in-flight tile requests.
#[derive(Resource, Default)]
struct TileRequestQueue {
    in_flight: Vec<InFlightTile>,
}

// ─── Meso Launcher Types ─────────────────────────────────────────────────────

/// Marker for the enlarged macro chunk sprite in the launcher.
#[derive(Component)]
struct LauncherMacroSprite;

/// Marker for meso tile sprites in the launcher grid.
#[derive(Component)]
struct LauncherMesoSprite;

/// Marker for the meso tile highlight overlay.
#[derive(Component)]
struct MesoHighlight;

/// Cache of generated meso tiles for the launcher.
#[derive(Resource, Default)]
struct MesoTileCache {
    tiles: HashMap<(i32, i32), MesoCachedTile>,
    sprite_entities: Vec<Entity>,
}

struct MesoCachedTile {
    biome_map: Arc<BiomeMap>,
    texture: Handle<Image>,
}

/// State for async meso tile pre-generation.
#[derive(Resource)]
struct MesoPregenState {
    total: usize,
    completed: usize,
    remaining: Vec<(i32, i32)>,
    in_flight: Vec<MesoInFlightTile>,
}

struct MesoInFlightTile {
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

// ─── Micro Launcher Types ─────────────────────────────────────────────────────

/// Marker for micro tile sprites in the launcher grid.
#[derive(Component)]
struct LauncherMicroSprite;

/// Marker for the micro tile highlight overlay.
#[derive(Component)]
struct MicroHighlight;

/// Cache of generated micro tiles for the launcher.
#[derive(Resource, Default)]
struct MicroTileCache {
    tiles: HashMap<(i32, i32), MicroCachedTile>,
    sprite_entities: Vec<Entity>,
}

struct MicroCachedTile {
    biome_map: Arc<BiomeMap>,
    texture: Handle<Image>,
}

/// State for async micro tile pre-generation.
#[derive(Resource)]
struct MicroPregenState {
    total: usize,
    completed: usize,
    remaining: Vec<(i32, i32)>,
    in_flight: Vec<MicroInFlightTile>,
}

struct MicroInFlightTile {
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

/// Application phase - config, generating, pre-generating macro tiles, or ready.
#[derive(States, Default, Clone, Eq, PartialEq, Hash, Debug)]
enum AppPhase {
    #[default]
    Config,
    Generating,          // Generate macro biome data
    GeneratingMacro,     // Pre-generate all 128 macro tiles
    GeneratingLifeGen,   // Run civilisation generation pipeline
    SavingArtifact,      // Prompt for tag name and save via rb_artifacts
    LoadingArtifact,     // Load layers from disk, populate resources, jump to Ready
    Ready,
}

/// Post-generation processing phase (one step per frame for UI updates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MacroPostPhase {
    Generating,
    RefreshingRivers,
    Normalizing,
    SavingDebugLayers,
    BuildingTerrain,
}

/// Tracks progress of macro tile pre-generation.
#[derive(Resource)]
struct MacroPregenState {
    total: usize,
    completed: usize,
    remaining: Vec<(i32, i32)>,
    in_flight: Vec<InFlightTile>,
    post_phase: MacroPostPhase,
}

// ─── Systems ─────────────────────────────────────────────────────────────────

fn setup_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

/// Config UI - seed input and Generate button.
fn config_ui(
    mut contexts: EguiContexts,
    mut params: ResMut<GeneratorParams>,
    mut next_phase: ResMut<NextState<AppPhase>>,
    mut commands: Commands,
) {
    let ctx = contexts.ctx_mut().unwrap();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    egui::Window::new("World Generator")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([300.0, 150.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Randlebrot");
                ui.add_space(20.0);

                ui.horizontal(|ui| {
                    ui.label("Seed:");
                    ui.add(egui::DragValue::new(&mut params.seed));
                });

                ui.add_space(20.0);

                if ui.button("Generate World").clicked() {
                    commands.insert_resource(GenerationStarted);
                    next_phase.set(AppPhase::Generating);
                }
            });
        });
}

/// Generate macro map synchronously, create sprites + pool, transition to Ready.
fn start_generation(
    mut commands: Commands,
    mut next_phase: ResMut<NextState<AppPhase>>,
    world_def: Res<WorldDefinition>,
    ui_state: Res<GeneratorUiState>,
) {
    commands.remove_resource::<GenerationStarted>();

    let seed = world_def.seed;
    let width = world_def.width;
    let height = world_def.height;
    let backend = ui_state.backend();

    // Generate macro map synchronously (fast)
    let backend_name = if backend == NoiseBackend::Gpu { "GPU" } else { "CPU" };
    println!("Generating macro map {}x{} ({})...", width, height, backend_name);
    let biome_map = BiomeMap::generate_with_backend(seed, width, height, backend);
    println!("  Macro map generated successfully");

    // Extract the global river network before wrapping in Arc
    if let Some(ref network) = biome_map.river_network {
        commands.insert_resource(GlobalRiverNetwork {
            network: network.clone(),
        });
        println!("  Global river network stored ({} segments)", network.segment_count());
    }

    commands.insert_resource(MacroBiomeData {
        biome_map: Arc::new(biome_map),
    });

    // Spawn chunk highlight (follows cursor)
    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 1.0, 0.8, 0.3),
            custom_size: Some(Vec2::splat(CHUNK_SIZE)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.5),
        ChunkHighlight,
    ));

    // Spawn persistent selection highlight (shows selected chunk)
    commands.spawn((
        Sprite {
            color: Color::srgba(0.2, 0.8, 1.0, 0.45),
            custom_size: Some(Vec2::splat(CHUNK_SIZE)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.4),
        Visibility::Hidden,
        ChunkSelectionHighlight,
    ));

    // Province highlight overlay (CivGenerator mode) — full-world-sized, hidden until hovered
    commands.spawn((
        Sprite {
            color: Color::WHITE,
            custom_size: Some(Vec2::new(MAP_WIDTH as f32, MAP_HEIGHT as f32)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.6),
        Visibility::Hidden,
        ProvinceHighlightSprite,
    ));

    // Settlement proximity marker (CivGenerator mode) — small yellow marker
    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 1.0, 0.3, 0.8),
            custom_size: Some(Vec2::splat(4.0)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.7),
        Visibility::Hidden,
        SettlementHighlightSprite,
    ));

    // Spawn sprite pool (macro + meso)
    let mut macro_sprites = Vec::with_capacity(MACRO_POOL_SIZE);
    let mut macro_assigned = Vec::with_capacity(MACRO_POOL_SIZE);
    for _i in 0..MACRO_POOL_SIZE {
        let entity = commands.spawn((
            Sprite { ..default() },
            Transform::from_xyz(-10000.0, -10000.0, 0.1),
            Visibility::Hidden,
            PoolSlot,
        )).id();
        macro_sprites.push(entity);
        macro_assigned.push(None);
    }

    commands.insert_resource(SpritePool {
        macro_sprites,
        macro_assigned,
    });
    commands.insert_resource(TileCache::default());
    commands.insert_resource(TileRequestQueue::default());

    // Queue all 128 macro tiles for pre-generation
    let chunks_x = (world_def.width as f32 / CHUNK_SIZE).ceil() as i32;
    let chunks_y = (world_def.height as f32 / CHUNK_SIZE).ceil() as i32;
    let mut remaining: Vec<(i32, i32)> = Vec::new();
    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            remaining.push((cx, cy));
        }
    }
    let total = remaining.len();
    commands.insert_resource(MacroPregenState {
        total,
        completed: 0,
        remaining,
        in_flight: Vec::new(),
        post_phase: MacroPostPhase::Generating,
    });

    next_phase.set(AppPhase::GeneratingMacro);
    println!("Macro map ready. Pre-generating {} macro tiles...", total);
}

// ─── Macro Pre-generation ────────────────────────────────────────────────────

/// Dispatch async tasks for macro tile pre-generation (higher concurrency than streaming).
fn dispatch_macro_pregen(
    mut pregen: ResMut<MacroPregenState>,
    world_textures: Option<Res<MacroBiomeData>>,
    world_def: Res<WorldDefinition>,
    ui_state: Res<GeneratorUiState>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();
    let river_net = global_rivers.map(|r| r.network.clone());

    while pregen.in_flight.len() < MACRO_PREGEN_CONCURRENCY {
        let Some(coord) = pregen.remaining.pop() else { break };
        let macro_map = world_textures.biome_map.clone();
        let river_net_clone = river_net.clone();
        let world_x = coord.0 as f64 * CHUNK_SIZE as f64;
        let world_y = coord.1 as f64 * CHUNK_SIZE as f64;

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let river_ref = river_net_clone.as_ref();
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed,
                world_x,
                world_y,
                CHUNK_SIZE as f64,
                TILE_MAP_SIZE,
                height,
                1, // detail_level = macro (octave_offset)
                None,
                backend,
                Some(&macro_map),
                river_ref,
            );
            (coord, Arc::new(biome_map))
        });

        pregen.in_flight.push(InFlightTile { coord, task });
    }
}

/// Poll all completions during pre-generation (no per-frame budget on loading screen).
fn poll_macro_pregen(
    mut pregen: ResMut<MacroPregenState>,
    mut tile_cache: ResMut<TileCache>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
    mut next_phase: ResMut<NextState<AppPhase>>,
    mut commands: Commands,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
    world_def: Res<WorldDefinition>,
    existing_lifegen: Option<Res<LifeGenData>>,
) {
    let frame = tile_cache.frame;

    let mut i = 0;
    while i < pregen.in_flight.len() {
        if let Some(result) = block_on(poll_once(&mut pregen.in_flight[i].task)) {
            pregen.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(current_layer.0);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            tile_cache.insert_macro(coord, CachedTile {
                biome_map,
                texture,
                last_used_frame: frame,
            });

            pregen.completed += 1;
        } else {
            i += 1;
        }
    }

    if pregen.completed >= pregen.total && pregen.post_phase == MacroPostPhase::Generating {
        println!("All {} macro tiles pre-generated.", pregen.total);
        pregen.post_phase = MacroPostPhase::RefreshingRivers;
        return; // yield frame for UI update
    }

    // Post-generation phases: one step per frame so the progress UI updates
    match pregen.post_phase {
        MacroPostPhase::Generating => {} // still generating tiles
        MacroPostPhase::RefreshingRivers => {
            if let Some(ref global_rivers) = global_rivers {
                use rb_noise::{rasterize_from_network, LOD_THRESHOLD_MACRO, derived, SEA_LEVEL};
                use rb_core::TileType;

                for cy in 0..8i32 {
                    for cx in 0..16i32 {
                        let coord = (cx, cy);
                        let Some(cached) = tile_cache.macro_tiles.get_mut(&coord) else { continue };
                        let Some(biome_map) = Arc::get_mut(&mut cached.biome_map) else { continue };
                        let world_x = cx as f64 * CHUNK_SIZE as f64;
                        let world_y = cy as f64 * CHUNK_SIZE as f64;
                        let new_rivers = rasterize_from_network(
                            &global_rivers.network, world_x, world_y,
                            CHUNK_SIZE as f64, TILE_MAP_SIZE, LOD_THRESHOLD_MACRO,
                        );
                        for idx in 0..biome_map.rivers.len() {
                            biome_map.rivers[idx] = new_rivers[idx];
                            biome_map.water_table[idx] = derived::derive_water_table(
                                new_rivers[idx], biome_map.humidity[idx], biome_map.heightmap[idx],
                                biome_map.precipitation_type[idx], biome_map.continentalness[idx],
                            );
                            if new_rivers[idx] > 0.0
                                && biome_map.continentalness[idx] >= SEA_LEVEL
                                && biome_map.temperature[idx] > -10.0
                                && biome_map.temperature[idx] < 70.0
                            {
                                biome_map.biomes[idx] = TileType::River;
                            }
                        }
                        let image_data = biome_map.to_layer_image(current_layer.0);
                        cached.texture = images.add(create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data));
                    }
                }
            }
            pregen.post_phase = MacroPostPhase::Normalizing;
        }
        MacroPostPhase::Normalizing => {
            let mut hmin = f64::MAX;
            let mut hmax = f64::MIN;
            for cached in tile_cache.macro_tiles.values() {
                for &v in &cached.biome_map.heightmap {
                    if v < hmin { hmin = v; }
                    if v > hmax { hmax = v; }
                }
            }
            let norm_hints = NormalizationHints {
                heightmap_min: if hmin < hmax { hmin } else { 0.0 },
                heightmap_max: if hmin < hmax { hmax } else { 1.0 },
            };
            commands.insert_resource(GlobalNormHints(norm_hints.clone()));

            for cached in tile_cache.macro_tiles.values_mut() {
                let image_data = cached.biome_map.to_layer_image_with_hints(current_layer.0, Some(&norm_hints));
                cached.texture = images.add(create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data));
            }
            pregen.post_phase = MacroPostPhase::SavingDebugLayers;
        }
        MacroPostPhase::SavingDebugLayers => {
            save_stitched_debug_layers(&tile_cache);
            pregen.post_phase = MacroPostPhase::BuildingTerrain;
        }
        MacroPostPhase::BuildingTerrain => {
            let tilebiome_maps: HashMap<(i32, i32), Arc<BiomeMap>> = tile_cache.macro_tiles.iter()
                .map(|(&coord, cached)| (coord, cached.biome_map.clone()))
                .collect();
            let chunks_x = (MAP_WIDTH as f32 / CHUNK_SIZE).ceil() as usize;
            let chunks_y = (MAP_HEIGHT as f32 / CHUNK_SIZE).ceil() as usize;

            commands.insert_resource(StoredTerrainView(
                MesoTerrainView::from_tile_map(&tilebiome_maps, chunks_x, chunks_y, TILE_MAP_SIZE),
            ));

            commands.remove_resource::<MacroPregenState>();

            // If LifeGenData already exists (loaded from artifact), skip LifeGen
            // and go straight to Ready.
            if existing_lifegen.is_some() {
                println!("LifeGenData already loaded from artifact, skipping generation.");
                next_phase.set(AppPhase::Ready);
            } else {
                // Spawn async LifeGen task
                let terrain_view = Arc::new(MesoTerrainView::from_tile_map(
                    &tilebiome_maps, chunks_x, chunks_y, TILE_MAP_SIZE,
                ));
                let civ_seed = world_def.civ_seed;
                let progress = rb_world::lifegen::new_progress();
                let progress_clone = progress.clone();
                let task = AsyncComputeTaskPool::get().spawn(async move {
                    let data = rb_world::lifegen::generate_with_progress(
                        terrain_view.as_ref(), civ_seed, Some(progress_clone),
                    );
                    data.save_debug_layers(std::path::Path::new("debug_layers"));
                    data
                });
                commands.insert_resource(LifeGenTask { task, progress });
                next_phase.set(AppPhase::GeneratingLifeGen);
            }
        }
    }
}

/// Stitch all pre-generated macro tiles into full-world debug PNGs.
/// This saves exactly what the app displays on the world map.
fn save_stitched_debug_layers(tile_cache: &TileCache) {
    use image::{ImageBuffer, Rgba, RgbaImage};
    use rb_noise::NoiseLayer;

    let chunks_x = (MAP_WIDTH as f32 / CHUNK_SIZE).ceil() as usize;  // 16
    let chunks_y = (MAP_HEIGHT as f32 / CHUNK_SIZE).ceil() as usize; // 8
    let full_w = (chunks_x * TILE_MAP_SIZE) as u32; // 8192
    let full_h = (chunks_y * TILE_MAP_SIZE) as u32; // 4096
    let tile_px = TILE_MAP_SIZE as u32;

    let out_dir = std::path::Path::new("debug_layers");
    let base_dir = out_dir.join("base");
    let derived_dir = out_dir.join("derived");
    for dir in [out_dir, &base_dir, &derived_dir] {
        let _ = std::fs::create_dir_all(dir);
    }

    println!("Saving debug layers ({full_w}x{full_h})...");

    for layer in NoiseLayer::all() {
        let name = layer.name();
        let mut full_img: RgbaImage = ImageBuffer::new(full_w, full_h);

        for cy in 0..chunks_y {
            for cx in 0..chunks_x {
                let coord = (cx as i32, cy as i32);
                let Some(cached) = tile_cache.macro_tiles.get(&coord) else { continue };
                let rgba_data = cached.biome_map.to_layer_image(*layer);
                let Some(tile_img) = ImageBuffer::<Rgba<u8>, _>::from_raw(tile_px, tile_px, rgba_data) else { continue };

                let ox = (cx as u32) * tile_px;
                let oy = (cy as u32) * tile_px;
                for py in 0..tile_px {
                    for px in 0..tile_px {
                        full_img.put_pixel(ox + px, oy + py, *tile_img.get_pixel(px, py));
                    }
                }
            }
        }

        let path = match layer {
            NoiseLayer::Biome => out_dir.join("biome.png"),
            NoiseLayer::Continentalness
            | NoiseLayer::Tectonic
            | NoiseLayer::Humidity
            | NoiseLayer::RockHardness
            | NoiseLayer::LightLevel => base_dir.join(format!("{name}.png")),
            _ => derived_dir.join(format!("{name}.png")),
        };

        // Downscale 2x to keep PNGs under 30MB (8192×4096 → 4096×2048)
        let half_w = full_w / 2;
        let half_h = full_h / 2;
        let mut small_img: RgbaImage = ImageBuffer::new(half_w, half_h);
        for sy in 0..half_h {
            for sx in 0..half_w {
                let x0 = sx * 2;
                let y0 = sy * 2;
                let [r0, g0, b0, a0] = full_img.get_pixel(x0, y0).0;
                let [r1, g1, b1, a1] = full_img.get_pixel(x0 + 1, y0).0;
                let [r2, g2, b2, a2] = full_img.get_pixel(x0, y0 + 1).0;
                let [r3, g3, b3, a3] = full_img.get_pixel(x0 + 1, y0 + 1).0;
                let avg = Rgba([
                    ((r0 as u16 + r1 as u16 + r2 as u16 + r3 as u16) / 4) as u8,
                    ((g0 as u16 + g1 as u16 + g2 as u16 + g3 as u16) / 4) as u8,
                    ((b0 as u16 + b1 as u16 + b2 as u16 + b3 as u16) / 4) as u8,
                    ((a0 as u16 + a1 as u16 + a2 as u16 + a3 as u16) / 4) as u8,
                ]);
                small_img.put_pixel(sx, sy, avg);
            }
        }

        if let Err(e) = small_img.save(&path) {
            eprintln!("Failed to save {}: {e}", path.display());
        } else {
            println!("  Saved {} ({}x{})", path.display(), half_w, half_h);
        }
    }
}

/// Show progress during macro tile pre-generation with phase list.
fn macro_pregen_progress_ui(
    mut contexts: EguiContexts,
    pregen: Res<MacroPregenState>,
) {
    let ctx = contexts.ctx_mut().unwrap();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    // Overall progress: tiles are 0-80%, post-processing phases are 80-100%
    let tile_progress = pregen.completed as f32 / pregen.total.max(1) as f32;
    let post_bonus = match pregen.post_phase {
        MacroPostPhase::Generating => 0.0,
        MacroPostPhase::RefreshingRivers => 0.05,
        MacroPostPhase::Normalizing => 0.10,
        MacroPostPhase::SavingDebugLayers => 0.15,
        MacroPostPhase::BuildingTerrain => 0.19,
    };
    let overall = (tile_progress * 0.80 + post_bonus).min(1.0);

    // Current post-phase index (0 = generating, 1-4 = post phases)
    let post_idx: usize = match pregen.post_phase {
        MacroPostPhase::Generating => 0,
        MacroPostPhase::RefreshingRivers => 1,
        MacroPostPhase::Normalizing => 2,
        MacroPostPhase::SavingDebugLayers => 3,
        MacroPostPhase::BuildingTerrain => 4,
    };

    egui::Window::new("Generating World")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([420.0, 200.0])
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(5.0);
                ui.add(egui::ProgressBar::new(overall).show_percentage());
                ui.add_space(10.0);

                let phases: &[(usize, &str, &str)] = &[
                    (0, "Macro Tiles", "128 terrain tiles at 512x512"),
                    (1, "River Refresh", "Syncing rivers from global network"),
                    (2, "Normalizing", "Global heightmap range + re-render"),
                    (3, "Saving Debug Layers", "Stitching terrain PNGs"),
                    (4, "Building Terrain View", "8192x4096 meso query surface"),
                ];

                for &(idx, name, desc) in phases {
                    let (icon, color) = if idx < post_idx {
                        ("\u{2714}", egui::Color32::from_rgb(100, 200, 100))
                    } else if idx == post_idx {
                        ("\u{25B6}", egui::Color32::from_rgb(255, 220, 80))
                    } else {
                        ("\u{25CB}", egui::Color32::from_rgb(120, 120, 120))
                    };

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icon).color(color).size(13.0));
                        ui.label(egui::RichText::new(name).color(color).strong().size(13.0));
                        if idx == 0 && post_idx == 0 {
                            ui.label(
                                egui::RichText::new(format!(
                                    "— {}/{}",
                                    pregen.completed, pregen.total
                                ))
                                .color(egui::Color32::from_rgb(180, 180, 180))
                                .size(12.0),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(desc)
                                    .color(egui::Color32::from_rgb(90, 90, 90))
                                    .size(11.0),
                            );
                        }
                    });
                }
            });
        });
}

/// Poll async LifeGen task. When complete, insert LifeGenData and transition to
/// SavingArtifact (to prompt the user for a tag) or Ready (if skipping save).
fn poll_lifegen_task(
    mut commands: Commands,
    mut task_res: ResMut<LifeGenTask>,
    mut next_phase: ResMut<NextState<AppPhase>>,
    world_def: Res<WorldDefinition>,
) {
    if let Some(lifegen) = block_on(poll_once(&mut task_res.task)) {
        println!(
            "LifeGen complete: {} provinces, {} factions, {} settlements, {} roads",
            lifegen.provinces.len(),
            lifegen.factions.len(),
            lifegen.settlement_seeds.len(),
            lifegen.road_segments.len(),
        );
        commands.insert_resource(lifegen);
        commands.remove_resource::<LifeGenTask>();

        // Pre-fill the tag input with the world name (sanitised for filesystem).
        let default_tag = sanitize_tag(&world_def.name);
        commands.insert_resource(ArtifactSaveState {
            tag_input: default_tag,
            error: None,
            saving: false,
        });
        next_phase.set(AppPhase::SavingArtifact);
    }
}

/// Show progress during LifeGen generation.
fn lifegen_progress_ui(
    mut contexts: EguiContexts,
    task: Res<LifeGenTask>,
) {
    let ctx = contexts.ctx_mut().unwrap();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    let (phase, label, detail, progress) = {
        let p = task.progress.lock().unwrap();
        (p.phase, p.phase_label.clone(), p.detail.clone(), p.progress)
    };

    egui::Window::new("Generating Civilisation")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([420.0, 180.0])
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.add_space(5.0);
                ui.add(egui::ProgressBar::new(progress).show_percentage());
                ui.add_space(10.0);

                let phase_names = [
                    (1, "Analysis Grids", "Habitability, navigation cost, resources"),
                    (2, "Provinces", "Poisson seeding, Voronoi tessellation, river borders"),
                    (3, "Factions", "Capital placement, territory expansion"),
                    (4, "Settlements", "Site selection within provinces"),
                    (5, "Roads", "MST + A* pathfinding"),
                    (6, "Trade", "Directed trade flow network"),
                ];

                for &(num, name, desc) in &phase_names {
                    let (icon, text_color) = if num < phase {
                        ("\u{2714}", egui::Color32::from_rgb(100, 200, 100)) // checkmark, green
                    } else if num == phase {
                        ("\u{25B6}", egui::Color32::from_rgb(255, 220, 80)) // play arrow, yellow
                    } else {
                        ("\u{25CB}", egui::Color32::from_rgb(120, 120, 120)) // circle, grey
                    };

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icon).color(text_color).size(13.0));
                        ui.label(
                            egui::RichText::new(format!("{name}"))
                                .color(text_color)
                                .strong()
                                .size(13.0),
                        );
                        if num == phase && !detail.is_empty() {
                            ui.label(
                                egui::RichText::new(format!("— {detail}"))
                                    .color(egui::Color32::from_rgb(180, 180, 180))
                                    .size(12.0),
                            );
                        } else if num < phase {
                            ui.label(
                                egui::RichText::new(desc)
                                    .color(egui::Color32::from_rgb(90, 90, 90))
                                    .size(11.0),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new(desc)
                                    .color(egui::Color32::from_rgb(90, 90, 90))
                                    .size(11.0),
                            );
                        }
                    });
                }
            });
        });
}

/// Sanitise a string into a valid artifact tag (alphanumeric + hyphens + underscores).
/// Replaces spaces with hyphens, strips invalid characters, and lowercases.
fn sanitize_tag(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else if c == ' ' {
                '-'
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "world".to_string()
    } else {
        sanitized
    }
}

// ─── Artifact Save (post-generation) ────────────────────────────────────────

/// UI system for the SavingArtifact phase.
/// Shows an egui dialog prompting for a tag name, then saves via rb_artifacts.
fn artifact_save_ui(
    mut contexts: EguiContexts,
    mut save_state: ResMut<ArtifactSaveState>,
    mut next_phase: ResMut<NextState<AppPhase>>,
    mut commands: Commands,
    macro_biome: Option<Res<MacroBiomeData>>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
    lifegen: Option<Res<LifeGenData>>,
    tile_cache: Option<Res<TileCache>>,
    norm_hints: Option<Res<GlobalNormHints>>,
    world_def: Res<WorldDefinition>,
    ui_state: Res<GeneratorUiState>,
) {
    let ctx = contexts.ctx_mut().unwrap();

    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    let mut do_save = false;
    let mut do_skip = false;

    egui::Window::new("Save World Artifact")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([380.0, 160.0])
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.label("Generation complete. Save as artifact?");
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label("Tag:");
                    ui.text_edit_singleline(&mut save_state.tag_input);
                });
                ui.add_space(5.0);

                if let Some(ref err) = save_state.error {
                    ui.label(
                        egui::RichText::new(err)
                            .color(egui::Color32::from_rgb(255, 100, 100))
                            .size(12.0),
                    );
                    ui.add_space(5.0);
                }

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() && !save_state.saving {
                        do_save = true;
                    }
                    if ui.button("Skip").clicked() {
                        do_skip = true;
                    }
                });

                if save_state.saving {
                    ui.add_space(5.0);
                    ui.label("Saving...");
                }
            });
        });

    if do_skip {
        commands.remove_resource::<ArtifactSaveState>();
        next_phase.set(AppPhase::Ready);
        return;
    }

    if do_save {
        let tag = save_state.tag_input.trim().to_string();

        // Validate tag
        if tag.is_empty() {
            save_state.error = Some("Tag must not be empty".to_string());
            return;
        }
        if !tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            save_state.error = Some(
                "Tag must contain only letters, numbers, hyphens, and underscores".to_string(),
            );
            return;
        }

        save_state.saving = true;
        save_state.error = None;

        // Perform the save synchronously (bincode serialisation is fast).
        let save_result = perform_artifact_save(
            &tag,
            macro_biome.as_deref(),
            global_rivers.as_deref(),
            lifegen.as_deref(),
            tile_cache.as_deref(),
            norm_hints.as_deref(),
            &world_def,
            &ui_state,
        );

        match save_result {
            Ok(path) => {
                println!("Artifact saved to {}", path);
                commands.remove_resource::<ArtifactSaveState>();
                next_phase.set(AppPhase::Ready);
            }
            Err(err) => {
                save_state.saving = false;
                save_state.error = Some(err);
            }
        }
    }
}

/// Perform the actual artifact save. Returns the artifact path on success.
fn perform_artifact_save(
    tag: &str,
    macro_biome: Option<&MacroBiomeData>,
    global_rivers: Option<&GlobalRiverNetwork>,
    lifegen: Option<&LifeGenData>,
    tile_cache: Option<&TileCache>,
    norm_hints: Option<&GlobalNormHints>,
    world_def: &WorldDefinition,
    ui_state: &GeneratorUiState,
) -> Result<String, String> {
    let store = rb_artifacts::ArtifactStore::new()
        .map_err(|e| format!("Failed to initialise artifact store: {e}"))?;

    // If artifact already exists, overwrite it (the user chose this tag in a dialog).
    if store.exists(rb_artifacts::ArtifactKind::Layers, tag) {
        store
            .delete(rb_artifacts::ArtifactKind::Layers, tag)
            .map_err(|e| format!("Failed to remove existing artifact '{tag}': {e}"))?;
    }

    let macro_biome = macro_biome.ok_or("No macro BiomeMap available for saving")?;
    let lifegen = lifegen.ok_or("No LifeGenData available for saving")?;

    // Build the river network for serialisation.
    // The GlobalRiverNetwork holds an Arc; we need an owned reference for save_layers.
    let empty_river_network: rb_noise::RiverNetwork =
        ron::de::from_str("(segments: [], lakes: [])").map_err(|e| format!("Internal error: {e}"))?;
    let river_network_ref = match global_rivers {
        Some(gr) => &*gr.network,
        None => &empty_river_network,
    };

    // Stitch layer images from the tile cache (same as save_stitched_debug_layers
    // but producing the HashMap format that save_layers expects).
    let images = if let (Some(cache), Some(hints)) = (tile_cache, norm_hints) {
        stitch_layer_images_for_artifact(cache, &hints.0)
    } else {
        HashMap::new()
    };

    let layer_image_names: Vec<String> = {
        let mut names: Vec<String> = images.keys().cloned().collect();
        names.sort();
        names
    };

    let backend_label = if ui_state.use_gpu { "gpu" } else { "cpu" };

    let manifest = rb_artifacts::LayerManifest {
        seed: world_def.seed,
        civ_seed: world_def.civ_seed,
        created: chrono::Utc::now().to_rfc3339(),
        world_width: world_def.width as u32,
        world_height: world_def.height as u32,
        backend: backend_label.to_string(),
        layer_images: layer_image_names,
    };

    store
        .save_layers(tag, &macro_biome.biome_map, river_network_ref, lifegen, &images, &manifest)
        .map_err(|e| format!("Failed to save artifact: {e}"))?;

    let artifact_path = store.base_path().join("layers").join(tag);
    Ok(artifact_path.display().to_string())
}

/// Stitch tile cache into layer PNGs for artifact saving.
/// Returns a map of filename -> (width, height, rgba_bytes).
fn stitch_layer_images_for_artifact(
    tile_cache: &TileCache,
    norm_hints: &NormalizationHints,
) -> HashMap<String, (u32, u32, Vec<u8>)> {
    use rb_noise::NoiseLayer;

    let chunks_x = (MAP_WIDTH as f32 / CHUNK_SIZE).ceil() as usize;
    let chunks_y = (MAP_HEIGHT as f32 / CHUNK_SIZE).ceil() as usize;
    let full_w = (chunks_x * TILE_MAP_SIZE) as u32;
    let full_h = (chunks_y * TILE_MAP_SIZE) as u32;
    let tile_px = TILE_MAP_SIZE as u32;
    let half_w = full_w / 2;
    let half_h = full_h / 2;

    let mut images: HashMap<String, (u32, u32, Vec<u8>)> = HashMap::new();

    for layer in NoiseLayer::all() {
        let stride_px = full_w as usize;
        let mut full: Vec<u8> = vec![0u8; (full_w as usize) * (full_h as usize) * 4];

        for cy in 0..chunks_y {
            for cx in 0..chunks_x {
                let coord = (cx as i32, cy as i32);
                let Some(cached) = tile_cache.macro_tiles.get(&coord) else { continue };
                let rgba = cached.biome_map.to_layer_image_with_hints(*layer, Some(norm_hints));
                if rgba.len() != (tile_px as usize) * (tile_px as usize) * 4 {
                    continue;
                }
                let ox = cx * TILE_MAP_SIZE;
                let oy = cy * TILE_MAP_SIZE;
                for py in 0..TILE_MAP_SIZE {
                    let src_row = py * TILE_MAP_SIZE * 4;
                    let dst_row = ((oy + py) * stride_px + ox) * 4;
                    let row_bytes = TILE_MAP_SIZE * 4;
                    full[dst_row..dst_row + row_bytes]
                        .copy_from_slice(&rgba[src_row..src_row + row_bytes]);
                }
            }
        }

        // Downscale 2x (box filter) to 4096x2048.
        let mut small: Vec<u8> = vec![0u8; (half_w as usize) * (half_h as usize) * 4];
        for sy in 0..half_h as usize {
            for sx in 0..half_w as usize {
                let x0 = sx * 2;
                let y0 = sy * 2;
                let idx = |x: usize, y: usize| (y * stride_px + x) * 4;
                let p0 = idx(x0, y0);
                let p1 = idx(x0 + 1, y0);
                let p2 = idx(x0, y0 + 1);
                let p3 = idx(x0 + 1, y0 + 1);
                let avg = |a: u8, b: u8, c: u8, d: u8| -> u8 {
                    ((a as u16 + b as u16 + c as u16 + d as u16) / 4) as u8
                };
                let dst = (sy * half_w as usize + sx) * 4;
                small[dst] = avg(full[p0], full[p1], full[p2], full[p3]);
                small[dst + 1] = avg(full[p0 + 1], full[p1 + 1], full[p2 + 1], full[p3 + 1]);
                small[dst + 2] = avg(full[p0 + 2], full[p1 + 2], full[p2 + 2], full[p3 + 2]);
                small[dst + 3] = avg(full[p0 + 3], full[p1 + 3], full[p2 + 3], full[p3 + 3]);
            }
        }

        let file_name = commands::generate_layers::layer_file_name(*layer);
        images.insert(file_name, (half_w, half_h, small));
    }

    images
}

// ─── Artifact Load (from CLI tag) ────────────────────────────────────────────

/// System that runs during AppPhase::LoadingArtifact.
/// Loads the layer artifact from disk, populates Bevy resources, and transitions
/// to AppPhase::GeneratingMacro (to re-generate the 128 macro tile textures
/// from the loaded BiomeMap, since textures cannot be serialised).
fn artifact_load_system(
    mut commands: Commands,
    mut next_phase: ResMut<NextState<AppPhase>>,
    load_tag: Option<Res<LoadLayersTag>>,
    mut contexts: EguiContexts,
) {
    // Show a loading screen while we work.
    let ctx = contexts.ctx_mut().unwrap();
    egui::CentralPanel::default()
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(30, 30, 30)))
        .show(ctx, |_| {});

    let Some(load_tag) = load_tag else {
        // No tag provided — should not happen, but fall back to Config.
        next_phase.set(AppPhase::Config);
        return;
    };
    let tag = load_tag.0.clone();

    egui::Window::new("Loading Artifact")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([380.0, 80.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(format!("Loading layer artifact '{tag}'..."));
                ui.add_space(10.0);
                ui.spinner();
            });
        });

    // Load the data from disk.
    let store = match rb_artifacts::ArtifactStore::new() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: failed to initialise artifact store: {e}");
            next_phase.set(AppPhase::Config);
            commands.remove_resource::<LoadLayersTag>();
            return;
        }
    };

    let manifest = match store.load_layer_manifest(&tag) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: failed to load manifest for '{tag}': {e}");
            next_phase.set(AppPhase::Config);
            commands.remove_resource::<LoadLayersTag>();
            return;
        }
    };

    let (mut biome_map, river_network, lifegen) = match store.load_layers_data(&tag) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("error: failed to load layer data for '{tag}': {e}");
            next_phase.set(AppPhase::Config);
            commands.remove_resource::<LoadLayersTag>();
            return;
        }
    };

    println!(
        "Loaded artifact '{tag}': seed={}, civ_seed={}, {}x{}, backend={}",
        manifest.seed, manifest.civ_seed, manifest.world_width, manifest.world_height,
        manifest.backend,
    );
    println!(
        "  BiomeMap: {}x{}, LifeGen: {} provinces, {} factions, {} settlements, {} roads",
        biome_map.width, biome_map.height,
        lifegen.provinces.len(), lifegen.factions.len(),
        lifegen.settlement_seeds.len(), lifegen.road_segments.len(),
    );

    // Reconnect the river network Arc to the BiomeMap (it is serde(skip)).
    let river_arc = Arc::new(river_network);
    biome_map.river_network = Some(river_arc.clone());

    // Update WorldDefinition from the manifest.
    commands.insert_resource(WorldDefinition {
        seed: manifest.seed,
        civ_seed: manifest.civ_seed,
        width: manifest.world_width as usize,
        height: manifest.world_height as usize,
        name: tag.clone(),
        ..Default::default()
    });

    // Insert core resources.
    commands.insert_resource(GlobalRiverNetwork {
        network: river_arc,
    });
    commands.insert_resource(MacroBiomeData {
        biome_map: Arc::new(biome_map),
    });
    commands.insert_resource(lifegen);
    commands.insert_resource(GenerationStarted);

    // Spawn chunk highlight (follows cursor).
    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 1.0, 0.8, 0.3),
            custom_size: Some(Vec2::splat(CHUNK_SIZE)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.5),
        ChunkHighlight,
    ));

    // Spawn persistent selection highlight.
    commands.spawn((
        Sprite {
            color: Color::srgba(0.2, 0.8, 1.0, 0.45),
            custom_size: Some(Vec2::splat(CHUNK_SIZE)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.4),
        Visibility::Hidden,
        ChunkSelectionHighlight,
    ));

    // Province highlight overlay.
    commands.spawn((
        Sprite {
            color: Color::WHITE,
            custom_size: Some(Vec2::new(MAP_WIDTH as f32, MAP_HEIGHT as f32)),
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.6),
        Visibility::Hidden,
        ProvinceHighlightSprite,
    ));

    // Settlement proximity marker.
    commands.spawn((
        Sprite {
            color: Color::srgba(1.0, 1.0, 0.3, 0.8),
            custom_size: Some(Vec2::splat(4.0)),
            ..default()
        },
        Transform::from_xyz(-10000.0, -10000.0, 0.7),
        Visibility::Hidden,
        SettlementHighlightSprite,
    ));

    // Spawn sprite pool.
    let mut macro_sprites = Vec::with_capacity(MACRO_POOL_SIZE);
    let mut macro_assigned = Vec::with_capacity(MACRO_POOL_SIZE);
    for _i in 0..MACRO_POOL_SIZE {
        let entity = commands.spawn((
            Sprite { ..default() },
            Transform::from_xyz(-10000.0, -10000.0, 0.1),
            Visibility::Hidden,
            PoolSlot,
        )).id();
        macro_sprites.push(entity);
        macro_assigned.push(None);
    }
    commands.insert_resource(SpritePool {
        macro_sprites,
        macro_assigned,
    });
    commands.insert_resource(TileCache::default());
    commands.insert_resource(TileRequestQueue::default());

    // Queue all macro tiles for texture generation (we have the BiomeMap data
    // but need to re-generate the 128 tile textures for display).
    let chunks_x = (manifest.world_width as f32 / CHUNK_SIZE).ceil() as i32;
    let chunks_y = (manifest.world_height as f32 / CHUNK_SIZE).ceil() as i32;
    let mut remaining: Vec<(i32, i32)> = Vec::new();
    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            remaining.push((cx, cy));
        }
    }
    let total = remaining.len();
    commands.insert_resource(MacroPregenState {
        total,
        completed: 0,
        remaining,
        in_flight: Vec::new(),
        post_phase: MacroPostPhase::Generating,
    });

    commands.remove_resource::<LoadLayersTag>();
    // Transition to GeneratingMacro to re-generate tile textures from the loaded data.
    // The LifeGen data is already loaded, so once macro tiles finish the
    // BuildingTerrain post-phase will skip the LifeGen task and go straight to Ready.
    next_phase.set(AppPhase::GeneratingMacro);
    println!("Artifact loaded. Pre-generating {} macro tile textures...", total);
}

// ─── Open / Save As Artifact (editor UI) ───────────────────────────────────

/// Handle the OpenArtifactRequest signal from the editor UI.
/// Inserts `LoadLayersTag` and transitions to `LoadingArtifact` phase.
fn handle_open_artifact_request(
    mut commands: Commands,
    mut next_phase: ResMut<NextState<AppPhase>>,
    request: Res<OpenArtifactRequest>,
) {
    let tag = request.tag.clone();
    println!("Opening artifact '{tag}' from editor UI...");
    commands.insert_resource(LoadLayersTag(tag));
    commands.remove_resource::<OpenArtifactRequest>();
    next_phase.set(AppPhase::LoadingArtifact);
}

/// Handle the SaveAsArtifactRequest signal from the editor UI.
/// Performs the save synchronously reusing `perform_artifact_save`.
fn handle_save_as_artifact_request(
    mut commands: Commands,
    request: Res<SaveAsArtifactRequest>,
    macro_biome: Option<Res<MacroBiomeData>>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
    lifegen: Option<Res<LifeGenData>>,
    tile_cache: Option<Res<TileCache>>,
    norm_hints: Option<Res<GlobalNormHints>>,
    world_def: Res<WorldDefinition>,
    mut ui_state: ResMut<GeneratorUiState>,
) {
    let tag = request.tag.clone();
    commands.remove_resource::<SaveAsArtifactRequest>();

    let save_result = perform_artifact_save(
        &tag,
        macro_biome.as_deref(),
        global_rivers.as_deref(),
        lifegen.as_deref(),
        tile_cache.as_deref(),
        norm_hints.as_deref(),
        &world_def,
        &ui_state,
    );

    match save_result {
        Ok(path) => {
            println!("Artifact saved to {path}");
            ui_state.show_save_as_dialog = false;
            ui_state.save_as_in_progress = false;
            ui_state.save_as_error = None;
            ui_state.save_as_confirm_overwrite = false;
            ui_state.status_message = Some((format!("Saved artifact '{tag}'"), 3.0));
        }
        Err(err) => {
            eprintln!("Save As failed: {err}");
            ui_state.save_as_in_progress = false;
            ui_state.save_as_error = Some(err);
        }
    }
}

/// Sync the `world_ready` flag on `GeneratorUiState` based on whether
/// the app is in `AppPhase::Ready` (i.e. a generated world exists).
fn sync_world_ready_flag(
    phase: Res<State<AppPhase>>,
    mut ui_state: ResMut<GeneratorUiState>,
) {
    let ready = *phase.get() == AppPhase::Ready;
    if ui_state.world_ready != ready {
        ui_state.world_ready = ready;
    }
}

/// Show progress bar during meso tile pre-generation.
fn meso_pregen_progress_ui(
    mut contexts: EguiContexts,
    pregen: Res<MesoPregenState>,
) {
    let ctx = contexts.ctx_mut().unwrap();
    egui::Window::new("Generating Meso Tiles")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([300.0, 80.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let progress = pregen.completed as f32 / pregen.total as f32;
                ui.label(format!("Generating meso tiles: {}/{}", pregen.completed, pregen.total));
                ui.add_space(10.0);
                ui.add(egui::ProgressBar::new(progress).show_percentage());
            });
        });
}

fn create_image(width: usize, height: usize, data: Vec<u8>) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        default(),
    );

    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        ..default()
    });

    image
}


// ─── Tile Streaming ──────────────────────────────────────────────────────────

/// Enqueue tile generation tasks for visible chunks not yet cached.
fn enqueue_and_dispatch_tiles(
    visible_range: Res<VisibleChunkRange>,
    tile_cache: Res<TileCache>,
    mut request_queue: ResMut<TileRequestQueue>,
    world_def: Res<WorldDefinition>,
    world_textures: Option<Res<MacroBiomeData>>,
    ui_state: Res<GeneratorUiState>,
    camera_query: Query<&Transform, With<Camera2d>>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let Ok(camera_transform) = camera_query.single() else { return };
    let camera_pos = camera_transform.translation;

    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();
    let river_net = global_rivers.map(|r| r.network.clone());
    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;

    let in_flight_coords: HashSet<(i32, i32)> = request_queue.in_flight.iter()
        .map(|t| t.coord)
        .collect();

    // Collect needed macro tiles only — meso is generated on demand in LevelLauncher
    let mut needed: Vec<((i32, i32), f32)> = Vec::new();
    for cy in visible_range.min_y..=visible_range.max_y {
        for cx in visible_range.min_x..=visible_range.max_x {
            let coord = (cx, cy);
            if tile_cache.macro_tiles.contains_key(&coord) || in_flight_coords.contains(&coord) {
                continue;
            }
            let sprite_x = cx as f32 * CHUNK_SIZE + CHUNK_SIZE / 2.0 - half_map_width;
            let sprite_y = half_map_height - cy as f32 * CHUNK_SIZE - CHUNK_SIZE / 2.0;
            let dist = (camera_pos.x - sprite_x).powi(2) + (camera_pos.y - sprite_y).powi(2);
            needed.push((coord, dist));
        }
    }

    needed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let macro_map = world_textures.biome_map.clone();
    for (coord, _dist) in needed {
        if request_queue.in_flight.len() >= MAX_CONCURRENT_TILES {
            break;
        }

        let macro_map_clone = macro_map.clone();
        let river_net_clone = river_net.clone();
        let world_x = coord.0 as f64 * CHUNK_SIZE as f64;
        let world_y = coord.1 as f64 * CHUNK_SIZE as f64;

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let river_ref = river_net_clone.as_ref();
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed,
                world_x,
                world_y,
                CHUNK_SIZE as f64,
                TILE_MAP_SIZE,
                height,
                1, // macro detail level
                None,
                backend,
                Some(&macro_map_clone),
                river_ref,
            );
            (coord, Arc::new(biome_map))
        });

        request_queue.in_flight.push(InFlightTile { coord, task });
    }
}

/// Poll in-flight tile tasks, up to POLL_BUDGET completions per frame.
fn poll_tile_results(
    mut request_queue: ResMut<TileRequestQueue>,
    mut tile_cache: ResMut<TileCache>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
    norm_hints: Option<Res<GlobalNormHints>>,
) {
    tile_cache.frame += 1;
    let frame = tile_cache.frame;
    let mut completed = 0;

    let mut i = 0;
    while i < request_queue.in_flight.len() && completed < POLL_BUDGET {
        if let Some(result) = block_on(poll_once(&mut request_queue.in_flight[i].task)) {
            request_queue.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let hints_ref = norm_hints.as_ref().map(|h| &h.0);
            let image_data = biome_map.to_layer_image_with_hints(current_layer.0, hints_ref);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            let cached = CachedTile {
                biome_map,
                texture,
                last_used_frame: frame,
            };

            tile_cache.insert_macro(coord, cached);

            completed += 1;
            // Don't increment i — swap_remove shifted an element into position i
        } else {
            i += 1;
        }
    }
}

// ─── Sprite Pool Management ─────────────────────────────────────────────────

/// Assign/unassign pool sprites based on visible tiles and cache contents.
fn manage_tile_sprites(
    visible_range: Res<VisibleChunkRange>,
    mut tile_cache: ResMut<TileCache>,
    mut pool: ResMut<SpritePool>,
    world_def: Res<WorldDefinition>,
    mut sprite_query: Query<(&mut Transform, &mut Sprite, &mut Visibility)>,
) {
    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;
    let frame = tile_cache.frame;

    let mut needed_macro: HashSet<(i32, i32)> = HashSet::new();
    for cy in visible_range.min_y..=visible_range.max_y {
        for cx in visible_range.min_x..=visible_range.max_x {
            needed_macro.insert((cx, cy));
        }
    }

    // Free slots assigned to tiles no longer visible
    for i in 0..pool.macro_assigned.len() {
        if let Some(coord) = pool.macro_assigned[i] {
            if !needed_macro.contains(&coord) || !tile_cache.macro_tiles.contains_key(&coord) {
                pool.macro_assigned[i] = None;
                let entity = pool.macro_sprites[i];
                if let Ok((mut transform, _, mut vis)) = sprite_query.get_mut(entity) {
                    *vis = Visibility::Hidden;
                    transform.translation.x = -10000.0;
                    transform.translation.y = -10000.0;
                }
            }
        }
    }

    let assigned_macro: HashSet<(i32, i32)> = pool.macro_assigned.iter()
        .filter_map(|a| *a)
        .collect();

    for coord in &needed_macro {
        if assigned_macro.contains(coord) { continue; }
        let (tex_handle, custom_size) = {
            let Some(cached) = tile_cache.macro_tiles.get(coord) else { continue };
            (cached.texture.clone(), Vec2::splat(CHUNK_SIZE))
        };

        let Some(slot_idx) = pool.macro_assigned.iter().position(|a| a.is_none()) else { break };
        pool.macro_assigned[slot_idx] = Some(*coord);
        let entity = pool.macro_sprites[slot_idx];

        let (cx, cy) = *coord;
        let sprite_x = cx as f32 * CHUNK_SIZE + CHUNK_SIZE / 2.0 - half_map_width;
        let sprite_y = half_map_height - cy as f32 * CHUNK_SIZE - CHUNK_SIZE / 2.0;

        if let Ok((mut transform, mut sprite, mut vis)) = sprite_query.get_mut(entity) {
            transform.translation = Vec3::new(sprite_x, sprite_y, 0.1);
            sprite.image = tex_handle;
            sprite.custom_size = Some(custom_size);
            *vis = Visibility::Inherited;
        }
    }

    for assignment in pool.macro_assigned.iter() {
        if let Some(coord) = assignment {
            if let Some(cached) = tile_cache.macro_tiles.get_mut(coord) {
                cached.last_used_frame = frame;
            }
        }
    }
}

// ─── Layer Change ────────────────────────────────────────────────────────────

/// Handle layer changes from the UI and re-render cached BiomeMaps.
fn handle_layer_change(
    mut ui_state: ResMut<GeneratorUiState>,
    mut current_layer: ResMut<CurrentLayer>,
    mut images: ResMut<Assets<Image>>,
    mut tile_cache: Option<ResMut<TileCache>>,
    pool: Option<Res<SpritePool>>,
    mut sprite_query: Query<(&PoolSlot, &mut Sprite)>,
    norm_hints: Option<Res<GlobalNormHints>>,
) {
    // Sync current layer to UI state
    ui_state.current_layer = Some(current_layer.0);

    let Some(new_layer) = ui_state.layer_changed.take() else {
        return;
    };

    current_layer.0 = new_layer;

    // Re-render all cached tiles from their BiomeMap data
    if let Some(ref mut cache) = tile_cache {
        // Re-render macro tiles (shrunk maps can only render Biome layer)
        for (_, cached) in cache.macro_tiles.iter_mut() {
            let effective_layer = if cached.biome_map.is_shrunk() {
                rb_noise::NoiseLayer::Biome
            } else {
                new_layer
            };
            let hints_ref = norm_hints.as_ref().map(|h| &h.0);
            let image_data = cached.biome_map.to_layer_image_with_hints(effective_layer, hints_ref);
            let new_image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            cached.texture = images.add(new_image);
        }

        // Update assigned pool sprite images
        if let Some(ref pool) = pool {
            for (i, assignment) in pool.macro_assigned.iter().enumerate() {
                if let Some(coord) = assignment {
                    if let Some(cached) = cache.macro_tiles.get(coord) {
                        let entity = pool.macro_sprites[i];
                        if let Ok((_, mut sprite)) = sprite_query.get_mut(entity) {
                            sprite.image = cached.texture.clone();
                        }
                    }
                }
            }
        }
    }
}

fn log_mode_transition(
    mut events: MessageReader<ModeTransitionEvent>,
) {
    for event in events.read() {
        println!("Mode: {} → {}", event.from.name(), event.to.name());
    }
}

fn highlight_info_ui(
    mut contexts: EguiContexts,
    info: Res<HighlightInfo>,
    cursor_pos: Res<CursorWorldPos>,
    selected_chunk: Option<Res<SelectedChunk>>,
    hovered_province: Res<HoveredProvince>,
    hovered_settlement: Res<HoveredSettlement>,
    lifegen: Option<Res<LifeGenData>>,
    app_mode: Res<State<AppMode>>,
) {
    egui::Window::new("Tile Info")
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 8.0])
        .resizable(false)
        .collapsible(false)
        .show(contexts.ctx_mut().unwrap(), |ui| {
            if let Some(ref sel) = selected_chunk {
                let (sx, sy) = sel.chunk_coord;
                ui.label(format!("Selected: ({sx}, {sy})"));
                ui.label("Press F4 to launch");
                ui.separator();
            }

            if *app_mode.get() == AppMode::CivGenerator {
                // CivGenerator: show province/settlement info
                if let (Some(lifegen), Some(pid)) = (lifegen.as_ref(), hovered_province.province_id) {
                    if let Some(prov) = lifegen.province_by_id(pid) {
                        ui.label(format!("Province #{}", prov.id));
                        ui.label(format!("  Biome: {:?}", prov.biome));
                        ui.label(format!("  Habitability: {:.0}%", prov.habitability * 100.0));
                        ui.label(format!("  Area: {} px", prov.area_px));
                        let mut tags = Vec::new();
                        if prov.is_coastal { tags.push("Coastal"); }
                        if prov.is_river_junction { tags.push("River Junction"); }
                        if !tags.is_empty() {
                            ui.label(format!("  {}", tags.join(" | ")));
                        }
                        ui.label(format!("  Elevation: {:.2}", prov.elevation_mean));
                        // Faction info
                        if let PoliticalState::Claimed { faction_id } = prov.political_state {
                            if let Some(faction) = lifegen.faction_by_id(faction_id) {
                                ui.label(format!("  Faction: {}", faction.name));
                            }
                        }
                        ui.label(format!("  State: {}", prov.political_state.name()));

                        // Settlement info
                        if let Some(sid) = hovered_settlement.settlement_id {
                            if let Some(settlement) = lifegen.settlement_seeds.iter().find(|s| s.id == sid) {
                                ui.separator();
                                ui.label("Settlement (nearby)");
                                ui.label(format!("  Tier: {} ({})", settlement.tier.name(), settlement.size_class.name()));
                                ui.label(format!("  Province: #{}", settlement.province_id));
                            }
                        }
                    } else {
                        ui.label(format!("Province #{} (no data)", pid));
                    }
                } else if !info.active {
                    ui.label("(ocean)");
                } else {
                    ui.label("(no province)");
                }
                ui.separator();
                ui.label(format!("Cursor: ({:.1}, {:.1})", cursor_pos.0.x, cursor_pos.0.y));
                return;
            }

            // WorldGenerator: existing behavior
            if !info.active {
                ui.label("(no tile)");
                return;
            }
            ui.label(format!("Domain: {}", info.domain));
            ui.label(format!("Tier: {}", info.tier));
            ui.label(format!("Tile: ({}, {})", info.tile_coord.0, info.tile_coord.1));
            ui.label(format!("World: ({:.1}, {:.1})", info.world_pos.0, info.world_pos.1));
            ui.label(format!("Cursor: ({:.1}, {:.1})", cursor_pos.0.x, cursor_pos.0.y));
        });
}

/// Regenerate world: clear caches, regenerate macro, let tiles re-stream.
fn regenerate_world(
    mut commands: Commands,
    mut regen_request: ResMut<RegenerationRequest>,
    world_def: Res<WorldDefinition>,
    mut textures: ResMut<MacroBiomeData>,
    mut tile_cache: ResMut<TileCache>,
    mut request_queue: ResMut<TileRequestQueue>,
    mut pool: ResMut<SpritePool>,
    mut sprite_query: Query<(&mut Transform, &mut Visibility), With<PoolSlot>>,
    ui_state: Res<GeneratorUiState>,
    mut next_phase: ResMut<NextState<AppPhase>>,
) {
    if !regen_request.pending {
        return;
    }
    regen_request.pending = false;

    let backend = ui_state.backend();
    let backend_name = if backend == NoiseBackend::Gpu { "GPU" } else { "CPU" };
    println!("Regenerating world map with seed {} ({})...", world_def.seed, backend_name);

    // Clear caches and in-flight tasks
    tile_cache.macro_tiles.clear();
    request_queue.in_flight.clear();

    // Hide all pool sprites
    for i in 0..pool.macro_assigned.len() {
        pool.macro_assigned[i] = None;
        let entity = pool.macro_sprites[i];
        if let Ok((mut transform, mut vis)) = sprite_query.get_mut(entity) {
            *vis = Visibility::Hidden;
            transform.translation.x = -10000.0;
        }
    }
    // Regenerate macro biome data
    let biome_map = Arc::new(BiomeMap::generate_with_backend(world_def.seed, world_def.width, world_def.height, backend));
    println!("  Macro biome data generated successfully");

    textures.biome_map = biome_map;

    // Queue all macro tiles for pre-generation
    let chunks_x = (world_def.width as f32 / CHUNK_SIZE).ceil() as i32;
    let chunks_y = (world_def.height as f32 / CHUNK_SIZE).ceil() as i32;
    let mut remaining: Vec<(i32, i32)> = Vec::new();
    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            remaining.push((cx, cy));
        }
    }
    let total = remaining.len();
    commands.insert_resource(MacroPregenState {
        total,
        completed: 0,
        remaining,
        in_flight: Vec::new(),
        post_phase: MacroPostPhase::Generating,
    });

    next_phase.set(AppPhase::GeneratingMacro);
    println!("World regenerated. Pre-generating {} macro tiles...", total);
}

/// Regenerate lifegen: run the full LifeGen pipeline using stored MesoTerrainView.
fn regenerate_lifegen(
    mut commands: Commands,
    mut regen_request: ResMut<RegenerateLifeGenRequest>,
    world_def: Res<WorldDefinition>,
    stored_terrain: Option<Res<StoredTerrainView>>,
    mut overlay_state: ResMut<LifeGenOverlayState>,
) {
    if !regen_request.pending {
        return;
    }
    regen_request.pending = false;

    let Some(terrain) = stored_terrain else {
        eprintln!("Cannot regenerate lifegen: no stored MesoTerrainView available");
        return;
    };

    let civ_seed = world_def.civ_seed;
    println!("Regenerating lifegen with civ_seed {}...", civ_seed);

    let lifegen = rb_world::lifegen::generate(&terrain.0, civ_seed);
    println!(
        "  LifeGenData populated: {} provinces, {} factions, {} settlements, {} road segments",
        lifegen.provinces.len(),
        lifegen.factions.len(),
        lifegen.settlement_seeds.len(),
        lifegen.road_segments.len(),
    );
    lifegen.save_debug_layers(std::path::Path::new("debug_layers"));
    commands.insert_resource(lifegen);

    overlay_state.data_generation += 1;
    overlay_state.current_layer.clear(); // Force re-render
}

// ─── Lifegen Overlay ─────────────────────────────────────────────────────────

fn manage_lifegen_overlay(
    mut commands: Commands,
    lifegen: Option<Res<LifeGenData>>,
    current_layer: Res<CurrentLifeGenLayer>,
    overlay_flags: Res<rb_editor::OverlayState>,
    mut overlay_state: ResMut<LifeGenOverlayState>,
    mut images: ResMut<Assets<Image>>,
    mut overlay_query: Query<(Entity, &mut Sprite), With<LifeGenOverlay>>,
    world_def: Res<WorldDefinition>,
) {
    let Some(lifegen) = lifegen else {
        return;
    };

    let layer_name = match current_layer.0 {
        LifeGenLayer::Habitability => "habitability",
        LifeGenLayer::NavigationCost => "navigation_cost",
        LifeGenLayer::ResourceDesirability => "resource_desirability",
        LifeGenLayer::Factions | LifeGenLayer::PoliticalState => "factions",
        LifeGenLayer::Prosperity => "habitability",
        LifeGenLayer::Settlements => "settlements",
        _ => "composite",
    };

    // Hash overlay flags to detect changes
    let overlay_hash = (overlay_flags.province_borders as u64)
        | ((overlay_flags.faction_borders as u64) << 1)
        | ((overlay_flags.settlement_icons as u64) << 2)
        | ((overlay_flags.road_network as u64) << 3)
        | ((overlay_flags.trade_routes as u64) << 4);

    let overlay_exists = !overlay_query.is_empty();
    if layer_name == overlay_state.current_layer
        && overlay_hash == overlay_state.overlay_hash
        && overlay_exists
    {
        return;
    }

    let rgba_data = lifegen.to_composited_image(
        layer_name,
        overlay_flags.province_borders,
        overlay_flags.faction_borders,
        overlay_flags.settlement_icons,
        overlay_flags.road_network,
        overlay_flags.trade_routes,
    );
    let image = create_image(lifegen.width, lifegen.height, rgba_data);
    let image_handle = images.add(image);

    if let Ok((_entity, mut sprite)) = overlay_query.single_mut() {
        sprite.image = image_handle;
    } else {
        commands.spawn((
            Sprite {
                image: image_handle,
                custom_size: Some(Vec2::new(
                    world_def.width as f32,
                    world_def.height as f32,
                )),
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.5),
            LifeGenOverlay,
        ));
    }

    overlay_state.current_layer = layer_name.to_string();
    overlay_state.overlay_hash = overlay_hash;
}

fn hide_lifegen_overlay(
    mut overlay_query: Query<&mut Visibility, With<LifeGenOverlay>>,
) {
    for mut vis in &mut overlay_query {
        *vis = Visibility::Hidden;
    }
}

fn show_lifegen_overlay(
    mut overlay_query: Query<&mut Visibility, With<LifeGenOverlay>>,
    mut overlay_state: ResMut<LifeGenOverlayState>,
) {
    for mut vis in &mut overlay_query {
        *vis = Visibility::Inherited;
    }
    overlay_state.current_layer.clear(); // Force re-render
}

// ─── Camera & Input ──────────────────────────────────────────────────────────

fn camera_zoom(
    mut scroll_events: MessageReader<MouseWheel>,
    mut query: Query<&mut Projection, With<Camera2d>>,
    mut contexts: EguiContexts,
) {
    if contexts.ctx_mut().unwrap().is_pointer_over_area() {
        scroll_events.clear();
        return;
    }

    let mut scroll_delta = 0.0;

    for event in scroll_events.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y * 0.1,
            MouseScrollUnit::Pixel => event.y * 0.001,
        };
    }

    if scroll_delta == 0.0 {
        return;
    }

    for mut projection in &mut query {
        let zoom_factor = 1.0 - scroll_delta;
        let new_scale = (ortho_scale(&projection) * zoom_factor).clamp(0.05, 10.0);
        set_ortho_scale(&mut projection, new_scale);
    }
}

fn camera_pan(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut motion_events: MessageReader<bevy::input::mouse::MouseMotion>,
    mut query: Query<(&mut Transform, &Projection), With<Camera2d>>,
    time: Res<Time>,
    mut contexts: EguiContexts,
) {
    let mut pan_delta = Vec2::ZERO;

    let pan_speed = 300.0;
    if keyboard.pressed(KeyCode::ArrowLeft) {
        pan_delta.x -= pan_speed * time.delta_secs();
    }
    if keyboard.pressed(KeyCode::ArrowRight) {
        pan_delta.x += pan_speed * time.delta_secs();
    }
    if keyboard.pressed(KeyCode::ArrowUp) {
        pan_delta.y += pan_speed * time.delta_secs();
    }
    if keyboard.pressed(KeyCode::ArrowDown) {
        pan_delta.y -= pan_speed * time.delta_secs();
    }

    let over_ui = contexts.ctx_mut().unwrap().is_pointer_over_area();
    if mouse.pressed(MouseButton::Left) && !over_ui {
        for event in motion_events.read() {
            pan_delta.x -= event.delta.x;
            pan_delta.y += event.delta.y;
        }
    } else {
        motion_events.clear();
    }

    if pan_delta == Vec2::ZERO {
        return;
    }

    for (mut transform, projection) in &mut query {
        let scale = ortho_scale(projection);
        transform.translation.x += pan_delta.x * scale;
        transform.translation.y += pan_delta.y * scale;
    }
}

fn update_cursor_world_pos(
    windows: Query<&Window>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut cursor_pos: ResMut<CursorWorldPos>,
) {
    let Ok(window) = windows.single() else { return };
    let Some(cursor_screen_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.single() else { return };

    if let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_screen_pos) {
        cursor_pos.0 = world_pos;
    }
}

fn update_chunk_highlight(
    cursor_pos: Res<CursorWorldPos>,
    world_def: Res<WorldDefinition>,
    mut highlight_query: Query<(&mut Transform, &mut Sprite), With<ChunkHighlight>>,
    mut contexts: EguiContexts,
    mut highlight_info: ResMut<HighlightInfo>,
) {
    let Ok((mut highlight_transform, mut highlight_sprite)) = highlight_query.single_mut() else { return };

    if contexts.ctx_mut().unwrap().is_pointer_over_area() {
        highlight_transform.translation.x = -10000.0;
        highlight_info.active = false;
        return;
    }

    let chunk_size = CHUNK_SIZE;
    let tier_name = "Macro";
    highlight_sprite.custom_size = Some(Vec2::splat(chunk_size));

    let half_width = world_def.width as f32 / 2.0;
    let half_height = world_def.height as f32 / 2.0;

    let map_x = cursor_pos.0.x + half_width;
    let map_y = half_height - cursor_pos.0.y;

    if map_x < 0.0 || map_x >= world_def.width as f32 || map_y < 0.0 || map_y >= world_def.height as f32 {
        highlight_transform.translation.x = -10000.0;
        highlight_info.active = false;
        return;
    }

    let chunk_x = (map_x / chunk_size).floor() * chunk_size;
    let chunk_y = (map_y / chunk_size).floor() * chunk_size;

    let tile_ix = (map_x / chunk_size).floor() as i32;
    let tile_iy = (map_y / chunk_size).floor() as i32;

    let world_x = chunk_x + chunk_size / 2.0 - half_width;
    let world_y = half_height - chunk_y - chunk_size / 2.0;

    highlight_transform.translation.x = world_x;
    highlight_transform.translation.y = world_y;

    highlight_info.active = true;
    highlight_info.tile_coord = (tile_ix, tile_iy);
    highlight_info.world_pos = (chunk_x, chunk_y);
    highlight_info.tier = tier_name;
    highlight_info.domain = "Terrain";
}

/// Province + settlement highlight for CivGenerator mode.
/// Replaces chunk highlighting: shows hovered province tinted and nearest settlement marker.
fn update_civ_highlight(
    cursor_pos: Res<CursorWorldPos>,
    world_def: Res<WorldDefinition>,
    lifegen: Option<Res<LifeGenData>>,
    mut hovered_province: ResMut<HoveredProvince>,
    mut hovered_settlement: ResMut<HoveredSettlement>,
    mut highlight_info: ResMut<HighlightInfo>,
    mut contexts: EguiContexts,
    mut chunk_highlight: Query<&mut Transform, With<ChunkHighlight>>,
    mut province_query: Query<(&mut Sprite, &mut Visibility), (With<ProvinceHighlightSprite>, Without<SettlementHighlightSprite>)>,
    mut settlement_query: Query<(&mut Transform, &mut Visibility), (With<SettlementHighlightSprite>, Without<ProvinceHighlightSprite>, Without<ChunkHighlight>)>,
    mut images: ResMut<Assets<Image>>,
) {
    // Hide the macro chunk highlight — not used in CivGenerator
    if let Ok(mut cht) = chunk_highlight.single_mut() {
        cht.translation.x = -10000.0;
    }

    let Some(lifegen) = lifegen else {
        highlight_info.active = false;
        return;
    };

    // If egui has pointer, clear hover state
    if contexts.ctx_mut().unwrap().is_pointer_over_area() {
        hovered_province.province_id = None;
        hovered_settlement.settlement_id = None;
        highlight_info.active = false;
        if let Ok((_, mut vis)) = province_query.single_mut() {
            *vis = Visibility::Hidden;
        }
        if let Ok((_, mut vis)) = settlement_query.single_mut() {
            *vis = Visibility::Hidden;
        }
        return;
    }

    let half_width = world_def.width as f32 / 2.0;
    let half_height = world_def.height as f32 / 2.0;

    // Convert cursor world pos → map coords → lifegen pixel coords
    let map_x = cursor_pos.0.x + half_width;
    let map_y = half_height - cursor_pos.0.y;

    if map_x < 0.0 || map_x >= world_def.width as f32 || map_y < 0.0 || map_y >= world_def.height as f32 {
        hovered_province.province_id = None;
        hovered_settlement.settlement_id = None;
        highlight_info.active = false;
        if let Ok((_, mut vis)) = province_query.single_mut() {
            *vis = Visibility::Hidden;
        }
        if let Ok((_, mut vis)) = settlement_query.single_mut() {
            *vis = Visibility::Hidden;
        }
        return;
    }

    let px = (map_x as f64 * (lifegen.width as f64 / world_def.width as f64)) as usize;
    let py = (map_y as f64 * (lifegen.height as f64 / world_def.height as f64)) as usize;

    let province_id = lifegen.province_at_pixel(px, py);

    // Only regenerate the province highlight texture when province changes
    if province_id != hovered_province.province_id {
        hovered_province.province_id = province_id;

        if let (Ok((mut sprite, mut vis)), Some(pid)) = (province_query.single_mut(), province_id) {
            // Generate downscaled highlight texture (world dimensions: 1024x512)
            let out_w = world_def.width;
            let out_h = world_def.height;
            let lg_w = lifegen.width;
            let lg_h = lifegen.height;
            let mut rgba = vec![0u8; out_w * out_h * 4];

            for oy in 0..out_h {
                let sy = oy * lg_h / out_h;
                for ox in 0..out_w {
                    let sx = ox * lg_w / out_w;
                    let idx = sy * lg_w + sx;
                    if idx < lifegen.province_ids.len() && lifegen.province_ids[idx] == pid {
                        let off = (oy * out_w + ox) * 4;
                        rgba[off] = 255;
                        rgba[off + 1] = 255;
                        rgba[off + 2] = 100;
                        rgba[off + 3] = 80;
                    }
                }
            }

            let image = create_image(out_w, out_h, rgba);
            let handle = images.add(image);
            sprite.image = handle;
            *vis = Visibility::Inherited;
        } else if let Ok((_, mut vis)) = province_query.single_mut() {
            *vis = Visibility::Hidden;
        }
    }

    // Find nearest settlement (80 lifegen px ~ 10 world units)
    let px_f = map_x as f64 * (lifegen.width as f64 / world_def.width as f64);
    let py_f = map_y as f64 * (lifegen.height as f64 / world_def.height as f64);
    let nearest = lifegen.nearest_settlement(px_f, py_f, 80.0);

    if let Some(settlement) = nearest {
        hovered_settlement.settlement_id = Some(settlement.id);
        if let Ok((mut transform, mut vis)) = settlement_query.single_mut() {
            // Convert settlement position (lifegen pixels) to world coords
            let s_world_x = (settlement.position.0 / lifegen.width as f64) * world_def.width as f64;
            let s_world_y = (settlement.position.1 / lifegen.height as f64) * world_def.height as f64;
            transform.translation.x = s_world_x as f32 - half_width;
            transform.translation.y = half_height - s_world_y as f32;
            *vis = Visibility::Inherited;
        }
    } else {
        hovered_settlement.settlement_id = None;
        if let Ok((_, mut vis)) = settlement_query.single_mut() {
            *vis = Visibility::Hidden;
        }
    }

    highlight_info.active = province_id.is_some();
    highlight_info.domain = "Civilization";
    highlight_info.tier = "Province";
}

/// Hide province/settlement highlights when leaving CivGenerator mode.
fn hide_civ_highlights(
    mut province_query: Query<&mut Visibility, (With<ProvinceHighlightSprite>, Without<SettlementHighlightSprite>)>,
    mut settlement_query: Query<&mut Visibility, (With<SettlementHighlightSprite>, Without<ProvinceHighlightSprite>)>,
    mut hovered_province: ResMut<HoveredProvince>,
    mut hovered_settlement: ResMut<HoveredSettlement>,
) {
    if let Ok(mut vis) = province_query.single_mut() {
        *vis = Visibility::Hidden;
    }
    if let Ok(mut vis) = settlement_query.single_mut() {
        *vis = Visibility::Hidden;
    }
    hovered_province.province_id = None;
    hovered_settlement.settlement_id = None;
}

/// Calculate which chunks are visible in the camera viewport.
fn calculate_visible_chunks(
    camera_query: Query<(&Transform, &Projection), With<Camera2d>>,
    windows: Query<&Window>,
    mut visible_range: ResMut<VisibleChunkRange>,
    world_def: Res<WorldDefinition>,
) {
    let Ok((camera_transform, projection)) = camera_query.single() else { return };
    let Ok(window) = windows.single() else { return };

    let camera_pos = camera_transform.translation;
    let scale = ortho_scale(projection);

    let half_viewport_width = (window.width() / 2.0) * scale;
    let half_viewport_height = (window.height() / 2.0) * scale;

    let world_min_x = camera_pos.x - half_viewport_width;
    let world_max_x = camera_pos.x + half_viewport_width;
    let world_min_y = camera_pos.y - half_viewport_height;
    let world_max_y = camera_pos.y + half_viewport_height;

    let half_map_width = world_def.width as f32 / 2.0;
    let half_map_height = world_def.height as f32 / 2.0;

    let map_min_x = world_min_x + half_map_width;
    let map_max_x = world_max_x + half_map_width;
    let map_min_y = half_map_height - world_max_y;
    let map_max_y = half_map_height - world_min_y;

    let padding = 1;
    visible_range.min_x = ((map_min_x / CHUNK_SIZE).floor() as i32 - padding).max(0);
    visible_range.max_x = ((map_max_x / CHUNK_SIZE).ceil() as i32 + padding)
        .min((world_def.width as f32 / CHUNK_SIZE).ceil() as i32 - 1);
    visible_range.min_y = ((map_min_y / CHUNK_SIZE).floor() as i32 - padding).max(0);
    visible_range.max_y = ((map_max_y / CHUNK_SIZE).ceil() as i32 + padding)
        .min((world_def.height as f32 / CHUNK_SIZE).ceil() as i32 - 1);
}

// ─── Level Chunk Streaming ──────────────────────────────────────────────────

/// Level chunk load radius (in level chunks around the player).
const LEVEL_LOAD_RADIUS: i32 = 5;

/// Level chunk unload radius.
const LEVEL_UNLOAD_RADIUS: i32 = 7;

/// Each level chunk is 64 tiles.
const LEVEL_CHUNK_TILES: f32 = 64.0;

/// An in-flight level chunk generation task.
struct LevelChunkTask {
    coord: (i32, i32),
    task: Task<((i32, i32), Arc<BiomeMap>)>,
}

/// Queue of in-flight level chunk generation tasks.
#[derive(Resource, Default)]
struct LevelChunkQueue {
    in_flight: Vec<LevelChunkTask>,
}

/// Hide world map sprites when entering play mode.
fn hide_world_map_on_play(
    pool: Option<ResMut<SpritePool>>,
    mut sprite_query: Query<(&mut Transform, &mut Visibility), With<PoolSlot>>,
    highlight_query: Query<Entity, With<ChunkHighlight>>,
) {
    // Hide all pool sprites
    if let Some(mut pool) = pool {
        for i in 0..pool.macro_assigned.len() {
            pool.macro_assigned[i] = None;
            let entity = pool.macro_sprites[i];
            if let Ok((mut transform, mut vis)) = sprite_query.get_mut(entity) {
                *vis = Visibility::Hidden;
                transform.translation.x = -10000.0;
            }
        }
    }

    // Hide chunk highlight
    for entity in &highlight_query {
        if let Ok((mut transform, mut vis)) = sprite_query.get_mut(entity) {
            *vis = Visibility::Hidden;
            transform.translation.x = -10000.0;
        }
    }
}

/// Load level chunks around the player's position.
fn level_chunk_load_system(
    level: Res<PlayableLevel>,
    player_query: Query<&Transform, With<Player>>,
    loaded_chunks: Res<LoadedChunks>,
    mut queue: ResMut<LevelChunkQueue>,
    world_textures: Option<Res<MacroBiomeData>>,
    ui_state: Res<GeneratorUiState>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let Ok(player_transform) = player_query.single() else { return };

    let player_pos = player_transform.translation;
    let player_chunk_x = (player_pos.x / LEVEL_CHUNK_TILES).floor() as i32;
    let player_chunk_y = ((-player_pos.y) / LEVEL_CHUNK_TILES).floor() as i32; // Y-flip

    let seed = level.seed;
    let height = level.world_height;
    let backend = ui_state.backend();
    let river_net = global_rivers.map(|r| r.network.clone());

    // Collect in-flight coords
    let in_flight_coords: HashSet<(i32, i32)> = queue.in_flight.iter()
        .map(|t| t.coord)
        .collect();

    for dy in -LEVEL_LOAD_RADIUS..=LEVEL_LOAD_RADIUS {
        for dx in -LEVEL_LOAD_RADIUS..=LEVEL_LOAD_RADIUS {
            let cx = player_chunk_x + dx;
            let cy = player_chunk_y + dy;
            let coord = (cx, cy);

            if loaded_chunks.chunks.contains_key(&coord) || in_flight_coords.contains(&coord) {
                continue;
            }

            if queue.in_flight.len() >= MAX_CONCURRENT_TILES {
                return;
            }

            // Map level chunk to world coordinates
            let world_x = level.origin.x + cx as f64 * MICRO_WORLD_SIZE;
            let world_y = level.origin.y + cy as f64 * MICRO_WORLD_SIZE;

            let macro_map = world_textures.biome_map.clone();
            let river_net_clone = river_net.clone();
            let task = AsyncComputeTaskPool::get().spawn(async move {
                let river_ref = river_net_clone.as_ref();
                let biome_map = BiomeMap::generate_meso_full_with_backend(
                    seed,
                    world_x,
                    world_y,
                    MICRO_WORLD_SIZE,
                    TILE_MAP_SIZE,
                    height,
                    3, // micro detail level
                    None,
                    backend,
                    Some(&macro_map),
                    river_ref,
                );
                (coord, Arc::new(biome_map))
            });

            queue.in_flight.push(LevelChunkTask { coord, task });
        }
    }
}

/// Poll completed level chunk tasks and spawn sprites.
fn level_chunk_poll_system(
    mut commands: Commands,
    mut queue: ResMut<LevelChunkQueue>,
    mut loaded_chunks: ResMut<LoadedChunks>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
) {
    let mut completed = 0;
    let mut i = 0;
    while i < queue.in_flight.len() && completed < POLL_BUDGET {
        if let Some(result) = block_on(poll_once(&mut queue.in_flight[i].task)) {
            queue.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(current_layer.0);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            // Position in level space: each chunk is LEVEL_CHUNK_TILES pixels
            // Y is flipped (positive Y = up in Bevy, but chunks grow downward)
            let sprite_x = coord.0 as f32 * LEVEL_CHUNK_TILES + LEVEL_CHUNK_TILES / 2.0;
            let sprite_y = -(coord.1 as f32 * LEVEL_CHUNK_TILES + LEVEL_CHUNK_TILES / 2.0);

            let entity = commands.spawn((
                Sprite {
                    image: texture,
                    custom_size: Some(Vec2::splat(LEVEL_CHUNK_TILES)),
                    ..default()
                },
                Transform::from_xyz(sprite_x, sprite_y, 0.0),
                LevelChunk { coord },
            )).id();

            loaded_chunks.chunks.insert(coord, entity);
            completed += 1;
        } else {
            i += 1;
        }
    }
}

/// Unload level chunks beyond the unload radius.
fn level_chunk_unload_system(
    mut commands: Commands,
    mut loaded_chunks: ResMut<LoadedChunks>,
    player_query: Query<&Transform, With<Player>>,
) {
    let Ok(player_transform) = player_query.single() else { return };

    let player_pos = player_transform.translation;
    let player_chunk_x = (player_pos.x / LEVEL_CHUNK_TILES).floor() as i32;
    let player_chunk_y = ((-player_pos.y) / LEVEL_CHUNK_TILES).floor() as i32;

    let to_remove: Vec<(i32, i32)> = loaded_chunks.chunks.keys()
        .filter(|(cx, cy)| {
            let dx = (cx - player_chunk_x).abs();
            let dy = (cy - player_chunk_y).abs();
            dx > LEVEL_UNLOAD_RADIUS || dy > LEVEL_UNLOAD_RADIUS
        })
        .copied()
        .collect();

    for coord in to_remove {
        if let Some(entity) = loaded_chunks.chunks.remove(&coord) {
            commands.entity(entity).despawn();
        }
    }
}

// ─── Click-to-Select-Chunk ───────────────────────────────────────────────────

/// Click on the world map to select a chunk. Just stores SelectedChunk — no mode switch.
/// The user then presses F4 to enter LevelLauncher, which auto-starts micro generation.
fn click_to_select_chunk(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    world_def: Res<WorldDefinition>,
    mut contexts: EguiContexts,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    if contexts.ctx_mut().unwrap().is_pointer_over_area() {
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else { return };

    let map_x = world_pos.x + (world_def.width as f32 / 2.0);
    let map_y = (world_def.height as f32 / 2.0) - world_pos.y;

    if map_x < 0.0 || map_x >= world_def.width as f32 || map_y < 0.0 || map_y >= world_def.height as f32 {
        return;
    }

    let chunk_x = (map_x as i32) / CHUNK_SIZE_I as i32;
    let chunk_y = (map_y as i32) / CHUNK_SIZE_I as i32;

    let origin = WorldPos::new(
        chunk_x as f64 * CHUNK_SIZE as f64,
        chunk_y as f64 * CHUNK_SIZE as f64,
    );

    commands.insert_resource(SelectedChunk {
        chunk_coord: (chunk_x, chunk_y),
        origin,
    });

    println!("Selected chunk ({}, {}) — press F4 to launch", chunk_x, chunk_y);
}

/// Update the persistent selection highlight to show the selected chunk.
fn update_chunk_selection_highlight(
    selected: Option<Res<SelectedChunk>>,
    world_def: Res<WorldDefinition>,
    mut query: Query<(&mut Transform, &mut Visibility), With<ChunkSelectionHighlight>>,
    app_mode: Res<State<AppMode>>,
) {
    let Ok((mut transform, mut vis)) = query.single_mut() else { return };

    // Hide selection highlight in CivGenerator — province highlight replaces it
    if *app_mode.get() == AppMode::CivGenerator {
        *vis = Visibility::Hidden;
        transform.translation.x = -10000.0;
        return;
    }

    let Some(selected) = selected else {
        *vis = Visibility::Hidden;
        transform.translation.x = -10000.0;
        return;
    };

    let half_width = world_def.width as f32 / 2.0;
    let half_height = world_def.height as f32 / 2.0;

    let (cx, cy) = selected.chunk_coord;
    let sprite_x = cx as f32 * CHUNK_SIZE + CHUNK_SIZE / 2.0 - half_width;
    let sprite_y = half_height - cy as f32 * CHUNK_SIZE - CHUNK_SIZE / 2.0;

    transform.translation.x = sprite_x;
    transform.translation.y = sprite_y;
    *vis = Visibility::Inherited;
}

// ─── Launcher Systems ────────────────────────────────────────────────────────

/// On entering LevelLauncher: hide world map, show the selected macro chunk enlarged.
fn enter_launcher_macro_view(
    mut commands: Commands,
    selected: Option<Res<SelectedChunk>>,
    tile_cache: Option<Res<TileCache>>,
    mut camera_query: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    mut pool_query: Query<&mut Visibility, With<PoolSlot>>,
    mut highlight_query: Query<&mut Visibility, (With<ChunkHighlight>, Without<PoolSlot>, Without<ChunkSelectionHighlight>)>,
    mut selection_query: Query<&mut Visibility, (With<ChunkSelectionHighlight>, Without<PoolSlot>, Without<ChunkHighlight>)>,
) {
    commands.insert_resource(LauncherPhase::MacroView);

    // Hide all world map pool sprites
    for mut vis in &mut pool_query {
        *vis = Visibility::Hidden;
    }
    // Hide chunk highlight and selection highlight
    for mut vis in &mut highlight_query {
        *vis = Visibility::Hidden;
    }
    for mut vis in &mut selection_query {
        *vis = Visibility::Hidden;
    }

    let Some(selected) = selected else { return };
    let Some(tile_cache) = tile_cache else { return };
    let Ok((mut cam_transform, mut projection)) = camera_query.single_mut() else { return };

    // Show the macro tile as a large centered sprite
    if let Some(cached) = tile_cache.macro_tiles.get(&selected.chunk_coord) {
        commands.spawn((
            Sprite {
                image: cached.texture.clone(),
                custom_size: Some(Vec2::splat(CHUNK_SIZE * 8.0)), // 512px display
                ..default()
            },
            Transform::from_xyz(0.0, 0.0, 0.5),
            LauncherMacroSprite,
        ));
    }

    // Center camera
    cam_transform.translation = Vec3::new(0.0, 0.0, cam_transform.translation.z);
    set_ortho_scale(&mut projection, 1.0);
}

/// Handle "Generate Mesomap" — start async generation of 64 meso tiles.
fn handle_generate_meso_request(
    mut commands: Commands,
    selected: Option<Res<SelectedChunk>>,
    macro_sprites: Query<Entity, With<LauncherMacroSprite>>,
) {
    commands.remove_resource::<GenerateMesoRequest>();
    let Some(selected) = selected else { return };

    // Despawn the macro sprite
    for entity in &macro_sprites {
        commands.entity(entity).despawn();
    }

    // Queue 64 meso tiles for generation
    let mut remaining = Vec::with_capacity(64);
    for my in 0..MESO_GRID_SIZE {
        for mx in 0..MESO_GRID_SIZE {
            remaining.push((mx, my));
        }
    }

    commands.insert_resource(MesoPregenState {
        total: remaining.len(),
        completed: 0,
        remaining,
        in_flight: Vec::new(),
    });
    commands.insert_resource(MesoTileCache::default());
    commands.insert_resource(LauncherPhase::GeneratingMeso);

    println!(
        "Generating 64 meso tiles for chunk ({}, {})",
        selected.chunk_coord.0, selected.chunk_coord.1
    );
}

/// Dispatch async meso tile generation tasks.
fn dispatch_meso_pregen(
    mut pregen: ResMut<MesoPregenState>,
    world_textures: Option<Res<MacroBiomeData>>,
    world_def: Res<WorldDefinition>,
    selected: Option<Res<SelectedChunk>>,
    ui_state: Res<GeneratorUiState>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let Some(selected) = selected else { return };

    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();
    let chunk_origin = selected.origin;
    let river_net = global_rivers.map(|r| r.network.clone());

    while pregen.in_flight.len() < MACRO_PREGEN_CONCURRENCY {
        let Some(coord) = pregen.remaining.pop() else { break };
        let macro_map = world_textures.biome_map.clone();
        let river_net_clone = river_net.clone();

        let world_x = chunk_origin.x + coord.0 as f64 * MESO_WORLD_SIZE;
        let world_y = chunk_origin.y + coord.1 as f64 * MESO_WORLD_SIZE;

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let river_ref = river_net_clone.as_ref();
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed, world_x, world_y, MESO_WORLD_SIZE, TILE_MAP_SIZE, height,
                2, // meso detail level
                None, backend, Some(&macro_map),
                river_ref,
            );
            (coord, Arc::new(biome_map))
        });

        pregen.in_flight.push(MesoInFlightTile { task });
    }
}

/// Poll meso tile tasks; when all done, spawn the grid and transition to MesoView.
fn poll_meso_pregen(
    mut commands: Commands,
    mut pregen: ResMut<MesoPregenState>,
    mut meso_cache: ResMut<MesoTileCache>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
) {
    let mut i = 0;
    while i < pregen.in_flight.len() {
        if let Some(result) = block_on(poll_once(&mut pregen.in_flight[i].task)) {
            pregen.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(current_layer.0);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            meso_cache.tiles.insert(coord, MesoCachedTile { biome_map, texture });
            pregen.completed += 1;
        } else {
            i += 1;
        }
    }

    if pregen.completed >= pregen.total {
        println!("All {} meso tiles generated.", pregen.total);
        commands.remove_resource::<MesoPregenState>();

        // Spawn 8×8 grid of meso tile sprites
        let grid_size = MESO_GRID_SIZE as f32 * MESO_TILE_DISPLAY_PX;
        let half_grid = grid_size / 2.0;

        for my in 0..MESO_GRID_SIZE {
            for mx in 0..MESO_GRID_SIZE {
                let coord = (mx, my);
                let Some(cached) = meso_cache.tiles.get(&coord) else { continue };

                let sprite_x = mx as f32 * MESO_TILE_DISPLAY_PX + MESO_TILE_DISPLAY_PX / 2.0 - half_grid;
                let sprite_y = half_grid - my as f32 * MESO_TILE_DISPLAY_PX - MESO_TILE_DISPLAY_PX / 2.0;

                let entity = commands.spawn((
                    Sprite {
                        image: cached.texture.clone(),
                        custom_size: Some(Vec2::splat(MESO_TILE_DISPLAY_PX)),
                        ..default()
                    },
                    Transform::from_xyz(sprite_x, sprite_y, 0.5),
                    LauncherMesoSprite,
                )).id();
                meso_cache.sprite_entities.push(entity);
            }
        }

        // Spawn meso highlight
        commands.spawn((
            Sprite {
                color: Color::srgba(1.0, 1.0, 0.0, 0.3),
                custom_size: Some(Vec2::splat(MESO_TILE_DISPLAY_PX)),
                ..default()
            },
            Transform::from_xyz(-10000.0, -10000.0, 0.8),
            MesoHighlight,
        ));

        commands.insert_resource(LauncherPhase::MesoView);
    }
}

/// Allow zoom in meso view.
fn launcher_camera_zoom(
    mut scroll_events: MessageReader<MouseWheel>,
    mut query: Query<&mut Projection, With<Camera2d>>,
    mut contexts: EguiContexts,
) {
    if contexts.ctx_mut().unwrap().is_pointer_over_area() {
        scroll_events.clear();
        return;
    }

    let mut scroll_delta = 0.0;
    for event in scroll_events.read() {
        scroll_delta += match event.unit {
            MouseScrollUnit::Line => event.y * 0.1,
            MouseScrollUnit::Pixel => event.y * 0.001,
        };
    }
    if scroll_delta == 0.0 { return; }
    for mut projection in &mut query {
        let zoom_factor = 1.0 - scroll_delta;
        let new_scale = (ortho_scale(&projection) * zoom_factor).clamp(0.2, 3.0);
        set_ortho_scale(&mut projection, new_scale);
    }
}

/// Click to select a meso tile in the grid.
fn click_to_select_meso_tile(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    selected_chunk: Option<Res<SelectedChunk>>,
    mut contexts: EguiContexts,
) {
    if !mouse.just_pressed(MouseButton::Left) { return; }
    if contexts.ctx_mut().unwrap().is_pointer_over_area() { return; }
    let Some(selected_chunk) = selected_chunk else { return };

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else { return };

    let grid_size = MESO_GRID_SIZE as f32 * MESO_TILE_DISPLAY_PX;
    let half_grid = grid_size / 2.0;

    let mx = ((world_pos.x + half_grid) / MESO_TILE_DISPLAY_PX).floor() as i32;
    let my = ((half_grid - world_pos.y) / MESO_TILE_DISPLAY_PX).floor() as i32;

    if mx < 0 || mx >= MESO_GRID_SIZE || my < 0 || my >= MESO_GRID_SIZE { return; }

    let origin = WorldPos::new(
        selected_chunk.origin.x + mx as f64 * MESO_WORLD_SIZE,
        selected_chunk.origin.y + my as f64 * MESO_WORLD_SIZE,
    );

    commands.insert_resource(SelectedMesoTile {
        meso_coord: (mx, my),
        origin,
    });

    println!("Selected meso tile ({}, {})", mx, my);
}

/// Update the meso highlight overlay to follow cursor.
fn update_meso_highlight(
    mut highlight_query: Query<&mut Transform, With<MesoHighlight>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    mut contexts: EguiContexts,
) {
    let Ok(mut highlight_tf) = highlight_query.single_mut() else { return };

    if contexts.ctx_mut().unwrap().is_pointer_over_area() {
        highlight_tf.translation.x = -10000.0;
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_screen) = window.cursor_position() else {
        highlight_tf.translation.x = -10000.0;
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_screen) else { return };

    let grid_size = MESO_GRID_SIZE as f32 * MESO_TILE_DISPLAY_PX;
    let half_grid = grid_size / 2.0;

    let mx = ((world_pos.x + half_grid) / MESO_TILE_DISPLAY_PX).floor() as i32;
    let my = ((half_grid - world_pos.y) / MESO_TILE_DISPLAY_PX).floor() as i32;

    if mx < 0 || mx >= MESO_GRID_SIZE || my < 0 || my >= MESO_GRID_SIZE {
        highlight_tf.translation.x = -10000.0;
        return;
    }

    let sprite_x = mx as f32 * MESO_TILE_DISPLAY_PX + MESO_TILE_DISPLAY_PX / 2.0 - half_grid;
    let sprite_y = half_grid - my as f32 * MESO_TILE_DISPLAY_PX - MESO_TILE_DISPLAY_PX / 2.0;
    highlight_tf.translation.x = sprite_x;
    highlight_tf.translation.y = sprite_y;
}

/// Handle "Launch Level" — start async generation of 64 micro tiles.
fn handle_launch_level_request(
    mut commands: Commands,
    selected_meso: Option<Res<SelectedMesoTile>>,
    meso_sprites: Query<Entity, With<LauncherMesoSprite>>,
    meso_highlight: Query<Entity, With<MesoHighlight>>,
) {
    commands.remove_resource::<LaunchLevelRequest>();
    let Some(selected_meso) = selected_meso else { return };

    // Hide meso grid sprites (keep cache for ESC to return)
    for entity in &meso_sprites {
        commands.entity(entity).insert(Visibility::Hidden);
    }
    for entity in &meso_highlight {
        commands.entity(entity).insert(Visibility::Hidden);
    }

    // Queue 64 micro tiles for generation
    let mut remaining = Vec::with_capacity((MICRO_GRID_SIZE * MICRO_GRID_SIZE) as usize);
    for uy in 0..MICRO_GRID_SIZE {
        for ux in 0..MICRO_GRID_SIZE {
            remaining.push((ux, uy));
        }
    }

    commands.insert_resource(MicroPregenState {
        total: remaining.len(),
        completed: 0,
        remaining,
        in_flight: Vec::new(),
    });
    commands.insert_resource(MicroTileCache::default());
    commands.insert_resource(LauncherPhase::GeneratingMicro);

    println!(
        "Generating 64 micro tiles for meso tile ({}, {})",
        selected_meso.meso_coord.0, selected_meso.meso_coord.1
    );
}

/// Re-show meso grid sprites when returning from Playing to MesoView (ESC).
fn reshow_meso_on_return(
    phase: Option<Res<LauncherPhase>>,
    meso_sprites: Query<Entity, With<LauncherMesoSprite>>,
    meso_highlight: Query<Entity, With<MesoHighlight>>,
    mut commands: Commands,
    playing: Option<Res<PlayableLevel>>,
) {
    // Only act when phase just changed to MesoView and no PlayableLevel exists
    let Some(phase) = phase else { return };
    if *phase != LauncherPhase::MesoView || playing.is_some() { return; }
    if !phase.is_changed() { return; }

    for entity in &meso_sprites {
        commands.entity(entity).insert(Visibility::Inherited);
    }
    for entity in &meso_highlight {
        commands.entity(entity).insert(Visibility::Inherited);
    }
}

// ─── Micro Launcher Systems ──────────────────────────────────────────────────

/// Dispatch async micro tile generation tasks.
fn dispatch_micro_pregen(
    mut pregen: ResMut<MicroPregenState>,
    world_textures: Option<Res<MacroBiomeData>>,
    world_def: Res<WorldDefinition>,
    selected_meso: Option<Res<SelectedMesoTile>>,
    ui_state: Res<GeneratorUiState>,
    global_rivers: Option<Res<GlobalRiverNetwork>>,
) {
    let Some(world_textures) = world_textures else { return };
    let Some(selected_meso) = selected_meso else { return };

    let seed = world_def.seed;
    let height = world_def.height as f64;
    let backend = ui_state.backend();
    let meso_origin = selected_meso.origin;
    let river_net = global_rivers.map(|r| r.network.clone());

    while pregen.in_flight.len() < MACRO_PREGEN_CONCURRENCY {
        let Some(coord) = pregen.remaining.pop() else { break };
        let macro_map = world_textures.biome_map.clone();
        let river_net_clone = river_net.clone();

        let world_x = meso_origin.x + coord.0 as f64 * MICRO_WORLD_SIZE;
        let world_y = meso_origin.y + coord.1 as f64 * MICRO_WORLD_SIZE;

        let task = AsyncComputeTaskPool::get().spawn(async move {
            let river_ref = river_net_clone.as_ref();
            let biome_map = BiomeMap::generate_meso_full_with_backend(
                seed, world_x, world_y, MICRO_WORLD_SIZE, TILE_MAP_SIZE, height,
                3, // micro detail level
                None, backend, Some(&macro_map),
                river_ref,
            );
            (coord, Arc::new(biome_map))
        });

        pregen.in_flight.push(MicroInFlightTile { task });
    }
}

/// Poll micro tile tasks; when all done, spawn the grid and transition to MicroView.
fn poll_micro_pregen(
    mut commands: Commands,
    mut pregen: ResMut<MicroPregenState>,
    mut micro_cache: ResMut<MicroTileCache>,
    mut images: ResMut<Assets<Image>>,
    current_layer: Res<CurrentLayer>,
) {
    let mut i = 0;
    while i < pregen.in_flight.len() {
        if let Some(result) = block_on(poll_once(&mut pregen.in_flight[i].task)) {
            pregen.in_flight.swap_remove(i);
            let (coord, biome_map) = result;

            let image_data = biome_map.to_layer_image(current_layer.0);
            let image = create_image(TILE_MAP_SIZE, TILE_MAP_SIZE, image_data);
            let texture = images.add(image);

            micro_cache.tiles.insert(coord, MicroCachedTile { biome_map, texture });
            pregen.completed += 1;
        } else {
            i += 1;
        }
    }

    if pregen.completed >= pregen.total {
        println!("All {} micro tiles generated.", pregen.total);
        commands.remove_resource::<MicroPregenState>();

        // Spawn 32×32 grid of micro tile sprites
        let grid_size = MICRO_GRID_SIZE as f32 * MICRO_TILE_DISPLAY_PX;
        let half_grid = grid_size / 2.0;

        for uy in 0..MICRO_GRID_SIZE {
            for ux in 0..MICRO_GRID_SIZE {
                let coord = (ux, uy);
                let Some(cached) = micro_cache.tiles.get(&coord) else { continue };

                let sprite_x = ux as f32 * MICRO_TILE_DISPLAY_PX + MICRO_TILE_DISPLAY_PX / 2.0 - half_grid;
                let sprite_y = half_grid - uy as f32 * MICRO_TILE_DISPLAY_PX - MICRO_TILE_DISPLAY_PX / 2.0;

                let entity = commands.spawn((
                    Sprite {
                        image: cached.texture.clone(),
                        custom_size: Some(Vec2::splat(MICRO_TILE_DISPLAY_PX)),
                        ..default()
                    },
                    Transform::from_xyz(sprite_x, sprite_y, 0.5),
                    LauncherMicroSprite,
                )).id();
                micro_cache.sprite_entities.push(entity);
            }
        }

        // Spawn micro highlight
        commands.spawn((
            Sprite {
                color: Color::srgba(1.0, 0.5, 0.0, 0.3),
                custom_size: Some(Vec2::splat(MICRO_TILE_DISPLAY_PX)),
                ..default()
            },
            Transform::from_xyz(-10000.0, -10000.0, 0.8),
            MicroHighlight,
        ));

        commands.insert_resource(LauncherPhase::MicroView);
    }
}

/// Show progress bar during micro tile pre-generation.
fn micro_pregen_progress_ui(
    mut contexts: EguiContexts,
    pregen: Res<MicroPregenState>,
) {
    let ctx = contexts.ctx_mut().unwrap();
    egui::Window::new("Generating Micro Tiles")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([300.0, 80.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                let progress = pregen.completed as f32 / pregen.total as f32;
                ui.label(format!("Generating micro tiles: {}/{}", pregen.completed, pregen.total));
                ui.add_space(10.0);
                ui.add(egui::ProgressBar::new(progress).show_percentage());
            });
        });
}

/// Click to select a micro tile in the grid.
fn click_to_select_micro_tile(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    selected_meso: Option<Res<SelectedMesoTile>>,
    mut contexts: EguiContexts,
) {
    if !mouse.just_pressed(MouseButton::Left) { return; }
    if contexts.ctx_mut().unwrap().is_pointer_over_area() { return; }
    let Some(selected_meso) = selected_meso else { return };

    let Ok(window) = windows.single() else { return };
    let Some(cursor_pos) = window.cursor_position() else { return };
    let Ok((camera, camera_transform)) = camera_query.single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else { return };

    let grid_size = MICRO_GRID_SIZE as f32 * MICRO_TILE_DISPLAY_PX;
    let half_grid = grid_size / 2.0;

    let ux = ((world_pos.x + half_grid) / MICRO_TILE_DISPLAY_PX).floor() as i32;
    let uy = ((half_grid - world_pos.y) / MICRO_TILE_DISPLAY_PX).floor() as i32;

    if ux < 0 || ux >= MICRO_GRID_SIZE || uy < 0 || uy >= MICRO_GRID_SIZE { return; }

    let origin = WorldPos::new(
        selected_meso.origin.x + ux as f64 * MICRO_WORLD_SIZE,
        selected_meso.origin.y + uy as f64 * MICRO_WORLD_SIZE,
    );

    commands.insert_resource(SelectedMicroTile {
        micro_coord: (ux, uy),
        origin,
    });

    println!("Selected micro tile ({}, {})", ux, uy);
}

/// Update the micro highlight overlay to follow cursor.
fn update_micro_highlight(
    mut highlight_query: Query<&mut Transform, With<MicroHighlight>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_query: Query<(&Camera, &GlobalTransform)>,
    mut contexts: EguiContexts,
) {
    let Ok(mut highlight_tf) = highlight_query.single_mut() else { return };

    if contexts.ctx_mut().unwrap().is_pointer_over_area() {
        highlight_tf.translation.x = -10000.0;
        return;
    }

    let Ok(window) = windows.single() else { return };
    let Some(cursor_screen) = window.cursor_position() else {
        highlight_tf.translation.x = -10000.0;
        return;
    };
    let Ok((camera, camera_transform)) = camera_query.single() else { return };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_screen) else { return };

    let grid_size = MICRO_GRID_SIZE as f32 * MICRO_TILE_DISPLAY_PX;
    let half_grid = grid_size / 2.0;

    let ux = ((world_pos.x + half_grid) / MICRO_TILE_DISPLAY_PX).floor() as i32;
    let uy = ((half_grid - world_pos.y) / MICRO_TILE_DISPLAY_PX).floor() as i32;

    if ux < 0 || ux >= MICRO_GRID_SIZE || uy < 0 || uy >= MICRO_GRID_SIZE {
        highlight_tf.translation.x = -10000.0;
        return;
    }

    let sprite_x = ux as f32 * MICRO_TILE_DISPLAY_PX + MICRO_TILE_DISPLAY_PX / 2.0 - half_grid;
    let sprite_y = half_grid - uy as f32 * MICRO_TILE_DISPLAY_PX - MICRO_TILE_DISPLAY_PX / 2.0;
    highlight_tf.translation.x = sprite_x;
    highlight_tf.translation.y = sprite_y;
}

/// Handle "Play" — consume StartPlayRequest, hide micro sprites, create PlayableLevel.
fn handle_start_play_request(
    mut commands: Commands,
    selected_micro: Option<Res<SelectedMicroTile>>,
    world_def: Res<WorldDefinition>,
    selected_chunk: Option<Res<SelectedChunk>>,
    micro_sprites: Query<Entity, With<LauncherMicroSprite>>,
    micro_highlight: Query<Entity, With<MicroHighlight>>,
) {
    commands.remove_resource::<StartPlayRequest>();
    let Some(selected_micro) = selected_micro else { return };
    let Some(selected_chunk) = selected_chunk else { return };

    // Hide micro grid sprites (keep cache for ESC to return)
    for entity in &micro_sprites {
        commands.entity(entity).insert(Visibility::Hidden);
    }
    for entity in &micro_highlight {
        commands.entity(entity).insert(Visibility::Hidden);
    }

    commands.insert_resource(PlayableLevel {
        origin: selected_micro.origin,
        chunk_coord: selected_chunk.chunk_coord,
        seed: world_def.seed,
        world_height: world_def.height as f64,
    });
    commands.init_resource::<LoadedChunks>();
    commands.insert_resource(LauncherPhase::Playing);

    println!(
        "Playing at micro tile ({}, {}), world ({:.2}, {:.2})",
        selected_micro.micro_coord.0, selected_micro.micro_coord.1,
        selected_micro.origin.x, selected_micro.origin.y
    );
}

/// ESC in MicroView: despawn micro grid, go back to MesoView.
fn escape_micro_view(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    phase: Option<Res<LauncherPhase>>,
    micro_sprites: Query<Entity, With<LauncherMicroSprite>>,
    micro_highlight: Query<Entity, With<MicroHighlight>>,
) {
    if !keyboard.just_pressed(KeyCode::Escape) { return; }
    if !phase.map_or(false, |p| *p == LauncherPhase::MicroView) { return; }

    for entity in &micro_sprites {
        commands.entity(entity).despawn();
    }
    for entity in &micro_highlight {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<MicroTileCache>();
    commands.remove_resource::<SelectedMicroTile>();
    commands.insert_resource(LauncherPhase::MesoView);

    println!("Returned to MesoView");
}

/// Re-show micro grid sprites when returning from Playing to MicroView (ESC).
fn reshow_micro_on_return(
    phase: Option<Res<LauncherPhase>>,
    micro_sprites: Query<Entity, With<LauncherMicroSprite>>,
    micro_highlight: Query<Entity, With<MicroHighlight>>,
    mut commands: Commands,
    playing: Option<Res<PlayableLevel>>,
) {
    let Some(phase) = phase else { return };
    if *phase != LauncherPhase::MicroView || playing.is_some() { return; }
    if !phase.is_changed() { return; }

    for entity in &micro_sprites {
        commands.entity(entity).insert(Visibility::Inherited);
    }
    for entity in &micro_highlight {
        commands.entity(entity).insert(Visibility::Inherited);
    }
}

/// Handle "Save Level" request: extract the micro BiomeMap from cache,
/// build a LevelManifest with global micro coordinates, and persist via rb_artifacts.
fn handle_save_level_request(
    mut commands: Commands,
    request: Res<SaveLevelRequest>,
    micro_cache: Option<Res<MicroTileCache>>,
    selected_micro: Option<Res<SelectedMicroTile>>,
    selected_meso: Option<Res<SelectedMesoTile>>,
    selected_chunk: Option<Res<SelectedChunk>>,
    world_def: Res<WorldDefinition>,
    mut save_ui: ResMut<SaveLevelUiState>,
) {
    // Always consume the request this frame.
    let tag = request.tag.clone();
    commands.remove_resource::<SaveLevelRequest>();

    // Validate we have all the data we need.
    let (Some(micro_cache), Some(selected_micro), Some(selected_meso), Some(selected_chunk)) =
        (micro_cache, selected_micro, selected_meso, selected_chunk)
    else {
        save_ui.status = Some(("No micro tile data available".to_string(), true));
        return;
    };

    let local_coord = selected_micro.micro_coord;
    let Some(cached_tile) = micro_cache.tiles.get(&local_coord) else {
        save_ui.status = Some((format!("Micro tile ({}, {}) not in cache", local_coord.0, local_coord.1), true));
        return;
    };

    // Convert local launcher coords to global CLI micro coords.
    // global_x = macro_chunk_x * 64 + meso_local_x * 8 + micro_local_x
    // global_y = macro_chunk_y * 64 + meso_local_y * 8 + micro_local_y
    // (64 = CHUNK_SIZE in world units, 8 = MESO_WORLD_SIZE, 1 = MICRO_WORLD_SIZE)
    let (chunk_x, chunk_y) = selected_chunk.chunk_coord;
    let (meso_x, meso_y) = selected_meso.meso_coord;
    let (micro_x, micro_y) = local_coord;
    // CHUNK_SIZE_I (64) = MESO_GRID_SIZE (8) * MICRO_GRID_SIZE (8) micro tiles across a macro chunk.
    let global_micro_x = chunk_x * CHUNK_SIZE_I as i32 + meso_x * MICRO_GRID_SIZE + micro_x;
    let global_micro_y = chunk_y * CHUNK_SIZE_I as i32 + meso_y * MICRO_GRID_SIZE + micro_y;

    let manifest = rb_artifacts::LevelManifest {
        parent_layers_tag: None,
        seed: world_def.seed,
        civ_seed: world_def.civ_seed,
        chunk_coord: (global_micro_x, global_micro_y),
        created: chrono_timestamp(),
    };

    // Save via rb_artifacts.
    match rb_artifacts::ArtifactStore::new() {
        Ok(store) => {
            match store.save_level(&tag, &cached_tile.biome_map, &manifest) {
                Ok(()) => {
                    println!(
                        "Saved level artifact '{}' at global chunk ({}, {})",
                        tag, global_micro_x, global_micro_y,
                    );
                    save_ui.status = Some((format!("Saved as '{tag}'"), false));
                }
                Err(e) => {
                    eprintln!("Failed to save level artifact: {e}");
                    save_ui.status = Some((format!("Save failed: {e}"), true));
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to open artifact store: {e}");
            save_ui.status = Some((format!("Store error: {e}"), true));
        }
    }
}

/// Generate an ISO 8601 timestamp string for manifest creation times.
fn chrono_timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Clean up all launcher-specific entities when leaving LevelLauncher mode.
fn cleanup_launcher_entities(
    mut commands: Commands,
    macro_sprites: Query<Entity, With<LauncherMacroSprite>>,
    meso_sprites: Query<Entity, With<LauncherMesoSprite>>,
    meso_highlight: Query<Entity, With<MesoHighlight>>,
    micro_sprites: Query<Entity, With<LauncherMicroSprite>>,
    micro_highlight: Query<Entity, With<MicroHighlight>>,
    mut pool_query: Query<&mut Visibility, With<PoolSlot>>,
    mut highlight_query: Query<&mut Visibility, (With<ChunkHighlight>, Without<PoolSlot>, Without<ChunkSelectionHighlight>)>,
    mut selection_query: Query<&mut Visibility, (With<ChunkSelectionHighlight>, Without<PoolSlot>, Without<ChunkHighlight>)>,
) {
    for entity in &macro_sprites {
        commands.entity(entity).despawn();
    }
    for entity in &meso_sprites {
        commands.entity(entity).despawn();
    }
    for entity in &meso_highlight {
        commands.entity(entity).despawn();
    }
    for entity in &micro_sprites {
        commands.entity(entity).despawn();
    }
    for entity in &micro_highlight {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<MesoTileCache>();
    commands.remove_resource::<MesoPregenState>();
    commands.remove_resource::<MicroTileCache>();
    commands.remove_resource::<MicroPregenState>();
    commands.remove_resource::<SelectedMicroTile>();

    // Re-show world map pool sprites, highlight, and selection highlight
    for mut vis in &mut pool_query {
        *vis = Visibility::Inherited;
    }
    for mut vis in &mut highlight_query {
        *vis = Visibility::Inherited;
    }
    for mut vis in &mut selection_query {
        *vis = Visibility::Inherited;
    }
}
