//! A playable counselor (Vanessa) driven by the f13-core movement and stamina rules.
//!
//! Placeholder controls: WASD move (running by default), Shift sprint, Caps Lock toggle walk, Ctrl crouch,
//! mouse look (click to capture, Esc to release); E interact and G drop (interact.rs), F leave a car (vehicle.rs). The capsule moves through the game's own
//! collision (collision.rs, f13-core::collision).

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use f13_core::counselor::{Counselor, CounselorEvent, CounselorTuning, Situation, Stance};
use f13_core::dodge::DodgeDirection;
use f13_core::movement::{calc_velocity, input_acceleration, MovementSettings};
use crate::vehicle::Driving;

/// Unreal units (cm) per Bevy unit (m).
const CM: f32 = 100.0;
/// Character turn rate (the counselor Blueprint's RotationRate yaw), degrees per second.
const TURN_RATE_DEG: f32 = 440.0;
const MAX_STEP_HEIGHT_M: f32 = 0.45;
/// Gravity (Unreal's -980 cm/s²) and the walkable floor angle (Unreal's default 44.765°).
pub(crate) const WALK: f13_core::collision::WalkSettings =
    f13_core::collision::WalkSettings { max_step: MAX_STEP_HEIGHT_M, walkable_normal_y: 0.71, gravity: 9.8 };
/// How far above a spawn point the floor is looked for (a spawn point is the capsule's centre, under a metre up).
const SPAWN_PROBE_M: f32 = 1.5;
pub(crate) const CAPSULE_RADIUS_M: f32 = 0.34;
/// Base_Counselor's CollisionCylinder: half height 82 cm, radius left at ACharacter's 34 cm.
pub(crate) const CAPSULE_HALF_HEIGHT_M: f32 = 0.82;
const CAMERA_DISTANCE_M: f32 = 3.0;
const CAR_CAMERA_DISTANCE_M: f32 = 7.0;
const CAMERA_HEIGHT_M: f32 = 1.6;

#[derive(Resource, Clone)]
pub struct PlayerTuning {
    pub counselor: CounselorTuning,
    pub movement: MovementSettings,
    pub stats: f13_core::stats::Stats,
    pub first_aid: f13_data::FirstAid,
    pub minigame: f13_core::minigame::MinigameTuning,
    pub repair_hold_time: f32,
    pub vehicle_start: f13_data::VehicleStart,
}

#[derive(Component)]
pub struct Player {
    pub counselor: Counselor,
    /// Horizontal velocity in Bevy's XZ plane, in cm/s (Unreal units).
    pub velocity: Vec2,
    pub walk_toggle: bool,
    /// Fear 0..=100 (no fear events are pushed in the client yet).
    pub fear: f32,
    /// A montage's root motion moving the counselor (dodge, stumble): local velocity (x right, y forward), cm/s.
    pub root_motion: Option<(Vec2, f32)>,
    /// The montage the rules started (dodge, stumble), for the animation.
    pub montage: Option<String>,
    /// Vertical speed (m/s, falling) and whether the last move ended on a walkable floor.
    pub vel_y: f32,
    pub grounded: bool,
}

/// Placed (spawned or teleported) but not yet put on the floor: the next move finds the floor under this point, or,
/// if there's none (under the map), the surface above it.
#[derive(Component)]
pub struct Unsettled;

impl Player {
    /// What the counselor rules need to know about this tick.
    pub fn situation(&self) -> Situation {
        Situation {
            speed_sq: self.velocity.length_squared(),
            fear: self.fear,
            dodging: self.counselor.dodging(),
            ..Default::default()
        }
    }
}

/// What the player wants this tick, from the keyboard or the demo script.
#[derive(Resource, Default)]
struct Intent {
    /// x = right, y = forward, relative to the camera.
    movement: Vec2,
    sprint_held: bool,
    sprint_changed: bool,
    toggle_walk: bool,
    toggle_crouch: bool,
    toggle_combat: bool,
    /// Double-tapped A/D/S (a dodge in combat stance).
    dodge: Option<DodgeDirection>,
}

/// Marks the player's own meshes so camera and interaction ray casts ignore them.
#[derive(Component)]
pub struct PlayerPart;

