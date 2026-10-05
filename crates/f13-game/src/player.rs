//! A playable counselor (Vanessa) driven by the f13-core movement and stamina rules.
//!
//! Placeholder controls: WASD move (running by default), Shift sprint, Caps Lock toggle walk, Ctrl crouch,
//! mouse look (click to capture, Esc to release); E interact and G drop (interact.rs), F leave a car (vehicle.rs). Ground height and walls come from ray casts against the map
//! meshes; there is no physics engine yet.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use bevy::world_serialization::WorldAssetRoot;
use f13_core::counselor::{Counselor, CounselorEvent, CounselorTuning, Situation, Stance};
use f13_core::movement::{calc_velocity, input_acceleration, MovementSettings};
use crate::vehicle::Driving;

/// Unreal units (cm) per Bevy unit (m).
const CM: f32 = 100.0;
/// Character turn rate (the counselor Blueprint's RotationRate yaw), degrees per second.
const TURN_RATE_DEG: f32 = 440.0;
const MAX_STEP_HEIGHT_M: f32 = 0.45;
const CAPSULE_RADIUS_M: f32 = 0.34;
const CAPSULE_HALF_HEIGHT_M: f32 = 0.88;
const CAMERA_DISTANCE_M: f32 = 3.0;
const CAR_CAMERA_DISTANCE_M: f32 = 7.0;
const CAMERA_HEIGHT_M: f32 = 1.6;

#[derive(Resource, Clone)]
pub struct PlayerTuning {
    pub counselor: CounselorTuning,
    pub movement: MovementSettings,
    pub stats: f13_core::stats::Stats,
    pub first_aid: f13_data::FirstAid,
}

#[derive(Component)]
pub struct Player {
    pub counselor: Counselor,
    /// Horizontal velocity in Bevy's XZ plane, in cm/s (Unreal units).
    pub velocity: Vec2,
    pub walk_toggle: bool,
    /// Fear 0..=100 (no fear events are pushed in the client yet).
    pub fear: f32,
}

