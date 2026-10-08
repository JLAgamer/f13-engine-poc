//! Interaction cameras (SCSpecialMoveComponent, decompiled): a special move whose handler has bTakeCameraControl
//! (the constructor's true, at +0x2f1 of its word write 0x100 at +0x2f0; the hiding spots' search handlers turn it
//! off: doors' open and close moves, cabinets' drawers, windows' open and close) takes the view to
//! one of the SCSplineCameras attached under the handler when she reaches the move's place (NativeDestinationReached ->
//! PlayInteractionCamera 0x1403548d0), and hands it back when the move completes or is aborted (StopInteractionCamera
//! 0x14035fbd0 -> ReturnCameraToCharacter).
//!
//! - Which camera: the attached cameras ranked by how closely their first frame faces the player's view (dot of the
//!   two directions), the first valid one, else the first; a camera without a spline is never "valid"
//!   (IsCameraValidArray), so with one camera per handler (the doors) that one plays. It stays where it is attached
//!   (no spline: its tick stops), at its FieldOfView (UCameraComponent's 90 unless set).
//! - The blends: SetViewTargetWithBlend over BlendInTime and BlendBackTime, VTBlend_EaseInOut with exponent 2
//!   (DAT_14209aa80), the outgoing view locked (bLockOutgoing = bForceNewCameraRotation, true by default): location and
//!   field of view lerp, rotation lerps as a rotator by the shortest way (FMinimalViewInfo::BlendViewInfo).
//! - Handing back with bForceNewCameraRotation: her view turns to the camera's yaw, pitch and roll 0.

use crate::player::PlayerCamera;
use bevy::prelude::*;
use f13_core::camera_spline::{quaternion, rotator};

/// VTBlend_EaseInOut's exponent for interaction cameras (DAT_14209aa80).
const BLEND_EXP: f32 = 2.0;
/// UCameraComponent's default FieldOfView (horizontal degrees).
const DEFAULT_FOV: f32 = 90.0;

pub struct InteractionCameraPlugin;

impl Plugin for InteractionCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(play).add_observer(stop).add_systems(
            Update,
            interaction_camera
                .after(crate::counselor_camera::counselor_camera)
                .after(crate::jason_player::jason_camera)
                .before(crate::kill_camera::kill_camera),
        );
    }
}

/// An actor's special move handlers that take the camera: handler name -> its attached cameras (actor space: Bevy
/// camera transforms, looking down -Z) with their horizontal field of view, and its blend in and back times (s).
#[derive(Component, Clone, Debug, Default)]
pub struct InteractionCameras(pub std::collections::HashMap<String, Handler>);

#[derive(Clone, Debug)]
pub struct Handler {
    pub cameras: Vec<(Transform, f32)>,
    pub blend_in: f32,
    pub blend_back: f32,
}

impl InteractionCameras {
    /// From an actor class's components: each SCSpecialMoveComponent with bTakeCameraControl and its SCSplineCameras.
    pub fn new(components: &[f13_data::level::ComponentInfo]) -> Option<Self> {
        let num = |c: &f13_data::level::ComponentInfo, k: &str, d: f32| c.props.get(k).and_then(|v| v.as_f64()).map_or(d, |v| v as f32);
        let mut out = std::collections::HashMap::new();
        for h in components.iter().filter(|c| c.kind == "SCSpecialMoveComponent" && c.props.get("bTakeCameraControl").and_then(|v| v.as_bool()) != Some(false)) {
            let cameras: Vec<(Transform, f32)> = components
                .iter()
                .filter(|c| c.kind == "SCSplineCamera" && c.parent.as_deref() == Some(h.name.as_str()))
                .filter_map(|c| {
                    let t = crate::to_transform(c.local.as_ref()?);
                    // The camera looks along Unreal +X (Bevy +X here); a Bevy camera looks along -Z.
                    Some((Transform { rotation: t.rotation * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2), ..t }, num(c, "FieldOfView", DEFAULT_FOV)))
                })
                .collect();
            if !cameras.is_empty() {
                // The constructor's BlendBackTime is 0.25; BlendInTime has no constructor value (0).
                out.insert(h.name.clone(), Handler { cameras, blend_in: num(h, "BlendInTime", 0.0), blend_back: num(h, "BlendBackTime", 0.25) });
            }
        }
        (!out.is_empty()).then_some(Self(out))
    }
}

/// PlayInteractionCamera: `actor`'s `handler` takes the view (when it has bTakeCameraControl and a camera).
#[derive(Event, Clone, Debug)]
pub struct PlayInteractionCamera {
    pub actor: Entity,
    pub handler: &'static str,
    /// Jason's move (else hers): only the played character's moves take the view.
    pub killer: bool,
}

/// StopInteractionCamera: the view goes back to her.
#[derive(Event, Clone, Copy, Debug)]
pub struct StopInteractionCamera {
    /// Jason's move (else hers).
    pub killer: bool,
}

/// The view's state: blending to the interaction camera (from a locked view), on it, or blending back.
#[derive(Resource, Clone, Copy, Debug)]
enum View {
    In { from: (Transform, f32), to: (Transform, f32), time: f32, t: f32, back: f32 },
    /// Back to her cameras (counselor_camera.rs places the camera and sets its field of view before this runs).
    Back { from: (Transform, f32), time: f32, t: f32 },
}

