//! CLI command implementations.
//!
//! These modules power the non-GUI subcommands of the `randlebrot` binary.
//! `generate_layers` and `generate_level` are headless (no Bevy window,
//! rayon + indicatif). `view_layers` and `launch` open a minimal Bevy window.

pub mod debug_level;
pub mod generate_layers;
pub mod generate_level;
pub mod launch;
pub mod view_layers;
