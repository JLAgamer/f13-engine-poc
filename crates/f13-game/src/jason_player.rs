//! Playing as Jason (`--play-jason`): the keyboard and mouse drive Jason and his own cameras follow him, while the
//! counselor stands idle (or a `--bot-*` plays her; it doesn't close the game when it's done).
//!
//! The bot plays her through the engine's input state, so Jason's input is taken from the raw keyboard and mouse
//! events before the engine sees them: the player's keys never reach her.
//!
//! Keys, as DefaultInput.ini maps the actions SCKillerCharacter's SetupPlayerInputComponent (0x140407900) binds:
//! WASD walk (GLB_Forward/Right_PC), Left Shift fast walk (KLR_FastWalk_PC, held), mouse look (Turn / LookUp); left
//! click attack (CMBT_KLR_Attack; held over 0.1 s in combat stance, the heavy attack), right click grab (KLR_Grab),
//! middle click held aims a knife and letting go throws it (KLR_KnifeThrow; right click puts it away,
//! KLR_KnifeCancel), C combat stance (GLB_Combat_PC), R trap (KLR_PlaceTrap), E interact (GLB_Interact1: a fuse box or
//! phone box at hand, a placed kill while holding her), 1 Morph (to the morph point nearest 30 m ahead of where he
//! looks), 2 Sense, 3 Shift, 4 Stalk (KLR_Morph/Sense/Shift/Stalk_PC). Holding her (GrabInput): 1-4 let go are the
//! right, bottom, left and top grab kills (CMBT_KLR_GrabKillRight/Bottom/Left/Top), a click the bottom one. Doors in
//! his way open or break as he walks into them. M opens and closes his map (GLB_Map); 1 opens it for a Morph: a click on it
//! morphs him to the morph point nearest that place (within 30 m), 1 again puts it away. Esc frees the mouse, a click
//! takes it again.
//!
//! His cameras (Jason_Base's graph, decompiled: pseudo_Jason_Base): his ILLDynamicCameraComponents on his capsule,
//! stacked as the counselor's are (counselor_camera.rs). ChangeCamera(BlendTime): CombatCamera in combat stance, else
//! IndoorCamera indoors, else OutdoorCamera; OnStanceChanged: ChangeCamera(0.5); OnIndoorsChanged: ChangeCamera(0.5
//! going in, 1.5 going out) unless shifting; OnShiftChange: ShiftCamera over 0.7 s with ED_Shake, then
//! ChangeCamera(0.5); OnUnderWater: WaterCamera over 0.5 s (its arm at the surface over him plus 10 cm, the view
//! pitched to -30), coming up ChangeCamera(0.5); BP_BeginThrow: ThrowingKnifeCam over 0.25 s until 0.5 s after the
//! throw (EndThrow: WaterCamera under water, else ChangeCamera(0.5)); a grab kill: ChokeCamera at once, its spline
//! camera over it (kill_camera.rs), then ChangeCamera(0.5). OnPossessed: OutdoorCamera.

use crate::counselor_camera::{add_control_rotation, control_rotation, mouse_turn, set_control_rotation, to_bevy, to_unreal, Cameras};
use crate::player::PlayerCamera;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{AccumulatedMouseMotion, MouseButtonInput};
use bevy::input::{ButtonInput, ButtonState};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};


const OUTDOOR: &str = "OutdoorCamera";
const INDOOR: &str = "IndoorCamera";
const COMBAT: &str = "CombatCamera";
const SHIFT: &str = "ShiftCamera";
const WATER: &str = "WaterCamera";
const CHOKE: &str = "ChokeCamera";
const KNIFE: &str = "ThrowingKnifeCam";
/// OnShiftChange's camera shake (scale 1).
const SHIFT_SHAKE: &str = "ED_Shake";
/// BP_PerformThrow's EndThrow timer (s).
const END_THROW_S: f32 = 0.5;
/// OnUnderWaterUpdate: the WaterCamera's arm over the surface (cm); OnUnderWater: the view's pitch (degrees).
const WATER_CAMERA_OVER_CM: f32 = 10.0;
const UNDERWATER_PITCH: f32 = -30.0;

