//! Kill cameras (f13-core::camera_spline): while Jason kills the counselor, the camera rides the kill's camera spline
//! (`kill_cameras.json`) in the kill's space, Jason's actor at the kill's start (the paired animations play around it),
//! with its field of view and handicam shake, then hands back to the follow camera.
//!
//! The escape camera (SCEscapeVolume, FUN_1404439f0 from OnOverlap: for the locally controlled character escaping on
//! foot, and each local passenger of a car escaping): her control rotation takes its EscapeCam's, TakeCameraControl
//! blends her view to the volume's over CameraLerpTime (SetViewTargetWithBlend, linear: 7 s on Crystal Lake), and
//! ActivateCamera starts EscapeCam (an SCSplineCamera) riding the volume's EscapeCameraSpline (two points over 8 s on
//! this map). It holds at the spline's end.

use crate::player::PlayerCamera;
use bevy::prelude::*;
use f13_core::camera_spline::{quaternion, rotator, CameraSpline, Handicam};
use f13_data::level::Xform;
use std::collections::HashMap;

/// The kills' SCSplineCamera templates' HandicamShakeFrequency where the export has none for a kill (the throat hack's)
/// (HandicamShakeFrequenceRandomizer is left at 0).
const HANDICAM_FREQUENCY: Vec3 = Vec3::new(1.0, 1.0, 0.5);

/// The kill, blend-back and escape cameras (the spectator's fixed views come after).
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct KillCameraSystems;

pub struct KillCameraPlugin;

impl Plugin for KillCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KillCameras>()
            .add_systems(Startup, load)
            .add_observer(start_escape_camera)
            .add_systems(Update, (kill_camera, blend_back, escape_camera).chain().in_set(KillCameraSystems).after(crate::counselor_camera::counselor_camera).after(crate::hiding::hiding_camera));
    }
}

/// Each kill's camera spline, and its place under the kill's root (Unreal cm and rotation).
#[derive(Resource, Default)]
pub struct KillCameras(HashMap<String, CameraSpline>, HashMap<String, (Vec3, Quat)>, HashMap<String, Vec3>);

/// The kill camera's BlendBackTime (the kills' special moves: 0.5 s): the camera eases from where the kill camera
/// left it to the follow camera.
const BLEND_BACK_SECONDS: f32 = 0.5;

/// Easing back from a kill camera: where it ended, and for how long it's been easing.
#[derive(Resource)]
struct BlendBack {
    from: Transform,
    t: f32,
}

fn blend_back(mut commands: Commands, time: Res<Time>, blend: Option<ResMut<BlendBack>>, mut camera: Single<&mut Transform, With<PlayerCamera>>, kills: Query<(), With<KillCam>>) {
    let Some(mut b) = blend else { return };
    if !kills.is_empty() {
        return;
    }
    b.t += time.delta_secs();
    let s = (b.t / BLEND_BACK_SECONDS).clamp(0.0, 1.0);
    let s = s * s * (3.0 - 2.0 * s);
    let to = **camera;
    camera.translation = b.from.translation.lerp(to.translation, s);
    camera.rotation = b.from.rotation.slerp(to.rotation, s);
    if s >= 1.0 {
        commands.remove_resource::<BlendBack>();
    }
}

/// A kill's camera running: the kill's origin (Jason's actor transform at its start, in Bevy's space) and time.
#[derive(Component)]
pub struct KillCam {
    pub kill: String,
    pub origin: Transform,
    /// Whether `origin` is the kill's root (a grab kill: the spline sits under it at its own place) rather than the
    /// spline's own frame.
    at_root: bool,
    pub t: f32,
    handicam: Handicam,
    /// The kill's victim (the killer is the entity it's on), and whether every player watches (bPlayForEveryone).
    /// SCContextKillComponent::ActivateKillCam (0x140333b40): only a locally controlled killer or victim sees it, else
    /// with bPlayForEveryone the first local player.
    pub victim: Option<Entity>,
    pub everyone: bool,
}

impl KillCam {
    /// The server's kill camera on a client (net.rs): its kill, origin and whether that's the kill's root.
    pub fn remote(kill: &str, origin: Transform, at_root: bool, victim: Option<Entity>, everyone: bool) -> Self {
        KillCam { at_root, victim, everyone, ..Self::at(kill, origin) }
    }

