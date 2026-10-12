//! Spectating: once the local counselor is dead or has escaped, the player looks on (SCPlayerController's spectate
//! mode, +0x828; decompiled: OnToggleSpectateMode 0x1403fecd0, the status texts FUN_1404121c0).
//!
//! The modes:
//! 1. **Her dead body** ("Your Dead Body"; "You Escaped!" escaped, with no camera of its own): a DeathCamera
//!    (FUN_1403eb270). Her view's yaw is tried with each offset of [`DEATH_YAWS`] in turn, pitched to
//!    [`DEATH_PITCH`], [`DEATH_DISTANCE_CM`] back from the body. The first whose line from the body is clear (the
//!    DeathCamera trace) is used, blended to over [`BLEND_S`].
//! 2. **Free camera** (not reached by the toggle).
//! 3. **Spectating a player** ("Spectating: <name>", or "No counselors to spectate"): through her own cameras. The
//!    players are the game state's characters in order. A counselor who isn't spectating herself and isn't out is
//!    picked; Jason only in a private match (FUN_1403e3a20). The next / previous one wraps around (FUN_1403ff230).
//! 4. **The camp cameras** ("Viewing: <DisplayName>", or "No Camp Cameras!"): the level's SCStaticSpectatorCameras in
//!    turn (FUN_1403ff400), blended to over [`BLEND_S`].
//!
//! The toggle (SPEC_ToggleMode, Space):
//! - from none: the dead body, once she's dead (escaped: its status only);
//! - from the dead body or free camera: the players (the next one);
//! - from the players: the camp cameras (the next one);
//! - from the camp cameras: the dead body when she's dead and didn't escape, else the players.
//!
//! SPEC_ViewNext / SPEC_ViewPrev (left and right click) go through the players or the camp cameras.
//!
//! PLACEHOLDERS:
//! - The unfreeze wait (1 s) starts as her kill camera ends.
//! - Jason isn't spectated (not a private match).
//! - The spectator HUD's intro shows once she's out and her kill camera is over (FUN_1404121c0 runs then).

use crate::controls::LocalPlayer;
use crate::player::{Player, PlayerCamera};
use bevy::prelude::*;

/// The death camera's distance back from her (cm), its pitch, and the yaws tried after her view's (degrees).
const DEATH_DISTANCE_CM: f32 = 600.0;
const DEATH_PITCH: f32 = -45.0;
const DEATH_YAWS: [f32; 8] = [0.0, -180.0, 90.0, -90.0, 45.0, -45.0, 135.0, -135.0];
/// AGameMode's MinRespawnDelay (s): the controller's unfreeze time after its pawn goes.
const UNFREEZE_S: f32 = 1.0;
/// SetViewTargetWithBlend's time into the death and camp cameras (DAT_14209aa80).
const BLEND_S: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    None,
    DeadBody,
    FreeCam,
    Players,
    CampCameras,
}

/// The local player's spectating: the mode, the player watched (by the game state's order) and the camp camera, the
/// death camera's place, and the blend into a fixed view (where from, time).
#[derive(Resource)]
pub struct Spectate {
    pub mode: Mode,
    index: i32,
    pub player: Option<Entity>,
    camera: i32,
    death: Option<Transform>,
    blend: Option<(Transform, f32)>,
}

impl Default for Spectate {
    /// No one watched yet (INDEX_NONE: the first step goes to the first), the first camp camera.
    fn default() -> Self {
        Spectate { mode: Mode::None, index: -1, player: None, camera: 0, death: None, blend: None }
    }
}

impl Spectate {
    /// The counselor whose cameras give the view: the watched one while spectating players.
    pub fn viewed(&self) -> Option<Entity> {
        (self.mode == Mode::Players).then_some(self.player).flatten()
    }
}

/// The level's camp cameras: their names and views (Bevy).
#[derive(Resource, Default)]
struct CampCameras(Vec<(String, Transform)>);

pub struct SpectatePlugin {
    /// `--bot-spectate`: the test bot ([`bot`]).
    pub bot: bool,
}

impl Plugin for SpectatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spectate>()
            .init_resource::<CampCameras>()
            .init_resource::<SpectatorStatus>()
            .add_systems(Startup, (load_cameras, spawn_status))
            .add_systems(Update, (toggle, view.after(crate::kill_camera::KillCameraSystems)).chain().after(crate::counselor_camera::counselor_camera))
            .add_systems(Update, status);
        if self.bot {
            app.init_resource::<Bot>().add_systems(Update, bot.before(toggle));
        }
    }
}