/// The player is Jason.
#[derive(Resource)]
pub struct PlayJason;

/// His map (KillerHUDWidget's Map_Overylay): open (bIsMapEnabled), for a Morph (bIsMorphActive: MorphInput's LMB picks
/// the place), a click on it waiting to be read (window px), and the world place it picked (ui.rs reads the click off
/// SC_KillerMap_Widget, jason.rs morphs there).
#[derive(Resource, Default, Debug)]
pub struct MorphMap {
    pub open: bool,
    pub morph: bool,
    pub click: Option<Vec2>,
    pub target: Option<Vec3>,
}

pub struct PlayJasonPlugin {
    /// The player is Jason from the start (`--play-jason`); else only once the host makes them Jason ([`become_jason`]).
    pub active: bool,
    /// Scripted input (testing): look at her, walk up, a knife from range, a swing, then grabs.
    pub demo: bool,
}

/// Makes the player Jason mid-match (the host's `jason`): his controls take over the keyboard and mouse and the
/// camera, looking where he faces; the counselor is left without a player.
pub fn become_jason(commands: &mut Commands) {
    commands.insert_resource(JasonControls::default());
    commands.insert_resource(PlayJason);
}

impl Plugin for PlayJasonPlugin {
    fn build(&self, app: &mut App) {
        let on = resource_exists::<PlayJason>;
        if self.demo {
            app.add_systems(PreUpdate, demo.after(capture).before(bevy::input::InputSystems).run_if(on));
        }
        if self.active {
            app.insert_resource(PlayJason).init_resource::<JasonControls>();
        }
        app.init_resource::<MorphMap>()
            .add_systems(PreUpdate, capture.before(bevy::input::InputSystems).run_if(on))
            .add_systems(Update, (look, hud, map_cursor).run_if(on))
            .add_systems(
                Update,
                jason_camera.after(look).after(crate::counselor_camera::counselor_camera).after(crate::hiding::hiding_camera).before(crate::kill_camera::kill_camera).run_if(on),
            )
            .add_systems(Startup, spawn_hud);
    }
}

/// The player's Jason input: held keys and buttons (from the raw events), the view's yaw and pitch, and the presses
/// not yet used by a fixed step.
#[derive(Resource, Default)]
pub struct JasonControls {
    keys: ButtonInput<KeyCode>,
    mouse: ButtonInput<MouseButton>,
    /// The control rotation (the player camera's, copied each frame).
    pub yaw: f32,
    pub pitch: f32,
    latched: Presses,
    /// 2: Sense, asked for and not yet used (senses.rs takes it).
    pub sense: bool,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Presses {
    pub attack: bool,
    pub grab: bool,
    /// The knife button pressed (he takes a knife out and aims while it's held).
    pub knife: bool,
    pub trap: bool,
    pub shift: bool,
    pub morph: bool,
    pub stalk: bool,
    /// C: in or out of combat stance.
    pub combat: bool,
    /// E: destroy the fuse box at hand; holding her, use the placed kill at hand.
    pub interact: bool,
    /// 1-4 let go while he holds her: that grab kill slot (jason.rs GRAB_KILL_SLOTS).
    pub kill: Option<usize>,
}

/// One fixed step's input.
#[derive(Clone, Copy, Debug)]
pub struct JasonInput {
    /// Where to walk (world XZ, unit or zero).
    pub movement: Vec2,
    pub run: bool,
    /// Where he looks: flat (world XZ, unit) and in 3D.
    pub forward: Vec2,
    pub look: Vec3,
    /// The attack and knife buttons held.
    pub attack_held: bool,
    pub knife_held: bool,
    pub presses: Presses,
}

impl JasonControls {
    /// This step's input; the presses are used up.
    pub fn take(&mut self) -> JasonInput {
        let (sin, cos) = self.yaw.sin_cos();
        let forward = Vec2::new(-sin, -cos);
        let right = Vec2::new(cos, -sin);
        let mut m = Vec2::ZERO;
        for (k, v) in [(KeyCode::KeyW, forward), (KeyCode::KeyS, -forward), (KeyCode::KeyD, right), (KeyCode::KeyA, -right)] {
            if self.keys.pressed(k) {
                m += v;
            }
        }
        let look = Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0) * Vec3::NEG_Z;
        JasonInput {
            movement: m.normalize_or_zero(),
            run: self.keys.pressed(KeyCode::ShiftLeft),
            forward,
            look,
            attack_held: self.mouse.pressed(MouseButton::Left),
            knife_held: self.mouse.pressed(MouseButton::Middle),
            presses: std::mem::take(&mut self.latched),
        }
    }
}

