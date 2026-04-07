---
issue: 41
title: "Level launcher block terrain — known rendering issues"
crates: [randlebrot, rb_voxel]
modifies:
  - src/commands/launch.rs
depends_on: [43]  # fBm split fixes the flat terrain, this issue fixes everything else
---

## Goal

Fix the remaining rendering, UX, and performance issues in the Bevy 3D block terrain launcher. Issue #43 (fBm octave split) fixes the flat heightmap; this issue covers everything else.

## Issues to Fix

### 1. Terrain height following

**Current**: Camera Y stays at spawn height as player moves.
**Fix**: Each frame, sample the loaded chunk mesh heights at the player's XZ position, set camera Y = ground_height + EYE_HEIGHT.

```rust
// In camera_input system, after updating world_x/world_z:
// Find the chunk the player is in, get its heightmap, sample at player position
// Set cam_transform.translation.y = sampled_ground + EYE_HEIGHT
```

### 2. HUD not rendering

**Current**: egui HUD system runs but nothing visible with Camera3d.
**Diagnosis needed**: Check if Camera3d + bevy_egui 0.39 requires specific setup (render order, UI camera, etc.). The GUI editor uses Camera2d with egui successfully. May need a separate UI camera entity.

### 3. Chunk seam normalization

**Current**: Each chunk normalizes height independently → visible height discontinuities at chunk boundaries.
**Fix**: Depends on #43. Once the fBm split provides absolute heights, local normalization is removed and seams disappear. If seams persist after #43, add cross-chunk height stitching (sample neighbor chunk edge heights and blend).

### 4. Side face color variation

**Current**: Side faces are uniform dark biome color.
**Fix**: Vary side face color by depth — top block side = grass-tinted, deeper blocks = dirt/stone:
```rust
let depth_below_top = (top_height - current_block_y) as f32;
let side_color = if depth_below_top < 2.0 {
    // Grass/dirt transition
    lerp(biome_color * 0.6, dirt_color, depth_below_top / 2.0)
} else {
    stone_color * 0.5
};
```

### 5. Greedy meshing (performance)

**Current**: Every block emits individual face quads. 64x64 blocks = up to ~25K quads per chunk.
**Fix**: Merge adjacent same-height blocks into larger quads. Standard greedy meshing algorithm:
- For each row, find runs of same-height blocks
- Extend runs vertically where possible
- Emit one quad per merged region

This can 5-10x reduce vertex count.

### 6. Mouse sensitivity

**Current**: `MOUSE_SENS = 0.002` — may feel too fast or too slow.
**Fix**: Tune to feel natural. Typical FPS values: 0.001-0.003. Add scroll wheel adjustment if needed.

### 7. Movement speed

**Current**: `MOVE_SPEED = 2.0` world units/sec.
**Fix**: With 64 blocks per chunk (1 world unit), that's 128 blocks/sec. Minecraft walk speed is ~4.3 blocks/sec. Scale to match: `MOVE_SPEED = 4.3 / 64.0 ≈ 0.067`.

## Verification

Use `randlebrot launch <tag> --flythrough` (issue #42) to verify all fixes visually. Check:
- frame_001: terrain visible from spawn height (not floating in sky)
- frame_004: looking down shows blocks at feet
- frame_006: moving forward shows terrain height changes (camera follows ground)
- No visible chunk seams in any frame
- Side faces show depth-based color variation

## Constraints

- World Rules tests must pass
- FPS must remain >30 at default settings
- No changes to rb_noise generation (that's #43's scope)