fn load_cameras(mut cams: ResMut<CampCameras>, level: Res<crate::Level>) {
    for a in level.layouts.iter().flat_map(|l| l.actors_of("SCStaticSpectatorCamera")) {
        let Some(t) = a.transform.as_ref() else { continue };
        let name = a.props["DisplayName"].as_str().unwrap_or(&a.name).to_string();
        let base = crate::to_transform(t);
        // The camera component looks along Unreal +X (Bevy +X); a Bevy camera looks along -Z.
        cams.0.push((name, Transform::from_translation(base.translation).with_rotation(base.rotation * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2))));
    }
    info!("spectate: {} camp cameras", cams.0.len());
}

/// The players to watch, in order: the counselors that are alive, not out and not the local one.
fn watchable(players: &Query<(Entity, &Player, Has<LocalPlayer>, Has<crate::vehicle::Escaped>)>) -> Vec<Entity> {
    let mut v: Vec<Entity> = players.iter().filter(|(_, p, local, out)| !local && !out && p.counselor.alive()).map(|(e, ..)| e).collect();
    v.sort();
    v
}

/// The next (or previous, or with `dir` 0 the same) one from `index`, wrapping round (FUN_1403ff230 /
/// FUN_1403e3a20 / FUN_1403ff400).
fn step(index: i32, dir: i32, n: usize) -> Option<i32> {
    if n == 0 {
        return None;
    }
    let n = n as i32;
    Some((index + dir).rem_euclid(n))
}

/// The death camera: where it looks at her body from (FUN_1403eb270), or none when every try is blocked.
fn death_camera(body: Vec3, yaw_deg: f32, world: &(impl f13_core::collision::TriSource + ?Sized)) -> Option<Transform> {
    for offset in DEATH_YAWS {
        let (yaw, pitch) = ((yaw_deg + offset).to_radians(), DEATH_PITCH.to_radians());
        // Unreal's rotator to a direction (x forward, y right, z up), in Bevy's axes.
        let dir_ue = Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin());
        let dir = crate::counselor_camera::to_bevy(dir_ue) / 100.0;
        let at = body - dir.normalize_or_zero() * DEATH_DISTANCE_CM / 100.0;
        if f13_core::collision::segment_hit(world, body, at, 1.0).is_none() {
            return Some(Transform::from_translation(at).looking_at(body, Vec3::Y));
        }
    }
    None
}

/// SPEC_ToggleMode, SPEC_ViewNext and SPEC_ViewPrev.
#[allow(clippy::too_many_arguments)]
fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut spec: ResMut<Spectate>,
    cams: Res<CampCameras>,
    me: Query<(&Player, &Transform, Has<crate::vehicle::Escaped>, Has<crate::kill_camera::KillCam>), With<LocalPlayer>>,
    players: Query<(Entity, &Player, Has<LocalPlayer>, Has<crate::vehicle::Escaped>)>,
    camera: Single<(&PlayerCamera, &Transform), Without<Player>>,
    (collision, dynamic): (Option<Res<crate::collision::WorldCollision>>, Res<crate::collision::DynamicColliders>),
    (real, mut out_since): (Res<Time<Real>>, Local<Option<f32>>),
) {
    let Ok((p, pxf, escaped, killing)) = me.single() else { return };
    let dead = !p.counselor.alive();
    if !(dead || escaped) || killing {
        *out_since = None;
        return;
    }
    // The first toggle waits out the controller's unfreeze time (GetTimeUntilUnFreeze: APlayerController's
    // UnFreeze timer, the game mode's MinRespawnDelay: AGameMode's 1 s, no SC class overrides it). PLACEHOLDER: from
    // the end of her kill camera (the controller goes inactive as her pawn is let go).
    let since = *out_since.get_or_insert(real.elapsed_secs());
    if spec.mode == Mode::None && real.elapsed_secs() - since < UNFREEZE_S {
        return;
    }
    let s = &mut *spec;
    let here = *camera.1;
    let mut to_players = |s: &mut Spectate, dir: i32| {
        let list = watchable(&players);
        s.mode = Mode::Players;
        s.index = step(s.index, dir, list.len()).unwrap_or(0);
        s.player = list.get(s.index as usize).copied();
    };
    let to_camera = |s: &mut Spectate, dir: i32| {
        s.mode = Mode::CampCameras;
        s.camera = step(s.camera, dir, cams.0.len()).unwrap_or(0);
        s.blend = Some((here, 0.0));
    };
    if keys.just_pressed(KeyCode::Space) {
        match s.mode {
            Mode::None | Mode::CampCameras if dead && !escaped => {
                s.mode = Mode::DeadBody;
                let (yaw, _) = crate::counselor_camera::control_rotation(camera.0);
                s.death = collision.as_deref().and_then(|c| death_camera(pxf.translation + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M, yaw, &c.with(&dynamic)));
                s.blend = Some((here, 0.0));
            }
            Mode::None => {
                s.mode = Mode::DeadBody;
                s.death = None;
            }
            Mode::DeadBody | Mode::FreeCam | Mode::CampCameras => to_players(s, 1),
            // The camp camera it was on (FUN_1403ff400 only keeps the index in range).
            Mode::Players => to_camera(s, 0),
        }
        info!("spectate: {:?}", s.mode);
    }
    let dir = if mouse.just_pressed(MouseButton::Left) {
        1
    } else if mouse.just_pressed(MouseButton::Right) {
        -1
    } else {
        0
    };
    if dir != 0 {
        match s.mode {
            Mode::Players => to_players(s, dir),
            Mode::CampCameras => to_camera(s, dir),
            _ => {}
        }
    }
    // The watched one gone (dead, escaped, left): the next.
    if s.mode == Mode::Players && s.player.is_some_and(|e| !watchable(&players).contains(&e)) {
        let list = watchable(&players);
        s.index = s.index.min(list.len() as i32 - 1).max(0);
        s.player = list.get(s.index as usize).copied();
    }
}