/// Takes the raw keyboard and mouse button events (before the engine's input state sees them).
fn capture(
    mut controls: ResMut<JasonControls>,
    mut keyboard: ResMut<Messages<KeyboardInput>>,
    mut buttons: ResMut<Messages<MouseButtonInput>>,
    mut cursor: Single<&mut CursorOptions>,
    mut map: ResMut<MorphMap>,
    window: Single<&bevy::window::Window, With<bevy::window::PrimaryWindow>>,
) {
    let c = &mut *controls;
    c.keys.clear();
    c.mouse.clear();
    for e in keyboard.drain() {
        match e.state {
            ButtonState::Pressed => c.keys.press(e.key_code),
            ButtonState::Released => c.keys.release(e.key_code),
        }
    }
    for e in buttons.drain() {
        match e.state {
            ButtonState::Pressed => c.mouse.press(e.button),
            ButtonState::Released => c.mouse.release(e.button),
        }
    }
    if c.keys.just_pressed(KeyCode::Escape) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
    // GLB_Map: his map open or shut (not while it's out for a Morph).
    if c.keys.just_pressed(KeyCode::KeyM) && !map.morph {
        map.open = !map.open;
    }
    // The map out for a Morph: a click picks the place (KLR_Morph_Activate).
    if map.morph {
        if c.mouse.just_pressed(MouseButton::Left) {
            map.click = window.cursor_position();
        }
        c.mouse.release(MouseButton::Left);
    }
    let locked = cursor.grab_mode == CursorGrabMode::Locked;
    if c.mouse.just_pressed(MouseButton::Left) && !locked && !map.morph {
        // The first click takes the mouse (no swing).
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
        c.mouse.release(MouseButton::Left);
        return;
    }
    let p = &mut c.latched;
    p.attack |= c.mouse.just_pressed(MouseButton::Left);
    p.grab |= c.mouse.just_pressed(MouseButton::Right);
    p.knife |= c.mouse.just_pressed(MouseButton::Middle);
    p.trap |= c.keys.just_pressed(KeyCode::KeyR);
    p.morph |= c.keys.just_pressed(KeyCode::Digit1);
    let sense = c.keys.just_pressed(KeyCode::Digit2);
    p.shift |= c.keys.just_pressed(KeyCode::Digit3);
    p.stalk |= c.keys.just_pressed(KeyCode::Digit4);
    p.combat |= c.keys.just_pressed(KeyCode::KeyC);
    p.interact |= c.keys.just_pressed(KeyCode::KeyE);
    // The grab kills fire as their key is let go (IE_Released).
    let digits = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4];
    if let Some(i) = digits.iter().position(|&k| c.keys.just_released(k)) {
        p.kill = Some(i);
    }
    c.sense |= sense;
}

