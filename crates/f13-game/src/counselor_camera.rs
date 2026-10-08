//! The counselor's cameras, as Base_Counselor has them (MODLOG "Counselor cameras"): its nine
//! ILLDynamicCameraComponents (f13-core::dynamic_camera) on her capsule, stacked and blended by ILLCharacter's camera
//! stack, switched by the Blueprint's ChangeCamera:
//!
//! - ChangeCamera(BlendTime): her stance's camera (Crouching, Swimming -> Water, Driving, Combat, Grabbed -> Grab;
//!   Standing, Prone, Finished and Trapped have none), else Chase while she's chased outdoors on foot
//!   (ShouldUseChaseCamera: not indoors, driving or in combat), else Indoor or Outdoor. In a boat
//!   (ShouldUseDrivingCamera: VehicleType 2) the stance's camera is skipped; the Chevette is a car (1).
//! - OnIndoorsChanged: ChangeCamera(0.5 going in, 1.5 going out). OnStanceChanged: ChangeCamera(0 into Driving, else
//!   0.5). OnChaseStart: ChangeCamera by the last stance event's rule; OnChaseEnd: by the last indoors event's.
//! - Possessed (OnPossessed): SetActiveCamera(OutdoorCamera).
//!
//! Her view (the control rotation) turns with the mouse as the engine does it: Turn / LookUp -> AddControllerYaw /
//! PitchInput of MouseX / MouseY x 0.07 (DefaultInput.ini's axis sensitivity, ILLInputSettingsSaveGame's default) x
//! the FOV scaling (0.01111 x the view's field of view, DefaultInput.ini), x InputYawScale 2.5 / InputPitchScale -2.5
//! (BaseInput.ini); pitch held to ±87 (SCPlayerCameraManager's ViewPitchMin/Max). Her own Turn / LookUp
//! (0x1403a53d0 / 0x1403a5280) also add the same input x CameraRotateScale to a look offset, clamped to
//! MaxVehicleCameraYaw/PitchAngle in a car; driving, her view is the car's rotation plus that offset (her tick in the
//! Driving stance). Taking the seat turns her view to the car's; leaving it, to the camera's yaw, level
//! (SetCurrentSeat, 0x1403d18e0).
//!
//! Chases (SCCounselorCharacter's chase update FUN_1403db8f0 and check FUN_1403bd080): she's chased while she moves and
//! Jason (not stalking or shifting; moving toward her, ChasingKillerMustBeMoving) is within StartChaseDistance
//! (EndChaseDistance once chased) and behind her: the dot of her back and the direction to him at least
//! acos(clamp(angle in radians, -1, 1)) of StartChaseKillerAngle / EndChaseKillerAngle (the game takes acosf of the
//! angle, not cos). It ends ChaseTimeout after the last time that held.
//!
//! The chase shakes the camera (ED_Shake2 at 0.08 from OnChaseStart until OnChaseEnd; UCameraShake's oscillators).
//!
//! PLACEHOLDERS: her seat is taken as
//! she starts getting in (Driving from its enter take on); dead, the arm stays on her capsule (the game's death and
//! spectating cameras aren't ported).

use crate::player::{Player, PlayerCamera};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use f13_core::counselor::Stance;
use f13_core::dynamic_camera::{CameraEvent, CameraStack, DynamicCamera, View};

/// MouseX / MouseY's axis sensitivity (DefaultInput.ini; ILLInputSettingsSaveGame's MouseSensitivity default).
const MOUSE_SENSITIVITY: f32 = 0.07;
/// UPlayerInput's FOV scaling of mouse axes (DefaultInput.ini bEnableFOVScaling, FOVScale): x this x the view's FOV.
const FOV_SCALE: f32 = 0.011110;
/// APlayerController's InputYawScale / InputPitchScale (BaseInput.ini).
const INPUT_YAW_SCALE: f32 = 2.5;
const INPUT_PITCH_SCALE: f32 = -2.5;
/// SCPlayerCameraManager's ViewPitchMin / ViewPitchMax (its constructor, 0x1403e11f0).
const VIEW_PITCH_LIMIT: f32 = 87.0;

/// Base_Counselor's camera components by the names its graph uses.
const OUTDOOR: &str = "OutdoorCamera";
const INDOOR: &str = "IndoorCamera";
const CHASE: &str = "ChaseCamera";
const CROUCHING: &str = "CrouchingCamera";
const WATER: &str = "WaterCamera";
const DRIVING: &str = "DrivingCamera";
const COMBAT: &str = "CombatCamera";
const GRAB: &str = "GrabCamera";
/// OnChaseStart's camera shake and its scale.
const CHASE_SHAKE: &str = "ED_Shake2";
const CHASE_SHAKE_SCALE: f32 = 0.08;

