use std::collections::HashMap;
use std::sync::Arc;
use rb_core::{TerrainQuery, TileType};
use crate::BiomeMap;

/// Provides a seamless TerrainQuery over the 16×8 grid of pre-generated
/// meso BiomeMap tiles. Each tile is 512×512 pixels, giving a total
/// resolution of 8192×4096.
pub struct MesoTerrainView {
    /// tiles[chunk_y][chunk_x], dimensions [chunks_y][chunks_x]
    tiles: Vec<Vec<Option<Arc<BiomeMap>>>>,
    tile_size: usize,
    chunks_x: usize,
    chunks_y: usize,
    sea_level: f64,
}

impl MesoTerrainView {
    /// Build from a HashMap of (chunk_x, chunk_y) → Arc<BiomeMap>.
    pub fn from_tile_map(
        tile_map: &HashMap<(i32, i32), Arc<BiomeMap>>,
        chunks_x: usize,
        chunks_y: usize,
        tile_size: usize,
    ) -> Self {
        Self::from_tile_map_with_sea_level(tile_map, chunks_x, chunks_y, tile_size, -0.025)
    }

    /// Build with a custom sea level.
    pub fn from_tile_map_with_sea_level(
        tile_map: &HashMap<(i32, i32), Arc<BiomeMap>>,
        chunks_x: usize,
        chunks_y: usize,
        tile_size: usize,
        sea_level: f64,
    ) -> Self {
        let mut tiles = vec![vec![None; chunks_x]; chunks_y];
        for (&(cx, cy), bm) in tile_map {
            if (cx as usize) < chunks_x && (cy as usize) < chunks_y {
                tiles[cy as usize][cx as usize] = Some(bm.clone());
            }
        }
        Self { tiles, tile_size, chunks_x, chunks_y, sea_level }
    }

    /// Helper: resolve global (x, y) to chunk + local offset, then read a f64 field.
    #[inline]
    fn sample_f64(&self, x: usize, y: usize, field: impl Fn(&BiomeMap, usize) -> f64) -> f64 {
        let cx = x / self.tile_size;
        let cy = y / self.tile_size;
        if cy >= self.chunks_y || cx >= self.chunks_x {
            return 0.0;
        }
        if let Some(ref bm) = self.tiles[cy][cx] {
            let lx = x % self.tile_size;
            let ly = y % self.tile_size;
            let idx = ly * bm.width + lx;
            field(bm, idx)
        } else {
            0.0
        }
    }
}

impl TerrainQuery for MesoTerrainView {
    fn width(&self) -> usize {
        self.chunks_x * self.tile_size
    }

    fn height(&self) -> usize {
        self.chunks_y * self.tile_size
    }

    fn heightmap_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.heightmap.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn biome_at(&self, x: usize, y: usize) -> TileType {
        let cx = x / self.tile_size;
        let cy = y / self.tile_size;
        if cy >= self.chunks_y || cx >= self.chunks_x {
            return TileType::Sea;
        }
        if let Some(ref bm) = self.tiles[cy][cx] {
            let lx = x % self.tile_size;
            let ly = y % self.tile_size;
            let idx = ly * bm.width + lx;
            bm.biomes.get(idx).copied().unwrap_or(TileType::Sea)
        } else {
            TileType::Sea
        }
    }

    fn temperature_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.temperature.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn humidity_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.humidity.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn continentalness_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.continentalness.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn erosion_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.erosion.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn light_level_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.light_level.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn rock_hardness_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.rock_hardness.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn river_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.rivers.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn drainage_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            if idx < bm.drainage_area.len() {
                bm.drainage_area[idx] as f64
            } else {
                0.0
            }
        })
    }

    fn tectonic_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.tectonic.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn peaks_valleys_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.peaks_valleys.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn aridity_at(&self, x: usize, y: usize) -> f64 {
        self.sample_f64(x, y, |bm, idx| {
            bm.aridity.get(idx).copied().unwrap_or(0.0)
        })
    }

    fn slope_at(&self, x: usize, y: usize) -> f64 {
        let w = self.width();
        let h = self.height();
        if w == 0 || h == 0 {
            return 0.0;
        }
        let x_lo = if x > 0 { x - 1 } else { 0 };
        let x_hi = if x + 1 < w { x + 1 } else { w - 1 };
        let y_lo = if y > 0 { y - 1 } else { 0 };
        let y_hi = if y + 1 < h { y + 1 } else { h - 1 };
        let dx = self.heightmap_at(x_hi, y) - self.heightmap_at(x_lo, y);
        let dy = self.heightmap_at(x, y_hi) - self.heightmap_at(x, y_lo);
        (dx * dx + dy * dy).sqrt()
    }

    fn is_ocean(&self, x: usize, y: usize) -> bool {
        self.continentalness_at(x, y) < self.sea_level
    }

    fn is_river(&self, x: usize, y: usize) -> bool {
        self.river_at(x, y) > 0.0
    }
}
