//! Canonical CLI coordinate conventions for the Randlebrot world grid.
//!
//! This module is the single source of truth for how the CLI interprets
//! chunk-level coordinates across every command (`generate level`,
//! `view levels <tag>`, etc.). The GUI launcher's
//! `SelectedMicroTile.micro_coord` uses **local** 0–7 coordinates *within*
//! a meso tile, which is a different convention — do not mix them.
//!
//! # World layout
//!
//! The Randlebrot world is **1024 × 512 world units**, organised into a
//! nested chunk hierarchy:
//!
//! | Level | World units per tile | Grid          | Total tiles |
//! |-------|----------------------|---------------|-------------|
//! | Macro | 64 × 64              | 16 × 8        | 128         |
//! | Meso  | 8 × 8                | 128 × 64      | 8192        |
//! | Chunk | 1 × 1                | 1024 × 512    | 524,288     |
//!
//! # CLI chunk coordinate convention
//!
//! A CLI `chunk_coord` is a **global** `(cx, cy)` pair where:
//!
//! * `cx ∈ [0, CHUNK_GRID_WIDTH)` (currently `[0, 1024)`)
//! * `cy ∈ [0, CHUNK_GRID_HEIGHT)` (currently `[0, 512)`)
//!
//! The world position of the tile's top-left corner is
//! `(cx * CHUNK_WORLD_SIZE, cy * CHUNK_WORLD_SIZE)`. With the current
//! `CHUNK_WORLD_SIZE = 1.0`, the global coordinate equals the integer
//! world position, which makes coordinates in CLI invocations directly
//! meaningful without conversion arithmetic:
//!
//! ```text
//! randlebrot generate level my-world 512,256 terminus-village
//! # → samples the 1×1 chunk whose top-left is at world (512, 256)
//! ```
//!
//! # Keeping this in sync with `src/main.rs`
//!
//! The GUI code in `src/main.rs` defines its own constant `MICRO_WORLD_SIZE`
//! (= 1.0, the internal name for the same value as `CHUNK_WORLD_SIZE` here)
//! and world dimensions (`MAP_WIDTH` = 1024, `MAP_HEIGHT` = 512). They must
//! match the values in this module. If `src/main.rs` changes
//! `MICRO_WORLD_SIZE`, update `CHUNK_WORLD_SIZE` below and regenerate any
//! affected level artifacts.

// ─── Core Constants ─────────────────────────────────────────────────────────

/// Full world width in world units.
pub const WORLD_WIDTH: usize = 1024;

/// Full world height in world units.
pub const WORLD_HEIGHT: usize = 512;

/// Size of a single chunk in world units.
///
/// Must stay in sync with `MICRO_WORLD_SIZE` (the internal name) in `src/main.rs`.
pub const CHUNK_WORLD_SIZE: f64 = 1.0;

/// Output resolution (pixels per side) of a generated chunk `BiomeMap`.
///
/// Must stay in sync with `TILE_MAP_SIZE` in `src/main.rs`.
pub const CHUNK_OUTPUT_SIZE: usize = 512;

/// Total chunks along the world x axis (global coordinate space).
pub const CHUNK_GRID_WIDTH: i32 = (WORLD_WIDTH as f64 / CHUNK_WORLD_SIZE) as i32;

/// Total chunks along the world y axis (global coordinate space).
pub const CHUNK_GRID_HEIGHT: i32 = (WORLD_HEIGHT as f64 / CHUNK_WORLD_SIZE) as i32;

// ─── Coordinate Conversion ──────────────────────────────────────────────────

/// Convert a global CLI chunk coordinate `(cx, cy)` to the world position
/// `(world_x, world_y)` of the tile's top-left corner.
///
/// See the module-level docs for the full convention. The CLI pipeline
/// passes the returned `(world_x, world_y)` pair straight to
/// `BiomeMap::generate_meso_full_with_backend` so identical `(seed, coord)`
/// inputs always produce identical terrain.
pub fn chunk_coord_to_world_pos(coord: (i32, i32)) -> (f64, f64) {
    (
        coord.0 as f64 * CHUNK_WORLD_SIZE,
        coord.1 as f64 * CHUNK_WORLD_SIZE,
    )
}