pub struct CounselorCameraPlugin;

impl Plugin for CounselorCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Chase>();
    }
}

/// Her chase (bIsBeingChased, LastChased_Timestamp, ChaseStart_Timestamp).
#[derive(Resource, Default, Debug)]
pub struct Chase {
    pub chased: bool,
    pub last: f32,
    pub start: f32,
    /// ChasingKillerDistance (cm, flat).
    pub distance: f32,
}

/// Unreal (cm, Z up) from Bevy (m, Y up), and back.
fn to_unreal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) * 100.0
}

fn to_bevy(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) / 100.0
}

/// The counselor values the chase and car look read (CharacterInfo values, exporter defaults otherwise).
fn value(info: &f13_data::character::CharacterInfo, name: &str, default: f32) -> f32 {
    info.values.get(name).copied().unwrap_or(default)
}

/// FUN_1403db8f0 and FUN_1403bd080.
pub(crate) fn chase(
    time: Res<Time>,
    mut chase: ResMut<Chase>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    player: Query<(&Player, &Transform, Has<crate::kill_camera::KillCam>), Without<crate::jason::Jason>>,
    jasons: Query<(&crate::jason::Jason, &Transform)>,
) {
    let (Some(counselor), Ok((p, xf, in_kill))) = (counselor, player.single()) else { return };
    let info = &counselor.info;
    let now = time.elapsed_secs();
    let centre = to_unreal(xf.translation) + Vec3::Z * info.capsule_half_height;
    let jason_hh = killer.as_ref().map_or(90.0, |k| k.info.capsule_half_height);
    let her_velocity = Vec3::new(p.velocity.x, p.velocity.y, p.vel_y * 100.0);
    let mut found = None;
    for (j, jxf) in &jasons {
        // A stalking or shifting killer doesn't chase.
        if j.stalking() || j.shifting() {
            continue;
        }
        let d = to_unreal(jxf.translation) + Vec3::Z * jason_hh - centre;
        if value(info, "ChasingKillerMustBeMoving", 1.0) != 0.0 {
            let v = Vec3::new(j.velocity.x, j.velocity.y, 0.0);
            if v.length_squared() <= 0.0 || v.dot(d) >= 0.0 {
                continue;
            }
        }
        let reach = if chase.chased { value(info, "EndChaseDistance", 3000.0) } else { value(info, "StartChaseDistance", 1500.0) };
        if d.length_squared() > reach * reach {
            continue;
        }
        let angle = if chase.chased { value(info, "EndChaseKillerAngle", 90.0) } else { value(info, "StartChaseKillerAngle", 45.0) };
        let threshold = (angle.to_radians()).clamp(-1.0, 1.0).acos();
        // Her back: her yaw + 180 (her capsule doesn't pitch).
        let forward = to_unreal(xf.rotation * Vec3::Z);
        let back = -Vec3::new(forward.x, forward.y, 0.0).normalize_or_zero();
        if back.dot(d.normalize_or_zero()) >= threshold {
            found = Some(d.truncate().length());
            break;
        }
    }
    if let Some(distance) = found.filter(|_| her_velocity.length_squared() > 0.0 && !in_kill) {
        if !chase.chased {
            chase.start = now;
            info!("chase: started ({distance:.0} cm)");
        }
        chase.chased = true;
        chase.last = now;
        chase.distance = distance;
    } else if chase.chased && value(info, "ChaseTimeout", 2.0) <= now - chase.last {
        chase.chased = false;
        info!("chase: ended after {:.1} s", now - chase.start);
    }
}

/// The view's horizontal field of view (degrees) from the camera's projection.
fn horizontal_fov(proj: &Projection, aspect: f32) -> f32 {
    match proj {
        Projection::Perspective(p) => (2.0 * ((p.fov / 2.0).tan() * aspect).atan()).to_degrees(),
        _ => 90.0,
    }
}

fn aspect(windows: &Query<&Window>) -> f32 {
    windows.iter().next().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0))
}

