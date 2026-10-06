//! Engine-independent gameplay rules. No engine types, no game data: tuning values are passed in,
//! loaded at runtime from the player's own install.

pub mod collision;
pub mod counselor;
pub mod curve;
pub mod dodge;
pub mod fear;
pub mod health;
pub mod killer_abilities;
pub mod minigame;
pub mod movement;
pub mod nav;
pub mod navmesh;
pub mod perception;
pub mod ragdoll;
pub mod recast;
pub mod spawn;
pub mod stats;
pub mod stumble;

/// An animation montage the rules time an action by. `name` keys its root motion in the export's anims.json.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Montage {
    pub name: String,
    /// Play length at rate 1, seconds.
    pub length: f32,
}
