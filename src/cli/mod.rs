//! CLI-specific support modules (coordinate conventions, shared parsers, etc.).
//!
//! Code here is shared between the top-level `main.rs` command dispatcher
//! and the individual command modules in `commands/`. It is kept in a
//! separate module so the CLI-layer invariants (like the global chunk
//! coordinate convention in [`coords`]) live in one place.

pub mod coords;
