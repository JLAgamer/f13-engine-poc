//! Kill cameras (f13-core::camera_spline): while Jason kills the counselor, the camera rides the kill's camera spline
//! (`kill_cameras.json`) in the kill's space, Jason's actor at the kill's start (the paired animations play around it),
//! with its field of view and handicam shake, then hands back to the follow camera.

use crate::player::PlayerCamera;
use bevy::prelude::*;
use f13_core::camera_spline::{quaternion, rotator, CameraSpline, Handicam};
use f13_data::level::Xform;
use std::collections::HashMap;

/// The kills' SCSplineCamera templates' HandicamShakeFrequency (the throat hack's) (HandicamShakeFrequenceRandomizer is left at 0).
const HANDICAM_FREQUENCY: Vec3 = Vec3::new(1.0, 1.0, 0.5);

pub struct KillCameraPlugin;

impl Plugin for KillCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KillCameras>()
            .add_systems(Startup, load)
            .add_systems(Update, (kill_camera, blend_back).chain().after(crate::player::follow_camera).after(crate::hiding::hiding_camera));
    }
}

/// Each kill's camera spline, and its place under the kill's root (Unreal cm and rotation).
#[derive(Resource, Default)]
pub struct KillCameras(HashMap<String, CameraSpline>, HashMap<String, (Vec3, Quat)>);

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
}

impl KillCam {
    pub fn new(kill: &str, jason: Transform) -> Self {
        let origin = Transform::from_matrix(jason.to_matrix() * Transform::from_rotation(crate::character::FACING).to_matrix());
        KillCam { at_root: true, ..Self::at(kill, origin) }
    }

    /// From its camera spline's own frame (a placed kill's, under its ContextKillComponent).
    pub fn at(kill: &str, origin: Transform) -> Self {
        KillCam { kill: kill.to_string(), origin, at_root: false, t: 0.0, handicam: Handicam::default() }
    }
}

fn load(export: Res<crate::ExportRoot>, mut cameras: ResMut<KillCameras>) {
    let path = export.0.join("kill_cameras.json");
    match f13_data::kill_camera::load(&path) {
        Ok(c) => cameras.0 = c,
        Err(e) => warn!("kill cameras: {e} (f13-export killcams): kills keep the follow camera"),
    }
    cameras.1 = f13_data::kill_camera::load_frames(&path).unwrap_or_default();
}

pub(crate) fn kill_camera(
    mut commands: Commands,
    time: Res<Time>,
    cameras: Res<KillCameras>,
    mut kills: Query<(Entity, &mut KillCam)>,
    mut camera: Single<(&mut Transform, &mut Projection), With<PlayerCamera>>,
    windows: Query<&Window>,
    mut normal_fov: Local<Option<f32>>,
    her: Query<&Transform, (With<crate::player::Player>, Without<PlayerCamera>)>,
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
) {
    let (ref mut xf, ref mut proj) = *camera;
    let Projection::Perspective(persp) = &mut **proj else { return };
    let Some((e, mut k)) = kills.iter_mut().next() else {
        if let Some(fov) = normal_fov.take() {
            persp.fov = fov;
        }
        return;
    };
    let Some(spline) = cameras.0.get(k.kill.as_str()) else {
        commands.entity(e).remove::<KillCam>();
        return;
    };
    let normal = *normal_fov.get_or_insert(persp.fov);
    let dt = time.delta_secs();
    k.t += dt;
    if k.t >= spline.duration() {
        persp.fov = normal;
        *normal_fov = None;
        commands.entity(e).remove::<KillCam>();
        commands.insert_resource(BlendBack { from: **xf, t: 0.0 });
        return;
    }
    let p = spline.at(k.t);
    let shake = k.handicam.tick(dt, HANDICAM_FREQUENCY, 0.0, 0.0, p.shake);
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
