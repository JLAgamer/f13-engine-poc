//! A playable counselor (Vanessa) driven by the f13-core movement and stamina rules.
//!
//! Controls (DefaultInput.ini's): WASD move (running by default), Shift sprint, X toggle walk, Ctrl crouch,
//! mouse look (click to capture, Esc to release); E interact, mouse wheel / 1-3 / G select and use a small item, Z drop
//! (interact.rs: the game's DefaultInput.ini keys), F leave a car (vehicle.rs). The capsule moves through the game's own
//! collision (collision.rs, f13-core::collision).

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
/// Falling faster than this (m/s) when she touches down makes a landing sound rather than a step's.
const LANDING_SPEED_M_S: f32 = 2.0;
/// Afloat, her centre is this far under the water's surface (Unreal floats a Buoyancy 1 swimmer with its centre at the
/// surface: just above, it isn't in water and falls back).
const SWIM_DEPTH_M: f32 = 0.05;

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
    /// Fear 0..=100 (fear_events.rs keeps it up to date).
    pub fear: f32,
    /// A montage's root motion moving the counselor (dodge, stumble): local velocity (x right, y forward), cm/s.
    pub root_motion: Option<(Vec2, f32)>,
    /// The montage the rules started (dodge, stumble), for the animation.
    pub montage: Option<String>,
    /// Vertical speed (m/s, falling) and whether the last move ended on a walkable floor.
    pub vel_y: f32,
    pub grounded: bool,
    /// In a hiding spot (hiding.rs).
    pub hidden: bool,
    /// Swimming: her input acceleration squared (cm²/s⁴; stamina only drains while she strokes).
    pub swim_accel_sq: f32,
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
            in_hiding: self.hidden,
            accel_sq: self.swim_accel_sq,
            ..Default::default()
        }
    }
}

/// What a counselor wants this tick, from her pad ([`crate::controls::Pads`]) or the demo script.
#[derive(Component, Default)]
pub(crate) struct Intent {
    /// x = right, y = forward, relative to her control yaw.
    movement: Vec2,
    /// Her control rotation's yaw (the local counselor's: the camera's).
    yaw: f32,
    /// When she last tapped A, D, S (double taps dodge).
    last_tap: [f32; 3],
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
    /// Other counselors' starts (--counselors).
    pub others: Vec<Vec3>,
    /// A placeholder capsule instead of the counselor's model.
    pub capsule: bool,
    /// A fixed camera instead of the follow camera: eye and look-at point relative to the counselor's feet (testing).
    pub camera: Option<(Vec3, Vec3)>,
    /// Scripted run: hold forward and sprint, logging stamina every second.
    pub demo: bool,
    /// Fear at the start (--fear holds it there: fear_events.rs).
    pub fear: f32,
    /// Starting health (testing: nothing deals damage in the client yet).
    pub health: Option<f32>,
}

