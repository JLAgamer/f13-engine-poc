//! Calling the police (SCPhoneJunctionBox, SCPolicePhone, SCPoliceSpawner, SCEscapeVolume):
//! - **The phone box** (PhoneJunctionBox_BP) starts without its fuse. Its FuseRepair needs a PhoneBoxFuse_C (the
//!   match spawns one, f13-core::spawn); installing it is a hold with the repair minigame (its Interactable's
//!   HoldTimeLimit, 15 s) and scores its RepairScoreEvent. It enables the phone it's wired to (ConnectedPhone).
//! - **Jason breaking the box** (Jason_PhoneBox_Destruction_Machete, jason.rs; DestroyBox): it's broken
//!   (bIsBroken: the phone is disabled and a call in progress is cut off) until a counselor repairs it with its
//!   Interactable's own minigame (15 s, its pips; no part). PLACEHOLDER: he can break it once its fuse is in and
//!   until the police have been called (his CanInteractWith isn't read).
//! - **The phone** (PolicePhone_BP): a 12 s hold (HoldTimeLimit) once enabled. When it's done the phone is used up,
//!   the police are on their way (StartPoliceTimer: f13-core::game_mode), the world settings' PoliceCalledSFX
//!   plays, she says her CallPolice line, and the phone's CallPoliceScoreEvent scores.
//! - **Arriving** (SpawnPolice): spawners in a random order (FRand), the first whose escape volume is
//!   unobstructed spawns the police car (Police_C: its car, siren and lights) at its own transform and activates
//!   that volume. The car's red and blue spot lights turn 8 degrees a frame (its Blueprint's tick).
//! - **Escaping** (SCEscapeVolume::OnOverlap, on foot): a counselor alive and free (not grabbed, not in a car, not
//!   hiding) inside the active volume starts escaping: she walks its ExitPath from its point nearest her, by
//!   herself, and the escape counts (the game mode's CharacterEscaped) after its EscapeDelay (6 s). Its camera
//!   spline isn't ported.
//! - **The level's blockers**: CrystalLake_Small_Design's Blueprint turns off the collision of the BlockingVolume in
//!   front of the escape the police came to (its PoliceSpawnedEvent handlers: Police_Blocker_A for PoliceSPawner12,
//!   Police_Blocker_B for PoliceSPawner2); until then it keeps counselors out.
//! - **Jason** entering the active volume is turned back along the spawner's InterceptDirection
//!   (JasonTriggerOnBeginOverlap); here he's set back on the volume's edge that way.
//!
//! PLACEHOLDER: IsUnobstructed is an overlap query whose filter isn't read in full; here a volume is obstructed while
//! Jason is inside it.

use crate::props::EscapeZone;
use bevy::prelude::*;
use f13_data::classes::ClassDb;
use f13_data::level::Xform;

pub struct PolicePlugin;

impl Plugin for PolicePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_phones).add_systems(Update, (arrive, turn_back_jason, escape_with_police, turn_lights, report_car).chain());
    }
}

/// The phone box (PhoneJunctionBox_BP): its fuse repair, and the phone it's wired to.
#[derive(Component)]
pub struct PhoneBox {
    pub class: String,
    pub phone: Option<Entity>,
    /// Its fuse is in.
    pub repaired: bool,
    /// Jason broke it (bIsBroken).
    pub broken: bool,
    /// The broken box's own repair (its Interactable's minigame) has pips.
    pub fix_pips: bool,
    /// Where it's worked on and where she stands (world, Bevy space).
    pub interact: Vec3,
    pub stand: Option<Transform>,
    /// cm.
    pub reach: f32,
    pub hold_time: f32,
    pub has_pips: bool,
    pub parts: Vec<String>,
}

/// Her takes of the fuse repair (AM_normal_stand_repair_phonebox's sections: enter, cycle, exit, fail).
pub const PHONE_BOX_TAKE: &str = "f_normal_stand_repair_phonebox";
/// Her takes of the call (AM_normal_stand_use_phone: start, cycle, stop, cancel).
pub const PHONE_TAKE: &str = "f_normal_stand_use_phone";

/// The police phone (PolicePhone_BP): usable once its box is repaired, once.
#[derive(Component)]
pub struct Phone {
    pub class: String,
    pub enabled: bool,
    pub called: bool,
    pub interact: Vec3,
    pub stand: Option<Transform>,
    pub reach: f32,
    pub hold_time: f32,
}

