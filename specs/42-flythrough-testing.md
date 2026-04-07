---
issue: 42
title: Add --flythrough flag for automated visual testing
crates: [randlebrot]
modifies:
  - src/commands/launch.rs
  - src/main.rs  # add --flythrough flag to Launch CLI struct
depends_on: []
---

## Goal

Add `randlebrot launch <tag> --flythrough` that runs an automated camera path, saves screenshots at each waypoint to `/tmp/randlebrot_flythrough/`, and exits. This is the standard feedback loop for visual iteration — Claude Code can build, run flythrough, check images, fix, repeat without manual testing.

## Implementation

### 1. Add CLI flag in `src/main.rs`

In the `Launch` command struct:
```rust
Launch {
    level_tag: String,
    #[arg(long, default_value_t = false)]
    flythrough: bool,
}
```

Pass to `commands::launch::run(level_tag, flythrough)`.

### 2. FlyThroughState resource in `src/commands/launch.rs`

```rust
#[derive(Resource)]
struct FlyThroughState {
    waypoints: Vec<FlyWaypoint>,
    current: usize,
    elapsed: f32,
    frame_count: u32,
    output_dir: PathBuf,
}

struct FlyWaypoint {
    position_offset: Vec3,  // relative to spawn
    look_dir: Vec3,
    duration: f32,          // seconds at this waypoint
}
```

### 3. Waypoint sequence (10 frames, ~8 seconds total)

```rust
vec![
    // 1. Spawn view — looking forward
    FlyWaypoint { offset: Vec3::ZERO, look: Vec3::new(1,0,0), duration: 1.0 },
    // 2. Turn left 90
    FlyWaypoint { offset: Vec3::ZERO, look: Vec3::new(0,0,-1), duration: 0.5 },
    // 3. Turn right 180
    FlyWaypoint { offset: Vec3::ZERO, look: Vec3::new(0,0,1), duration: 0.5 },
    // 4. Look down at feet
    FlyWaypoint { offset: Vec3::ZERO, look: Vec3::new(1,-1,0).normalize(), duration: 0.5 },
    // 5. Look up at sky
    FlyWaypoint { offset: Vec3::ZERO, look: Vec3::new(1,1,0).normalize(), duration: 0.5 },
    // 6. Move forward 30 blocks
    FlyWaypoint { offset: Vec3::new(30,0,0), look: Vec3::new(1,0,0), duration: 2.0 },
    // 7. Move to high ground (up 20)
    FlyWaypoint { offset: Vec3::new(30,20,0), look: Vec3::new(1,-0.3,0).normalize(), duration: 1.0 },
    // 8. Panoramic spin
    FlyWaypoint { offset: Vec3::new(30,20,0), look: Vec3::new(-1,0,0), duration: 0.5 },
    // 9. Back at ground
    FlyWaypoint { offset: Vec3::new(30,0,0), look: Vec3::new(1,0,0), duration: 0.5 },
    // 10. Final forward view
    FlyWaypoint { offset: Vec3::new(50,0,0), look: Vec3::new(1,0,0), duration: 1.0 },
]
```

### 4. flythrough_system

```rust
fn flythrough_system(
    mut state: ResMut<FlyThroughState>,
    time: Res<Time>,
    mut camera_q: Query<&mut Transform, With<Camera3d>>,
    mut exit: MessageWriter<AppExit>,
    // Bevy screenshot API
) {
    state.elapsed += time.delta_secs();
    let wp = &state.waypoints[state.current];

    if state.elapsed >= wp.duration {
        // Take screenshot
        // save to state.output_dir / format!("frame_{:03}.png", state.frame_count)
        state.frame_count += 1;
        state.current += 1;
        state.elapsed = 0.0;

        if state.current >= state.waypoints.len() {
            eprintln!("Flythrough complete: {} frames saved to {:?}",
                state.frame_count, state.output_dir);
            exit.write(AppExit::Success);
            return;
        }
    }

    // Interpolate camera to current waypoint
    // ...
}
```

### 5. Skip macOS trampoline when --flythrough

No keyboard focus needed — skip the `.app` bundle trampoline:
```rust
#[cfg(target_os = "macos")]
if !flythrough {
    macos_ensure_app_bundle(&std::env::args().collect::<Vec<_>>());
}
```

### 6. Register system conditionally

```rust
if flythrough {
    app.insert_resource(FlyThroughState::new(spawn_pos));
    app.add_systems(Update, flythrough_system.run_if(loaded));
    // Skip: camera_input, grab_cursor, hud_system
} else {
    app.add_systems(Update, camera_input.run_if(loaded));
    // ... normal systems
}
```

## Verification

```bash
randlebrot launch peaks-test --flythrough
ls /tmp/randlebrot_flythrough/
# frame_001.png frame_002.png ... frame_010.png

# Check images:
open /tmp/randlebrot_flythrough/frame_001.png
```

## Constraints

- Flythrough exits automatically — no manual intervention
- No cursor grab in flythrough mode
- Skip macOS .app trampoline in flythrough mode
- Screenshots via Bevy's built-in screenshot API (not screencapture)
- Total duration < 10 seconds
