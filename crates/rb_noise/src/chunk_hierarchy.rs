use rb_core::{ChunkCoord, DetailLevel, NoiseStrategy};
use std::collections::HashMap;
use std::time::Instant;

/// Configuration for chunk caches.
#[derive(Clone, Debug)]
pub struct CacheConfig {
    pub macro_cache_size: usize,
    pub meso_cache_size: usize,
    pub micro_cache_size: usize,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            macro_cache_size: 64,
            meso_cache_size: 256,
            micro_cache_size: 1024,
        }
    }
}

/// A cached chunk at the micro (finest) detail level.
/// 128×128 samples.
#[derive(Clone)]
pub struct MicroChunk {
    pub coord: ChunkCoord,
    pub data: Vec<f64>,
    pub last_accessed: Instant,
}

impl MicroChunk {
    pub const SIZE: usize = 128;

    pub fn new(coord: ChunkCoord, strategy: &dyn NoiseStrategy, world_offset: (f64, f64)) -> Self {
        let mut data = Vec::with_capacity(Self::SIZE * Self::SIZE);

        for y in 0..Self::SIZE {
            for x in 0..Self::SIZE {
                let world_x = world_offset.0 + x as f64;
                let world_y = world_offset.1 + y as f64;
                let value = strategy.generate(world_x, world_y, DetailLevel::Micro.as_u32());
                data.push(value);
            }
        }

        Self {
            coord,
            data,
            last_accessed: Instant::now(),
        }
    }

    pub fn get(&self, x: usize, y: usize) -> f64 {
        self.data[y * Self::SIZE + x]
    }

    pub fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }
}

/// LRU cache for MicroChunks.
pub struct MicroCache {
    chunks: HashMap<ChunkCoord, MicroChunk>,
    max_size: usize,
}

impl MicroCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            chunks: HashMap::new(),
            max_size,
        }
    }

    pub fn get_or_create(
        &mut self,
        coord: ChunkCoord,
        strategy: &dyn NoiseStrategy,
        world_offset: (f64, f64),
    ) -> &MicroChunk {
        if !self.chunks.contains_key(&coord) {
            self.evict_if_needed();
            let chunk = MicroChunk::new(coord, strategy, world_offset);
            self.chunks.insert(coord, chunk);
        }

        let chunk = self.chunks.get_mut(&coord).unwrap();
        chunk.touch();
        chunk
    }

    fn evict_if_needed(&mut self) {
        if self.chunks.len() >= self.max_size {
            // Find oldest chunk
            if let Some((&oldest_key, _)) = self
                .chunks
                .iter()
                .min_by_key(|(_, chunk)| chunk.last_accessed)
            {
                self.chunks.remove(&oldest_key);
            }
        }
    }
}

/// A cached chunk at the meso (medium) detail level.
/// 64×64 samples, owns a cache of MicroChunks.
#[derive(Clone)]
pub struct MesoChunk {
    pub coord: ChunkCoord,
    pub data: Vec<f64>,
    pub last_accessed: Instant,
}

impl MesoChunk {
    pub const SIZE: usize = 64;

    pub fn new(coord: ChunkCoord, strategy: &dyn NoiseStrategy, world_offset: (f64, f64)) -> Self {
        let mut data = Vec::with_capacity(Self::SIZE * Self::SIZE);

        for y in 0..Self::SIZE {
            for x in 0..Self::SIZE {
                let world_x = world_offset.0 + x as f64;
                let world_y = world_offset.1 + y as f64;
                let value = strategy.generate(world_x, world_y, DetailLevel::Meso.as_u32());
                data.push(value);
            }
        }

        Self {
            coord,
            data,
            last_accessed: Instant::now(),
        }
    }

    pub fn get(&self, x: usize, y: usize) -> f64 {
        self.data[y * Self::SIZE + x]
    }

    pub fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }
}

/// LRU cache for MesoChunks.
pub struct MesoCache {
    chunks: HashMap<ChunkCoord, MesoChunk>,
    max_size: usize,
    micro_cache: MicroCache,
}

