//! What Jason knows about the counselor (f13-core::perception): what he sees, the sound blips her noise puts on his
//! screen, and Sense. jason.rs decides what to do with it.
//!
//! From the game: sight reaches her `MaxVisibilityDistance` (Base_Counselor: 100 m) along an unblocked line
//! (`SCCharacter::LineOfSight`); blips, Sense radii and timings are the killer's and the counselor's values (see
//! f13-core::perception); his abilities unlock and recharge as f13-core::killer_abilities (Sense second, 46 s in).
//!
//! AI choices (the game's Jason is a player; his AI was cut): he sees within 62 degrees of where he faces (the cut
//! AI's SCJasonAIProperties PeripheralVisionAngle) or anywhere within 3 m, from his eyes (APawn's BaseEyeHeight, 64 cm
//! above his capsule's centre, which no Jason Blueprint overrides: 1.52 m) to her head or chest, through the world's
//! blocking collision only (foliage and darkness don't hide her). He uses Sense whenever it's ready and he can't see
//! her. PLACEHOLDER: indoors (where Sense shows her cabin, not her) without the cabins' SCIndoorComponent volumes
//! (fear_events.rs) is something above her head within 6 m.

use bevy::prelude::*;
use f13_core::killer_abilities::{Ability, AbilityEvent, Blockers, KillerAbilities, KillerTuning};
use f13_core::perception::{self, HearingTuning, NoiseTuning, SenseTuning};

use crate::character::{CounselorCharacter, KillerCharacter};
use crate::hiding::{HidePhase, Hiding};
use crate::jason::{Jason, JasonState};
use crate::player::Player;

/// His capsule's half height (88 cm) plus BaseEyeHeight (64 cm).
const EYE_M: f32 = 1.52;
/// SCJasonAIProperties' PeripheralVisionAngle (its constructor).
const FIELD_OF_VIEW_DEG: f32 = 62.0;
const NOTICE_M: f32 = 3.0;
/// Sight checks per second (ray casts over up to 100 m).
const SIGHT_RATE: f32 = 10.0;
const INDOORS_ROOF_M: f32 = 6.0;

/// Jason's ability timings (tuning.json).
#[derive(Resource, Clone)]
pub struct KillerAbilityTuning(pub KillerTuning);

#[derive(Resource)]
struct SenseValues {
    noise: NoiseTuning,
    hearing: HearingTuning,
    sense: SenseTuning,
    /// cm
    max_visibility: f32,
}

/// How he learnt where she is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clue {
    Seen,
    Heard,
    Sensed,
    /// Sense lit up the cabin she's in.
    SensedIndoors,
}

#[derive(Component)]
pub struct Knowledge {
    /// She's in sight this tick.
    pub sees: bool,
    /// Where he last learnt she was, when (game time) and how.
    pub last_known: Option<(Vec3, f32, Clue)>,
    /// The hiding spot he watched her get into (or Sense showed her in).
    pub hiding_spot: Option<Entity>,
    pub abilities: KillerAbilities,
    /// Sense is on until this game time.
    pub sense_until: f32,
    /// What Sense showed last (logged when it changes).
    sense_shows: Option<Clue>,
    next_blip: f32,
    next_sight: f32,
}

impl Knowledge {
    pub fn sensing(&self, now: f32) -> bool {
        now < self.sense_until
    }
}

pub struct SensesPlugin;

impl Plugin for SensesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, setup).add_systems(FixedUpdate, perceive.before(crate::jason::update_jason));
    }
}

