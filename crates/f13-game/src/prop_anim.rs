//! Skeletal props' own animations (`props/<mesh>.json` from `f13-export props`): the tent's flaps as she goes in and
//! out (hiding.rs); the bear trap opening as it's armed
//! (BearTrap_C ArmedAnimation), snapping shut on a foot (TriggeredAnimation) and opening as she steps out
//! (EscapedAnimation), shut (A_Trap_Bear_01_Close_Idle_01) while it lies about or is carried: the mesh's bind pose
//! is the open trap. Each plays once and holds its last frame, sampled onto the prop's joints by bone name.

use bevy::prelude::*;
use f13_data::character::{AnimData, AnimInfo};
use std::collections::HashMap;

/// The bear trap's animations.
pub const TRAP_ARMED: &str = "A_Trap_Bear_01_Open_M";
pub const TRAP_SPRUNG: &str = "A_Trap_Bear_01_Enter_01_Anim";
pub const TRAP_ESCAPED: &str = "A_Trap_Bear_01_Exit_01_Anim";
pub const TRAP_CLOSED: &str = "A_Trap_Bear_01_Close_Idle_01";
/// The Chevette's hood: open (Chevette_AnimBP's bIsHoodOpen, its default: the idle_open loop) until the hood's
/// repair (its OnRepair's CloseHood: CloseHoodAnim, whose CloseHood notify clears it), on the Hood branch only.
pub const HOOD_OPEN: &str = "normal_stand_car_repair_hood_idle_open";
pub const HOOD_CLOSE: &str = "normal_stand_car_repair_hood_close";
/// The tent's, as she goes in and out of it.
pub const TENT_ENTER: &str = "normal_stand_hidespot_tent_enter";
pub const TENT_EXIT: &str = "normal_stand_hidespot_tent_exit";
/// The bear trap items.
const TRAP_CLASSES: &[&str] = &["BearTrap_C"];

pub struct PropAnimPlugin;

impl Plugin for PropAnimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PropAnims>().add_systems(Startup, load).add_systems(Update, (shut_traps, car_hoods, play).chain());
    }
}

/// Animation name -> (samples, length).
#[derive(Resource, Default)]
pub struct PropAnims(HashMap<String, (AnimData, f32)>);

/// A prop playing `name` from its start (`t` seconds in).
#[derive(Component)]
pub struct PropAnim {
    pub name: &'static str,
    pub t: f32,
    /// Only this bone and those under it (a LayeredBoneBlend's branch filter, BlendDepth 0).
    pub branch: Option<&'static str>,
}

impl PropAnim {
    pub fn new(name: &'static str) -> Self {
        PropAnim { name, t: 0.0, branch: None }
    }

    pub fn on_branch(name: &'static str, branch: &'static str) -> Self {
        PropAnim { name, t: 0.0, branch: Some(branch) }
    }
}

fn load(export: Res<crate::ExportRoot>, mut anims: ResMut<PropAnims>) {
    for mesh in ["A_Vehicle_Chevette_LOD0", "A_Trap_Bear_01", "A_Camp_Tent_Yellow_01_LOD0", "birdbath_rig", "A_Interactive_WaterPump_01", "adirondack_chair"] {
        let file = export.0.join("props").join(format!("{mesh}.json"));
        let Some(index) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<HashMap<String, AnimInfo>>(&t).ok()) else {
            warn!("{}: missing (f13-export props): {mesh} doesn't animate", file.display());
            continue;
        };
        for (name, info) in index {
            match AnimData::load(&export.0.join(&info.file)) {
                Ok(data) => {
                    anims.0.insert(name, (data, info.length));
                }
                Err(e) => warn!("{e}"),
            }
        }
    }
}

/// Bear traps not yet armed are shut.
fn shut_traps(mut commands: Commands, items: Query<(Entity, &crate::props::Item), (Added<crate::props::Item>, Without<PropAnim>)>) {
    for (e, item) in &items {
        if TRAP_CLASSES.contains(&item.class.as_str()) {
            commands.entity(e).insert(PropAnim::new(TRAP_CLOSED));
        }
    }
}

/// The cars' hoods: open while the hood's repair (the battery) isn't done, closing once it is.
fn car_hoods(mut commands: Commands, cars: Query<(Entity, &crate::props::Vehicle, Option<&PropAnim>)>) {
    for (e, v, anim) in &cars {
        let Some(hood) = v.repairs.iter().find(|r| r.name == "HoodRepair") else { continue };
        let want = if hood.done { HOOD_CLOSE } else { HOOD_OPEN };
        if anim.is_none_or(|a| a.name != want) {
            commands.entity(e).insert(PropAnim::on_branch(want, "Hood"));
        }
    }
}

/// Advances each prop's animation and poses its joints (every frame: the prop's scene may come in later).
fn play(
    time: Res<Time>,
    anims: Res<PropAnims>,
    mut props: Query<(Entity, &mut PropAnim)>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut transforms: Query<&mut Transform>,
) {
    for (e, mut p) in &mut props {
        let Some((data, length)) = anims.0.get(p.name) else { continue };
        p.t = (p.t + time.delta_secs()).min(*length);
        let frames = data.frames.max(1);
        let f = if *length > 0.0 { p.t / length * (frames - 1) as f32 } else { 0.0 };
        let (i, a) = (f.floor() as usize, f.fract());
        let j = (i + 1).min(frames - 1);
        let root = match p.branch {
            Some(b) => match children.iter_descendants(e).find(|&c| names.get(c).is_ok_and(|n| n.as_str() == b)) {
                Some(r) => r,
                None => continue,
            },
            None => e,
        };
        for bone in std::iter::once(root).filter(|_| p.branch.is_some()).chain(children.iter_descendants(root)) {
            let Ok(name) = names.get(bone) else { continue };
            let Some(track) = data.tracks.iter().find(|t| t.bone == name.as_str()) else { continue };
            let Ok(mut xf) = transforms.get_mut(bone) else { continue };
            if let (Some(q0), Some(q1)) = (track.rotation.get(i), track.rotation.get(j)) {
                xf.rotation = Quat::from_array(*q0).normalize().slerp(Quat::from_array(*q1).normalize(), a);
            }
            if let Some((t0, t1)) = track.translation.as_ref().and_then(|t| Some((t.get(i)?, t.get(j)?))) {
                xf.translation = Vec3::from_array(*t0).lerp(Vec3::from_array(*t1), a);
            }
        }
    }
}