impl MesoCache {
    pub fn new(max_size: usize, micro_cache_size: usize) -> Self {
        Self {
            chunks: HashMap::new(),
            max_size,
            micro_cache: MicroCache::new(micro_cache_size),
        }
    }

    pub fn get_or_create(
        &mut self,
        coord: ChunkCoord,
        strategy: &dyn NoiseStrategy,
        world_offset: (f64, f64),
    ) -> &MesoChunk {
        if !self.chunks.contains_key(&coord) {
            self.evict_if_needed();
            let chunk = MesoChunk::new(coord, strategy, world_offset);
            self.chunks.insert(coord, chunk);
        }

        let chunk = self.chunks.get_mut(&coord).unwrap();
        chunk.touch();
        chunk
    }

    pub fn get_micro(
        &mut self,
        coord: ChunkCoord,
        strategy: &dyn NoiseStrategy,
        world_offset: (f64, f64),
    ) -> &MicroChunk {
        self.micro_cache.get_or_create(coord, strategy, world_offset)
    }

    fn evict_if_needed(&mut self) {
        if self.chunks.len() >= self.max_size {
            if let Some((&oldest_key, _)) = self
                .chunks
                .iter()
                .min_by_key(|(_, chunk)| chunk.last_accessed)
            {
                self.chunks.remove(&oldest_key);
            }
        }
    }
}

/// A cached chunk at the macro (coarsest) detail level.
/// 32×32 samples, owns a cache of MesoChunks.
#[derive(Clone)]
pub struct MacroChunk {
    pub coord: ChunkCoord,
    pub data: Vec<f64>,
    pub last_accessed: Instant,
}

impl MacroChunk {
    pub const SIZE: usize = 32;

    pub fn new(coord: ChunkCoord, strategy: &dyn NoiseStrategy, world_offset: (f64, f64)) -> Self {
        let mut data = Vec::with_capacity(Self::SIZE * Self::SIZE);

        for y in 0..Self::SIZE {
            for x in 0..Self::SIZE {
                let world_x = world_offset.0 + x as f64;
                let world_y = world_offset.1 + y as f64;
                let value = strategy.generate(world_x, world_y, DetailLevel::Macro.as_u32());
                data.push(value);
            }
        }

        Self {
            coord,
            data,
            last_accessed: Instant::now(),
        }
    }

    pub fn get(&self, x: usize, y: usize) -> f64 {
        self.data[y * Self::SIZE + x]
    }

    pub fn touch(&mut self) {
        self.last_accessed = Instant::now();
    }
}

/// Top-level chunk manager with hierarchical caching.
/// Owns the macro-level cache and provides access to all detail levels.
pub struct ChunkHierarchy {
    macro_chunks: HashMap<ChunkCoord, MacroChunk>,
    meso_cache: MesoCache,
    config: CacheConfig,
}

