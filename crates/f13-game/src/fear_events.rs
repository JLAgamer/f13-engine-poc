//! The counselor's fear events, pushed and popped where the game does it; f13-core::fear (through f13-bevy's
//! `CounselorFear`) turns them into her fear, which drives her stumbles, stamina and Jason's Sense.
//!
//! From the game:
//! - Each tick (SCCounselorCharacter's tick, `FUN_1403dbd10`): Darkness when she isn't lit (f13-core::light, from
//!   the map's point and spot lights; pushed and popped only when that changes), Wounded, Indoor (her capsule
//!   overlaps a cabin's SCIndoorComponent box), and Jason Near with his 2D distance mapped from
//!   JasonNearMinRange..JasonNearMaxRange to 0..1 (the event's curve input), popped at the max range.
//! - Jason Spotted while he's in her view (`FUN_1402ec1d0`: within her MaxVisibilityDistance, an unblocked line
//!   from the camera to him, and more than half the view's 90 degrees, in cm, ahead along the camera; the game
//!   compares the unnormalised offset, so that's "in front of the camera").
//! - Near Friendly while another counselor (IsFriendly 0x1402e3470: the same side, alive or not) is within her
//!   MaxNearbyDistance (SCCharacter's constructor: 1500 cm) and MaxVisibilityDistance (bFriendlysNearby, FUN_1402ec1d0).
//! - Corpse Spotted when a dead counselor comes into her view (the same sight test as Jason's; OnCharacterInView
//!   FUN_1403c05e0 pushes it and counts, +0x16d4); one going out of view while dead counts down, popping it at 0
//!   (FUN_1403c07e0).
//! - Hidden from going into a hiding spot until she comes out; Grab while Jason holds her.
//! - Each item she picks up pushes its PickedUpFearEvent (weapons: PickedUpWeaponFearEvent); dropping it pops it.
//!
//! Not pushed yet: the custom match option's max fear at the start.

use bevy::prelude::*;
use f13_bevy::{CounselorFear, CounselorStats, F13GameplayPlugin};
use f13_core::fear::{Fear, FearEventData};
use f13_core::light::{capsule_bounds_radius, is_lit, Light};
use f13_data::classes::ClassDb;
use f13_data::level::Xform;
use std::collections::{BTreeMap, HashMap};

use crate::character::{CounselorCharacter, KillerCharacter};
use crate::hiding::{HidePhase, Hiding};
use crate::interact::Inventory;
use crate::jason::{Held, Jason};
use crate::player::{Player, PlayerCamera};
use crate::props::Item;

/// The fear values from tuning.json.
#[derive(Resource, Clone)]
pub struct FearTuning {
    pub update_delay: f32,
    pub fear_modifier: f13_core::stats::StatModifiedValue,
    pub events: BTreeMap<String, FearEventData>,
    /// JasonNearMinRange, JasonNearMaxRange (cm).
    pub jason_near_range: (f32, f32),
}

/// What the map gives the fear events: its lights and indoor volumes (Unreal space, cm).
#[derive(Resource, Default)]
pub struct Surroundings {
    pub lights: Vec<Light>,
    /// Each light's power grid (power.rs): it only lights while that grid is on.
    light_grids: Vec<Option<usize>>,
    /// SCIndoorComponent boxes: transform, half extent, and its owner's match actor index if that's a cabin
    /// (SCCabin).
    indoors: Vec<(Xform, Vec3, Option<usize>)>,
    /// Item class -> its PickedUpFearEvent.
    item_events: HashMap<String, String>,
}

impl Surroundings {
    /// Whether a counselor capsule at `feet` (Bevy space) overlaps an indoor volume.
    pub fn indoors(&self, feet: Vec3) -> bool {
        self.overlapping(feet).next().is_some()
    }

    /// The cabin (match actor index) of the first indoor volume a capsule at `feet` overlaps whose owner is one.
    pub fn cabin_at(&self, feet: Vec3) -> Option<usize> {
        self.overlapping(feet).find_map(|(_, _, cabin)| *cabin)
    }