/// Validate a CLI global chunk coordinate against the world grid bounds.
///
/// Returns `Ok(())` if the coordinate is inside `[0, CHUNK_GRID_WIDTH) ×
/// [0, CHUNK_GRID_HEIGHT)`, otherwise returns a descriptive error.
pub fn validate_chunk_coord(coord: (i32, i32)) -> Result<(), CoordError> {
    if coord.0 < 0 || coord.0 >= CHUNK_GRID_WIDTH {
        return Err(CoordError::OutOfRange {
            axis: "x",
            value: coord.0,
            max_exclusive: CHUNK_GRID_WIDTH,
        });
    }
    if coord.1 < 0 || coord.1 >= CHUNK_GRID_HEIGHT {
        return Err(CoordError::OutOfRange {
            axis: "y",
            value: coord.1,
            max_exclusive: CHUNK_GRID_HEIGHT,
        });
    }
    Ok(())
}

// ─── Error Type ─────────────────────────────────────────────────────────────

/// Error returned from [`validate_chunk_coord`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoordError {
    /// A coordinate axis was outside the world grid bounds.
    OutOfRange {
        axis: &'static str,
        value: i32,
        max_exclusive: i32,
    },
}

impl std::fmt::Display for CoordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange {
                axis,
                value,
                max_exclusive,
            } => write!(
                f,
                "chunk {axis} coordinate {value} out of range [0, {max_exclusive})"
            ),
        }
    }
}

impl std::error::Error for CoordError {}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_dimensions_match_world_size() {
        assert_eq!(CHUNK_GRID_WIDTH, 1024);
        assert_eq!(CHUNK_GRID_HEIGHT, 512);
        // Total global chunks
        let total = CHUNK_GRID_WIDTH as i64 * CHUNK_GRID_HEIGHT as i64;
        assert_eq!(total, 524_288);
    }

    #[test]
    fn origin_maps_to_world_zero() {
        assert_eq!(chunk_coord_to_world_pos((0, 0)), (0.0, 0.0));
    }

    #[test]
    fn middle_coord_maps_to_world_middle() {
        assert_eq!(chunk_coord_to_world_pos((512, 256)), (512.0, 256.0));
    }

    #[test]
    fn bottom_right_interior_maps_to_world_1023_511() {
        let max = (CHUNK_GRID_WIDTH - 1, CHUNK_GRID_HEIGHT - 1);
        assert_eq!(chunk_coord_to_world_pos(max), (1023.0, 511.0));
    }

    #[test]
    fn validate_accepts_origin() {
        assert!(validate_chunk_coord((0, 0)).is_ok());
    }

    #[test]
    fn validate_accepts_max_interior() {
        assert!(validate_chunk_coord((CHUNK_GRID_WIDTH - 1, CHUNK_GRID_HEIGHT - 1)).is_ok());
    }

    #[test]
    fn validate_rejects_negative_x() {
        let err = validate_chunk_coord((-1, 0)).unwrap_err();
        assert!(matches!(err, CoordError::OutOfRange { axis: "x", .. }));
    }

    #[test]
    fn validate_rejects_negative_y() {
        let err = validate_chunk_coord((0, -1)).unwrap_err();
        assert!(matches!(err, CoordError::OutOfRange { axis: "y", .. }));
    }

    #[test]
    fn validate_rejects_x_at_limit() {
        let err = validate_chunk_coord((CHUNK_GRID_WIDTH, 0)).unwrap_err();
        assert!(matches!(err, CoordError::OutOfRange { axis: "x", .. }));
    }

    #[test]
    fn validate_rejects_y_at_limit() {
        let err = validate_chunk_coord((0, CHUNK_GRID_HEIGHT)).unwrap_err();
        assert!(matches!(err, CoordError::OutOfRange { axis: "y", .. }));
    }

    #[test]
    fn error_message_contains_axis_and_range() {
        let err = validate_chunk_coord((99_999, 0)).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("x"));
        assert!(msg.contains("99999"));
        assert!(msg.contains("1024"));
    }
}
