//! Loads `tuning.json` (made by `tools/extract_tuning.py` from the player's own install) into f13-core types.

pub mod character;
pub mod classes;
pub mod collision;
pub mod kill_camera;
pub mod level;
pub mod sound;
pub mod umg;
pub mod vehicle;
mod native_classes;
pub mod world;

use f13_core::counselor::CounselorTuning;
use f13_core::curve::{Interp, Key, RichCurve};
use f13_core::fear::FearEventData;
use f13_core::game_mode::{MatchRules, ScoreEvent};
use f13_core::health::HealthTuning;
use f13_core::killer_abilities::KillerTuning;
use f13_core::movement::MovementSettings;
use f13_core::stats::{StatModifiedValue, Stats, STAT_COUNT};
use f13_core::dodge::DodgeTuning;
use f13_core::minigame::MinigameTuning;
use f13_core::stumble::StumbleTuning;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Deserialize)]
struct File {
    killer: KillerSection,
    fear: FearSection,
    counselor: CounselorSection,
    items: ItemsSection,
    vehicle: VehicleJson,
    #[serde(rename = "match")]
    match_: MatchSection,
}

#[derive(Deserialize)]
struct MatchSection {
    round_time: i32,
    post_match_outro_wait_time: i32,
    post_match_wait_time: i32,
    score_events: BTreeMap<String, ScoreEventJson>,
}

#[derive(Deserialize)]
struct ScoreEventJson {
    experience: i32,
    category: String,
    only_once: bool,
    all_counselors: bool,
    description: String,
}

#[derive(Deserialize)]
struct VehicleJson {
    min_restart_time: f32,
    max_restart_time: f32,
    restart_time_mod: StatJson,
}

#[derive(Deserialize)]
struct ItemsSection {
    first_aid: FirstAidJson,
}

#[derive(Deserialize)]
struct FirstAidJson {
    health_restored: f32,
    use_length: f32,
}

#[derive(Deserialize)]
struct HealthJson {
    max_health: f32,
    blocking_damage_mod: StatJson,
}

#[derive(Deserialize)]
struct CounselorSection {
    #[serde(rename = "Base_Counselor")]
    base: CounselorJson,
    characters: BTreeMap<String, CharacterJson>,
}

#[derive(Deserialize)]
struct CharacterJson {
    stats: Stats,
}

#[derive(Deserialize)]
struct CounselorJson {
    max_walk_speed: f32,
    max_walk_speed_crouched: f32,
    max_slow_run_speed: f32,
    max_run_speed: f32,
    max_sprint_speed: f32,
    max_combat_speed: f32,
    max_swim_speed: f32,
    max_acceleration: f32,
    dodging_speed_mod: f32,
    stumbling_speed_mod: f32,
    water_speed_mod: f32,
    speed_stat_mod: StatJson,
    stamina_max: f32,
    stamina_min_threshold: f32,
    stamina_deplete_rate: f32,
    stamina_refill_rate: StatJson,
    stamina_idle_modifier: f32,
    stamina_hiding_modifier: f32,
    stamina_walking_modifier: f32,
    stamina_running_modifier: f32,
    stamina_sprinting_modifier: f32,
    stamina_swimming_modifier: f32,
    stamina_crouching_modifier: f32,
    stamina_stat_mod: StatJson,
    grabbed_stamina_refill: StatJson,
    stamina_fear_curve: Option<String>,
    force_stumble_on_stamina_depletion: bool,
    wounded_threshold: f32,
    jason_near_min_range: f32,
    jason_near_max_range: f32,
    health: HealthJson,
    stumble: StumbleJson,
    dodge: DodgeJson,
    minigame: MinigameJson,
}

#[derive(Deserialize)]
struct MinigameJson {
    min_hold_time: f32,
    repair_hold_time: f32,
    allowable_fails: f32,
    max_pip_count: i32,
    min_pip_count: i32,
    max_pip_width: f32,
    min_pip_width: f32,
    max_pip_count_mod: StatJson,
    min_pip_count_mod: StatJson,
    max_pip_width_mod: StatJson,
    min_pip_width_mod: StatJson,
    speed_mod: StatJson,
}