impl Player {
    /// What the counselor rules need to know about this tick.
    pub fn situation(&self) -> Situation {
        Situation { speed_sq: self.velocity.length_squared(), fear: self.fear, ..Default::default() }
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
}

/// Marks the player's own meshes so ground, wall and camera ray casts ignore them.
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
    /// Show the counselor's mesh (bind pose, no outfit yet) instead of a placeholder capsule.
    pub show_model: bool,
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
    show_model: bool,
    demo: bool,
    fear: f32,
    health: Option<f32>,
}

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PlayerConfig { spawn: self.spawn, show_model: self.show_model, demo: self.demo, fear: self.fear, health: self.health })
            .init_resource::<Intent>()
            .add_systems(Startup, spawn_player)
            .add_systems(FixedUpdate, (read_intent, update_counselor, move_player).chain())
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
) {
    let mut counselor = Counselor::new(tuning.stats);
    counselor.health = config.health.unwrap_or(tuning.counselor.health.max_health).clamp(1.0, tuning.counselor.health.max_health);
    counselor.update_wounded(counselor.health, true, &tuning.counselor);
    let mut player = commands.spawn((
        Player { counselor, velocity: Vec2::ZERO, walk_toggle: false, fear: config.fear },
        Transform::from_translation(config.spawn),
        Visibility::default(),
    ));
    if config.show_model {
        let body = "SummerCamp/Content/Characters/Counselors/Athlete/Meshes/Body";
        player.with_children(|p| {
            for part in ["Athlete_Body", "Athlete_Face", "Athlete_Hair_SM"] {
                let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("{body}/{part}.glb")));
                p.spawn((WorldAssetRoot(scene), PlayerPart));
            }
        });
    } else {
        let capsule = meshes.add(Capsule3d::new(CAPSULE_RADIUS_M, 2.0 * (CAPSULE_HALF_HEIGHT_M - CAPSULE_RADIUS_M)));
        let material = materials.add(StandardMaterial { base_color: Color::srgb(0.75, 0.55, 0.35), ..default() });
        player.with_child((
            Mesh3d(capsule),
            MeshMaterial3d(material),
            Transform::from_xyz(0.0, CAPSULE_HALF_HEIGHT_M, 0.0),
            PlayerPart,
        ));
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

fn read_intent(keys: Res<ButtonInput<KeyCode>>, config: Res<PlayerConfig>, mut intent: ResMut<Intent>) {
    if config.demo {
        let was = intent.sprint_held;
        *intent = Intent { movement: Vec2::Y, sprint_held: true, sprint_changed: !was, ..default() };
        return;
    }
    let mut movement = Vec2::ZERO;
    for (k, v) in [(KeyCode::KeyW, Vec2::Y), (KeyCode::KeyS, -Vec2::Y), (KeyCode::KeyD, Vec2::X), (KeyCode::KeyA, -Vec2::X)] {
        if keys.pressed(k) {
            movement += v;
        }
    }
    *intent = Intent {
        movement,
        sprint_held: keys.pressed(KeyCode::ShiftLeft),
        sprint_changed: keys.just_pressed(KeyCode::ShiftLeft) || keys.just_released(KeyCode::ShiftLeft),
        toggle_walk: keys.just_pressed(KeyCode::CapsLock),
        toggle_crouch: keys.just_pressed(KeyCode::ControlLeft),
    };
}

/// Applies sprint/run/crouch input, ticks stamina and rolls stumbles.
#[allow(clippy::too_many_arguments)]
fn update_counselor(
    intent: Res<Intent>,
    config: Res<PlayerConfig>,
    time: Res<Time>,
    tuning: Res<PlayerTuning>,
    mut rng: ResMut<crate::GameRng>,
    mut player: Single<&mut Player>,
    mut demo_log: Local<u32>,
) {
    let now = time.elapsed_secs();
    let p = &mut **player;
    if intent.toggle_walk {
        p.walk_toggle = !p.walk_toggle;
    }
    let c = &mut p.counselor;
    if intent.toggle_crouch {
        c.stance = if c.stance == Stance::Crouching { Stance::Standing } else { Stance::Crouching };
    }
    if intent.sprint_changed {
        c.set_sprint_input(intent.sprint_held);
    }
    if !intent.sprint_held {
        c.set_running(!p.walk_toggle);
    }
    let situation = p.situation();
    let t = &tuning.counselor;
    let mut events = Vec::new();
    p.counselor.tick_stamina(time.delta_secs(), &situation, t, &mut events);
    for e in events {
        match e {
            CounselorEvent::StaminaDepleted { forced_stumble } => {
                info!("out of stamina");
                if forced_stumble && p.counselor.start_stumble(now, true, &t.stumble) {
                    info!("stumbled (out of stamina)");
                }
            }
            CounselorEvent::StaminaRecovered => info!("stamina recovered"),
        }
    }
    let roll = || f13_core::stumble::roll_from_unit(rng.0.frand());
    if p.counselor.tick_stumble(time.delta_secs(), now, &situation, t.stamina_max, &t.stumble, roll) {
        info!("stumbled (chance {:.0}%)", t.stumble.chance(&p.counselor, p.fear, t.stamina_max));
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
    mut player: Single<(&mut Player, &mut Transform, Option<&crate::interact::UsingItem>), Without<Driving>>,
    mut ray_cast: MeshRayCast,
    parts: Query<(), With<PlayerPart>>,
) {
    let dt = time.delta_secs();
    let (ref mut p, ref mut xf, using) = *player;
    let (sin, cos) = camera.yaw.sin_cos();
    let forward = Vec2::new(-sin, -cos);
    let right = Vec2::new(cos, -sin);
    let busy = !p.counselor.alive() || using.is_some();
    let wish = if busy { Vec2::ZERO } else { forward * intent.movement.y + right * intent.movement.x };

    let situation = p.situation();
    let max_speed = p.counselor.max_speed(&situation, &tuning.counselor);
    let accel = input_acceleration(wish.to_array(), &tuning.movement);
    let mut v = p.velocity.to_array();
    calc_velocity(&mut v, accel, max_speed, dt, &tuning.movement);
    p.velocity = Vec2::from_array(v);

    // Move, stopping at walls and following the ground.
    let step = p.velocity * dt / CM;
    let not_player = |e: Entity| !parts.contains(e);
    let settings = MeshRayCastSettings::default().with_filter(&not_player);
    let ground_at = |ray_cast: &mut MeshRayCast, p: Vec3| {
        ray_cast.cast_ray(Ray3d::new(p + Vec3::Y * (MAX_STEP_HEIGHT_M + 1.0), Dir3::NEG_Y), &settings).first().map(|(_, h)| h.point.y)
    };
    // Started (or got pushed) slightly below the surface: stand on it instead of treating it as a wall.
    if let Some(y) = ground_at(&mut ray_cast, xf.translation) {
        if y > xf.translation.y {
            xf.translation.y = y;
        }
    }
    let mut pos = xf.translation;
    if step.length_squared() > 0.0 {
        // Like Unreal's SlideAlongSurface: on a hit, drop the part of the move that goes into the surface.
        let mut step = step;
        for _ in 0..2 {
            let Ok(dir) = Dir3::new(Vec3::new(step.x, 0.0, step.y)) else { break };
            let hit = ray_cast
                .cast_ray(Ray3d::new(pos + Vec3::Y * 1.0, dir), &settings)
                .first()
                .filter(|(_, hit)| hit.distance < step.length() + CAPSULE_RADIUS_M)
                .map(|(_, hit)| Vec2::new(hit.normal.x, hit.normal.z).normalize_or_zero());
            let Some(n) = hit else { break };
            step -= n * step.dot(n).min(0.0);
            let into = p.velocity.dot(n).min(0.0);
            p.velocity -= n * into;
            if n == Vec2::ZERO {
                step = Vec2::ZERO;
                p.velocity = Vec2::ZERO;
            }
        }
        pos += Vec3::new(step.x, 0.0, step.y);
    }
    if let Some(ground) = ground_at(&mut ray_cast, pos) {
        if ground <= xf.translation.y + MAX_STEP_HEIGHT_M {
            pos.y = ground;
        } else {
            pos = xf.translation; // too steep: stay put
            p.velocity = Vec2::ZERO;
        }
    }
    xf.translation = pos;

    // Orient to movement (the model faces +Z).
    if p.velocity.length_squared() > 1.0 {
        let target = p.velocity.x.atan2(p.velocity.y);
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
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    player: Single<(&Transform, Has<Driving>), (With<Player>, Without<PlayerCamera>)>,
    mut camera: Single<(&mut PlayerCamera, &mut Transform)>,
    mut ray_cast: MeshRayCast,
    parts: Query<(), With<PlayerPart>>,
    vehicle_parts: Query<(), With<crate::props::VehiclePart>>,
) {
    let (player, driving) = *player;
    let max_distance = if driving { CAR_CAMERA_DISTANCE_M } else { CAMERA_DISTANCE_M };
    let (ref mut cam, ref mut xf) = *camera;
    if cursor.grab_mode == CursorGrabMode::Locked {
        cam.yaw -= motion.delta.x * 0.003;
        cam.pitch = (cam.pitch - motion.delta.y * 0.003).clamp(-1.2, 0.8);
    }
    let rot = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    let target = player.translation + Vec3::Y * CAMERA_HEIGHT_M;
    let back = rot * Vec3::Z;
    let not_player = |e: Entity| !parts.contains(e) && !(driving && vehicle_parts.contains(e));
    let settings = MeshRayCastSettings::default().with_filter(&not_player);
    let distance = ray_cast
        .cast_ray(Ray3d::new(target, Dir3::new(back).unwrap()), &settings)
        .first()
        .map_or(max_distance, |(_, hit)| (hit.distance - 0.2).clamp(0.3, max_distance));
    xf.translation = target + back * distance;
    xf.look_at(target, Vec3::Y);
}

fn update_hud(player: Single<&Player>, tuning: Res<PlayerTuning>, mut hud: Single<&mut Text, With<Hud>>) {
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
        "Vanessa  health {:3.0}{}  stamina {:5.1}{}  fear {:3.0}\n{:?} {}{}  speed {:4.0} / {:4.0} cm/s",
        c.health,
        state,
        c.stamina,
        if c.stamina_depleted { " (depleted)" } else { "" },
        player.fear,
        c.stance,
        gait,
        if c.stumbling { " STUMBLING" } else { "" },
        player.velocity.length(),
        c.max_speed(&situation, &tuning.counselor),
    );
}