/// Once the characters are loaded: the values, and Jason's knowledge.
fn setup(
    mut commands: Commands,
    counselor: Option<Res<CounselorCharacter>>,
    killer: Option<Res<KillerCharacter>>,
    abilities: Option<Res<KillerAbilityTuning>>,
    values: Option<Res<SenseValues>>,
    jasons: Query<Entity, (With<Jason>, Without<Knowledge>)>,
) {
    let (Some(c), Some(k), Some(a)) = (counselor, killer, abilities) else { return };
    if values.is_none() {
        let cv = |n: &str, d: f32| c.info.values.get(n).copied().unwrap_or(d);
        let kv = |n: &str, d: f32| k.info.values.get(n).copied().unwrap_or(d);
        if c.info.stat("NoiseStatMod").is_none() || k.info.curve("SenseFearRadiusCurve").is_none() {
            warn!("the export has no noise or Sense values (re-export with tools/f13-export counselor and killer): using constructor defaults");
        }
        let flat = |v: f32| f13_core::curve::RichCurve { keys: vec![f13_core::curve::Key { time: 0.0, value: v, interp: f13_core::curve::Interp::Constant, arrive_tangent: 0.0, leave_tangent: 0.0 }] };
        commands.insert_resource(SenseValues {
            noise: NoiseTuning {
                crouching: cv("CrouchingNoiseMod", 1.0),
                walking: cv("WalkingNoiseMod", 2.0),
                running: cv("RunningNoiseMod", 3.0),
                sprinting: cv("SprintingNoiseMod", 4.0),
                swimming: cv("SwimmingNoiseMod", 10.0),
                stat_mod: c.info.stat("NoiseStatMod").unwrap_or(f13_core::stats::StatModifiedValue::constant(1.0)),
            },
            hearing: HearingTuning {
                min_blip_radius: kv("MinBlipRadius", 1000.0),
                noise_level_threshold: kv("NoiseLevelThreshold", 2.0),
                noise_distance_threshold: kv("NoiseDistanceThreshold", 50000.0),
                min_blip_frequency: kv("MinBlipFrequencyTime", 0.5),
                max_blip_frequency: kv("MaxBlipFrequencyTime", 2.0),
                blip_life: kv("SoundBlipLifeSpan", 6.0),
            },
            sense: SenseTuning {
                duration: kv("SenseDuration", 15.0),
                fear_radius: k.info.curve("SenseFearRadiusCurve").unwrap_or_else(|| flat(5000.0)),
                fear_hiding_radius: k.info.curve("SenseFearHidingRadiusCurve").unwrap_or_else(|| flat(5000.0)),
                rage_distance: k.info.curve("RageSenseDistanceCurve").unwrap_or_else(|| flat(1.0)),
                raged_distance_modifier: kv("RagedSenseDistanceModifier", 1.0),
                rage_unlock_time: kv("RageUnlockTime", a.0.rage_unlock_time),
            },
            max_visibility: cv("MaxVisibilityDistance", 6000.0),
        });
    }
    for j in &jasons {
        commands.entity(j).insert(Knowledge {
            sees: false,
            last_known: None,
            hiding_spot: None,
            abilities: KillerAbilities::new(a.0.clone(), None),
            sense_until: 0.0,
            sense_shows: None,
            next_blip: 0.0,
            next_sight: 0.0,
        });
    }
}

/// Whether anything blocks the line between two points.
fn blocked(world: &(impl f13_core::collision::TriSource + ?Sized), a: Vec3, b: Vec3) -> bool {
    f13_core::collision::segment_hit(world, a, b, 4.0).is_some()
}