/// A police spawner: where the car stops, the escape volume it opens and the way Jason is turned back.
#[derive(Component)]
pub struct PoliceSpawner {
    zone: String,
    intercept: Vec3,
    car_class: String,
}

/// The level Blueprint's PoliceSpawnedEvent handlers: the blocker each spawner's arrival takes down.
const BLOCKERS: [(&str, &str); 2] = [("PoliceSPawner12", "Police_Blocker_A"), ("PoliceSPawner2", "Police_Blocker_B")];

/// One of the level's police blockers (a collider of its own, see collision.rs).
#[derive(Component)]
pub struct Blocker;

pub fn is_blocker(actor: &str) -> bool {
    BLOCKERS.iter().any(|(_, b)| *b == actor)
}

/// The police car (Police_C).
#[derive(Component)]
pub struct PoliceCar;

/// The police have arrived: the escape volume they opened.
#[derive(Resource)]
pub struct PoliceArrived {
    pub zone: Entity,
}

fn world_of(classes: &ClassDb, class: &str, component: &str, actor: &Xform) -> Option<Transform> {
    let c = classes.get(class)?.components.iter().find(|c| c.name == component)?;
    Some(crate::to_transform(&c.local.unwrap_or(Xform::IDENTITY).then(actor)))
}

/// The class's interact component's prop.
fn interact_prop(classes: &ClassDb, class: &str, component: &str, prop: &str) -> Option<f32> {
    classes.get(class)?.components.iter().find(|c| c.name == component)?.props[prop].as_f64().map(|v| v as f32)
}

fn spawn_phones(mut commands: Commands, level: Res<crate::Level>, export: Res<crate::ExportRoot>) {
    let Ok(classes) = ClassDb::load(&export.0.join("classes.json")) else { return };
    let actors = || level.layouts.iter().flat_map(|l| &l.actors).filter(|a| a.transform.is_some());
    let mut phones = std::collections::HashMap::new();
    for a in actors().filter(|a| classes.is_child_of(&a.class, "SCPolicePhone")) {
        let t = a.transform.unwrap();
        let Some(interact) = world_of(&classes, &a.class, "Interactable", &t) else { continue };
        let e = commands
            .spawn((
                Phone {
                    class: a.class.clone(),
                    enabled: false,
                    called: false,
                    interact: interact.translation,
                    stand: world_of(&classes, &a.class, "UsePhone_SpecialMove", &t),
                    reach: interact_prop(&classes, &a.class, "Interactable", "DistanceLimit").unwrap_or(150.0),
                    hold_time: interact_prop(&classes, &a.class, "Interactable", "HoldTimeLimit").unwrap_or(12.0),
                },
                crate::to_transform(&t),
                Name::new(a.name.clone()),
            ))
            .id();
        phones.insert(a.name.clone(), e);
    }
    for a in actors().filter(|a| classes.is_child_of(&a.class, "SCPhoneJunctionBox")) {
        let t = a.transform.unwrap();
        let Some(interact) = world_of(&classes, &a.class, "Interactable", &t) else { continue };
        // "PolicePhone_BP_C'PolicePhone_BP2'".
        let phone = a.props["ConnectedPhone"].as_str().and_then(|s| s.split('\'').nth(1)).and_then(|n| phones.get(n).copied());
        let repair = classes.get(&a.class).and_then(|c| c.components.iter().find(|c| classes.is_child_of(&c.kind, "SCRepairComponent")));
        info!("phone box {} at {:.1}, phone {:?}", a.name, interact.translation, phone);
        commands.spawn((
            PhoneBox {
                class: a.class.clone(),
                phone,
                repaired: false,
                broken: false,
                fix_pips: classes.get(&a.class).and_then(|c| c.components.iter().find(|c| c.name == "Interactable")).is_some_and(|c| c.props["Pips"].as_array().is_some_and(|p| !p.is_empty())),
                interact: interact.translation,
                stand: world_of(&classes, &a.class, "InteractSpecialMove", &t),
                reach: interact_prop(&classes, &a.class, "Interactable", "DistanceLimit").unwrap_or(150.0),
                hold_time: interact_prop(&classes, &a.class, "Interactable", "HoldTimeLimit").unwrap_or(15.0),
                has_pips: repair.is_some_and(|r| r.props["Pips"].as_array().is_some_and(|p| !p.is_empty())),
                parts: repair.map(|r| f13_data::level::str_list(&r.props["RequiredPartClasses"])).unwrap_or_default(),
            },
            crate::to_transform(&t),
            Name::new(a.name.clone()),
        ));
    }
    for a in actors().filter(|a| classes.is_child_of(&a.class, "SCPoliceSpawner")) {
        let t = a.transform.unwrap();
        let Some(zone) = a.props["EscapeVolume"].as_str().and_then(|s| s.split('\'').nth(1)) else { continue };
        let intercept = a.components.iter().find(|c| c.name == "InterceptDirection").and_then(|c| c.world).map(|w| crate::to_transform(&w).translation);
        commands.spawn((
            PoliceSpawner {
                zone: zone.to_string(),
                intercept: intercept.unwrap_or_else(|| crate::to_transform(&t).translation),
                car_class: classes.prop(&a.class, "PoliceSpawnClass").as_str().unwrap_or("Police_C").to_string(),
            },
            crate::to_transform(&t),
            Name::new(a.name.clone()),
        ));
    }
}

