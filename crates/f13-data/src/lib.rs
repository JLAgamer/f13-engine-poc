//! Loads `tuning.json` (made by `tools/extract_tuning.py` from the player's own install) into f13-core types.

pub mod classes;
pub mod level;
mod native_classes;
pub mod world;

use f13_core::counselor::CounselorTuning;
use f13_core::curve::{Interp, Key, RichCurve};
use f13_core::fear::FearEventData;
use f13_core::killer_abilities::KillerTuning;
use f13_core::movement::MovementSettings;
use f13_core::stats::{StatModifiedValue, Stats, STAT_COUNT};
use f13_core::stumble::StumbleTuning;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Deserialize)]
struct File {
    format: u32,
    killer: KillerSection,
    fear: FearSection,
    counselor: CounselorSection,
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
    stumble: StumbleJson,
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
    sprint_length: f32,
    run_length: f32,
    walk_length: f32,
    out_of_stamina_length: f32,
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
    /// Playable counselors by name (e.g. "Vanessa") -> stats.
    pub characters: BTreeMap<String, Stats>,
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Format(u32),
    MissingCurve(String),
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
pub const FORMAT: u32 = 3;

pub fn load(path: &Path) -> Result<Tuning, LoadError> {
    parse(&std::fs::read_to_string(path).map_err(LoadError::Io)?)
}

pub fn parse(text: &str) -> Result<Tuning, LoadError> {
    let f: File = serde_json::from_str(text).map_err(LoadError::Json)?;
    if f.format != FORMAT {
        return Err(LoadError::Format(f.format));
    }
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
        sprint_length: st.sprint_length,
        run_length: st.run_length,
        walk_length: st.walk_length,
        out_of_stamina_length: st.out_of_stamina_length,
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
        stumble,
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
        characters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic values (not the game's): exercises the schema only.
    const SAMPLE: &str = r#"{
      "format": 3,
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
          "stumble": {"fear_frequency_curve": null, "stamina_frequency_curve": "C", "fear_chance_curve": "C",
              "stamina_chance_curve": null, "stat_mod": {"base": 1, "modifiers": [0,0,0,0,0,0,0]},
              "wounded_chance": 3, "cooldown": 15, "sprint_length": 1, "run_length": 1, "walk_length": 1,
              "out_of_stamina_length": 1}},
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
        assert!(t.counselor.stumble.run_length > 0.0 && t.counselor.stumble.cooldown > 0.0);
    }
}