/// Turn / LookUp: the mouse turns her control rotation (and her car look offset).
pub(crate) fn look(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    play_jason: Option<Res<crate::jason_player::PlayJason>>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    mut rig: Option<ResMut<Rig>>,
    driving: Query<(), (With<Player>, With<crate::vehicle::Driving>)>,
    mut camera: Single<(&mut PlayerCamera, &Projection)>,
    windows: Query<&Window>,
) {
    if cursor.grab_mode != CursorGrabMode::Locked || play_jason.is_some() {
        return;
    }
    let (ref mut cam, proj) = *camera;
    let fov = horizontal_fov(proj, aspect(&windows));
    // UPlayerInput: the raw counts x sensitivity x FOV scaling; MouseY's LookUp mapping has scale -1 (and Bevy's y
    // grows downward).
    let turn = motion.delta.x * MOUSE_SENSITIVITY * FOV_SCALE * fov;
    let look_up = motion.delta.y * MOUSE_SENSITIVITY * FOV_SCALE * fov;
    let (yaw, pitch) = (turn * INPUT_YAW_SCALE, look_up * INPUT_PITCH_SCALE);
    let (mut uyaw, mut upitch) = control_rotation(cam);
    uyaw += yaw;
    upitch = (upitch + pitch).clamp(-VIEW_PITCH_LIMIT, VIEW_PITCH_LIMIT);
    set_control_rotation(cam, uyaw, upitch);
    // Her own handlers: the look offset a car uses, clamped while she's in one.
    if let (Some(rig), Some(c)) = (rig.as_mut(), counselor) {
        let scale = Vec2::new(value(&c.info, "CameraRotateScaleX", 0.5), value(&c.info, "CameraRotateScaleY", 0.5));
        rig.vehicle_look.x += yaw * scale.y;
        rig.vehicle_look.y += pitch * scale.x;
        if !driving.is_empty() {
            let (max_yaw, max_pitch) = (value(&c.info, "MaxVehicleCameraYawAngle", 70.0), value(&c.info, "MaxVehicleCameraPitchAngle", 40.0));
            rig.vehicle_look.x = rig.vehicle_look.x.clamp(-max_yaw, max_yaw);
            rig.vehicle_look.y = rig.vehicle_look.y.clamp(-max_pitch, max_pitch);
        }
    }
}

/// Her control rotation as Unreal (yaw, pitch) degrees.
pub fn control_rotation(cam: &PlayerCamera) -> (f32, f32) {
    (-cam.yaw.to_degrees() - 90.0, cam.pitch.to_degrees())
}

pub fn set_control_rotation(cam: &mut PlayerCamera, yaw: f32, pitch: f32) {
    cam.yaw = (-(yaw + 90.0)).to_radians();
    cam.pitch = pitch.to_radians();
}

/// Her cameras and their stack.
#[derive(Resource)]
pub struct Rig {
    names: Vec<String>,
    cameras: Vec<DynamicCamera>,
    active: Vec<bool>,
    stack: CameraStack,
    /// What last called ChangeCamera, as the graph keeps it: her stance, indoors, chased; the last stance and indoors
    /// events' values.
    stance: Stance,
    indoors: bool,
    chased: bool,
    driving: bool,
    stance_event: Stance,
    indoors_event: bool,
    /// Her look offset in a car (yaw, pitch degrees).
    pub vehicle_look: Vec2,
    pub view: View,
    /// What the stack asked of the cameras when the rig was made (applied on its first frame).
    pending: Vec<CameraEvent>,
    /// The chase's camera shake while it plays.
    shake: Option<f13_core::dynamic_camera::ShakeInstance>,
}

