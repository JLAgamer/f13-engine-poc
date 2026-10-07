//! Cabin windows (open, close, vault, dive) and door barricades.
//!
//! Windows, from `SCWindow` (CanInteractWith / OnInteract / Native*Completed):
//! - Inside/outside as for doors: inside is behind the window's forward (+X) axis.
//! - **Press**: sprinting and not wounded → **dive** through (breaks a closed window; `DiveDamage` if it was closed
//!   or already broken). Otherwise a closed, intact window **opens**; an open or broken one is **vaulted** (through
//!   a broken one costs `VaultDamage`).
//! - **Hold** (only while open and intact): **close** it.
//! - Second-storey exits (`WoundHeight`/`HeightDamage`) aren't modelled: Crystal Lake's cabins have one storey.
//!
//! Barricades, from `SCBarricade`: a door's barricade replaces its lock. Holding interact on the door from inside
//! toggles the barricade (`OnUsedBarricade`), only while the door is closed; a barricaded door doesn't open.
//!
//! The barricade's plank (its "Mesh", `S_Interactive_Door_Barricade_01_b`) stands on end beside the door, over the wall
//! brackets (the `..._01_a` mesh). Its pitch is all that moves (SCBarricade, decompiled): during the counselor's
//! montage, UseBarricadeDriverTick (0x140364d10) sets it to [`PLANK_PITCH`] x clamp(her "Barricade" curve, 0, 1) on
//! the plank's own rotation; OnRep_Enabled (0x140352c40) sets it to [`PLANK_PITCH`] when barred, 0 when not.
//!
//! PLACEHOLDERS: the vault/dive/open animations are instant (the counselor is moved through the window, 1.2 m past
//! it); linking a barricade to the nearest door is a guess.

use bevy::prelude::*;

/// How far past the window plane the counselor lands after a vault or dive.
const THROUGH_M: f32 = 1.2;
/// The plank's pitch across the door (DAT_142121618, degrees).
pub const PLANK_PITCH: f32 = -90.0;

#[derive(Component, Debug)]
pub struct Window {
    pub open: bool,
    pub destroyed: bool,
    /// Interact point (window local) and range in cm.
    pub interact: Vec3,
    pub reach: f32,
    pub dive_damage: f32,
    pub vault_damage: f32,
    pub take_vault_damage: bool,
    /// Sliding pane: entity, closed and open local transforms.
    pub pane: Option<(Entity, Transform, Transform)>,
    /// Glass meshes hidden once broken.
    pub glass: Vec<Entity>,
    /// Where she starts climbing through and diving, from inside and from outside (its CounselorVault/Dive
    /// special move components, window local).
    pub vault: [Option<Transform>; 2],
    pub dive: [Option<Transform>; 2],
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum WindowAction {
    Open,
    Vault,
    Dive,
}

impl Window {
    pub fn is_inside(window: &GlobalTransform, at: Vec3) -> bool {
        crate::doors::Door::is_inside(window, at)
    }

    /// The press action for a counselor (`None` if nothing).
    pub fn press_action(&self, sprinting: bool, wounded: bool) -> WindowAction {
        if sprinting && !wounded {
            WindowAction::Dive
        } else if !self.open && !self.destroyed {
            WindowAction::Open
        } else {
            WindowAction::Vault
        }
    }

    pub fn can_hold(&self) -> bool {
        self.open && !self.destroyed
    }

    /// Damage for going through, by the rules above.
    pub fn damage(&self, action: WindowAction) -> f32 {
        match action {
            WindowAction::Dive if self.destroyed || !self.open => self.dive_damage,
            WindowAction::Vault if self.take_vault_damage && self.destroyed => self.vault_damage,
            _ => 0.0,
        }
    }