    /// Its victim.
    pub fn of(self, victim: Entity) -> Self {
        KillCam { victim: Some(victim), ..self }
    }

    /// Every player watches it (bPlayForEveryone).
    pub fn for_everyone(self) -> Self {
        KillCam { everyone: true, ..self }
    }

    /// The parts net.rs sends.
    pub fn parts(&self) -> (&str, Transform, bool) {
        (&self.kill, self.origin, self.at_root)
    }

    pub fn new(kill: &str, jason: Transform) -> Self {
        let origin = Transform::from_matrix(jason.to_matrix() * Transform::from_rotation(crate::character::FACING).to_matrix());
        KillCam { at_root: true, ..Self::at(kill, origin) }
    }

    /// From its camera spline's own frame (a placed kill's, under its ContextKillComponent).
    pub fn at(kill: &str, origin: Transform) -> Self {
        KillCam { kill: kill.to_string(), origin, at_root: false, t: 0.0, handicam: Handicam::default(), victim: None, everyone: false }
    }
}

fn load(export: Res<crate::ExportRoot>, mut cameras: ResMut<KillCameras>) {
    let path = export.0.join("kill_cameras.json");
    match f13_data::kill_camera::load(&path) {
        Ok(c) => cameras.0 = c,
        Err(e) => warn!("kill cameras: {e} (f13-export killcams): kills keep the follow camera"),
    }
    cameras.1 = f13_data::kill_camera::load_frames(&path).unwrap_or_default();
    cameras.2 = f13_data::kill_camera::load_frequencies(&path).unwrap_or_default();
}

pub(crate) fn kill_camera(
    mut commands: Commands,
    time: Res<Time>,
    cameras: Res<KillCameras>,
    mut kills: Query<(Entity, &mut KillCam)>,
    mut camera: Single<(&mut Transform, &mut Projection), With<PlayerCamera>>,
    windows: Query<&Window>,
    her: Query<&Transform, (With<crate::player::Player>, Without<PlayerCamera>)>,
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
    (play, locals, jasons): (Option<Res<crate::jason_player::PlayJason>>, Query<(), With<crate::controls::LocalPlayer>>, Query<(), With<crate::jason::Jason>>),
) {
    let (ref mut xf, ref mut proj) = *camera;
    let Projection::Perspective(persp) = &mut **proj else { return };
    // No kill here: her cameras (counselor_camera.rs) have the view and its field of view. A kill is this player's when
    // they play its killer (Jason, the entity it's on) or its victim, or it's for everyone.
    let mine = |e: Entity, k: &KillCam| k.everyone || (play.is_some() && jasons.contains(e)) || k.victim.is_some_and(|v| locals.contains(v));
    let Some((e, mut k)) = kills.iter_mut().find(|(e, k)| mine(*e, k)) else { return };
    let Some(spline) = cameras.0.get(k.kill.as_str()) else {
        commands.entity(e).remove::<KillCam>();
        return;
    };
    let dt = time.delta_secs();
    if k.t == 0.0 {
        info!("kill camera: {} ({})", k.kill, if k.everyone { "everyone's" } else { "this player's" });
    }
    k.t += dt;
    if k.t >= spline.duration() {
        commands.entity(e).remove::<KillCam>();
        commands.insert_resource(BlendBack { from: **xf, t: 0.0 });
        return;
    }
    let p = spline.at(k.t);
    let frequency = cameras.2.get(k.kill.as_str()).copied().unwrap_or(HANDICAM_FREQUENCY);
    let shake = k.handicam.tick(dt, frequency, 0.0, 0.0, p.shake);
    let (mut at, mut r) = (p.location, quaternion(rotator(p.rotation) + shake));
    // A grab kill's spline sits at its own place under the kill's root.
    if let Some(&(t, q)) = k.at_root.then(|| cameras.1.get(k.kill.as_str())).flatten() {
        (at, r) = (t + q * at, q * r);
    }
    let local = Xform { t: at.to_array(), r: r.to_array(), s: [1.0; 3] }.to_gltf_space();
    let local = Transform { translation: Vec3::from_array(local.translation), rotation: Quat::from_array(local.rotation), scale: Vec3::ONE };
    let world = k.origin.mul_transform(local);
    // The camera looks along Unreal +X (Bevy +X here); a Bevy camera looks along -Z.
    xf.rotation = world.rotation * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
    let mut fov = p.fov;
    xf.translation = match her.iter().next() {
        Some(h) => trace(&collision.with(&dynamic), world.translation, world.rotation * Vec3::X, h.translation + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M, &mut fov),
        None => world.translation,
    };
    // Unreal's FieldOfView is horizontal.
    let aspect = windows.iter().next().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
    persp.fov = 2.0 * ((fov.to_radians() / 2.0).tan() / aspect).atan();
}

