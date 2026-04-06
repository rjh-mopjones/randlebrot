//! Headless CLI command implementations.
//!
//! These modules power the non-GUI subcommands of the `randlebrot` binary.
//! They run without Bevy (no `App::new()`, no window), using rayon for
//! parallelism and `indicatif` for terminal progress bars.

pub mod generate_layers;
pub mod generate_level;
pub mod view_layers;