    fn overlapping(&self, feet: Vec3) -> impl Iterator<Item = &(Xform, Vec3, Option<usize>)> {
        let centre = to_unreal(feet + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M);
        let (r, hh) = (crate::player::CAPSULE_RADIUS_M * 100.0, crate::player::CAPSULE_HALF_HEIGHT_M * 100.0);
        self.indoors.iter().filter(move |(xf, ext, _)| {
            let q = Quat::from_array(xf.r).inverse();
            let local = q * (centre - Vec3::from_array(xf.t));
            let ext = *ext * Vec3::from_array(xf.s);
            local.x.abs() <= ext.x + r && local.y.abs() <= ext.y + r && local.z.abs() <= ext.z + hh
        })
    }
}

/// Bevy (metres, Y up) to Unreal (cm, Z up).
fn to_unreal(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.z, v.y) * 100.0
}

/// What was pushed last, for the events the game pushes once and pops once.
#[derive(Component, Default)]
pub(crate) struct Pushed {
    lit: Option<bool>,
    /// Jason is in her view (JasonSpotted).
    pub(crate) spotted: bool,
    hidden: bool,
    grabbed: bool,
    items: Vec<(Entity, String)>,
    /// The other counselors in her view, and the dead ones counted in (CorpseSpotted, +0x16d4).
    in_view: Vec<Entity>,
    corpses: i32,
}

pub struct FearEventsPlugin {
    /// --fear: fear held at this level, no events.
    pub locked: Option<f32>,
}

#[derive(Resource)]
struct LockedFear(Option<f32>);

impl Plugin for FearEventsPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<F13GameplayPlugin>() {
            app.add_plugins(F13GameplayPlugin);
        }
        app.insert_resource(LockedFear(self.locked))
            .add_systems(Startup, load_surroundings)
            .add_systems(Update, setup)
            .add_systems(FixedUpdate, (perk_fear, push_events).chain().before(crate::player::move_player).run_if(crate::net::simulating));
    }
}

/// Lights, indoor volumes and item fear events from the level and the classes.
fn load_surroundings(mut commands: Commands, level: Res<crate::Level>, export: Res<crate::ExportRoot>) {
    let classes = match ClassDb::load(&export.0.join("classes.json")) {
        Ok(c) => c,
        Err(e) => {
            warn!("fear events: no classes.json ({e}): no lights or indoor volumes");
            commands.insert_resource(Surroundings::default());
            return;
        }
    };
    let mut s = Surroundings::default();
    // Unreal's BoxExtent default is 32.
    for (_, world, props) in crate::lighting::placed_components(&level, &classes, |k| classes.is_child_of(k, "SCIndoorComponent") || k == "SCIndoorComponent") {
        let ext = |k: &str| props.get("BoxExtent").and_then(|e| e.get(k)).and_then(|v| v.as_f64()).map_or(32.0, |v| v as f32);
        let cabin = props.get("ActorClass").and_then(|c| c.as_str()).filter(|c| classes.is_child_of(c, "SCCabin")).and_then(|_| props.get("ActorIndex")?.as_u64());
        s.indoors.push((world, Vec3::new(ext("X"), ext("Y"), ext("Z")), cabin.map(|i| i as usize)));
    }
    let grids = crate::power::PowerGrids::from_level(&level);
    let placed = crate::lighting::placed_lights(&level, &classes);
    let lights: Vec<_> = placed.iter().filter(|l| l.kind != crate::lighting::LightKind::Directional && l.visible()).collect();
    s.lights = lights.iter().map(|l| l.rule_light()).collect();
    s.light_grids = lights.iter().map(|l| l.powered.then(|| l.grid(&grids)).flatten()).collect();
    for (name, c) in &classes.classes {
        if let Some(e) = classes.prop(name, "PickedUpFearEvent").as_str() {
            s.item_events.insert(c.name.clone(), e.trim_end_matches("_C").to_string());
        }
    }
    info!("fear events: {} lights, {} indoor volumes", s.lights.len(), s.indoors.len());
    commands.insert_resource(s);
}