impl PhoneBox {
    /// A counselor can work on it: to put the fuse in, or to repair it once Jason broke it.
    pub fn needs_work(&self) -> bool {
        !self.repaired || self.broken
    }

    /// Jason can break it: it works, and the police haven't been called.
    pub fn breakable(&self, phones: &Query<&mut Phone>) -> bool {
        self.repaired && !self.broken && self.phone.and_then(|e| phones.get(e).ok()).is_some_and(|p| !p.called)
    }
}

/// The phone box works again (its fuse in, or repaired after Jason broke it): so does its phone, unless it's been
/// used.
pub fn repair_phone_box(b: &mut PhoneBox, phones: &mut Query<&mut Phone>) {
    b.repaired = true;
    b.broken = false;
    if let Some(mut p) = b.phone.and_then(|e| phones.get_mut(e).ok()) {
        p.enabled = !p.called;
    }
}

/// Jason broke the phone box (DestroyBox): its phone stops working.
pub fn break_phone_box(b: &mut PhoneBox, phones: &mut Query<&mut Phone>) {
    b.broken = true;
    if let Some(mut p) = b.phone.and_then(|e| phones.get_mut(e).ok()) {
        p.enabled = false;
    }
}

/// The call went through: the police are on their way.
pub fn call_police(commands: &mut Commands, phone: &mut Phone, game: Option<&mut crate::match_flow::GameMatch>, tuning: Option<&crate::match_flow::MatchTuning>, player: Entity) {
    phone.called = true;
    phone.enabled = false;
    if let (Some(g), Some(t)) = (game, tuning) {
        g.state.start_police_timer(&t.rules, 0.0);
        info!("police called: they arrive in {} s", g.state.police_time_remaining);
    }
    commands.trigger(crate::audio::PlaySound { cue: "PoliceCalled".into(), at: None });
    commands.trigger(crate::audio::ActorSound { actor: player, prop: "VO:CallPolice" });
    commands.trigger(crate::match_flow::ClassScore::new(&phone.class, "CallPoliceScoreEvent"));
}

/// SpawnPolice when the countdown runs out.
#[allow(clippy::too_many_arguments)]
fn arrive(
    mut commands: Commands,
    game: Option<ResMut<crate::match_flow::GameMatch>>,
    mut rng: ResMut<crate::GameRng>,
    spawners: Query<(&PoliceSpawner, &Transform, &Name)>,
    mut blockers: Query<(&Name, &mut Visibility), With<Blocker>>,
    mut zones: Query<(Entity, &mut EscapeZone, &GlobalTransform, &Name)>,
    jasons: Query<&Transform, With<crate::jason::Jason>>,
    export: Res<crate::ExportRoot>,
    assets: Res<AssetServer>,
) {
    let Some(mut game) = game else { return };
    if !game.state.take_police_due() {
        return;
    }
    let mut order: Vec<_> = spawners.iter().collect();
    while !order.is_empty() {
        let (s, xf, name) = order.remove(rng.0.index(order.len()));
        let Some((ze, mut zone, zxf, _)) = zones.iter_mut().find(|z| z.3.as_str() == s.zone) else { continue };
        let center = zxf.translation();
        if jasons.iter().any(|j| j.translation.distance(center) <= zone.radius_m) {
            info!("police: {} is obstructed", s.zone);
            continue;
        }
        zone.active = true;
        info!("police: arrived at {name} ({:.0}, {:.0}, {:.0}); escape at {}", xf.translation.x, xf.translation.y, xf.translation.z, s.zone);
        spawn_car(&mut commands, &export, &assets, &s.car_class, *xf);
        for (_, b) in BLOCKERS.iter().filter(|(sp, _)| *sp == name.as_str()) {
            for (_, mut v) in blockers.iter_mut().filter(|(n, _)| n.as_str() == *b) {
                *v = Visibility::Hidden;
                info!("police: {b} down");
            }
        }
        commands.insert_resource(PoliceArrived { zone: ze });
        return;
    }
    warn!("police: all police spawners reported unable to spawn");
}