impl ChunkHierarchy {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            macro_chunks: HashMap::new(),
            meso_cache: MesoCache::new(config.meso_cache_size, config.micro_cache_size),
            config,
        }
    }

    /// Get or create a macro chunk at the given coordinate.
    pub fn get_macro(
        &mut self,
        coord: ChunkCoord,
        strategy: &dyn NoiseStrategy,
    ) -> &MacroChunk {
        let world_offset = Self::coord_to_world_offset(coord, MacroChunk::SIZE);

        if !self.macro_chunks.contains_key(&coord) {
            self.evict_macro_if_needed();
            let chunk = MacroChunk::new(coord, strategy, world_offset);
            self.macro_chunks.insert(coord, chunk);
        }

        let chunk = self.macro_chunks.get_mut(&coord).unwrap();
        chunk.touch();
        chunk
    }

    /// Get or create a meso chunk at the given coordinate.
    pub fn get_meso(
        &mut self,
        coord: ChunkCoord,
        strategy: &dyn NoiseStrategy,
    ) -> &MesoChunk {
        let world_offset = Self::coord_to_world_offset(coord, MesoChunk::SIZE);
        self.meso_cache.get_or_create(coord, strategy, world_offset)
    }

    /// Get or create a micro chunk at the given coordinate.
    pub fn get_micro(
        &mut self,
        coord: ChunkCoord,
        strategy: &dyn NoiseStrategy,
    ) -> &MicroChunk {
        let world_offset = Self::coord_to_world_offset(coord, MicroChunk::SIZE);
        self.meso_cache.get_micro(coord, strategy, world_offset)
    }

    /// Sample noise at a specific world position and detail level.
    pub fn sample(
        &mut self,
        x: f64,
        y: f64,
        detail_level: DetailLevel,
        strategy: &dyn NoiseStrategy,
    ) -> f64 {
        match detail_level {
            DetailLevel::Macro => {
                let (chunk_coord, local_x, local_y) = Self::world_to_chunk(x, y, MacroChunk::SIZE);
                let chunk = self.get_macro(chunk_coord, strategy);
                chunk.get(local_x, local_y)
            }
            DetailLevel::Meso => {
                let (chunk_coord, local_x, local_y) = Self::world_to_chunk(x, y, MesoChunk::SIZE);
                let chunk = self.get_meso(chunk_coord, strategy);
                chunk.get(local_x, local_y)
            }
            DetailLevel::Micro => {
                let (chunk_coord, local_x, local_y) = Self::world_to_chunk(x, y, MicroChunk::SIZE);
                let chunk = self.get_micro(chunk_coord, strategy);
                chunk.get(local_x, local_y)
            }
        }
    }

    /// Convert chunk coordinate to world offset.
    fn coord_to_world_offset(coord: ChunkCoord, chunk_size: usize) -> (f64, f64) {
        (
            coord.x as f64 * chunk_size as f64,
            coord.y as f64 * chunk_size as f64,
        )
    }

    /// Convert world position to chunk coordinate and local position.
    fn world_to_chunk(x: f64, y: f64, chunk_size: usize) -> (ChunkCoord, usize, usize) {
        let chunk_x = (x / chunk_size as f64).floor() as i32;
        let chunk_y = (y / chunk_size as f64).floor() as i32;

        let local_x = ((x % chunk_size as f64) + chunk_size as f64) as usize % chunk_size;
        let local_y = ((y % chunk_size as f64) + chunk_size as f64) as usize % chunk_size;

        (ChunkCoord::new(chunk_x, chunk_y), local_x, local_y)
    }

    fn evict_macro_if_needed(&mut self) {
        if self.macro_chunks.len() >= self.config.macro_cache_size {
            if let Some((&oldest_key, _)) = self
                .macro_chunks
                .iter()
                .min_by_key(|(_, chunk)| chunk.last_accessed)
            {
                self.macro_chunks.remove(&oldest_key);
            }
        }
    }

    /// Clear all caches.
    pub fn clear(&mut self) {
        self.macro_chunks.clear();
        self.meso_cache = MesoCache::new(
            self.config.meso_cache_size,
            self.config.micro_cache_size,
        );
    }

    /// Get cache statistics for debugging.
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            macro_chunks: self.macro_chunks.len(),
            meso_chunks: self.meso_cache.chunks.len(),
            micro_chunks: self.meso_cache.micro_cache.chunks.len(),
        }
    }
}

/// Statistics about cache usage.
#[derive(Debug, Clone, Copy)]
pub struct CacheStats {
    pub macro_chunks: usize,
    pub meso_chunks: usize,
    pub micro_chunks: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::ContinentalnessStrategy;

    #[test]
    fn chunk_hierarchy_generates_data() {
        let config = CacheConfig::default();
        let mut hierarchy = ChunkHierarchy::new(config);
        let strategy = ContinentalnessStrategy::new(42);

        let coord = ChunkCoord::new(0, 0);
        let chunk = hierarchy.get_macro(coord, &strategy);

        assert_eq!(chunk.data.len(), MacroChunk::SIZE * MacroChunk::SIZE);
    }

    #[test]
    fn sample_returns_cached_value() {
        let config = CacheConfig::default();
        let mut hierarchy = ChunkHierarchy::new(config);
        let strategy = ContinentalnessStrategy::new(42);

        let value1 = hierarchy.sample(50.0, 50.0, DetailLevel::Macro, &strategy);
        let value2 = hierarchy.sample(50.0, 50.0, DetailLevel::Macro, &strategy);

        assert_eq!(value1, value2);
    }
}