#[derive(Component)]
pub struct PlayerCamera {
    pub yaw: f32,
    pub pitch: f32,
}

impl PlayerCamera {
    /// The direction the camera looks.
    pub fn look(&self) -> Vec3 {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0) * Vec3::NEG_Z
    }
}

#[derive(Component)]
struct Hud;

pub struct PlayerPlugin {
    pub spawn: Vec3,
    /// A placeholder capsule instead of the counselor's model.
    pub capsule: bool,
    /// A fixed camera instead of the follow camera: eye and look-at point relative to the counselor's feet (testing).
    pub camera: Option<(Vec3, Vec3)>,
    /// Scripted run: hold forward and sprint, logging stamina every second.
    pub demo: bool,
    /// Fixed fear level (testing: fear events aren't pushed in the client yet).
    pub fear: f32,
    /// Starting health (testing: nothing deals damage in the client yet).
    pub health: Option<f32>,
}

#[derive(Resource)]
struct PlayerConfig {
    spawn: Vec3,
    capsule: bool,
    camera: Option<(Vec3, Vec3)>,
    demo: bool,
    fear: f32,
    health: Option<f32>,
}

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PlayerConfig { spawn: self.spawn, capsule: self.capsule, camera: self.camera, demo: self.demo, fear: self.fear, health: self.health })
            .init_resource::<Intent>()
            .add_systems(Startup, spawn_player)
            // Input is read once per frame; one-shot actions stay latched until a fixed step uses them, so a frame
            // that runs several fixed steps (low frame rate, --speed) doesn't repeat a key press.
            .add_systems(RunFixedMainLoop, read_intent.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop))
            .add_systems(FixedUpdate, (update_counselor, move_player).chain())
            .add_systems(Update, (tag_player_parts, grab_cursor, aim_demo, follow_camera, update_hud).chain());
    }
}

fn spawn_player(
    mut commands: Commands,
    assets: Res<AssetServer>,
    config: Res<PlayerConfig>,
    tuning: Res<PlayerTuning>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    character: Option<Res<crate::character::CounselorCharacter>>,
) {
    let mut counselor = Counselor::new(tuning.stats);
    counselor.health = config.health.unwrap_or(tuning.counselor.health.max_health).clamp(1.0, tuning.counselor.health.max_health);
    counselor.update_wounded(counselor.health, true, &tuning.counselor);
    let player = commands.spawn((
        Player { counselor, velocity: Vec2::ZERO, walk_toggle: false, fear: config.fear, root_motion: None, montage: None, vel_y: 0.0, grounded: false },
        Transform::from_translation(config.spawn),
        Visibility::default(),
        Unsettled,
    ));
    let player = player.id();
    match character.as_deref().filter(|_| !config.capsule) {
        Some(character) => {
            crate::character::spawn_model(&mut commands, &assets, character, crate::character::Model::Counselor, player, PlayerPart);
        }
        None => {
            let capsule = meshes.add(Capsule3d::new(CAPSULE_RADIUS_M, 2.0 * (CAPSULE_HALF_HEIGHT_M - CAPSULE_RADIUS_M)));
            let material = materials.add(StandardMaterial { base_color: Color::srgb(0.75, 0.55, 0.35), ..default() });
            commands.spawn((
                Mesh3d(capsule),
                MeshMaterial3d(material),
                Transform::from_xyz(0.0, CAPSULE_HALF_HEIGHT_M, 0.0),
                PlayerPart,
                ChildOf(player),
            ));
        }
    }
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(config.spawn + Vec3::new(0.0, CAMERA_HEIGHT_M, CAMERA_DISTANCE_M)),
        PlayerCamera { yaw: 0.0, pitch: -0.35 },
    ));
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        Node { position_type: PositionType::Absolute, top: Val::Px(12.0), left: Val::Px(12.0), ..default() },
        Hud,
    ));
}