/// The escape volume `zone`'s escape camera starts (she's escaping through it).
#[derive(Event, Clone, Copy, Debug)]
pub struct EscapeCamera {
    pub zone: Entity,
}

/// The escape camera running: its spline and frame (world), the blend's length, and its time.
#[derive(Resource)]
struct EscapeCam {
    spline: CameraSpline,
    origin: Transform,
    lerp: f32,
    t: f32,
    handicam: Handicam,
}

fn start_escape_camera(ev: On<EscapeCamera>, mut commands: Commands, zones: Query<&crate::props::EscapeZone>, play: Option<Res<crate::jason_player::PlayJason>>) {
    // Hers only when she's the local player.
    if play.is_some() {
        return;
    }
    let Some((spline, origin, lerp)) = zones.get(ev.zone).ok().and_then(|z| z.camera.clone().map(|(s, o)| (s, o, z.camera_lerp))) else { return };
    info!("escape camera: {:.1} s spline, {lerp:.1} s blend", spline.duration());
    commands.insert_resource(EscapeCam { spline, origin, lerp, t: 0.0, handicam: Handicam::default() });
}

/// The escape camera's view, blended linearly from her own view (her camera's, as her cameras keep setting it) over
/// the blend time.
fn escape_camera(
    time: Res<Time>,
    cam: Option<ResMut<EscapeCam>>,
    mut camera: Single<(&mut Transform, &mut Projection), With<PlayerCamera>>,
    windows: Query<&Window>,
) {
    let Some(mut c) = cam else { return };
    let (ref mut xf, ref mut proj) = *camera;
    let Projection::Perspective(persp) = &mut **proj else { return };
    let dt = time.delta_secs();
    c.t += dt;
    let p = c.spline.at(c.t);
    let shake = c.handicam.tick(dt, HANDICAM_FREQUENCY, 0.0, 0.0, p.shake);
    let local = Xform { t: p.location.to_array(), r: quaternion(rotator(p.rotation) + shake).to_array(), s: [1.0; 3] }.to_gltf_space();
    let local = Transform { translation: Vec3::from_array(local.translation), rotation: Quat::from_array(local.rotation), scale: Vec3::ONE };
    let world = c.origin.mul_transform(local);
    // ActivateCamera(EscapeCam, null, ...): no focus actor, so no CameraTrace.
    let (fov, at) = (p.fov, world.translation);
    let rotation = world.rotation * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
    let aspect = windows.iter().next().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
    let vfov = 2.0 * ((fov.to_radians() / 2.0).tan() / aspect).atan();
    let s = if c.lerp > 0.0 { (c.t / c.lerp).clamp(0.0, 1.0) } else { 1.0 };
    xf.translation = xf.translation.lerp(at, s);
    xf.rotation = xf.rotation.slerp(rotation, s);
    persp.fov += (vfov - persp.fov) * s;
}

/// SCSplineCamera's CameraTrace (its tick, 0x140362f40): from the point it looks at, as far ahead as its focus (here
/// the counselor) is, back to the camera; something in between pulls the camera to it (5 cm off the surface) and
/// widens the field of view by the distance squared over the trace's length (cm), kept in 30..120 degrees.
fn trace(world: &(impl f13_core::collision::TriSource + ?Sized), cam: Vec3, forward: Vec3, focus: Vec3, fov: &mut f32) -> Vec3 {
    let len = cam.distance(focus);
    let ahead = cam + forward * len;
    let Some(f) = f13_core::collision::segment_hit(world, ahead, cam, 1.0) else { return cam };
    let hit = ahead + (cam - ahead) * f;
    let d = cam.distance(hit) * 100.0;
    *fov = (*fov + d * d / (len * 100.0).max(1.0)).clamp(30.0, 120.0);
    hit + forward * 0.05
}