/// The police car (Police_C): its meshes, and its siren looping.
fn spawn_car(commands: &mut Commands, export: &crate::ExportRoot, assets: &AssetServer, class: &str, at: Transform) {
    let Ok(classes) = ClassDb::load(&export.0.join("classes.json")) else { return };
    let Some(info) = classes.get(class) else {
        warn!("police: no class {class} in the export");
        return;
    };
    let sounds = info.components.iter().filter(|c| c.kind == "AudioComponent").filter_map(|c| Some((c.name.clone(), crate::audio::cue_name(&c.props["Sound"])?))).collect();
    let root = commands.spawn((at, Visibility::default(), Name::new(class.to_string()), crate::audio::ClassSounds(sounds), PoliceCar)).id();
    for c in info.components.iter().filter(|c| !c.hidden) {
        let Some(mesh) = &c.mesh else { continue };
        let scene = assets.load(GltfAssetLabel::Scene(0).from_asset(mesh.clone()));
        commands.spawn((bevy::world_serialization::WorldAssetRoot(scene), crate::to_transform(&c.local.unwrap_or(Xform::IDENTITY)), ChildOf(root)));
    }
    // Its red and blue lights (their IES profiles aren't ported), turned by its tick.
    for c in info.components.iter().filter(|c| c.kind == "SpotLightComponent") {
        let local = crate::to_transform(&c.local.unwrap_or(Xform::IDENTITY));
        let byte = |k: &str| c.props["LightColor"][k].as_u64().unwrap_or(255) as u8;
        let outer = (c.props["OuterConeAngle"].as_f64().unwrap_or(44.0) as f32).clamp(0.5, 89.0).to_radians();
        commands.spawn((
            SpotLight {
                color: Color::srgb_u8(byte("R"), byte("G"), byte("B")),
                intensity: c.props["Intensity"].as_f64().unwrap_or(5000.0) as f32,
                range: c.props["AttenuationRadius"].as_f64().unwrap_or(1000.0) as f32 / 100.0,
                inner_angle: (c.props["InnerConeAngle"].as_f64().unwrap_or(0.0) as f32).to_radians().min(outer),
                outer_angle: outer,
                shadow_maps_enabled: false,
                ..default()
            },
            // Unreal lights shine along +X; Bevy's along -Z.
            Transform::from_translation(local.translation).looking_to(local.rotation * Vec3::X, Vec3::Y),
            SirenLight,
            ChildOf(root),
        ));
    }
    commands.trigger(crate::audio::ActorLoop { actor: root, prop: "Siren" });
}

/// Logs once where the police car's meshes ended up (they load a few frames after it spawns).
fn report_car(mut done: Local<bool>, cars: Query<Entity, With<PoliceCar>>, children: Query<&Children>, meshes: Query<&GlobalTransform, With<Mesh3d>>) {
    let Some(car) = cars.iter().next().filter(|_| !*done) else { return };
    let found: Vec<Vec3> = children.iter_descendants(car).filter_map(|e| meshes.get(e).ok()).map(|g| g.translation()).collect();
    if let Some(p) = found.first() {
        *done = true;
        info!("police: the car's {} meshes are in, at ({:.1}, {:.1}, {:.1})", found.len(), p.x, p.y, p.z);
    }
}

/// One of the police car's lights.
#[derive(Component)]
struct SirenLight;