/// Her perks' share of her fear rate (FUN_140331060, over her ActivePerks): FearModifier for active events;
/// HidespotFearModifier while she's in a hiding spot; CounselorsAroundFearModifier, Teamwork's (her TeamworkPerk) x the
/// other living counselors within 20 m (FUN_1403b7ef0, 2000 cm, 3D), Lone Wolf's (her LoneWolfPerk) with none.
fn perk_fear(
    perks: Option<Res<crate::perks::Perks>>,
    mut counselors: Query<(Entity, &Player, &Transform, Option<&crate::perks::CounselorPerks>, Has<Hiding>, &mut f13_bevy::CounselorStats)>,
) {
    let Some(perks) = perks else { return };
    let alive: Vec<(Entity, Vec3)> = counselors.iter().filter(|(_, p, ..)| p.counselor.alive()).map(|(e, _, xf, ..)| (e, xf.translation)).collect();
    for (e, p, xf, picked, hiding, mut s) in &mut counselors {
        let mut always = if hiding { p.counselor.perks.get("HidespotFearModifier") } else { 0.0 };
        let near = alive.iter().filter(|(o, at)| *o != e && at.distance(xf.translation) <= 20.0).count();
        for name in picked.map_or(&[][..], |c| c.0.as_slice()) {
            let m = perks.0.get(name).and_then(|d| d.modifiers.get("CounselorsAroundFearModifier")).copied().unwrap_or(0.0);
            match name.as_str() {
                "TeamWork" => always += near as f32 * m,
                "LoneWolf" if near == 0 => always += m,
                _ => {}
            }
        }
        let active = p.counselor.perks.get("FearModifier");
        // Her DarkFearEvent's and CorpseSpottedFearEvent's caps (FUN_140330cb0).
        let caps = vec![("DarknessFearEvent".to_string(), p.counselor.perks.get("DarknessFearModifier")), ("CorpseSpottedFearEvent".to_string(), p.counselor.perks.get("DeadBodyFearModifier"))];
        if s.perk_fear_rate != always || s.perk_fear_active != active || s.perk_caps != caps {
            s.perk_fear_rate = always;
            s.perk_fear_active = active;
            s.perk_caps = caps;
        }
    }
}