/// The fixed views: the death camera and the camp cameras, blended to.
fn view(time: Res<Time>, mut spec: ResMut<Spectate>, cams: Res<CampCameras>, mut camera: Single<&mut Transform, With<PlayerCamera>>) {
    let s = &mut *spec;
    let target = match s.mode {
        Mode::DeadBody => s.death,
        Mode::CampCameras => cams.0.get(s.camera as usize).map(|c| c.1),
        _ => None,
    };
    let Some(target) = target else { return };
    let xf = match s.blend.as_mut() {
        Some((from, t)) => {
            *t += time.delta_secs();
            let a = (*t / BLEND_S).clamp(0.0, 1.0);
            // SetViewTargetWithBlend's default: linear.
            let out = Transform::from_translation(from.translation.lerp(target.translation, a)).with_rotation(from.rotation.slerp(target.rotation, a));
            if a >= 1.0 {
                s.blend = None;
            }
            out
        }
        None => target,
    };
    **camera = xf;
}

#[derive(Component)]
struct Status;

fn spawn_status(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(22.0), ..default() },
        TextLayout::justify(Justify::Center),
        Node { position_type: PositionType::Absolute, bottom: Val::Px(40.0), left: Val::Percent(25.0), width: Val::Percent(50.0), ..default() },
        Visibility::Hidden,
        Status,
    ));
}

/// What the spectator HUD shows (ui.rs, SpectatorHUDWidget): she's out (dead or escaped, her kill camera over:
/// FUN_1404121c0 then shows its intro), died or escaped, whether the intro is over (the first toggle:
/// OnSpectatorIntroFinished), and FUN_1404121c0's status and next-mode texts.
#[derive(Resource, Default, Clone, PartialEq, Debug)]
pub struct SpectatorStatus {
    pub out: bool,
    pub died: bool,
    pub intro_over: bool,
    pub status: String,
    pub next: &'static str,
}

/// The status and the next mode (FUN_1404121c0's texts), and the keys: for the spectator HUD, and as a line of text
/// when its widget isn't exported.
#[allow(clippy::type_complexity)]
fn status(
    spec: Res<Spectate>,
    cams: Res<CampCameras>,
    me: Query<(&Player, Has<crate::vehicle::Escaped>, Has<crate::kill_camera::KillCam>), With<LocalPlayer>>,
    names: Query<&Name>,
    mut text: Single<(&mut Text, &mut Visibility), With<Status>>,
    hud: Option<Res<crate::ui::Hud>>,
    mut out: ResMut<SpectatorStatus>,
) {
    let (ref mut text, ref mut vis) = *text;
    let (alive, escaped, killing) = me.single().map_or((true, false, false), |(p, e, k)| (p.counselor.alive(), e, k));
    let widget = hud.is_some_and(|h| h.has_spectator());
    let mut st = SpectatorStatus { out: (!alive || escaped) && !killing, died: !alive && !escaped, intro_over: spec.mode != Mode::None, ..default() };
    let (now, next) = match spec.mode {
        Mode::None => {
            vis.set_if_neq(Visibility::Hidden);
            out.set_if_neq(st);
            return;
        }
        Mode::DeadBody if escaped => ("You Escaped!".to_string(), "Spectate Players"),
        Mode::DeadBody => ("Your Dead Body".to_string(), "Spectate Players"),
        Mode::FreeCam => ("Free Camera".to_string(), "Spectate Players"),
        Mode::Players => (
            match spec.player {
                Some(e) => format!("Spectating: {}", names.get(e).map_or("counselor".to_string(), |n| n.to_string())),
                None => "No counselors to spectate".to_string(),
            },
            "Camp Cameras",
        ),
        Mode::CampCameras => (
            match cams.0.get(spec.camera as usize) {
                Some((n, _)) => format!("Viewing: {n}"),
                None => "No Camp Cameras!".to_string(),
            },
            if escaped { "Spectate Players" } else { "Your Dead Body" },
        ),
    };
    st.status = now.clone();
    st.next = next;
    out.set_if_neq(st);
    if widget {
        vis.set_if_neq(Visibility::Hidden);
        return;
    }
    let s = format!("{now}\nSpace: {next}    click: next / previous");
    if text.0 != s {
        text.0 = s;
    }
    vis.set_if_neq(Visibility::Inherited);
}