#[derive(Deserialize)]
struct StumbleJson {
    fear_frequency_curve: Option<String>,
    stamina_frequency_curve: Option<String>,
    fear_chance_curve: Option<String>,
    stamina_chance_curve: Option<String>,
    stat_mod: StatJson,
    wounded_chance: f32,
    cooldown: f32,
    sprint: MontageJson,
    run: MontageJson,
    walk: MontageJson,
    out_of_stamina: MontageJson,
}

#[derive(Deserialize)]
struct DodgeJson {
    stamina_cost: StatJson,
    left: MontageJson,
    right: MontageJson,
    back: MontageJson,
}

#[derive(Deserialize)]
struct MontageJson {
    name: Option<String>,
    length: f32,
}

impl MontageJson {
    fn montage(self) -> f13_core::Montage {
        f13_core::Montage { name: self.name.unwrap_or_default(), length: self.length }
    }
}

#[derive(Deserialize)]
struct KillerSection {
    #[serde(rename = "Jason_Base")]
    jason_base: KillerJson,
    stalk_active_time: f32,
}

#[derive(Deserialize)]
struct KillerJson {
    unlock_time: [f32; 4],
    recharge_time: [f32; 4],
    raged_recharge_modifier: f32,
    rage_unlock_time: f32,
}

#[derive(Deserialize)]
struct FearSection {
    update_fear_delay: f32,
    fear_modifier: StatJson,
    events: BTreeMap<String, FearEventJson>,
    curves: BTreeMap<String, Vec<KeyJson>>,
}

#[derive(Deserialize)]
struct StatJson {
    base: f32,
    modifiers: [f32; STAT_COUNT],
}

#[derive(Deserialize)]
struct FearEventJson {
    increase: f32,
    decrease: f32,
    increase_delay: f32,
    decrease_delay: f32,
    max_fear_cap: f32,
    decrease_when_active: bool,
    auto_deactivate_time: f32,
    distance_curve: Option<String>,
}

#[derive(Deserialize)]
struct KeyJson {
    time: f32,
    value: f32,
    interp: String,
    arrive_tangent: f32,
    leave_tangent: f32,
}

/// Engine start (restart) time of the car: `random[min, max] * restart_time_mod(stats, inverted)` seconds of holding.
#[derive(Debug, Clone)]
pub struct VehicleStart {
    pub min_restart_time: f32,
    pub max_restart_time: f32,
    pub restart_time_mod: StatModifiedValue,
}

#[derive(Debug, Clone, Copy)]
pub struct FirstAid {
    pub health_restored: f32,
    /// Seconds the use animation takes; the heal lands when it ends.
    pub use_length: f32,
}

/// Everything the gameplay rules need, ready to use.
#[derive(Debug, Clone)]
pub struct Tuning {
    pub jason: KillerTuning,
    pub stalk_active_time: f32,
    pub update_fear_delay: f32,
    pub fear_modifier: StatModifiedValue,
    pub fear_events: BTreeMap<String, FearEventData>,
    pub curves: BTreeMap<String, RichCurve>,
    pub counselor: CounselorTuning,
    pub counselor_movement: MovementSettings,
    /// "Jason near" fear event distance range (min, max), in cm.
    pub jason_near_range: (f32, f32),
    pub first_aid: FirstAid,
    pub vehicle_start: VehicleStart,
    pub minigame: MinigameTuning,
    /// `SCRepairComponent` default hold time (s); a repair point may override it.
    pub repair_hold_time: f32,
    /// Playable counselors by name (e.g. "Vanessa") -> stats.
    pub characters: BTreeMap<String, Stats>,
    pub match_rules: MatchRules,
    /// Score events by class name without `_C` (e.g. "Escape").
    pub score_events: BTreeMap<String, ScoreEvent>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Format(u32),
    MissingCurve(String),
    /// An exported file that doesn't parse (e.g. an `.anim` from another exporter version).
    Invalid(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "reading tuning file: {e} (run tools/extract_tuning.py first)"),
            LoadError::Json(e) => write!(f, "parsing tuning file: {e}"),
            LoadError::Format(v) => {
                write!(f, "tuning file has format {v}, this build needs {FORMAT}: run tools/extract_tuning.py again")
            }
            LoadError::MissingCurve(c) => write!(f, "fear event references missing curve {c}"),
            LoadError::Invalid(what) => write!(f, "{what} (re-export it)"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Default location written by the extractor: `<home>/.f13-rs/tuning.json`.
/// (Not `%APPDATA%`: packaged Windows apps get their AppData writes redirected, hiding the file from other programs.)
pub fn default_path() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".f13-rs").join("tuning.json")
}