/// Gives the counselor her fear component once she's spawned.
fn setup(
    mut commands: Commands,
    tuning: Option<Res<FearTuning>>,
    player_tuning: Option<Res<crate::player::PlayerTuning>>,
    locked: Res<LockedFear>,
    players: Query<(Entity, &Player), Without<CounselorFear>>,
) {
    let (Some(t), Some(pt)) = (tuning, player_tuning) else { return };
    for (e, p) in &players {
        let mut fear = Fear::new(t.update_delay);
        if let Some(v) = locked.0 {
            fear.lock_to(v, true);
        } else {
            fear.lock_to(p.fear, false);
        }
        commands.entity(e).insert((
            CounselorFear(fear),
            CounselorStats { stats: p.counselor.stats, fear_modifier: t.fear_modifier.clone(), perk_fear_rate: 0.0, perk_fear_active: 0.0, perk_caps: Vec::new() },
            Pushed::default(),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn push_events(
    tuning: Option<Res<FearTuning>>,
    surroundings: Option<Res<Surroundings>>,
    (counselor, killer): (Option<Res<CounselorCharacter>>, Option<Res<KillerCharacter>>),
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
    mut players: Query<(Entity, &mut Player, &Transform, &mut CounselorFear, &mut Pushed, Option<&Inventory>, Option<&Hiding>, Has<Held>, Option<&crate::lighting::Flashlight>, Has<crate::controls::LocalPlayer>), Without<Jason>>,
    pads: crate::controls::Pads,
    jasons: Query<&Transform, (With<Jason>, Without<Player>)>,
    camera: Query<&Transform, (With<PlayerCamera>, Without<Player>, Without<Jason>)>,
    items: Query<&Item>,
    (time, mut next_log, power): (Res<Time>, Local<f32>, Option<Res<crate::power::PowerGrids>>),
) {
    let (Some(t), Some(s)) = (tuning, surroundings) else { return };
    // Every counselor: where her capsule's centre is, and whether she's alive (for the others' tests).
    let counselors: Vec<(Entity, Vec3, bool)> = players.iter().map(|(e, p, xf, ..)| (e, xf.translation + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M, p.counselor.alive())).collect();
    let log = time.elapsed_secs() >= *next_log;
    if log {
        *next_log = time.elapsed_secs() + 5.0;
    }
    for (pe, mut p, xf, mut fear, mut pushed, inventory, hiding, held, flashlight, local) in &mut players {
    p.fear = fear.0.fear;
    if fear.0.locked {
        continue;
    }
    if log {
        let active: Vec<&str> = fear.0.events.iter().filter(|e| e.state.active).map(|e| e.key.trim_end_matches("FearEvent")).collect();
        debug!("fear {:.1} ({}) at {:.1}", fear.0.fear, active.join(", "), xf.translation);
    }
    let fear = &mut fear.0;
    let mut set = |name: &str, on: bool, distance: f32| {
        let Some(d) = t.events.get(name) else { return };
        if on {
            fear.push(name.to_string(), d, distance);
        } else {
            fear.pop(name.to_string(), d);
        }
    };
    let cv = |name: &str, d: f32| counselor.as_ref().and_then(|c| c.info.values.get(name).copied()).unwrap_or(d);
    let (r, hh) = (crate::player::CAPSULE_RADIUS_M * 100.0, crate::player::CAPSULE_HALF_HEIGHT_M * 100.0);
    let centre = xf.translation + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M;

    // Her tick, in the game's order.
    // The lights whose power is on.
    let on: Vec<Light> = s.lights.iter().zip(&s.light_grids).filter(|(_, g)| g.is_none_or(|g| power.as_ref().is_none_or(|p| p.on(g)))).map(|(l, _)| *l).collect();
    let lit = flashlight.is_some_and(|f| f.on) || is_lit(&on, to_unreal(centre), capsule_bounds_radius(r, hh), cv("MinLightStrength", 0.3));
    if pushed.lit != Some(lit) {
        debug!("fear: {}", if lit { "in the light" } else { "in the dark" });
        set("DarknessFearEvent", !lit, 0.0);
        pushed.lit = Some(lit);
    }
    set("WoundedFearEvent", p.counselor.wounded, 0.0);
    set("IndoorFearEvent", s.indoors(xf.translation), 0.0);
    // In water: the water volume at her (CurrentWaterVolume; she swims there).
    set("InWaterFearEvent", p.counselor.stance == f13_core::counselor::Stance::Swimming, 0.0);
    let visibility_cm = cv("MaxVisibilityDistance", 6000.0);
    let near = counselors.iter().any(|(e, c, _)| *e != pe && c.distance(centre) * 100.0 < MAX_NEARBY_DISTANCE_CM.min(visibility_cm));
    set("NearFriendlyFearEvent", near, 0.0);
    // A blackout: inside a power grid's volume while its power is out (SCPowerGridVolume FUN_140422a30 pushes its fear
    // event on the counselors overlapping it as the power goes, pops it as it comes back; the event, unset in the
    // map, taken as BlackoutFearEvent).
    let blackout = power.as_ref().is_some_and(|p| p.grid_at(to_unreal(centre)).is_some_and(|g| !p.on(g)));
    set("BlackoutFearEvent", blackout, 0.0);
    let jason = jasons.iter().next();
    if let Some(j) = jason {
        let (min, max) = t.jason_near_range;
        let d = Vec2::new(j.translation.x - xf.translation.x, j.translation.z - xf.translation.z).length() * 100.0;
        set("JasonNearFearEvent", d < max, ((d - min) / (max - min)).clamp(0.0, 1.0));
    }

    // Jason in her view.
    // Her view: the local counselor's camera; another's control rotation from her eyes.
    let view = if local {
        camera.iter().next().map(|c| (c.translation, c.forward().as_vec3()))
    } else {
        let (yaw, pitch) = pads.look(pe);
        Some((centre + Vec3::Y * crate::interact::BASE_EYE_HEIGHT_M, PlayerCamera { yaw, pitch }.look()))
    };
    let in_view = jason.zip(view).is_some_and(|(j, (eye, forward))| {
        let jason_hh = killer.as_ref().map_or(90.0, |k| k.info.capsule_half_height) / 100.0;
        let target = j.translation + Vec3::Y * jason_hh;
        let ahead = (target - eye).dot(forward) * 100.0;
        target.distance(centre) * 100.0 < cv("MaxVisibilityDistance", 6000.0)
            && ahead >= VIEW_FOV_DEG * 0.5
            && f13_core::collision::segment_hit(&collision.with(&dynamic), eye, target, 4.0).is_none()
    });
    if in_view != pushed.spotted {
        debug!("fear: Jason {} view", if in_view { "in" } else { "out of" });
        set("JasonSpottedFearEvent", in_view, 0.0);
        pushed.spotted = in_view;
    }
    // The other counselors in her view: a dead one coming in is a corpse spotted; one going out while dead counts
    // down.
    for &(e, c, alive) in counselors.iter().filter(|(e, ..)| *e != pe) {
        let seen = view.is_some_and(|(eye, forward)| {
            c.distance(centre) * 100.0 < visibility_cm && (c - eye).dot(forward) * 100.0 >= VIEW_FOV_DEG * 0.5 && f13_core::collision::segment_hit(&collision.with(&dynamic), eye, c, 4.0).is_none()
        });
        let was = pushed.in_view.contains(&e);
        if seen && !was {
            pushed.in_view.push(e);
            if !alive {
                debug!("fear: a dead counselor in view");
                set("CorpseSpottedFearEvent", true, 0.0);
                pushed.corpses += 1;
            }
        } else if !seen && was {
            pushed.in_view.retain(|x| *x != e);
            if !alive {
                pushed.corpses -= 1;
                if pushed.corpses == 0 {
                    set("CorpseSpottedFearEvent", false, 0.0);
                }
            }
        }
    }

    let hidden = hiding.is_some_and(|h| h.phase != HidePhase::Exiting);
    if hidden != pushed.hidden {
        set("HiddenFearEvent", hidden, 0.0);
        pushed.hidden = hidden;
    }
    if held != pushed.grabbed {
        set("GrabFearEvent", held, 0.0);
        pushed.grabbed = held;
    }

    // Items: picked up pushes, dropped (or used up) pops.
    let carried: Vec<Entity> = inventory.map_or(vec![], |i| i.hand.iter().chain(&i.pockets).copied().collect());
    for (e, event) in std::mem::take(&mut pushed.items) {
        if carried.contains(&e) {
            pushed.items.push((e, event));
        } else {
            set(&event, false, 0.0);
        }
    }
    for e in carried {
        if pushed.items.iter().any(|(i, _)| *i == e) {
            continue;
        }
        let Some(event) = items.get(e).ok().and_then(|i| s.item_events.get(&i.class)) else { continue };
        set(event, true, 0.0);
        pushed.items.push((e, event.clone()));
    }
    }
}

/// SCCharacter's MaxNearbyDistance (its constructor; no Blueprint changes it), cm.
const MAX_NEARBY_DISTANCE_CM: f32 = 1500.0;

/// The view's field of view the game passes to its sight test (a constant for the counselor's view).
const VIEW_FOV_DEG: f32 = 90.0;