/// Police_C's tick: each light turns 8 degrees of yaw a frame (K2_AddLocalRotation; per frame, as the Blueprint does).
fn turn_lights(mut lights: Query<&mut Transform, With<SirenLight>>) {
    for mut t in &mut lights {
        t.rotate_y(-8f32.to_radians());
    }
}

/// Jason doesn't get into the police's escape volume: he's set back on its edge, toward its InterceptDirection.
fn turn_back_jason(arrived: Option<Res<PoliceArrived>>, zones: Query<(&EscapeZone, &GlobalTransform)>, spawners: Query<&PoliceSpawner>, names: Query<&Name>, mut jasons: Query<&mut Transform, With<crate::jason::Jason>>) {
    let Some(a) = arrived else { return };
    let Ok((zone, zxf)) = zones.get(a.zone) else { return };
    let name = names.get(a.zone).map(|n| n.as_str().to_string()).unwrap_or_default();
    let Some(s) = spawners.iter().find(|s| s.zone == name) else { return };
    let center = zxf.translation();
    for mut j in &mut jasons {
        if j.translation.with_y(0.0).distance(center.with_y(0.0)) >= zone.radius_m {
            continue;
        }
        let away = (s.intercept - center).with_y(0.0).normalize_or(Vec3::X);
        let to = center + away * zone.radius_m;
        info!("police: Jason turned back from the escape");
        j.translation = Vec3::new(to.x, j.translation.y, to.z);
    }
}

/// Escaping on foot: the exit path's points still ahead of her, and the time left until the escape counts.
#[derive(Component)]
pub struct EscapingOnFoot {
    path: Vec<Vec3>,
    left: f32,
}

/// On foot, inside the police's volume: she starts escaping, walks the way out, and escapes after the delay.
#[allow(clippy::type_complexity)]
fn escape_with_police(
    mut commands: Commands,
    time: Res<Time>,
    arrived: Option<Res<PoliceArrived>>,
    escaped: Option<Res<crate::vehicle::Escaped>>,
    zones: Query<(&EscapeZone, &GlobalTransform)>,
    mut player: Single<
        (Entity, &Transform, &mut crate::player::Player, Option<&mut EscapingOnFoot>),
        (Without<crate::vehicle::Driving>, Without<crate::jason::Held>, Without<crate::hiding::Hiding>),
    >,
) {
    let (Some(a), None) = (arrived, escaped) else { return };
    let (pe, xf, ref mut p, ref mut escaping) = *player;
    if let Some(e) = escaping.as_mut() {
        // Stamina stays full while she leaves (bLockStamina).
        p.counselor.stamina = 100.0;
        while e.path.first().is_some_and(|q| q.with_y(0.0).distance(xf.translation.with_y(0.0)) < 1.0) {
            e.path.remove(0);
        }
        let dir = e.path.first().map_or(Vec2::ZERO, |q| Vec2::new(q.x - xf.translation.x, q.z - xf.translation.z));
        commands.entity(pe).insert(crate::player::Autopilot(dir));
        e.left -= time.delta_secs();
        if e.left <= 0.0 {
            info!("escaped with the police at ({:.1}, {:.1}, {:.1}), {} exit points ahead", xf.translation.x, xf.translation.y, xf.translation.z, e.path.len());
            commands.entity(pe).remove::<(EscapingOnFoot, crate::player::Autopilot)>();
            commands.insert_resource(crate::vehicle::Escaped { how: "with the police".into() });
        }
        return;
    }
    let Ok((zone, zxf)) = zones.get(a.zone) else { return };
    let d = xf.translation.distance(zxf.translation());
    if !p.counselor.alive() || d > zone.radius_m {
        return;
    }
    // The path from its point nearest her (in the ground plane) to its end.
    let flat = |q: &Vec3| q.with_y(0.0).distance(xf.translation.with_y(0.0));
    let nearest = zone.exit.iter().enumerate().min_by(|a, b| flat(a.1).total_cmp(&flat(b.1))).map_or(0, |(i, _)| i);
    info!("escaping with the police: {d:.1} m from the volume's centre (radius {:.1}), {} exit points, {:.0} s", zone.radius_m, zone.exit.len() - nearest, zone.delay);
    commands.entity(pe).insert(EscapingOnFoot { path: zone.exit[nearest..].to_vec(), left: zone.delay });
}