/// The vertical field of view of a horizontal one at the window's aspect.
fn vertical(fov_h: f32, aspect: f32) -> f32 {
    2.0 * ((fov_h.to_radians() / 2.0).tan() / aspect).atan()
}

fn play(
    ev: On<PlayInteractionCamera>,
    mut commands: Commands,
    actors: Query<(&InteractionCameras, &GlobalTransform)>,
    camera: Single<(&Transform, &Projection), With<PlayerCamera>>,
    windows: Query<&Window>,
    play_jason: Option<Res<crate::jason_player::PlayJason>>,
) {
    if ev.killer != play_jason.is_some() {
        return;
    }
    let Ok((cams, g)) = actors.get(ev.actor) else { return };
    let Some(h) = cams.0.get(ev.handler) else { return };
    let (xf, proj) = *camera;
    let Projection::Perspective(p) = proj else { return };
    // The camera whose view is nearest the player's (a camera without a spline is never "valid": the first ranked).
    let look = xf.forward();
    let world = |t: &Transform| Transform::from_matrix(g.to_matrix() * t.to_matrix());
    let Some((cam, fov)) = h.cameras.iter().max_by(|a, b| world(&a.0).forward().dot(*look).total_cmp(&world(&b.0).forward().dot(*look))) else { return };
    let aspect = windows.iter().next().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
    info!("interaction camera: {} ({:.1}s in, {:.1}s back)", ev.handler, h.blend_in, h.blend_back);
    commands.insert_resource(View::In { from: (*xf, p.fov), to: (world(cam), vertical(*fov, aspect)), time: h.blend_in, t: 0.0, back: h.blend_back });
}

fn stop(
    ev: On<StopInteractionCamera>,
    mut commands: Commands,
    view: Option<Res<View>>,
    mut player_camera: Single<&mut PlayerCamera>,
    rig: Option<ResMut<crate::counselor_camera::Rig>>,
    jason_rig: Option<ResMut<crate::jason_player::JasonRig>>,
    play_jason: Option<Res<crate::jason_player::PlayJason>>,
) {
    if ev.killer != play_jason.is_some() {
        return;
    }
    let Some(View::In { to, back, t, time, from }) = view.as_deref().copied() else { return };
    // Where the view is now (mid blend, if it hadn't arrived).
    let now = blend(from, to, ease(t, time));
    // bForceNewCameraRotation: her view takes the camera's yaw, level.
    let f = now.0.forward();
    player_camera.yaw = (-f.x).atan2(-f.z);
    player_camera.pitch = 0.0;
    // ReturnCameraToCharacter ends in ResetCameras: her cameras' arms snap to that view.
    if let Some(mut rig) = rig {
        rig.reset_cameras();
    }
    if let Some(mut rig) = jason_rig {
        rig.reset_cameras();
    }
    commands.insert_resource(View::Back { from: now, time: back, t: 0.0 });
}

/// FMath::InterpEaseInOut(0, 1, t / time, BLEND_EXP).
fn ease(t: f32, time: f32) -> f32 {
    let a = if time > 0.0 { (t / time).clamp(0.0, 1.0) } else { 1.0 };
    if a < 0.5 { 0.5 * (2.0 * a).powf(BLEND_EXP) } else { 1.0 - 0.5 * (2.0 * (1.0 - a)).powf(BLEND_EXP) }
}

/// A Bevy camera rotation as an Unreal rotator (the camera looking along Unreal +X), and back.
pub(crate) fn to_rotator(q: Quat) -> Vec3 {
    let q = q * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
    rotator(Quat::from_xyzw(-q.x, -q.z, -q.y, q.w))
}

pub(crate) fn from_rotator(r: Vec3) -> Quat {
    let u = quaternion(r);
    Quat::from_xyzw(-u.x, -u.z, -u.y, u.w) * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)
}

/// FMinimalViewInfo::BlendViewInfo: location and field of view lerp; the rotator moves the shortest way.
fn blend(a: (Transform, f32), b: (Transform, f32), s: f32) -> (Transform, f32) {
    let (ra, rb) = (to_rotator(a.0.rotation), to_rotator(b.0.rotation));
    let wrap = |d: f32| (d + 180.0).rem_euclid(360.0) - 180.0;
    let d = Vec3::new(wrap(rb.x - ra.x), wrap(rb.y - ra.y), wrap(rb.z - ra.z));
    let t = Transform { translation: a.0.translation.lerp(b.0.translation, s), rotation: from_rotator(ra + d * s), ..a.0 };
    (t, a.1 + (b.1 - a.1) * s)
}

fn interaction_camera(mut commands: Commands, time: Res<Time>, view: Option<ResMut<View>>, mut camera: Single<(&mut Transform, &mut Projection), With<PlayerCamera>>) {
    let Some(mut view) = view else { return };
    let dt = time.delta_secs();
    let (ref mut xf, ref mut proj) = *camera;
    let Projection::Perspective(p) = &mut **proj else { return };
    match &mut *view {
        View::In { from, to, time, t, .. } => {
            *t += dt;
            let (x, fov) = blend(*from, *to, ease(*t, *time));
            **xf = x;
            p.fov = fov;
        }
        View::Back { from, time, t } => {
            *t += dt;
            // To her live view (this frame's, already placed).
            let (x, fov) = blend(*from, (**xf, p.fov), ease(*t, *time));
            **xf = x;
            if *t >= *time {
                commands.remove_resource::<View>();
            }
            p.fov = fov;
        }
    }
}

