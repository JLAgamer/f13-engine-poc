//! Engine-independent gameplay rules. No engine types, no game data: tuning values are passed in,
//! loaded at runtime from the player's own install.

pub mod breath;
pub mod camera_spline;
pub mod collision;
pub mod color_grading;
pub mod counselor;
pub mod curve;
pub mod dodge;
pub mod dynamic_camera;
pub mod fear;
pub mod game_mode;
pub mod health;
pub mod killer_abilities;
pub mod killer_moves;
pub mod light;
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
pub mod stun;
pub mod vehicle;

/// An animation montage the rules time an action by. `name` keys its root motion in the export's anims.json.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Montage {
    pub name: String,
    /// Play length at rate 1, seconds.
    pub length: f32,
}