/// tuning.json format this build reads; older files must be re-extracted.
pub const FORMAT: u32 = 8;

pub fn load(path: &Path) -> Result<Tuning, LoadError> {
    parse(&std::fs::read_to_string(path).map_err(LoadError::Io)?)
}

pub fn parse(text: &str) -> Result<Tuning, LoadError> {
    // Check the format first so an old file says "re-extract" instead of naming a missing field.
    #[derive(Deserialize)]
    struct Header {
        format: u32,
    }
    let header: Header = serde_json::from_str(text).map_err(LoadError::Json)?;
    if header.format != FORMAT {
        return Err(LoadError::Format(header.format));
    }
    let f: File = serde_json::from_str(text).map_err(LoadError::Json)?;
    let curves: BTreeMap<String, RichCurve> = f
        .fear
        .curves
        .into_iter()
        .map(|(name, keys)| {
            let keys = keys
                .into_iter()
                .map(|k| Key {
                    time: k.time,
                    value: k.value,
                    interp: match k.interp.as_str() {
                        "linear" => Interp::Linear,
                        "constant" => Interp::Constant,
                        _ => Interp::Cubic,
                    },
                    arrive_tangent: k.arrive_tangent,
                    leave_tangent: k.leave_tangent,
                })
                .collect();
            (name, RichCurve { keys })
        })
        .collect();
    let curve_fn = |name: &Option<String>| -> Result<Option<f13_core::fear::Curve>, LoadError> {
        match name {
            None => Ok(None),
            Some(c) => {
                let curve = curves.get(c).ok_or_else(|| LoadError::MissingCurve(c.clone()))?.clone();
                Ok(Some(Arc::new(move |x: f32| curve.eval(x)) as f13_core::fear::Curve))
            }
        }
    };
    let mut fear_events = BTreeMap::new();
    for (name, e) in f.fear.events {
        let distance_curve = curve_fn(&e.distance_curve)?;
        fear_events.insert(
            name,
            FearEventData {
                increase: e.increase,
                decrease: e.decrease,
                increase_delay: e.increase_delay,
                decrease_delay: e.decrease_delay,
                max_fear_cap: e.max_fear_cap,
                decrease_when_active: e.decrease_when_active,
                auto_deactivate_time: e.auto_deactivate_time,
                distance_curve,
            },
        );
    }
    let smv = |s: StatJson| StatModifiedValue { base: s.base, modifiers: s.modifiers };
    let c = f.counselor.base;
    let st = c.stumble;
    let stumble = StumbleTuning {
        fear_frequency: curve_fn(&st.fear_frequency_curve)?,
        stamina_frequency: curve_fn(&st.stamina_frequency_curve)?,
        fear_chance: curve_fn(&st.fear_chance_curve)?,
        stamina_chance: curve_fn(&st.stamina_chance_curve)?,
        stat_mod: smv(st.stat_mod),
        wounded_chance: st.wounded_chance,
        cooldown: st.cooldown,
        sprint: st.sprint.montage(),
        run: st.run.montage(),
        walk: st.walk.montage(),
        out_of_stamina: st.out_of_stamina.montage(),
    };
    let mg = c.minigame;
    let repair_hold_time = mg.repair_hold_time;
    let minigame = MinigameTuning {
        min_hold_time: mg.min_hold_time,
        max_pip_count: mg.max_pip_count,
        min_pip_count: mg.min_pip_count,
        max_pip_width: mg.max_pip_width,
        min_pip_width: mg.min_pip_width,
        max_pip_count_mod: smv(mg.max_pip_count_mod),
        min_pip_count_mod: smv(mg.min_pip_count_mod),
        max_pip_width_mod: smv(mg.max_pip_width_mod),
        min_pip_width_mod: smv(mg.min_pip_width_mod),
        speed_mod: smv(mg.speed_mod),
        allowable_fails: mg.allowable_fails,
    };
    let dg = c.dodge;
    let dodge = DodgeTuning {
        stamina_cost: smv(dg.stamina_cost),
        left: dg.left.montage(),
        right: dg.right.montage(),
        back: dg.back.montage(),
    };
    let counselor = CounselorTuning {
        max_walk_speed: c.max_walk_speed,
        max_walk_speed_crouched: c.max_walk_speed_crouched,
        max_slow_run_speed: c.max_slow_run_speed,
        max_run_speed: c.max_run_speed,
        max_sprint_speed: c.max_sprint_speed,
        max_combat_speed: c.max_combat_speed,
        max_swim_speed: c.max_swim_speed,
        dodging_speed_mod: c.dodging_speed_mod,
        stumbling_speed_mod: c.stumbling_speed_mod,
        water_speed_mod: c.water_speed_mod,
        speed_stat_mod: smv(c.speed_stat_mod),
        stamina_max: c.stamina_max,
        stamina_min_threshold: c.stamina_min_threshold,
        stamina_deplete_rate: c.stamina_deplete_rate,
        stamina_refill_rate: smv(c.stamina_refill_rate),
        stamina_idle_modifier: c.stamina_idle_modifier,
        stamina_hiding_modifier: c.stamina_hiding_modifier,
        stamina_walking_modifier: c.stamina_walking_modifier,
        stamina_running_modifier: c.stamina_running_modifier,
        stamina_sprinting_modifier: c.stamina_sprinting_modifier,
        stamina_swimming_modifier: c.stamina_swimming_modifier,
        stamina_crouching_modifier: c.stamina_crouching_modifier,
        stamina_stat_mod: smv(c.stamina_stat_mod),
        grabbed_stamina_refill: smv(c.grabbed_stamina_refill),
        stamina_fear_curve: curve_fn(&c.stamina_fear_curve)?,
        force_stumble_on_stamina_depletion: c.force_stumble_on_stamina_depletion,
        wounded_threshold: c.wounded_threshold,
        health: HealthTuning { max_health: c.health.max_health, blocking_damage_mod: smv(c.health.blocking_damage_mod) },
        stumble,
        dodge,
    };
    let characters = f.counselor.characters.into_iter().map(|(k, v)| (k, v.stats)).collect();
    let j = f.killer.jason_base;
    Ok(Tuning {
        jason: KillerTuning {
            unlock_time: j.unlock_time,
            recharge_time: j.recharge_time,
            raged_recharge_modifier: j.raged_recharge_modifier,
            rage_unlock_time: j.rage_unlock_time,
        },
        stalk_active_time: f.killer.stalk_active_time,
        update_fear_delay: f.fear.update_fear_delay,
        fear_modifier: smv(f.fear.fear_modifier),
        fear_events,
        curves,
        counselor,
        counselor_movement: MovementSettings { max_acceleration: c.max_acceleration, ..Default::default() },
        jason_near_range: (c.jason_near_min_range, c.jason_near_max_range),
        minigame,
        repair_hold_time,
        vehicle_start: VehicleStart {
            min_restart_time: f.vehicle.min_restart_time,
            max_restart_time: f.vehicle.max_restart_time,
            restart_time_mod: smv(f.vehicle.restart_time_mod),
        },
        first_aid: FirstAid { health_restored: f.items.first_aid.health_restored, use_length: f.items.first_aid.use_length },
        characters,
        match_rules: MatchRules {
            round_time: f.match_.round_time,
            pre_match_wait_time: 0,
            post_match_outro_wait_time: f.match_.post_match_outro_wait_time,
            post_match_wait_time: f.match_.post_match_wait_time,
        },
        score_events: f
            .match_
            .score_events
            .into_iter()
            .map(|(name, e)| {
                let event = ScoreEvent {
                    experience: e.experience,
                    category: e.category,
                    only_once: e.only_once,
                    all_counselors: e.all_counselors,
                    description: e.description,
                };
                (name, event)
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic values (not the game's): exercises the schema only.
    const SAMPLE: &str = r#"{
      "format": 8,
      "match": {"round_time": 60, "post_match_outro_wait_time": 1, "post_match_wait_time": 2,
                "score_events": {"Win": {"experience": 5, "category": "Escaped", "only_once": true,
                                         "all_counselors": false, "description": "Won +<exp>"}}},
      "items": {"first_aid": {"health_restored": 100, "use_length": 2}},
      "vehicle": {"min_restart_time": 5, "max_restart_time": 5, "restart_time_mod": {"base": 1, "modifiers": [0,0,0,0,0,0,0]}},
      "counselor": {"Base_Counselor": {"max_walk_speed": 1, "max_walk_speed_crouched": 1, "max_slow_run_speed": 1,
          "max_run_speed": 1, "max_sprint_speed": 1, "max_combat_speed": 1, "max_swim_speed": 1, "max_acceleration": 1,
          "dodging_speed_mod": 1, "stumbling_speed_mod": 1, "water_speed_mod": 1,
          "speed_stat_mod": {"base": 1, "modifiers": [0,0,0,0,0,0,0]}, "stamina_max": 100,
          "stamina_min_threshold": 10, "stamina_deplete_rate": 1, "stamina_refill_rate": {"base": 1, "modifiers": [0,0,0,0,0,0,0]},
          "stamina_idle_modifier": 1, "stamina_hiding_modifier": 1, "stamina_walking_modifier": -1,
          "stamina_running_modifier": 1, "stamina_sprinting_modifier": 1, "stamina_swimming_modifier": 1,
          "stamina_crouching_modifier": 1, "stamina_stat_mod": {"base": 1, "modifiers": [0,0,0,0,0,0,0]},
          "grabbed_stamina_refill": {"base": 1, "modifiers": [0,0,0,0,0,0,0]}, "stamina_fear_curve": "C",
          "force_stumble_on_stamina_depletion": true, "wounded_threshold": 10,
          "jason_near_min_range": 1, "jason_near_max_range": 2,
          "health": {"max_health": 100, "blocking_damage_mod": {"base": 0.2, "modifiers": [0,0,0,0,0,0,0]}},
          "stumble": {"fear_frequency_curve": null, "stamina_frequency_curve": "C", "fear_chance_curve": "C",
              "stamina_chance_curve": null, "stat_mod": {"base": 1, "modifiers": [0,0,0,0,0,0,0]},
              "wounded_chance": 3, "cooldown": 15, "sprint": {"name": "S", "length": 1}, "run": {"name": "R", "length": 1},
              "walk": {"name": null, "length": 0}, "out_of_stamina": {"name": "R", "length": 1}},
          "dodge": {"stamina_cost": {"base": 20, "modifiers": [0,0,0,0,0,0,0]}, "left": {"name": "L", "length": 0.5},
              "right": {"name": "R", "length": 0.5}, "back": {"name": "B", "length": 0.5}},
          "minigame": {"min_hold_time": 0.2, "repair_hold_time": 15, "allowable_fails": 3, "max_pip_count": 10,
              "min_pip_count": 5, "max_pip_width": 30, "min_pip_width": 15,
              "max_pip_count_mod": {"base": 1, "modifiers": [0,0,0,0,0,0,0]}, "min_pip_count_mod": {"base": 1, "modifiers": [0,0,0,0,0,0,0]},
              "max_pip_width_mod": {"base": 20, "modifiers": [0,0,0,0,0,0,0]}, "min_pip_width_mod": {"base": 10, "modifiers": [0,0,0,0,0,0,0]},
              "speed_mod": {"base": 1.5, "modifiers": [0,0,0,0,0,0,0]}}},
        "characters": {"Test": {"stats": [1,2,3,4,5,6,7]}}},
      "killer": {"Jason_Base": {"unlock_time": [1,2,3,4], "recharge_time": [5,6,7,8],
                 "raged_recharge_modifier": 2.0, "rage_unlock_time": 10.0}, "stalk_active_time": 3.0},
      "fear": {"update_fear_delay": 0.1,
               "fear_modifier": {"base": 1.0, "modifiers": [0,0,0,0,0,50,0], "stat_order": []},
               "events": {"Near": {"increase": 1, "decrease": 1, "increase_delay": 0, "decrease_delay": 0,
                          "max_fear_cap": 10, "decrease_when_active": false, "auto_deactivate_time": 0,
                          "distance_curve": "C"}},
               "curves": {"C": [{"time": 0, "value": 2, "interp": "linear", "arrive_tangent": 0, "leave_tangent": 0},
                                {"time": 1, "value": 0, "interp": "linear", "arrive_tangent": 0, "leave_tangent": 0}]}}
    }"#;

    #[test]
    fn parses_schema_and_wires_curves() {
        let t = parse(SAMPLE).unwrap();
        assert_eq!(t.jason.unlock_time, [1.0, 2.0, 3.0, 4.0]);
        let near = &t.fear_events["Near"];
        assert_eq!((near.distance_curve.as_ref().unwrap())(0.5), 1.0);
        assert_eq!(t.characters["Test"], [1, 2, 3, 4, 5, 6, 7]);
        assert_eq!((t.counselor.stamina_fear_curve.as_ref().unwrap())(1.0), 0.0);
        assert!(t.counselor.stumble.fear_frequency.is_none());
        assert_eq!((t.counselor.stumble.fear_chance.as_ref().unwrap())(0.5), 1.0);
        assert_eq!(t.match_rules.round_time, 60);
        assert_eq!(t.score_events["Win"].text(5), "Won +5");
    }

    /// Loads the real extracted file when present (skipped on machines without one).
    #[test]
    fn loads_local_extraction_if_present() {
        let p = default_path();
        if !p.exists() {
            return;
        }
        let t = load(&p).unwrap();
        assert_eq!(t.fear_events.len(), 17);
        assert!(t.jason.rage_unlock_time > 0.0);
        assert!(t.characters.contains_key("Vanessa"));
        assert!(t.counselor.max_sprint_speed > t.counselor.max_run_speed);
        assert!(t.counselor.stumble.run.length > 0.0 && t.counselor.stumble.cooldown > 0.0);
        assert!(t.counselor.dodge.back.length > 0.0 && t.counselor.dodge.stamina_cost.base > 0.0);
        assert!(t.repair_hold_time > 0.0 && t.minigame.max_pip_count >= t.minigame.min_pip_count);
        assert!(t.counselor.health.max_health > t.counselor.wounded_threshold);
        assert!(t.first_aid.health_restored > 0.0 && t.first_aid.use_length > 0.0);
        assert!(t.match_rules.round_time > 0 && t.match_rules.post_match_wait_time > 0);
        for e in ["Escape", "CompletedMatch", "CounselorTimeBonus", "Unscathed", "TeamEscaped"] {
            assert!(t.score_events[e].experience > 0, "{e}");
        }
    }
}