    /// Where the counselor ends up after going through from `from`.
    pub fn landing(window: &GlobalTransform, interact: Vec3, from: Vec3) -> Vec3 {
        let forward = (window.rotation() * Vec3::X).with_y(0.0).normalize_or_zero();
        let side = if Window::is_inside(window, from) { 1.0 } else { -1.0 };
        let at = window.transform_point(interact);
        Vec3::new(at.x, from.y, at.z) + forward * side * THROUGH_M
    }
}

#[derive(Component, Debug)]
pub struct Barricade {
    pub enabled: bool,
    /// The door it bars (nearest door, linked after spawning).
    pub door: Option<Entity>,
    /// The plank and its relative transform (Unreal, in the barricade's space) as placed: stood on end.
    pub plank: Option<(Entity, f13_data::level::Xform)>,
}

impl Barricade {
    /// The plank and its transform at `pitch` (degrees), keeping its yaw and roll (SetRelativeRotation of its stored
    /// rotation with the pitch replaced).
    pub fn plank_at(&self, pitch: f32) -> Option<(Entity, Transform)> {
        use f13_core::camera_spline::{quaternion, rotator};
        let (e, placed) = self.plank?;
        let mut r = rotator(Quat::from_array(placed.r));
        r.x = pitch;
        Some((e, crate::to_transform(&f13_data::level::Xform { r: quaternion(r).to_array(), ..placed })))
    }
}

/// OnRep_Enabled: the plank across the door when barred, stood up when not.
pub fn place_planks(barricades: Query<&Barricade, Changed<Barricade>>, mut transforms: Query<&mut Transform>) {
    for b in &barricades {
        if let Some((e, at)) = b.plank_at(if b.enabled { PLANK_PITCH } else { 0.0 }) {
            if let Ok(mut t) = transforms.get_mut(e) {
                *t = at;
            }
        }
    }
}

/// Shows window panes and barricade meshes for their state.
pub fn update_window_visuals(
    windows: Query<&Window, Changed<Window>>,
    mut transforms: Query<&mut Transform>,
    mut visibility: Query<&mut Visibility>,
) {
    let mut show = |e: Entity, on: bool| {
        if let Ok(mut v) = visibility.get_mut(e) {
            *v = if on { Visibility::Inherited } else { Visibility::Hidden };
        }
    };
    for w in &windows {
        if let Some((pane, closed, open)) = w.pane {
            if let Ok(mut t) = transforms.get_mut(pane) {
                *t = if w.open { open } else { closed };
            }
        }
        for &g in &w.glass {
            show(g, !w.destroyed);
        }
    }
}

/// Links each barricade to the nearest door (within 3 m) once both exist.
pub fn link_barricades(
    mut barricades: Query<(&mut Barricade, &GlobalTransform)>,
    doors: Query<(Entity, &GlobalTransform), With<crate::doors::Door>>,
    mut done: Local<bool>,
) {
    if *done || doors.is_empty() || barricades.is_empty() {
        return;
    }
    *done = true;
    for (mut b, bxf) in &mut barricades {
        b.door = doors
            .iter()
            .map(|(e, d)| (e, d.translation().distance(bxf.translation())))
            .filter(|(_, d)| *d < 3.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(e, _)| e);
    }
    let linked = barricades.iter().filter(|(b, _)| b.door.is_some()).count();
    info!("barricades linked to doors: {linked}/{}", barricades.iter().count());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> Window {
        Window {
            open: false,
            destroyed: false,
            interact: Vec3::ZERO,
            reach: 150.0,
            dive_damage: 20.0,
            vault_damage: 10.0,
            take_vault_damage: true,
            pane: None,
            glass: vec![],
            vault: [None; 2],
            dive: [None; 2],
        }
    }

    #[test]
    fn press_opens_vaults_or_dives() {
        let mut w = window();
        assert_eq!(w.press_action(false, false), WindowAction::Open);
        assert_eq!(w.press_action(true, false), WindowAction::Dive);
        assert_eq!(w.press_action(true, true), WindowAction::Open); // wounded counselors don't dive
        w.open = true;
        assert_eq!(w.press_action(false, false), WindowAction::Vault);
        assert!(w.can_hold());
    }

    #[test]
    fn damage_by_window_state() {
        let mut w = window();
        assert_eq!(w.damage(WindowAction::Dive), 20.0); // through glass
        w.open = true;
        assert_eq!(w.damage(WindowAction::Dive), 0.0);
        assert_eq!(w.damage(WindowAction::Vault), 0.0);
        w.destroyed = true;
        assert_eq!(w.damage(WindowAction::Vault), 10.0); // over broken glass
        assert_eq!(w.damage(WindowAction::Dive), 20.0);
    }
}
