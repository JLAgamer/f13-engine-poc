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
            .add_systems(Update, kill_camera.after(crate::player::follow_camera).after(crate::hiding::hiding_camera));
    }
}

#[derive(Resource, Default)]
pub struct KillCameras(HashMap<String, CameraSpline>);

/// A kill's camera running: the kill's origin (Jason's actor transform at its start, in Bevy's space) and time.
#[derive(Component)]
pub struct KillCam {
    pub kill: &'static str,
    pub origin: Transform,
    pub t: f32,
    handicam: Handicam,
}

impl KillCam {
    pub fn new(kill: &'static str, jason: Transform) -> Self {
        KillCam { kill, origin: Transform::from_matrix(jason.to_matrix() * Transform::from_rotation(crate::character::FACING).to_matrix()), t: 0.0, handicam: Handicam::default() }
    }
}

fn load(export: Res<crate::ExportRoot>, mut cameras: ResMut<KillCameras>) {
    match f13_data::kill_camera::load(&export.0.join("kill_cameras.json")) {
        Ok(c) => cameras.0 = c,
        Err(e) => warn!("kill cameras: {e} (f13-export killcams): kills keep the follow camera"),
    }
}

pub(crate) fn kill_camera(
    mut commands: Commands,
    time: Res<Time>,
    cameras: Res<KillCameras>,
    mut kills: Query<(Entity, &mut KillCam)>,
    mut camera: Single<(&mut Transform, &mut Projection), With<PlayerCamera>>,
    windows: Query<&Window>,
    mut normal_fov: Local<Option<f32>>,
) {
    let (ref mut xf, ref mut proj) = *camera;
    let Projection::Perspective(persp) = &mut **proj else { return };
    let Some((e, mut k)) = kills.iter_mut().next() else {
        if let Some(fov) = normal_fov.take() {
            persp.fov = fov;
        }
        return;
    };
    let Some(spline) = cameras.0.get(k.kill) else {
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
        return;
    }
    let p = spline.at(k.t);
    let shake = k.handicam.tick(dt, HANDICAM_FREQUENCY, 0.0, 0.0, p.shake);
    let r = quaternion(rotator(p.rotation) + shake);
    let local = Xform { t: p.location.to_array(), r: r.to_array(), s: [1.0; 3] }.to_gltf_space();
    let local = Transform { translation: Vec3::from_array(local.translation), rotation: Quat::from_array(local.rotation), scale: Vec3::ONE };
    let world = k.origin.mul_transform(local);
    // The camera looks along Unreal +X (Bevy +X here); a Bevy camera looks along -Z.
    xf.translation = world.translation;
    xf.rotation = world.rotation * Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2);
    // Unreal's FieldOfView is horizontal.
    let aspect = windows.iter().next().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0));
    persp.fov = 2.0 * ((p.fov.to_radians() / 2.0).tan() / aspect).atan();
}
