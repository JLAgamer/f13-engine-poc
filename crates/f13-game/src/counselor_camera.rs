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
//! Her seat is hers from the end of her enter take (OnEnterVehicleComplete -> SetCurrentSeat) to the start of her exit
//! (MULTICAST_ExitVehicle -> SetCurrentSeat(none)): her Driving stance, the car's view and the look offset's clamp.
//!
//! PLACEHOLDER: dead, the arm stays on her capsule (the game's death and spectating cameras aren't ported).

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

/// Her chase (bIsBeingChased, LastChased_Timestamp, ChaseStart_Timestamp): each counselor's (a component), and the
/// local one's (the resource: her camera and music).
#[derive(Resource, Default, Debug, Clone)]
pub struct Chase {
    pub chased: bool,
    pub last: f32,
    pub start: f32,
    /// ChasingKillerDistance (cm, flat).
    pub distance: f32,
}

/// Each counselor's chase.
#[derive(Component, Default, Debug, Clone)]
pub struct HerChase(pub Chase);

/// Unreal (cm, Z up) from Bevy (m, Y up), and back.
pub(crate) fn to_unreal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) * 100.0
}

pub(crate) fn to_bevy(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) / 100.0
}

/// The counselor values the chase and car look read (CharacterInfo values, exporter defaults otherwise).
fn value(info: &f13_data::character::CharacterInfo, name: &str, default: f32) -> f32 {
    info.values.get(name).copied().unwrap_or(default)
}