impl Rig {
    /// SCCharacter::ResetCameras (from ReturnCameraToCharacter): every stacked camera reactivates, its arm snapping
    /// (on the next frame here).
    pub fn reset_cameras(&mut self) {
        let cams: Vec<usize> = self.stack.entries.iter().map(|e| e.camera).collect();
        self.pending.extend(cams.into_iter().map(CameraEvent::Activate));
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    /// The Blueprint's ChangeCamera.
    fn change_camera(&mut self, blend: f32, now: f32, events: &mut Vec<CameraEvent>) {
        // In a boat the stance's camera is skipped (ShouldUseDrivingCamera); the Chevette is a car.
        let stance_camera = match self.stance {
            Stance::Crouching => Some(CROUCHING),
            Stance::Swimming => Some(WATER),
            Stance::Driving => Some(DRIVING),
            Stance::Combat => Some(COMBAT),
            Stance::Grabbed => Some(GRAB),
            _ => None,
        };
        let chase = self.chased && !(self.indoors || self.stance == Stance::Combat || self.stance == Stance::Driving);
        let name = match stance_camera.filter(|n| self.index(n).is_some()) {
            Some(n) => n,
            None if chase => CHASE,
            None if self.indoors => INDOOR,
            None => OUTDOOR,
        };
        if let Some(i) = self.index(name) {
            debug!("camera: {name} over {blend} s");
            self.stack.set_active_with_blend(i, blend, now, events);
        }
    }
}

/// Builds her rig from the counselor's cameras, the Outdoor camera active (OnPossessed).
fn new_rig(info: &f13_data::character::CharacterInfo, now: f32, events: &mut Vec<CameraEvent>) -> Option<Rig> {
    let mut names: Vec<String> = info.cameras.keys().cloned().collect();
    names.sort();
    let cameras: Vec<DynamicCamera> = names.iter().map(|n| {
        let c = &info.cameras[n];
        DynamicCamera::new(c.settings(), c.use_2d_velocity, c.spring_arm())
    }).collect();
    let mut rig = Rig {
        active: vec![false; names.len()],
        names,
        cameras,
        stack: CameraStack::default(),
        stance: Stance::Standing,
        indoors: false,
        chased: false,
        driving: false,
        stance_event: Stance::Standing,
        indoors_event: false,
        vehicle_look: Vec2::ZERO,
        view: View::default(),
        pending: Vec::new(),
        shake: None,
    };
    let outdoor = rig.index(OUTDOOR)?;
    rig.stack.set_active(outdoor, now, events);
    rig.pending = std::mem::take(events);
    Some(rig)
}

/// Unreal rotator of a Bevy rotation of an actor exported through `to_transform` (its +X is Unreal's forward).
fn actor_rotator(q: Quat) -> Vec3 {
    f13_core::camera_spline::rotator(Quat::from_xyzw(-q.x, -q.z, -q.y, q.w))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn counselor_camera(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<crate::player::PlayerConfig>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    rig: Option<ResMut<Rig>>,
    chase: Res<Chase>,
    surroundings: Option<Res<crate::fear_events::Surroundings>>,
    player: Single<(&Player, &Transform, Option<&crate::vehicle::Driving>, Has<crate::jason::Held>, Has<crate::items::Trapped>), Without<PlayerCamera>>,
    vehicles: Query<&GlobalTransform>,
    mut camera: Single<(&mut PlayerCamera, &mut Transform, &mut Projection)>,
    windows: Query<&Window>,
    (collision, dynamic, blockers): (Option<Res<crate::collision::WorldCollision>>, Res<crate::collision::DynamicColliders>, Query<(), With<crate::collision::CameraBlocker>>),
    mut rng: ResMut<crate::GameRng>,
) {
    let (p, pxf, driving, held, trapped) = *player;
    let (ref mut cam, ref mut xf, ref mut proj) = *camera;
    if let Some((eye, target)) = config.camera {
        xf.translation = pxf.translation + eye;
        xf.look_at(pxf.translation + target, Vec3::Y);
        return;
    }
    let now = time.elapsed_secs();
    let mut events = Vec::new();
    let mut rig = match rig {
        Some(r) => r,
        None => {
            let Some(r) = counselor.as_ref().and_then(|c| new_rig(&c.info, now, &mut events)) else { return };
            // Possession turns her view to her facing, level (AController::Possess).
            let f = to_unreal(pxf.rotation * Vec3::Z);
            set_control_rotation(cam, f.y.atan2(f.x).to_degrees(), 0.0);
            commands.insert_resource(r);
            return;
        }
    };
    let rig = &mut *rig;
    events.append(&mut rig.pending);

    // Her stance as the game keeps it (Driving in a seat, Grabbed in his hands, Trapped in a bear trap).
    let stance = if driving.is_some() {
        Stance::Driving
    } else if held {
        Stance::Grabbed
    } else if trapped {
        Stance::Trapped
    } else {
        p.counselor.stance
    };
    let indoors = surroundings.as_ref().is_some_and(|s| s.indoors(pxf.translation));
    let vehicle = driving.and_then(|d| vehicles.get(d.vehicle).ok()).map(|g| actor_rotator(g.compute_transform().rotation));
    // SetCurrentSeat: getting in, her view turns to the car's; getting out, to the camera's yaw, level.
    if driving.is_some() != rig.driving {
        rig.driving = driving.is_some();
        match vehicle {
            Some(v) if rig.driving => set_control_rotation(cam, v.y, v.x),
            _ => set_control_rotation(cam, rig.view.rotation.y, 0.0),
        }
    }
    // Driving: her view is the car's rotation plus her look offset.
    if let (Some(v), Stance::Driving) = (vehicle, stance) {
        set_control_rotation(cam, v.y + rig.vehicle_look.x, v.x + rig.vehicle_look.y);
    }
    if stance != rig.stance {
        rig.stance = stance;
        rig.stance_event = stance;
        rig.change_camera(if stance == Stance::Driving { 0.0 } else { 0.5 }, now, &mut events);
    }
    if indoors != rig.indoors {
        rig.indoors = indoors;
        rig.indoors_event = indoors;
        rig.change_camera(if indoors { 0.5 } else { 1.5 }, now, &mut events);
    }
    if chase.chased != rig.chased {
        rig.chased = chase.chased;
        // OnChaseStart: PlayCameraShake(ED_Shake2, 0.08); OnChaseEnd: StopCameraShakeInstances(ED_Shake2), at once.
        rig.shake = match (chase.chased, counselor.as_ref().and_then(|c| c.info.camera_shakes.get(CHASE_SHAKE))) {
            (true, Some(s)) => Some(f13_core::dynamic_camera::ShakeInstance::new(f13_data::character::camera_shake(s), CHASE_SHAKE_SCALE, &mut || rng.0.frand())),
            _ => None,
        };
        let blend = if chase.chased {
            if rig.stance_event == Stance::Driving { 0.0 } else { 0.5 }
        } else if rig.indoors_event {
            0.5
        } else {
            1.5
        };
        rig.change_camera(blend, now, &mut events);
    }

    // The arms hang off her capsule's centre.
    let origin = to_unreal(pxf.translation) + Vec3::Z * crate::player::CAPSULE_HALF_HEIGHT_M * 100.0;
    let facing = to_unreal(pxf.rotation * Vec3::Z);
    let component_rot = Vec3::new(0.0, facing.y.atan2(facing.x).to_degrees(), 0.0);
    let (yaw, pitch) = control_rotation(cam);
    let control = Vec3::new(pitch, yaw, 0.0);
    let velocity = Vec3::new(p.velocity.x, p.velocity.y, p.vel_y * 100.0);
    let dt = time.delta_secs();

    let apply = |rig: &mut Rig, events: &mut Vec<CameraEvent>| {
        for e in events.drain(..) {
            match e {
                CameraEvent::Activate(i) => {
                    // The dynamic camera's Activate (0x140216330): its arm snaps with what it last had (a camera's
                    // first time, the fresh USpringArmComponent its OnComponentCreated made: 300 cm, no offsets).
                    rig.active[i] = true;
                    rig.cameras[i].arm.snap(origin, component_rot, control);
                }
                CameraEvent::Deactivate(i) => rig.active[i] = false,
            }
        }
    };
    apply(rig, &mut events);

    // Each active camera: its settings for her speed, then its arm.
    let sweep = |from: Vec3, to: Vec3, radius: f32| -> Option<Vec3> {
        let world = collision.as_ref()?;
        let lists: Vec<&f13_core::collision::TriList> = dynamic.iter().filter(|(e, _)| blockers.contains(*e)).map(|(_, l)| l).collect();
        let f = f13_core::collision::sphere_sweep(&(&*world.camera_grid, lists.as_slice()), to_bevy(from), to_bevy(to), radius / 100.0)?;
        Some(from + (to - from) * f)
    };
    for i in 0..rig.cameras.len() {
        if !rig.active[i] {
            continue;
        }
        rig.cameras[i].tick(now, velocity);
        rig.cameras[i].arm.tick(origin, component_rot, control, dt, sweep);
    }
    let active = rig.active.clone();
    rig.stack.update(now, |i| active[i], &mut events);
    apply(rig, &mut events);
    let views: Vec<View> = rig.cameras.iter().map(|c| View { location: c.arm.socket.0, rotation: c.arm.socket.1, fov: c.arm.settings.field_of_view }).collect();
    let Some(mut view) = rig.stack.view(|i| views[i]) else { return };
    rig.view = view;
    // The camera manager's shakes, in the camera's own space.
    if let Some(s) = rig.shake.as_mut() {
        match s.update(dt) {
            Some(o) => {
                let q = f13_core::camera_spline::quaternion(view.rotation);
                view.location += q * o.location;
                view.rotation = f13_core::camera_spline::rotator(q * f13_core::camera_spline::quaternion(o.rotation));
                view.fov += o.fov;
            }
            None => rig.shake = None,
        }
    }
    xf.translation = to_bevy(view.location);
    xf.rotation = crate::interaction_camera::from_rotator(view.rotation);
    if let Projection::Perspective(persp) = &mut **proj {
        persp.fov = 2.0 * ((view.fov.to_radians() / 2.0).tan() / aspect(&windows)).atan();
    }
}