#[derive(Resource)]
pub(crate) struct PlayerConfig {
    spawn: Vec3,
    others: Vec<Vec3>,
    capsule: bool,
    pub(crate) camera: Option<(Vec3, Vec3)>,
    demo: bool,
    fear: f32,
    health: Option<f32>,
}

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(PlayerConfig { others: self.others.clone(), spawn: self.spawn, capsule: self.capsule, camera: self.camera, demo: self.demo, fear: self.fear, health: self.health })
            .add_systems(Startup, spawn_player)
            // Input is read once per frame; one-shot actions stay latched until a fixed step uses them, so a frame
            // that runs several fixed steps (low frame rate, --speed) doesn't repeat a key press.
            .add_systems(RunFixedMainLoop, read_intent.in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop))
            // A network client's counselors move on the server (net.rs).
            .add_systems(FixedUpdate, (update_counselor, move_player).chain().run_if(not(resource_exists::<crate::net::NetClient>)))
            .add_systems(Update, (
                    tag_player_parts,
                    grab_cursor,
                    aim_demo,
                    crate::counselor_camera::chase,
                    crate::counselor_camera::look,
                    // Her cameras, not when Jason's played (jason_player.rs has the view).
                    crate::counselor_camera::counselor_camera.run_if(not(resource_exists::<crate::jason_player::PlayJason>)),
                    update_hud,
                    toggle_debug_hud,
                )
                    .chain());
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
        Player { counselor, velocity: Vec2::ZERO, walk_toggle: false, fear: config.fear, root_motion: None, montage: None, vel_y: 0.0, grounded: false, hidden: false, swim_accel_sq: 0.0 },
        Transform::from_translation(config.spawn),
        Visibility::default(),
        Unsettled,
        crate::controls::LocalPlayer,
        crate::controls::Controls::default(),
        Intent::default(),
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
    // The other counselors (--counselors): at their starts, their controls idle (a network or bot player's later).
    for &at in &config.others {
        spawn_counselor(&mut commands, &assets, character.as_deref().filter(|_| !config.capsule), &tuning, at);
    }
    commands.spawn((
        Camera3d::default(),
        // Placed by counselor_camera.rs from the first frame.
        Transform::from_translation(config.spawn),
        PlayerCamera { yaw: 0.0, pitch: 0.0 },
        IsDefaultUiCamera,
    ));
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        Node { position_type: PositionType::Absolute, top: Val::Px(12.0), left: Val::Px(12.0), ..default() },
        // The game's HUD (ui.rs) shows; F3 shows this instead.
        Visibility::Hidden,
        GlobalZIndex(5),
        Hud,
    ));
}

/// A counselor other than the local one, at `at`: full health, her controls idle until her player's (a network or
/// bot player's) come in.
pub(crate) fn spawn_counselor(commands: &mut Commands, assets: &AssetServer, character: Option<&crate::character::CounselorCharacter>, tuning: &PlayerTuning, at: Vec3) -> Entity {
    let mut counselor = Counselor::new(tuning.stats);
    counselor.health = tuning.counselor.health.max_health;
    let other = commands
        .spawn((
            Player { counselor, velocity: Vec2::ZERO, walk_toggle: false, fear: 0.0, root_motion: None, montage: None, vel_y: 0.0, grounded: false, hidden: false, swim_accel_sq: 0.0 },
            Transform::from_translation(at),
            Visibility::default(),
            Unsettled,
            crate::controls::Controls::default(),
            Intent::default(),
            Name::new("counselor"),
        ))
        .id();
    if let Some(character) = character {
        crate::character::spawn_model(commands, assets, character, crate::character::Model::Counselor, other, ());
    }
    info!("counselor {other} at {at}");
    other
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
    pads: crate::controls::Pads,
    time: Res<Time<Real>>,
    config: Res<PlayerConfig>,
    mut counselors: Query<(Entity, &mut Intent, Has<crate::controls::LocalPlayer>), With<Player>>,
) {
    use crate::controls::Button;
    for (who, mut intent, local) in &mut counselors {
        intent.yaw = pads.look(who).0;
        if config.demo && local {
            let was = intent.sprint_held;
            intent.movement = Vec2::Y;
            intent.sprint_held = true;
            intent.sprint_changed |= !was;
            continue;
        }
        intent.movement = pads.movement(who);
        intent.sprint_held = pads.pressed(who, Button::Sprint);
        intent.sprint_changed |= pads.just_pressed(who, Button::Sprint) || pads.just_released(who, Button::Sprint);
        // CSLR_Run (X), on release.
        intent.toggle_walk |= pads.just_released(who, Button::Walk);
        intent.toggle_crouch |= pads.just_pressed(who, Button::Crouch);
        intent.toggle_combat |= pads.just_pressed(who, Button::Combat);
        let now = time.elapsed_secs();
        for (i, (b, dir)) in [(Button::Left, DodgeDirection::Left), (Button::Right, DodgeDirection::Right), (Button::Back, DodgeDirection::Back)].into_iter().enumerate() {
            if pads.just_pressed(who, b) {
                if now - intent.last_tap[i] <= DOUBLE_TAP_SECONDS {
                    intent.dodge = Some(dir);
                    intent.last_tap[i] = f32::MIN;
                } else {
                    intent.last_tap[i] = now;
                }
            }
        }
    }
}