#[allow(clippy::too_many_arguments)]
fn perceive(
    mut commands: Commands,
    time: Res<Time>,
    values: Option<Res<SenseValues>>,
    mut rng: ResMut<crate::GameRng>,
    (collision, dynamic): (Res<crate::collision::WorldCollision>, Res<crate::collision::DynamicColliders>),
    mut jasons: Query<(&Jason, &Transform, &mut Knowledge)>,
    players: Query<(&Player, &Transform, Option<&Hiding>), Without<Jason>>,
    surroundings: Option<Res<crate::fear_events::Surroundings>>,
    mut controls: Option<ResMut<crate::jason_player::JasonControls>>,
) {
    let Some(v) = values else { return };
    // Played, he uses Sense when asked (4); the computer Jason when it's ready and she's out of sight.
    let asked = controls.as_mut().map(|c| std::mem::take(&mut c.sense));
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let world = collision.with(&dynamic);
    let Ok((p, pxf, hiding)) = players.single() else { return };
    for (j, jxf, mut k) in &mut jasons {
        // Abilities run from the start of the match.
        let blockers = Blockers { stunned: matches!(j.state, JasonState::Stunned { .. }), ..default() };
        let mut events = Vec::new();
        k.abilities.update(dt, true, &blockers, &mut events);
        for e in events {
            match e {
                AbilityEvent::Unlocked(a) => {
                    info!("Jason: {a:?} unlocked");
                    // Counselors hear it (their music object's JasonAbilityUnlockWarning).
                    commands.trigger(crate::audio::MusicStinger("JasonAbilityUnlockWarning"));
                }
                AbilityEvent::RageActivated => info!("Jason: rage"),
                AbilityEvent::RequestRageFromServer => {}
            }
        }
        if matches!(j.state, JasonState::Waiting(_) | JasonState::Done) || !p.counselor.alive() {
            k.sees = false;
            continue;
        }
        let hidden = hiding.is_some_and(|h| matches!(h.phase, HidePhase::Hidden | HidePhase::Killed));
        let her = pxf.translation;
        let flat = Vec2::new(her.x - jxf.translation.x, her.z - jxf.translation.z);
        let dist = jxf.translation.distance(her);

        // Sight.
        if now >= k.next_sight {
            k.next_sight = now + 1.0 / SIGHT_RATE;
            let was = k.sees;
            let yaw = jxf.rotation.to_euler(EulerRot::YXZ).0;
            let facing = Vec2::new(yaw.sin(), yaw.cos());
            let in_view = dist < NOTICE_M || flat.normalize_or_zero().dot(facing) >= FIELD_OF_VIEW_DEG.to_radians().cos();
            let eye = jxf.translation + Vec3::Y * EYE_M;
            k.sees = !hidden
                && dist * 100.0 < v.max_visibility
                && in_view
                && [1.0, 1.6].iter().any(|&h| !blocked(&world, eye, her + Vec3::Y * h));
            if k.sees != was {
                info!("Jason: {} Vanessa ({dist:.0} m)", if k.sees { "sees" } else { "lost sight of" });
            }
            if k.sees {
                // Watching her get into a hiding spot gives it away.
                if let Some(h) = hiding.filter(|h| h.phase == HidePhase::Entering) {
                    if k.hiding_spot != Some(h.spot) {
                        info!("Jason: saw her hide");
                    }
                    k.hiding_spot = Some(h.spot);
                }
            }
        }
        if k.sees {
            k.last_known = Some((her, now, Clue::Seen));
            if hiding.is_none() {
                k.hiding_spot = None;
            }
        }

        // Hearing: her noise puts a blip on his screen.
        if now >= k.next_blip {
            let moving = !hidden && p.velocity.length_squared() > 1.0;
            let c = &p.counselor;
            let noise = perception::noise_level(&v.noise, &c.stats, c.stance, c.sprinting, c.running, moving);
            if perception::makes_blip(&v.hearing, noise, dist * 100.0, k.abilities.rage_active) {
                k.next_blip = now + perception::next_blip_delay(&v.hearing, rng.0.frand());
                if !k.sees {
                    debug!("Jason: heard her ({dist:.0} m, noise {noise:.1})");
                    k.last_known = Some((her, now, Clue::Heard));
                }
            }
        }

        // Sense: used when ready and she's out of sight.
        if asked.unwrap_or(!k.sees) && !k.sensing(now) && k.abilities.is_available(Ability::Sense, &blockers) {
            k.abilities.consume(Ability::Sense);
            k.sense_until = now + v.sense.duration;
            info!("Jason: Sense ({:.0} s)", v.sense.duration);
        }
        let mut shows = None;
        if k.sensing(now) {
            let radius = perception::sense_radius(&v.sense, p.fear, hidden, k.abilities.rage_active, k.abilities.rage_timer) / 100.0;
            if flat.length() <= radius {
                let indoors = surroundings.as_ref().map_or_else(
                    || f13_core::collision::segment_hit(&world, her + Vec3::Y * 1.7, her + Vec3::Y * INDOORS_ROOF_M, 2.0).is_some(),
                    |s| s.indoors(her),
                );
                let clue = if indoors { Clue::SensedIndoors } else { Clue::Sensed };
                shows = Some(clue);
                if k.sense_shows != shows {
                    info!("Jason: Sense shows her ({}, {:.0} m, radius {radius:.0} m)", if indoors { "in a cabin" } else { "outside" }, flat.length());
                }
                if !k.sees {
                    k.last_known = Some((her, now, clue));
                }
                if let Some(h) = hiding.filter(|_| hidden) {
                    k.hiding_spot = Some(h.spot);
                }
            }
        }
        k.sense_shows = shows;
    }
}