/// The scripted player: what a player would press, by distance to her.
fn demo(
    mut controls: ResMut<JasonControls>,
    jason: Query<(&crate::jason::Jason, &Transform)>,
    her: Query<&Transform, (With<crate::player::Player>, Without<crate::jason::Jason>)>,
    fuse_boxes: Query<&crate::power::FuseBox>,
    mut camera: Single<&mut PlayerCamera>,
    mut done: Local<(bool, bool, f32)>,
    time: Res<Time>,
    (know, mut map, window, mut opened): (Query<&crate::senses::Knowledge>, ResMut<MorphMap>, Single<&bevy::window::Window, With<bevy::window::PrimaryWindow>>, Local<Option<f32>>),
    grab_kills: Res<crate::jason::GrabKillsAvailable>,
) {
    let (Some((j, jxf)), Some(h)) = (jason.iter().next(), her.iter().next()) else { return };
    let d = h.translation - jxf.translation;
    let dist = Vec2::new(d.x, d.z).length();
    // Yaw so that the view's forward (-sin, -cos) points at her.
    camera.yaw = (-d.x).atan2(-d.z);
    camera.pitch = -0.15;
    // Up to her, not into her.
    if dist > 1.3 {
        controls.keys.press(KeyCode::KeyW);
    } else {
        controls.keys.release(KeyCode::KeyW);
    }
    controls.keys.press(KeyCode::ShiftLeft);
    // The knife's let go once it's out.
    if matches!(j.state, crate::jason::JasonState::Throwing { t, .. } if t > 0.6) {
        controls.mouse.release(MouseButton::Middle);
    }
    // Sense whenever it's ready while she's not close.
    controls.sense |= dist > 20.0;
    let now = time.elapsed_secs();
    // Morph by his map once it's ready and she's far: open it (1), then a click at its centre 8 s later.
    let morph_ready = know.iter().next().is_some_and(|k| k.abilities.is_available(f13_core::killer_abilities::Ability::Morph, &f13_core::killer_abilities::Blockers::default()));
    match *opened {
        None if morph_ready && dist > 40.0 && matches!(j.state, crate::jason::JasonState::Hunting) => {
            controls.latched.morph = true;
            *opened = Some(now);
            info!("jason demo: map out for a Morph");
        }
        Some(at) if map.morph && now - at > 8.0 => {
            map.click = Some(Vec2::new(window.width(), window.height()) / 2.0);
            *opened = Some(f32::MAX);
            info!("jason demo: Morph map click at its centre");
        }
        _ => {}
    }
    let (knife, swung, last) = &mut *done;
    debug!("jason demo: {:?} at {:.1}, {dist:.1} m from her, velocity {:.0}", j.state, jxf.translation, j.velocity);
    // Holding her: a kill by its key (3: the left slot, else the first that fits).
    if matches!(j.state, crate::jason::JasonState::Holding { t } if t > 1.5) {
        controls.latched.kill = [2, 0, 1, 3].into_iter().find(|&s| grab_kills.0[s]);
    }
    if !matches!(j.state, crate::jason::JasonState::Hunting) || now - *last < 1.0 {
        return;
    }
    // An intact fuse box at hand: break it (E).
    if fuse_boxes.iter().any(|f| !f.destroyed && Vec2::new(f.interact.x - jxf.translation.x, f.interact.z - jxf.translation.z).length() * 100.0 <= f.reach) {
        *last = now;
        controls.latched.interact = true;
        info!("jason demo: breaking the fuse box at hand");
    } else if !*knife && j.knives > 0 && (8.0..20.0).contains(&dist) {
        *knife = true;
        *last = now;
        controls.latched.knife = true;
        controls.mouse.press(MouseButton::Middle);
        info!("jason demo: knife at {dist:.1} m");
    } else if dist < 1.9 && !*swung {
        *swung = true;
        *last = now;
        controls.latched.attack = true;
        info!("jason demo: swing at {dist:.1} m");
    } else if dist < 2.3 && *swung {
        *last = now;
        controls.latched.grab = true;
        info!("jason demo: grab at {dist:.1} m");
    }
}

/// The map out for a Morph frees the mouse (its cursor at the screen's centre: SetMouseLocation); put away, the mouse
/// turns his view again.
fn map_cursor(map: Res<MorphMap>, mut cursor: Single<&mut CursorOptions>, mut window: Single<&mut bevy::window::Window, With<bevy::window::PrimaryWindow>>, mut was: Local<bool>) {
    if map.morph != *was {
        *was = map.morph;
        if map.morph {
            cursor.visible = true;
            cursor.grab_mode = CursorGrabMode::None;
            let centre = Vec2::new(window.width(), window.height()) / 2.0;
            window.set_cursor_position(Some(centre));
        } else {
            cursor.visible = false;
            cursor.grab_mode = CursorGrabMode::Locked;
        }
    }
}