/// Tags everything spawned under the player's model so ray casts can skip it.
fn tag_player_parts(
    mut commands: Commands,
    roots: Query<Entity, With<PlayerPart>>,
    children: Query<&Children>,
    tagged: Query<(), With<PlayerPart>>,
) {
    for root in &roots {
        for e in children.iter_descendants(root) {
            if !tagged.contains(e) {
                // Skinned meshes stay in bind pose for now; their bounds don't follow the skeleton.
                commands.entity(e).insert((PlayerPart, bevy::camera::visibility::NoFrustumCulling));
            }
        }
    }
}

fn grab_cursor(mut cursor: Single<&mut CursorOptions>, mouse: Res<ButtonInput<MouseButton>>, keys: Res<ButtonInput<KeyCode>>) {
    if mouse.just_pressed(MouseButton::Left) {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

/// The engine's `DoubleClickTime` (the game's DefaultInput.ini).
const DOUBLE_TAP_SECONDS: f32 = 0.2;

fn read_intent(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    config: Res<PlayerConfig>,
    mut intent: ResMut<Intent>,
    mut last_tap: Local<[f32; 3]>,
) {
    if config.demo {
        let was = intent.sprint_held;
        intent.movement = Vec2::Y;
        intent.sprint_held = true;
        intent.sprint_changed |= !was;
        return;
    }
    let mut movement = Vec2::ZERO;
    for (k, v) in [(KeyCode::KeyW, Vec2::Y), (KeyCode::KeyS, -Vec2::Y), (KeyCode::KeyD, Vec2::X), (KeyCode::KeyA, -Vec2::X)] {
        if keys.pressed(k) {
            movement += v;
        }
    }
    intent.movement = movement;
    intent.sprint_held = keys.pressed(KeyCode::ShiftLeft);
    intent.sprint_changed |= keys.just_pressed(KeyCode::ShiftLeft) || keys.just_released(KeyCode::ShiftLeft);
    intent.toggle_walk |= keys.just_pressed(KeyCode::CapsLock);
    intent.toggle_crouch |= keys.just_pressed(KeyCode::ControlLeft);
    intent.toggle_combat |= keys.just_pressed(KeyCode::KeyC);
    let now = time.elapsed_secs();
    for (i, (key, dir)) in [(KeyCode::KeyA, DodgeDirection::Left), (KeyCode::KeyD, DodgeDirection::Right), (KeyCode::KeyS, DodgeDirection::Back)].into_iter().enumerate() {
        if keys.just_pressed(key) {
            if now - last_tap[i] <= DOUBLE_TAP_SECONDS {
                intent.dodge = Some(dir);
                last_tap[i] = f32::MIN;
            } else {
                last_tap[i] = now;
            }
        }
    }
}

/// Applies sprint/run/crouch input, ticks stamina and rolls stumbles.
#[allow(clippy::too_many_arguments)]
fn update_counselor(
    mut intent: ResMut<Intent>,
    config: Res<PlayerConfig>,
    time: Res<Time>,
    tuning: Res<PlayerTuning>,
    anims: Res<crate::Anims>,
    mut rng: ResMut<crate::GameRng>,
    mut player: Single<&mut Player>,
    mut demo_log: Local<u32>,
) {
    let now = time.elapsed_secs();
    // Consume this frame's one-shot actions (see PlayerPlugin).
    let once = Intent { movement: intent.movement, sprint_held: intent.sprint_held, ..default() };
    let intent = std::mem::replace(&mut *intent, once);
    let p = &mut **player;
    if intent.toggle_walk {
        p.walk_toggle = !p.walk_toggle;
    }
    let c = &mut p.counselor;
    if intent.toggle_crouch {
        c.stance = if c.stance == Stance::Crouching { Stance::Standing } else { Stance::Crouching };
    }
    if intent.toggle_combat {
        c.stance = if c.stance == Stance::Combat { Stance::Standing } else { Stance::Combat };
        c.set_sprinting(false);
        info!("combat stance {}", c.stance == Stance::Combat);
    }
    if intent.sprint_changed {
        c.set_sprint_input(intent.sprint_held);
    }
    if !intent.sprint_held {
        c.set_running(!p.walk_toggle);
    }
    let dt = time.delta_secs();
    let t = &tuning.counselor;
    let root_motion = |name: &str, length: f32| anims.0.get(name).map(|v| (*v, length));
    p.counselor.tick_dodge(dt);
    if let Some((_, left)) = &mut p.root_motion {
        *left -= dt;
        if *left <= 0.0 {
            p.root_motion = None;
        }
    }
    if let Some(dir) = intent.dodge.filter(|_| p.counselor.stance == Stance::Combat && p.counselor.alive()) {
        let situation = p.situation();
        if let Some(m) = p.counselor.try_dodge(dir, &situation, false, &t.dodge) {
            info!("dodged {dir:?} (stamina {:.1})", p.counselor.stamina);
            p.root_motion = root_motion(&m.name, m.length);
            p.montage = Some(m.name.clone());
        }
    }
    let situation = p.situation();
    let mut events = Vec::new();
    p.counselor.tick_stamina(time.delta_secs(), &situation, t, &mut events);
    for e in events {
        match e {
            CounselorEvent::StaminaDepleted { forced_stumble } => {
                info!("out of stamina");
                if forced_stumble && p.counselor.start_stumble(now, true, &t.stumble) {
                    info!("stumbled (out of stamina)");
                    let m = &p.counselor.stumble.montage;
                    p.root_motion = root_motion(&m.name, m.length);
                    p.montage = Some(m.name.clone());
                }
            }
            CounselorEvent::StaminaRecovered => info!("stamina recovered"),
        }
    }
    let roll = || f13_core::stumble::roll_from_unit(rng.0.frand());
    if p.counselor.tick_stumble(dt, now, &situation, t.stamina_max, &t.stumble, roll) {
        info!("stumbled (chance {:.0}%)", t.stumble.chance(&p.counselor, p.fear, t.stamina_max));
        let m = &p.counselor.stumble.montage;
        p.root_motion = root_motion(&m.name, m.length);
        p.montage = Some(m.name.clone());
    }
    if config.demo && time.elapsed_secs() as u32 > *demo_log {
        *demo_log = time.elapsed_secs() as u32;
        let c = &p.counselor;
        info!(
            "demo t={:>2}s stamina {:5.1} sprinting {} running {} depleted {} speed {:4.0} cm/s",
            *demo_log, c.stamina, c.sprinting, c.running, c.stamina_depleted, p.velocity.length()
        );
    }
}

fn move_player(
    intent: Res<Intent>,
    time: Res<Time>,
    tuning: Res<PlayerTuning>,
    camera: Single<&PlayerCamera>,
    mut player: Single<
        (Entity, &mut Player, &mut Transform, Option<&crate::interact::UsingItem>, Option<&Unsettled>, Has<crate::jason::Held>, Has<crate::jason::Staggered>),
        Without<Driving>,
    >,
    interaction: Option<Res<crate::interact::Interaction>>,
    collision: Res<crate::collision::WorldCollision>,
    dynamic: Res<crate::collision::DynamicColliders>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    let (entity, ref mut p, ref mut xf, using, settle, held, staggered) = *player;
    if held {
        // In Jason's hands: he places her.
        p.velocity = Vec2::ZERO;
        return;
    }
    let (sin, cos) = camera.yaw.sin_cos();
    let forward = Vec2::new(-sin, -cos);
    let right = Vec2::new(cos, -sin);
    let repairing = interaction.as_ref().is_some_and(|i| i.busy());
    let busy = !p.counselor.alive() || using.is_some() || repairing || staggered;
    let wish = if busy { Vec2::ZERO } else { forward * intent.movement.y + right * intent.movement.x };

    let (facing, _, _) = xf.rotation.to_euler(EulerRot::YXZ);
    if let Some((local, _)) = p.root_motion {
        // The montage moves the counselor (relative to where they face), whatever the input.
        let fwd = Vec2::new(facing.sin(), facing.cos());
        let rt = Vec2::new(-fwd.y, fwd.x);
        p.velocity = fwd * local.y + rt * local.x;
    } else {
        let situation = p.situation();
        let max_speed = p.counselor.max_speed(&situation, &tuning.counselor);
        let accel = input_acceleration(wish.to_array(), &tuning.movement);
        let mut v = p.velocity.to_array();
        calc_velocity(&mut v, accel, max_speed, dt, &tuning.movement);
        p.velocity = Vec2::from_array(v);
    }

    // Move the capsule through the world's collision (walls, steps, slopes, falling).
    let world = collision.with(&dynamic);
    if settle.is_some() {
        if let Some(at) = f13_core::collision::spawn_floor(&world, xf.translation, SPAWN_PROBE_M, &WALK) {
            if at.distance(xf.translation) > 0.01 {
                info!("placed on the floor at {at} (was {})", xf.translation);
            }
            xf.translation = at;
        }
        p.vel_y = 0.0;
        p.grounded = true;
        commands.entity(entity).remove::<Unsettled>();
    }
    let cap = f13_core::collision::Capsule { radius: CAPSULE_RADIUS_M, half_height: CAPSULE_HALF_HEIGHT_M };
    let mut pos = xf.translation;
    let (grounded, delta) = (p.grounded, p.velocity * dt / CM);
    let r = f13_core::collision::move_capsule(&world, &mut pos, &mut p.vel_y, grounded, delta, dt, &cap, &WALK);
    p.grounded = r.grounded;
    // Like Unreal's SlideAlongSurface: drop the velocity going into the walls hit.
    if let Some(n) = r.wall_normal.try_normalize() {
        let into = p.velocity.dot(n).min(0.0);
        p.velocity -= n * into;
    }
    // Fell out of the world (no surface under the feet at all): back onto the surface above.
    if !r.grounded && f13_core::collision::surface_below(&world, pos.x, pos.z, pos.y + MAX_STEP_HEIGHT_M, pos.y - 100.0).is_none() {
        if let Some(at) = f13_core::collision::spawn_floor(&world, pos, SPAWN_PROBE_M, &WALK) {
            warn!("fell out of the map at {pos}; back to {at}");
            pos = at;
            p.vel_y = 0.0;
        }
    }
    xf.translation = pos;

    // Orient to movement (the model faces +Z); in combat stance face where the camera looks. Not during root motion.
    let combat = p.counselor.stance == Stance::Combat;
    if p.root_motion.is_none() && (combat || p.velocity.length_squared() > 1.0) {
        let target = if combat { forward.x.atan2(forward.y) } else { p.velocity.x.atan2(p.velocity.y) };
        let (current, _, _) = xf.rotation.to_euler(EulerRot::YXZ);
        let delta = (target - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        let max = TURN_RATE_DEG.to_radians() * dt;
        xf.rotation = Quat::from_rotation_y(current + delta.clamp(-max, max));
    }
}

/// Demo: once the map has loaded, turn the camera toward the longest clear line so the run isn't a tree hug.
/// Aims again whenever the run is blocked (props stream in after the first aim).
fn aim_demo(
    config: Res<PlayerConfig>,
    time: Res<Time>,
    player: Single<(&Transform, &Player)>,
    mut camera: Single<&mut PlayerCamera>,
    mut ray_cast: MeshRayCast,
    parts: Query<(), With<PlayerPart>>,
    mut last_aim: Local<Option<f32>>,
) {
    let now = time.elapsed_secs();
    let (player, p) = *player;
    let blocked = p.velocity.length_squared() < 50.0 * 50.0;
    let due = match *last_aim {
        None => now >= 0.5,
        Some(t) => blocked && now - t >= 1.0,
    };
    if !config.demo || !due {
        return;
    }
    *last_aim = Some(now);
    let not_player = |e: Entity| !parts.contains(e);
    let settings = MeshRayCastSettings::default().with_filter(&not_player);
    let origin = player.translation + Vec3::Y * 1.0;
    let best = (0..16)
        .map(|i| i as f32 * std::f32::consts::TAU / 16.0)
        .map(|yaw| {
            let dir = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
            let clear = ray_cast
                .cast_ray(Ray3d::new(origin, Dir3::new(dir).unwrap()), &settings)
                .first()
                .map_or(f32::MAX, |(_, h)| h.distance);
            (yaw, clear)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    info!("demo heading yaw {:.2} rad, clear for {:.0} m", best.0, best.1.min(9999.0));
    camera.yaw = best.0;
}

/// Third-person camera on a "spring arm": pulled in when the map is between the camera and the counselor.
fn follow_camera(
    config: Res<PlayerConfig>,
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    player: Single<(&Transform, Has<Driving>), (With<Player>, Without<PlayerCamera>)>,
    mut camera: Single<(&mut PlayerCamera, &mut Transform)>,
    mut ray_cast: MeshRayCast,
    parts: Query<(), With<PlayerPart>>,
    vehicle_parts: Query<(), With<crate::props::VehiclePart>>,
    ragdolls: Query<&crate::ragdoll::Ragdolled>,
    jason_parts: Query<(), With<crate::jason::JasonPart>>,
) {
    let (player, driving) = *player;
    // Dead: the camera follows the body.
    let body = ragdolls.iter().find_map(|r| r.center());
    let player = &Transform::from_translation(body.map_or(player.translation, |c| c - Vec3::Y * 0.9));
    if let Some((eye, target)) = config.camera {
        let (_, ref mut xf) = *camera;
        xf.translation = player.translation + eye;
        xf.look_at(player.translation + target, Vec3::Y);
        return;
    }
    let max_distance = if driving { CAR_CAMERA_DISTANCE_M } else { CAMERA_DISTANCE_M };
    let (ref mut cam, ref mut xf) = *camera;
    if cursor.grab_mode == CursorGrabMode::Locked {
        cam.yaw -= motion.delta.x * 0.003;
        cam.pitch = (cam.pitch - motion.delta.y * 0.003).clamp(-1.2, 0.8);
    }
    let rot = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    let target = player.translation + Vec3::Y * CAMERA_HEIGHT_M;
    let back = rot * Vec3::Z;
    // Characters don't pull the camera in, only the world.
    let not_player = |e: Entity| !parts.contains(e) && !jason_parts.contains(e) && !(driving && vehicle_parts.contains(e));
    let settings = MeshRayCastSettings::default().with_filter(&not_player);
    let distance = ray_cast
        .cast_ray(Ray3d::new(target, Dir3::new(back).unwrap()), &settings)
        .first()
        .map_or(max_distance, |(_, hit)| (hit.distance - 0.2).clamp(0.3, max_distance));
    xf.translation = target + back * distance;
    xf.look_at(target, Vec3::Y);
}

fn update_hud(
    player: Single<(&Player, &Transform)>,
    jason: Query<(&crate::jason::Jason, &Transform), Without<Player>>,
    tuning: Res<PlayerTuning>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    mut hud: Single<&mut Text, With<Hud>>,
) {
    use bevy::diagnostic::FrameTimeDiagnosticsPlugin as Ftd;
    let fps = diagnostics.get(&Ftd::FPS).and_then(|d| d.smoothed()).unwrap_or(0.0);
    let frame_ms = diagnostics.get(&Ftd::FRAME_TIME).and_then(|d| d.smoothed()).unwrap_or(0.0);
    let (player, pxf) = *player;
    let c = &player.counselor;
    let gait = if c.sprinting {
        "sprinting"
    } else if c.running {
        "running"
    } else {
        "walking"
    };
    let situation = player.situation();
    let state = if !c.alive() {
        " (DEAD)"
    } else if c.wounded {
        " (wounded)"
    } else {
        ""
    };
    hud.0 = format!(
        "Vanessa  health {:3.0}{}  stamina {:5.1}{}  fear {:3.0}  |  {:3.0} fps ({:4.1} ms)\n{:?} {}{}  speed {:4.0} / {:4.0} cm/s",
        c.health,
        state,
        c.stamina,
        if c.stamina_depleted { " (depleted)" } else { "" },
        player.fear,
        fps,
        frame_ms,
        c.stance,
        gait,
        if c.stumbling {
            " STUMBLING"
        } else if c.dodging() {
            " DODGING"
        } else {
            ""
        },
        player.velocity.length(),
        c.max_speed(&situation, &tuning.counselor),
    );
    for (j, jxf) in &jason {
        hud.0.push_str(&format!("
Jason: {:?}, {:.0} m away", j.state, jxf.translation.distance(pxf.translation)));
    }
}