/// FUN_1403db8f0 and FUN_1403bd080.
pub(crate) fn chase(
    mut commands: Commands,
    time: Res<Time>,
    mut local_chase: ResMut<Chase>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    mut players: Query<(Entity, &Player, &Transform, Has<crate::kill_camera::KillCam>, Option<&mut HerChase>, Has<crate::controls::LocalPlayer>), Without<crate::jason::Jason>>,
    jasons: Query<(&crate::jason::Jason, &Transform)>,
) {
    let Some(counselor) = counselor else { return };
    for (pe, p, xf, in_kill, chase, local) in &mut players {
        let Some(mut chase) = chase else {
            commands.entity(pe).insert(HerChase::default());
            continue;
        };
        chase_tick(&time, &mut chase.0, &counselor, killer.as_deref(), p, xf, in_kill, &jasons);
        if local {
            *local_chase = chase.0.clone();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn chase_tick(time: &Time, chase: &mut Chase, counselor: &crate::character::CounselorCharacter, killer: Option<&crate::character::KillerCharacter>, p: &Player, xf: &Transform, in_kill: bool, jasons: &Query<(&crate::jason::Jason, &Transform)>) {
    let info = &counselor.info;
    let now = time.elapsed_secs();
    let centre = to_unreal(xf.translation) + Vec3::Z * info.capsule_half_height;
    let jason_hh = killer.as_ref().map_or(90.0, |k| k.info.capsule_half_height);
    let her_velocity = Vec3::new(p.velocity.x, p.velocity.y, p.vel_y * 100.0);
    let mut found = None;
    for (j, jxf) in jasons {
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
pub(crate) fn horizontal_fov(proj: &Projection, aspect: f32) -> f32 {
    match proj {
        Projection::Perspective(p) => (2.0 * ((p.fov / 2.0).tan() * aspect).atan()).to_degrees(),
        _ => 90.0,
    }
}

pub(crate) fn aspect(windows: &Query<&Window>) -> f32 {
    windows.iter().next().map_or(16.0 / 9.0, |w| w.width() / w.height().max(1.0))
}

/// Turn / LookUp's change to the control rotation (yaw, pitch degrees) for a mouse movement.
pub(crate) fn mouse_turn(delta: Vec2, proj: &Projection, windows: &Query<&Window>) -> (f32, f32) {
    let fov = horizontal_fov(proj, aspect(windows));
    // UPlayerInput: the raw counts x sensitivity x FOV scaling; MouseY's LookUp mapping has scale -1 (and Bevy's y
    // grows downward).
    let turn = delta.x * MOUSE_SENSITIVITY * FOV_SCALE * fov;
    let look_up = delta.y * MOUSE_SENSITIVITY * FOV_SCALE * fov;
    (turn * INPUT_YAW_SCALE, look_up * INPUT_PITCH_SCALE)
}

/// Adds to the control rotation, its pitch held to the camera manager's limits.
pub(crate) fn add_control_rotation(cam: &mut PlayerCamera, yaw: f32, pitch: f32) {
    let (uyaw, upitch) = control_rotation(cam);
    set_control_rotation(cam, uyaw + yaw, (upitch + pitch).clamp(-VIEW_PITCH_LIMIT, VIEW_PITCH_LIMIT));
}

/// Turn / LookUp: the mouse turns her control rotation (and her car look offset).
pub(crate) fn look(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    play_jason: Option<Res<crate::jason_player::PlayJason>>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    mut rig: Option<ResMut<Rig>>,
    driving: Query<&crate::vehicle::Driving, With<Player>>,
    mut camera: Single<(&mut PlayerCamera, &Projection)>,
    windows: Query<&Window>,
    wheel: Option<Res<crate::emotes::EmoteWheel>>,
) {
    // (The emote wheel takes the mouse while it's open.)
    if cursor.grab_mode != CursorGrabMode::Locked || play_jason.is_some() || wheel.is_some() {
        return;
    }
    let (ref mut cam, proj) = *camera;
    let (yaw, pitch) = mouse_turn(motion.delta, proj, &windows);
    add_control_rotation(cam, yaw, pitch);
    // Her own handlers: the look offset a car uses, clamped while she's in one.
    if let (Some(rig), Some(c)) = (rig.as_mut(), counselor) {
        let scale = Vec2::new(value(&c.info, "CameraRotateScaleX", 0.5), value(&c.info, "CameraRotateScaleY", 0.5));
        rig.vehicle_look.x += yaw * scale.y;
        rig.vehicle_look.y += pitch * scale.x;
        if driving.iter().any(|d| d.seat_move.is_none()) {
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

/// A character's ILLDynamicCameraComponents and ILLCharacter's camera stack over them.
pub struct Cameras {
    names: Vec<String>,
    cameras: Vec<DynamicCamera>,
    active: Vec<bool>,
    stack: CameraStack,
    /// What the stack asked of the cameras since the last tick (applied at its start).
    pending: Vec<CameraEvent>,
}

impl Cameras {
    pub fn new(info: &f13_data::character::CharacterInfo) -> Self {
        let mut names: Vec<String> = info.cameras.keys().cloned().collect();
        names.sort();
        let cameras = names.iter().map(|n| {
            let c = &info.cameras[n];
            DynamicCamera::new(c.settings(), c.use_2d_velocity, c.spring_arm())
        }).collect();
        Cameras { active: vec![false; names.len()], names, cameras, stack: CameraStack::default(), pending: Vec::new() }
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    /// ILLCharacter::SetActiveCamera.
    pub fn set_active(&mut self, name: &str, now: f32) -> bool {
        let Some(i) = self.index(name) else { return false };
        self.stack.set_active(i, now, &mut self.pending);
        true
    }

    /// ILLCharacter::SetActiveCameraWithBlend.
    pub fn set_active_with_blend(&mut self, name: &str, blend: f32, now: f32) {
        if let Some(i) = self.index(name) {
            debug!("camera: {name} over {blend} s");
            self.stack.set_active_with_blend(i, blend, now, &mut self.pending);
        }
    }

    /// SCCharacter::ResetCameras: every stacked camera reactivates, its arm snapping (on the next tick).
    pub fn reset(&mut self) {
        let cams: Vec<usize> = self.stack.entries.iter().map(|e| e.camera).collect();
        self.pending.extend(cams.into_iter().map(CameraEvent::Activate));
    }

    /// A frame: the stack's events, each active camera's settings for the character's velocity (cm/s) and its arm
    /// from `origin(camera name)` (Unreal cm), then the stack's blend of their views.
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        now: f32,
        dt: f32,
        velocity: Vec3,
        component_rot: Vec3,
        control: Vec3,
        origin: impl Fn(&str) -> Vec3,
        sweep: impl Fn(Vec3, Vec3, f32) -> Option<Vec3>,
    ) -> Option<View> {
        let apply = |c: &mut Cameras| {
            for e in std::mem::take(&mut c.pending) {
                match e {
                    CameraEvent::Activate(i) => {
                        // The dynamic camera's Activate (0x140216330): its arm snaps with what it last had (a camera's
                        // first time, the fresh USpringArmComponent its OnComponentCreated made: 300 cm, no offsets).
                        c.active[i] = true;
                        let o = origin(&c.names[i]);
                        c.cameras[i].arm.snap(o, component_rot, control);
                    }
                    CameraEvent::Deactivate(i) => c.active[i] = false,
                }
            }
        };
        apply(self);
        for i in 0..self.cameras.len() {
            if !self.active[i] {
                continue;
            }
            let o = origin(&self.names[i]);
            self.cameras[i].tick(now, velocity);
            self.cameras[i].arm.tick(o, component_rot, control, dt, &sweep);
        }
        let active = self.active.clone();
        let mut events = Vec::new();
        self.stack.update(now, |i| active[i], &mut events);
        self.pending = events;
        apply(self);
        let views: Vec<View> = self.cameras.iter().map(|c| View { location: c.arm.socket.0, rotation: c.arm.socket.1, fov: c.arm.settings.field_of_view }).collect();
        self.stack.view(|i| views[i])
    }
}

/// Her cameras and their stack.
#[derive(Resource)]
pub struct Rig {
    cams: Cameras,
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
    /// The chase's camera shake while it plays.
    shake: Option<f13_core::dynamic_camera::ShakeInstance>,
    /// The counselor whose cameras these are (the local one, or the one spectated).
    subject: Option<Entity>,
}

impl Rig {
    /// SCCharacter::ResetCameras (from ReturnCameraToCharacter): every stacked camera reactivates, its arm snapping
    /// (on the next frame here).
    pub fn reset_cameras(&mut self) {
        self.cams.reset();
    }

    /// The Blueprint's ChangeCamera.
    fn change_camera(&mut self, blend: f32, now: f32) {
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
        let name = match stance_camera.filter(|n| self.cams.index(n).is_some()) {
            Some(n) => n,
            None if chase => CHASE,
            None if self.indoors => INDOOR,
            None => OUTDOOR,
        };
        self.cams.set_active_with_blend(name, blend, now);
    }
}

/// Builds her rig from the counselor's cameras, the Outdoor camera active (OnPossessed).
fn new_rig(info: &f13_data::character::CharacterInfo, now: f32) -> Option<Rig> {
    let mut rig = Rig {
        cams: Cameras::new(info),
        stance: Stance::Standing,
        indoors: false,
        chased: false,
        driving: false,
        stance_event: Stance::Standing,
        indoors_event: false,
        vehicle_look: Vec2::ZERO,
        view: View::default(),
        shake: None,
        subject: None,
    };
    rig.cams.set_active(OUTDOOR, now).then_some(rig)
}

/// The spring arms' probe sweep (Unreal cm) against what blocks the camera.
pub(crate) fn camera_sweep<'a>(
    collision: Option<&'a crate::collision::WorldCollision>,
    dynamic: &'a crate::collision::DynamicColliders,
    blockers: &'a Query<(), With<crate::collision::CameraBlocker>>,
) -> impl Fn(Vec3, Vec3, f32) -> Option<Vec3> + 'a {
    move |from: Vec3, to: Vec3, radius: f32| -> Option<Vec3> {
        let world = collision?;
        let lists: Vec<&f13_core::collision::TriList> = dynamic.iter().filter(|(e, _)| blockers.contains(*e)).map(|(_, l)| l).collect();
        let f = f13_core::collision::sphere_sweep(&(&*world.camera_grid, lists.as_slice()), to_bevy(from), to_bevy(to), radius / 100.0)?;
        Some(from + (to - from) * f)
    }
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
    players: Query<(&Player, &Transform, Option<&crate::vehicle::Driving>, Has<crate::jason::Held>, Has<crate::items::Trapped>, Option<&crate::controls::Controls>), Without<PlayerCamera>>,
    (local, spectate): (Query<Entity, (With<Player>, With<crate::controls::LocalPlayer>)>, Option<Res<crate::spectate::Spectate>>),
    vehicles: Query<&GlobalTransform>,
    mut camera: Single<(&mut PlayerCamera, &mut Transform, &mut Projection)>,
    windows: Query<&Window>,
    (collision, dynamic, blockers): (Option<Res<crate::collision::WorldCollision>>, Res<crate::collision::DynamicColliders>, Query<(), With<crate::collision::CameraBlocker>>),
    mut rng: ResMut<crate::GameRng>,
) {
    // Her cameras, or (spectating) the watched counselor's, turned as her controls are.
    let watched = spectate.as_ref().and_then(|s| s.viewed());
    let Some(subject) = watched.or_else(|| local.single().ok()) else { return };
    let Ok((p, pxf, driving, held, trapped, controls)) = players.get(subject) else { return };
    // In her seat (SetCurrentSeat): not while getting in or out.
    let driving = driving.filter(|d| d.seat_move.is_none());
    let (ref mut cam, ref mut xf, ref mut proj) = *camera;
    if let (Some(_), Some(c)) = (watched, controls) {
        cam.yaw = c.yaw;
        cam.pitch = c.pitch;
    }
    if let Some((eye, target)) = config.camera {
        xf.translation = pxf.translation + eye;
        xf.look_at(pxf.translation + target, Vec3::Y);
        return;
    }
    let now = time.elapsed_secs();
    let mut rig = match rig.filter(|r| r.subject == Some(subject)) {
        Some(r) => r,
        None => {
            let Some(mut r) = counselor.as_ref().and_then(|c| new_rig(&c.info, now)) else { return };
            r.subject = Some(subject);
            // Possession turns her view to her facing, level (AController::Possess).
            let f = to_unreal(pxf.rotation * Vec3::Z);
            set_control_rotation(cam, f.y.atan2(f.x).to_degrees(), 0.0);
            commands.insert_resource(r);
            return;
        }
    };
    let rig = &mut *rig;

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
        rig.change_camera(if stance == Stance::Driving { 0.0 } else { 0.5 }, now);
    }
    if indoors != rig.indoors {
        rig.indoors = indoors;
        rig.indoors_event = indoors;
        rig.change_camera(if indoors { 0.5 } else { 1.5 }, now);
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
        rig.change_camera(blend, now);
    }

    // The arms hang off her capsule's centre.
    let origin = to_unreal(pxf.translation) + Vec3::Z * crate::player::CAPSULE_HALF_HEIGHT_M * 100.0;
    let facing = to_unreal(pxf.rotation * Vec3::Z);
    let component_rot = Vec3::new(0.0, facing.y.atan2(facing.x).to_degrees(), 0.0);
    let (yaw, pitch) = control_rotation(cam);
    let control = Vec3::new(pitch, yaw, 0.0);
    let velocity = Vec3::new(p.velocity.x, p.velocity.y, p.vel_y * 100.0);
    let dt = time.delta_secs();
    let sweep = camera_sweep(collision.as_deref(), &dynamic, &blockers);
    let Some(mut view) = rig.cams.tick(now, dt, velocity, component_rot, control, |_| origin, sweep) else { return };
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