/// Turn / LookUp (0x14044c9c0 / 0x14041d274): the mouse turns his control rotation as the counselor's does.
fn look(
    mut controls: ResMut<JasonControls>,
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    mut camera: Single<(&mut PlayerCamera, &Projection)>,
    windows: Query<&Window>,
) {
    let (ref mut cam, proj) = *camera;
    if cursor.grab_mode == CursorGrabMode::Locked {
        let (yaw, pitch) = mouse_turn(motion.delta, proj, &windows);
        add_control_rotation(cam, yaw, pitch);
    }
    controls.yaw = cam.yaw;
    controls.pitch = cam.pitch;
}

/// His cameras and what last switched them.
#[derive(Resource)]
pub struct JasonRig {
    cams: Cameras,
    combat: bool,
    indoors: bool,
    underwater: bool,
    shifting: bool,
    throwing: bool,
    killing: bool,
    shake: Option<f13_core::dynamic_camera::ShakeInstance>,
}

impl JasonRig {
    /// Jason_Base's ChangeCamera (its spline cameras' check: the kill camera has the view while one plays).
    fn change_camera(&mut self, blend: f32, now: f32) {
        let name = if self.combat {
            COMBAT
        } else if self.indoors {
            INDOOR
        } else {
            OUTDOOR
        };
        self.cams.set_active_with_blend(name, blend, now);
    }
}

#[allow(clippy::too_many_arguments)]
fn jason_camera(
    mut commands: Commands,
    time: Res<Time>,
    killer: Option<Res<crate::character::KillerCharacter>>,
    rig: Option<ResMut<JasonRig>>,
    jason: Query<(&crate::jason::Jason, &Transform), Without<PlayerCamera>>,
    surroundings: Option<Res<crate::fear_events::Surroundings>>,
    water: Option<Res<crate::water::Water>>,
    mut camera: Single<(&mut PlayerCamera, &mut Transform, &mut Projection)>,
    windows: Query<&Window>,
    (collision, dynamic, blockers): (Option<Res<crate::collision::WorldCollision>>, Res<crate::collision::DynamicColliders>, Query<(), With<crate::collision::CameraBlocker>>),
    mut rng: ResMut<crate::GameRng>,
) {
    let (Some(killer), Some((j, jxf))) = (killer, jason.iter().next()) else { return };
    let (ref mut cam, ref mut xf, ref mut proj) = *camera;
    let now = time.elapsed_secs();
    let facing = to_unreal(jxf.rotation * Vec3::Z);
    let Some(mut rig) = rig else {
        let mut r = JasonRig { cams: Cameras::new(&killer.info), combat: false, indoors: false, underwater: false, shifting: false, throwing: false, killing: false, shake: None };
        // Possession turns the view to his facing, level (AController::Possess); OnPossessed: the Outdoor camera.
        set_control_rotation(cam, facing.y.atan2(facing.x).to_degrees(), 0.0);
        if r.cams.set_active(OUTDOOR, now) {
            commands.insert_resource(r);
        }
        return;
    };
    let rig = &mut *rig;
    let hh = killer.info.capsule_half_height;
    let centre = to_unreal(jxf.translation) + Vec3::Z * hh;
    let indoors = surroundings.as_ref().is_some_and(|s| s.indoors(jxf.translation));
    let shifting = j.shifting();
    let throwing = match j.state {
        crate::jason::JasonState::Throwing { t, release, .. } => release.is_none_or(|r| t < r + END_THROW_S),
        _ => false,
    };
    let killing = matches!(j.state, crate::jason::JasonState::Killing { .. });
    if j.combat != rig.combat {
        rig.combat = j.combat;
        rig.change_camera(0.5, now);
    }
    if indoors != rig.indoors {
        rig.indoors = indoors;
        if !rig.shifting {
            rig.change_camera(if indoors { 0.5 } else { 1.5 }, now);
        }
    }
    if shifting != rig.shifting {
        rig.shifting = shifting;
        if rig.underwater {
        } else if shifting {
            rig.cams.set_active_with_blend(SHIFT, 0.7, now);
            rig.shake = killer.info.camera_shakes.get(SHIFT_SHAKE).map(|s| f13_core::dynamic_camera::ShakeInstance::new(f13_data::character::camera_shake(s), 1.0, &mut || rng.0.frand()));
        } else {
            rig.change_camera(0.5, now);
            rig.shake = None;
        }
    }
    if j.underwater != rig.underwater {
        rig.underwater = j.underwater;
        if j.underwater {
            rig.cams.set_active_with_blend(WATER, 0.5, now);
            let (yaw, _) = control_rotation(cam);
            set_control_rotation(cam, yaw, UNDERWATER_PITCH);
        } else {
            rig.change_camera(0.5, now);
        }
    }
    if throwing != rig.throwing {
        rig.throwing = throwing;
        if throwing {
            rig.cams.set_active_with_blend(KNIFE, 0.25, now);
        } else if rig.underwater {
            rig.cams.set_active_with_blend(WATER, 0.5, now);
        } else {
            rig.change_camera(0.5, now);
        }
    }
    if killing != rig.killing {
        rig.killing = killing;
        if killing {
            rig.cams.set_active_with_blend(CHOKE, 0.0, now);
        } else {
            rig.cams.reset();
            rig.change_camera(0.5, now);
        }
    }

    let component_rot = Vec3::new(0.0, facing.y.atan2(facing.x).to_degrees(), 0.0);
    let (yaw, pitch) = control_rotation(cam);
    let control = Vec3::new(pitch, yaw, 0.0);
    let velocity = Vec3::new(j.velocity.x, j.velocity.y, 0.0);
    // The WaterCamera's arm sits at the surface over him (bAbsoluteLocation, OnUnderWaterUpdate).
    let surface = water.as_ref().and_then(|w| w.surface_at(jxf.translation + Vec3::Y * hh / 100.0));
    let origin = |name: &str| match surface.filter(|_| name == WATER) {
        Some(s) => Vec3::new(centre.x, centre.y, s * 100.0 + WATER_CAMERA_OVER_CM),
        None => centre,
    };
    let sweep = crate::counselor_camera::camera_sweep(collision.as_deref(), &dynamic, &blockers);
    let dt = time.delta_secs();
    let Some(mut view) = rig.cams.tick(now, dt, velocity, component_rot, control, origin, sweep) else { return };
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
        persp.fov = 2.0 * ((view.fov.to_radians() / 2.0).tan() / crate::counselor_camera::aspect(&windows)).atan();
    }
}