/// Applies sprint/run/crouch input, ticks stamina and rolls stumbles.
#[allow(clippy::too_many_arguments)]
fn update_counselor(
    config: Res<PlayerConfig>,
    time: Res<Time>,
    tuning: Res<PlayerTuning>,
    anims: Res<crate::Anims>,
    mut rng: ResMut<crate::GameRng>,
    mut counselors: Query<(&mut Player, &mut Intent, Has<crate::controls::LocalPlayer>)>,
    mut demo_log: Local<u32>,
) {
    for (mut player, mut intent, local) in &mut counselors {
        update_one(&mut player, &mut intent, local, &config, &time, &tuning, &anims, &mut rng, &mut demo_log);
    }
}

/// One counselor's [`update_counselor`].
#[allow(clippy::too_many_arguments)]
fn update_one(p: &mut Player, intent: &mut Intent, local: bool, config: &PlayerConfig, time: &Time, tuning: &PlayerTuning, anims: &crate::Anims, rng: &mut crate::GameRng, demo_log: &mut u32) {
    let now = time.elapsed_secs();
    // Consume this frame's one-shot actions (see PlayerPlugin).
    let once = Intent { movement: intent.movement, sprint_held: intent.sprint_held, yaw: intent.yaw, last_tap: intent.last_tap, ..default() };
    let intent = std::mem::replace(intent, once);
    // The run toggle (0x1403cab00): crouching, she stands first (0x1403c09c0); then bIsRunning flips.
    if intent.toggle_walk {
        if p.counselor.stance == Stance::Crouching {
            p.counselor.stance = Stance::Standing;
        }
        p.walk_toggle = !p.walk_toggle;
    }
    let c = &mut p.counselor;
    let swimming = c.stance == Stance::Swimming;
    if intent.toggle_crouch && !swimming {
        c.stance = if c.stance == Stance::Crouching { Stance::Standing } else { Stance::Crouching };
    }
    if intent.toggle_combat && !swimming {
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
    if config.demo && local && time.elapsed_secs() as u32 > *demo_log {
        *demo_log = time.elapsed_secs() as u32;
        let c = &p.counselor;
        info!(
            "demo t={:>2}s stamina {:5.1} sprinting {} running {} depleted {} speed {:4.0} cm/s",
            *demo_log, c.stamina, c.sprinting, c.running, c.stamina_depleted, p.velocity.length()
        );
    }
}

/// A direction she walks in by herself (Bevy x, z), ignoring the keys: escaping on foot (police.rs).
#[derive(Component, Clone, Copy)]
pub struct Autopilot(pub Vec2);

pub(crate) fn move_player(
    autopilot: Query<&Autopilot>,
    time: Res<Time>,
    tuning: Res<PlayerTuning>,
    mut counselors: Query<
        (Entity, &mut Player, &mut Transform, &Intent, Option<&crate::interact::UsingItem>, Option<&Unsettled>, (Has<crate::jason::Held>, Has<crate::hiding::Hiding>, Has<crate::items::Trapped>, (Has<crate::interact::DoorMove>, Has<crate::interact::WindowMove>, Has<crate::interact::BarricadeMove>, Has<crate::interact::MoveToSpot>, Has<crate::cabinets::CabinetUse>)), Has<crate::jason::Staggered>, Option<&crate::items::ItemAction>, Option<&crate::items::GunAim>, Option<&crate::interact::Interaction>),
        Without<Driving>,
    >,
    collision: Res<crate::collision::WorldCollision>,
    dynamic: Res<crate::collision::DynamicColliders>,
    water: Option<Res<crate::water::Water>>,
    mut commands: Commands,
) {
    let dt = time.delta_secs();
    for (entity, mut p, mut xf, intent, using, settle, (held, hiding, trapped, (door_move, window_move, barricade_move, sliding, searching)), staggered, item_action, gun_aim, interaction) in &mut counselors {
        p.hidden = hiding;
        if held || hiding || sliding {
            // In Jason's hands (he places her), in a hiding spot (hiding.rs places her) or sliding onto a move's spot
            // (interact.rs places her).
            p.velocity = Vec2::ZERO;
            continue;
        }
        let (sin, cos) = intent.yaw.sin_cos();
        let forward = Vec2::new(-sin, -cos);
        let right = Vec2::new(cos, -sin);
        let repairing = interaction.is_some_and(|i| i.busy());
        // A swing's montage moves her by its root motion while it plays (RootMotionFromMontagesOnly), her input ignored.
        let swing_root = item_action.and_then(|a| a.root);
        let item_action = item_action.is_some();
        // Aiming a gun: the controller ignores move input (SetIgnoreMoveInput) and she takes the control yaw (items.rs).
        let aiming = gun_aim.is_some_and(|a| a.aiming);
        let busy = aiming || !p.counselor.alive() || using.is_some() || repairing || staggered || item_action || trapped || door_move || window_move || barricade_move || sliding || searching;
        let wish = match autopilot.get(entity) {
            _ if busy => Vec2::ZERO,
            Ok(a) => a.0.normalize_or_zero(),
            Err(_) => forward * intent.movement.y + right * intent.movement.x,
        };

        let (facing, _, _) = xf.rotation.to_euler(EulerRot::YXZ);
        if let Some(local) = p.root_motion.map(|(l, _)| l).or(swing_root) {
            // The montage moves the counselor (relative to where they face), whatever the input.
            let fwd = Vec2::new(facing.sin(), facing.cos());
            let rt = Vec2::new(-fwd.y, fwd.x);
            p.velocity = fwd * local.y + rt * local.x;
        } else {
            let situation = p.situation();
            let max_speed = p.counselor.max_speed(&situation, &tuning.counselor);
            let accel = input_acceleration(wish.to_array(), &tuning.movement);
            p.swim_accel_sq = accel[0] * accel[0] + accel[1] * accel[1];
            let mut v = p.velocity.to_array();
            calc_velocity(&mut v, accel, max_speed, dt, &tuning.movement);
            p.velocity = Vec2::from_array(v);
        }

        // Move the capsule through the world's collision (walls, steps, slopes, falling).
        let world = collision.with(&dynamic);
        if settle.is_some() {
            if let Some(at) = f13_core::collision::spawn_floor(&world, xf.translation, SPAWN_PROBE_M, CAPSULE_HALF_HEIGHT_M * 2.0, &WALK) {
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
        // Water: she swims while her centre is under the surface (Unreal's IsInWater at the pawn's location); with
        // Buoyancy 1 she floats with her centre at the surface (SWIM_DEPTH_M under it, so she stays in).
        let surface = water.as_ref().and_then(|w| w.surface_at(pos + Vec3::Y * CAPSULE_HALF_HEIGHT_M));
        let swimming = p.counselor.stance == Stance::Swimming;
        if let (Some(top), false) = (surface, swimming) {
            info!("swimming (surface {top:.2})");
            p.counselor.stance = Stance::Swimming;
            p.counselor.set_sprinting(false);
        }
        let swim_feet = surface.map(|top| top - SWIM_DEPTH_M - CAPSULE_HALF_HEIGHT_M);
        if let Some(feet) = swim_feet {
            // Afloat (from wherever she was under the surface).
            pos.y = feet;
            p.vel_y = 0.0;
        }
        let falling = p.vel_y;
        let r = f13_core::collision::move_capsule(&world, &mut pos, &mut p.vel_y, grounded && swim_feet.is_none(), delta, dt, &cap, &WALK);
        p.grounded = r.grounded;
        // Down from a fall: her landing sound (GetLandingSound).
        if !grounded && r.grounded && falling < -LANDING_SPEED_M_S && swim_feet.is_none() {
            commands.trigger(crate::audio::Footstep { owner: entity, left: true, land: true });
        }
        if let (Some(feet), Stance::Swimming) = (swim_feet, p.counselor.stance) {
            // Out where the bottom comes up to where she floats, else afloat.
            let floor = f13_core::collision::surface_below(&world, pos.x, pos.z, feet + MAX_STEP_HEIGHT_M, feet - 3.0).map(|s| s.0);
            if floor.is_some_and(|y| y >= feet) {
                info!("out of the water");
                p.counselor.stance = Stance::Standing;
                pos.y = floor.unwrap_or(pos.y).max(pos.y);
                p.grounded = true;
            } else {
                pos.y = feet;
                p.vel_y = 0.0;
                p.grounded = false;
            }
        } else if p.counselor.stance == Stance::Swimming {
            info!("out of the water");
            p.counselor.stance = Stance::Standing;
        }
        // Like Unreal's SlideAlongSurface: drop the velocity going into the walls hit.
        if let Some(n) = r.wall_normal.try_normalize() {
            let into = p.velocity.dot(n).min(0.0);
            p.velocity -= n * into;
        }
        // Fell out of the world (no surface under the feet at all): back onto the surface above.
        if !r.grounded && f13_core::collision::surface_below(&world, pos.x, pos.z, pos.y + MAX_STEP_HEIGHT_M, pos.y - 100.0).is_none() {
            if let Some(at) = f13_core::collision::spawn_floor(&world, pos, SPAWN_PROBE_M, CAPSULE_HALF_HEIGHT_M * 2.0, &WALK) {
                warn!("fell out of the map at {pos}; back to {at}");
                pos = at;
                p.vel_y = 0.0;
            }
        }
        xf.translation = pos;

        // Orient to movement (the model faces +Z); in combat stance face where the camera looks. Not during root motion.
        let combat = p.counselor.stance == Stance::Combat;
        if p.root_motion.is_none() && !aiming && (combat || p.velocity.length_squared() > 1.0) {
            let target = if combat { forward.x.atan2(forward.y) } else { p.velocity.x.atan2(p.velocity.y) };
            let (current, _, _) = xf.rotation.to_euler(EulerRot::YXZ);
            let delta = (target - current + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let max = TURN_RATE_DEG.to_radians() * dt;
            xf.rotation = Quat::from_rotation_y(current + delta.clamp(-max, max));
        }
    }
}

/// Demo: once the map has loaded, turn the camera toward the longest clear line so the run isn't a tree hug.
/// Aims again whenever the run is blocked (props stream in after the first aim).
fn aim_demo(
    config: Res<PlayerConfig>,
    time: Res<Time>,
    player: Single<(&Transform, &Player), With<crate::controls::LocalPlayer>>,
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
/// F3: the debug text (it shows when the game's HUD is missing).
fn toggle_debug_hud(keys: Res<ButtonInput<KeyCode>>, game_hud: Option<Res<crate::ui::Hud>>, mut hud: Single<&mut Visibility, With<Hud>>) {
    if keys.just_pressed(KeyCode::F3) || game_hud.is_none() {
        let v = if game_hud.is_none() || **hud == Visibility::Hidden { Visibility::Inherited } else { Visibility::Hidden };
        hud.set_if_neq(v);
    }
}

fn update_hud(
    player: Single<(&Player, &Transform, Option<&crate::hiding::Breathing>, Has<crate::hiding::Hiding>), With<crate::controls::LocalPlayer>>,
    jason: Query<(&crate::jason::Jason, &Transform), Without<Player>>,
    tuning: Res<PlayerTuning>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    navmesh: Res<crate::navigation::NavmeshStatus>,
    mut hud: Single<&mut Text, With<Hud>>,
) {
    use bevy::diagnostic::FrameTimeDiagnosticsPlugin as Ftd;
    let fps = diagnostics.get(&Ftd::FPS).and_then(|d| d.smoothed()).unwrap_or(0.0);
    let frame_ms = diagnostics.get(&Ftd::FRAME_TIME).and_then(|d| d.smoothed()).unwrap_or(0.0);
    let (player, pxf, breathing, hiding) = *player;
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
    if let (Some(b), true) = (breathing, hiding) {
        hud.0.push_str(&format!(
            "\nBreath {:3.0}{}  [Space] hold your breath",
            b.breath.current,
            if b.breath.holding { " (holding)" } else if b.breath.can_hold { "" } else { " (catching it)" }
        ));
    }
    for (j, jxf) in &jason {
        hud.0.push_str(&format!("
Jason: {:?}, {:.0} m away", j.state, jxf.translation.distance(pxf.translation)));
    }
    hud.0.push('\n');
    hud.0.push_str(&navmesh.0);
}