/// The spectating test's progress.
#[derive(Resource, Default)]
struct Bot {
    step: u8,
    t: f32,
    failed: Vec<&'static str>,
}

/// `--bot-spectate` (with `--counselors 2`): she dies; Space, her dead body: the death camera 6 m from her; Space, the
/// other counselor through her cameras; Space, the first camp camera, blended to.
#[allow(clippy::too_many_arguments)]
fn bot(
    mut b: ResMut<Bot>,
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    spec: Res<Spectate>,
    cams: Res<CampCameras>,
    mut me: Query<(&mut Player, &Transform), With<LocalPlayer>>,
    others: Query<&Transform, (With<Player>, Without<LocalPlayer>, Without<PlayerCamera>)>,
    camera: Single<&Transform, (With<PlayerCamera>, Without<Player>)>,
    mut exit: MessageWriter<AppExit>,
) {
    b.t += time.delta_secs();
    keys.release(KeyCode::Space);
    let Ok((mut p, pxf)) = me.single_mut() else { return };
    let at = camera.translation;
    let next = |b: &mut Bot| {
        b.step += 1;
        b.t = 0.0;
    };
    match b.step {
        // Let the map load (real time), then she dies.
        0 if real.elapsed_secs() > 8.0 && b.t > 3.0 => {
            p.counselor.health = 0.0;
            info!("bot: spectate: she dies");
            next(&mut b);
        }
        // Space until the toggle takes (after the controller's unfreeze time).
        1 if spec.mode != Mode::None => next(&mut b),
        1 if b.t > 2.0 => keys.press(KeyCode::Space),
        2 if b.t > BLEND_S + 0.5 => {
            let d = at.distance(pxf.translation + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M);
            info!("bot: spectate: {:?}, the camera {d:.1} m from her body", spec.mode);
            if spec.mode != Mode::DeadBody || (d - DEATH_DISTANCE_CM / 100.0).abs() > 0.3 {
                b.failed.push("dead body");
            }
            keys.press(KeyCode::Space);
            next(&mut b);
        }
        3 if b.t > 2.0 => {
            let other = others.iter().next().map(|o| o.translation);
            let d = other.map_or(f32::MAX, |o| at.distance(o));
            info!("bot: spectate: {:?} {:?}, the camera {d:.1} m from her", spec.mode, spec.player);
            if spec.mode != Mode::Players || spec.player.is_none() || d > 6.0 {
                b.failed.push("players");
            }
            keys.press(KeyCode::Space);
            next(&mut b);
        }
        4 if b.t > BLEND_S + 0.5 => {
            let want = cams.0.first().map(|c| c.1.translation);
            let d = want.map_or(f32::MAX, |w| at.distance(w));
            info!("bot: spectate: {:?}, the camera {d:.2} m from {:?}", spec.mode, cams.0.first().map(|c| &c.0));
            if spec.mode != Mode::CampCameras || d > 0.1 {
                b.failed.push("camp cameras");
            }
            if b.failed.is_empty() {
                info!("bot: spectate: passed");
                exit.write(AppExit::Success);
            } else {
                error!("bot: spectate: failed {:?}", b.failed);
                exit.write(AppExit::error());
            }
            next(&mut b);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_wraps_round() {
        assert_eq!(step(0, 1, 3), Some(1));
        assert_eq!(step(2, 1, 3), Some(0));
        assert_eq!(step(0, -1, 3), Some(2));
        assert_eq!(step(0, 1, 0), None);
    }

    #[test]
    fn the_death_camera_looks_down_at_her_from_600_cm() {
        let world = f13_core::collision::TriList::new(vec![]);
        let at = death_camera(Vec3::ZERO, 0.0, &world).unwrap();
        assert!((at.translation.length() - 6.0).abs() < 1e-3);
        // Pitched down 45 degrees: as high as it is far out.
        assert!((at.translation.y - at.translation.xz().length()).abs() < 1e-3);
    }
}