#[derive(Component)]
struct JasonHud;

/// A debug readout (F3, as the counselor's; the game's own HUD is ui.rs's KillerHUDWidget).
fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        Node { position_type: PositionType::Absolute, bottom: Val::Px(16.0), left: Val::Px(16.0), ..default() },
        Visibility::Hidden,
        JasonHud,
    ));
}

/// What Jason has: his state, knives, traps, the abilities ready.
fn hud(
    controls: Res<JasonControls>,
    game_hud: Option<Res<crate::ui::Hud>>,
    jason: Query<(&crate::jason::Jason, Option<&crate::senses::Knowledge>)>,
    mut text: Single<(&mut Text, &mut Visibility), With<JasonHud>>,
) {
    let (ref mut text, ref mut vis) = *text;
    if controls.keys.just_pressed(KeyCode::F3) || game_hud.is_none() {
        let v = if game_hud.is_none() || **vis == Visibility::Hidden { Visibility::Inherited } else { Visibility::Hidden };
        vis.set_if_neq(v);
    }
    let Some((j, know)) = jason.iter().next() else { return };
    use f13_core::killer_abilities::{Ability, Blockers};
    let ready: Vec<&str> = [(Ability::Morph, "1 Morph"), (Ability::Sense, "2 Sense"), (Ability::Shift, "3 Shift"), (Ability::Stalk, "4 Stalk")]
        .iter()
        .filter(|(a, _)| know.is_some_and(|k| k.abilities.is_available(*a, &Blockers::default())))
        .map(|(_, n)| *n)
        .collect();
    let s = format!(
        "Jason: {:?}{}\nknives {} [MMB]  traps {} [R]  ready: {}\nLMB attack  RMB grab  C combat  Shift fast walk",
        j.state,
        if j.combat { " (combat)" } else { "" },
        j.knives,
        j.traps,
        if ready.is_empty() { "-".to_string() } else { ready.join(", ") }
    );
    if text.0 != s {
        text.0 = s;
    }
}
