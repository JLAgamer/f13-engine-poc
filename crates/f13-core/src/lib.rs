//! Engine-independent gameplay rules. No engine types, no game data: tuning values are passed in,
//! loaded at runtime from the player's own install.

pub mod counselor;
pub mod curve;
pub mod fear;
pub mod health;
pub mod killer_abilities;
pub mod movement;
pub mod spawn;
pub mod stats;
pub mod stumble;
