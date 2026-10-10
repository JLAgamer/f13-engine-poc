//! The match's weather and its storm, decompiled:
//! - SCWorldSettings (FUN_1404328d0, on level load, after the cabin layout pick): rain when rand()/32767 <= RainChance
//!   (its constructor's 0.5; Crystal Lake keeps it); the URL option `rain=` overrides it (`--rain on|off` here). It
//!   loads RainEnabledLevel (CLsm_RainOn) or RainDisabledLevel (CLsm_RainOff): their moon, fog and sky (main.rs).
//! - SCZeus (CLsm_RainOn's SCZeus_BP: InitialDelay 5, MinTimeBetweenLightning 15, MaxTimeBetweenLightning 40,
//!   LightningSoundThreshold 3, three LightningCurves). BeginPlay (0x140337940): its light's intensity is the base;
//!   lightning timings are drawn (FUN_140340a70) from InitialDelay over max(RemainingTime + 60, 300) s: each one a
//!   random 0..MaxTimeBetweenLightning s after the last base, a random curve, the next base MinTimeBetweenLightning
//!   after it; the rain loop (RainAudio) plays. Tick (0x1403619b0): when the match's ElapsedInProgressTime (whole
//!   seconds) reaches the next timing, that curve plays from the world's time: the light at curve x base, and the
//!   thunder (ThunderAudio) once as the curve rises past LightningSoundThreshold; on the last timing 60 s more are
//!   drawn. At the curve's end the light goes back to its base.
//!
//! While a curve plays, Rain_MPC's LightiningIntensity is clamp((v - min) / max(max, eps), 0, 1) over the curve's
//! value range, 0 after ([`RainMpc`]: the sky's material brightens by exp(x 8) with it, shaders/masters.wgsl).
//!
//! Not ported: the rain itself (particles) and wet materials.

use bevy::prelude::*;

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RainMpc>().add_systems(Update, (rain_mpc, start_storm, storm).chain());
    }
}

/// Rain_MPC's parameters the game writes: Rain (SCWorldSettings::OnRep_IsRaining 0x140439ca0: 1 when raining, else
/// 0) and LightiningIntensity (SCZeus's tick).
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct RainMpc {
    pub rain: f32,
    pub lightning: f32,
}

/// OnRep_IsRaining: Rain_MPC's Rain follows the weather.
fn rain_mpc(weather: Option<Res<Weather>>, mut mpc: ResMut<RainMpc>) {
    let Some(w) = weather else { return };
    let rain = if w.rain { 1.0 } else { 0.0 };
    if mpc.rain != rain {
        mpc.rain = rain;
    }
}

/// The match's weather roll.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Weather {
    pub rain: bool,
}

/// SCWorldSettings' rain roll: `roll` is rand()/32767.
pub fn rains(rain_chance: f32, roll: f32) -> bool {
    roll <= rain_chance
}

/// The moon (the weather level's directional light: the storm's, when it rains).
#[derive(Component)]
pub struct Moon;

/// SCZeus's lightning timings (FUN_140340a70): from `from` until `until`, (second, curve). `rand01` is
/// rand() * (1/32767).
pub fn lightning_timings(from: i32, until: i32, curves: usize, min_between: i32, max_between: i32, rand01: &mut impl FnMut() -> f32) -> Vec<(i32, usize)> {
    let mut out = Vec::new();
    let mut base = from;
    loop {
        let curve = if curves == 0 { 0 } else { ((rand01() * curves as f32) as usize).min(curves - 1) };
        let delay = if max_between + 1 < 1 { 0 } else { ((rand01() * (max_between + 1) as f32) as i32).min(max_between) };
        let at = base + delay;
        out.push((at, curve));
        base = at + min_between;
        if base >= until {
            return out;
        }
    }
}

#[derive(Resource)]
struct Storm {
    curves: Vec<f13_core::curve::RichCurve>,
    threshold: f32,
    min_between: i32,
    max_between: i32,
    timings: Vec<(i32, usize)>,
    /// The last timing started (-1: none yet).
    current: i32,
    last_second: i32,
    base: f32,
    /// The curve playing: which, since when (world seconds), its last value, whether the thunder played.
    active: Option<(usize, f32, f32, bool)>,
}

fn start_storm(
    mut commands: Commands,
    weather: Option<Res<Weather>>,
    storm: Option<Res<Storm>>,
    export: Res<crate::ExportRoot>,
    game: Option<Res<crate::match_flow::GameMatch>>,
    mut rng: ResMut<crate::GameRng>,
    moons: Query<&DirectionalLight, With<Moon>>,
) {
    if storm.is_some() || !weather.is_some_and(|w| w.rain) {
        return;
    }
    let (Some(game), Ok(moon)) = (game, moons.single()) else { return };
    let path = export.0.join("weather.json");
    let Some(json) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else {
        warn!("{}: missing (re-export: f13-export levels): no storm", path.display());
        commands.insert_resource(Storm { curves: vec![], threshold: 0.0, min_between: 0, max_between: 0, timings: vec![], current: -1, last_second: -1, base: moon.illuminance, active: None });
        return;
    };
    let i = |k: &str, d: i64| json[k].as_i64().unwrap_or(d) as i32;
    let curves: Vec<f13_core::curve::RichCurve> = json["LightningCurves"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| serde_json::from_value::<Vec<f13_data::character::CurveKey>>(c.clone()).ok())
        .map(|keys| f13_data::character::rich_curve(&keys))
        .collect();
    let (initial, min_between, max_between) = (i("InitialDelay", 2), i("MinTimeBetweenLightning", 8), i("MaxTimeBetweenLightning", 25));
    let mut rand01 = || rng.0.frand();
    let timings = if curves.is_empty() { vec![] } else { lightning_timings(initial, initial + (game.state.remaining_time + 60).max(300), curves.len(), min_between, max_between, &mut rand01) };
    info!("weather: storm, {} lightning strikes drawn (first at {:?} s)", timings.len(), timings.first().map(|t| t.0));
    commands.trigger(crate::audio::PlaySound { cue: "RainMedium_Cue".into(), at: None });
    commands.insert_resource(Storm {
        curves,
        threshold: json["LightningSoundThreshold"].as_f64().unwrap_or(3.0) as f32,
        min_between,
        max_between,
        timings,
        current: -1,
        last_second: -1,
        base: moon.illuminance,
        active: None,
    });
}

fn storm(
    mut commands: Commands,
    time: Res<Time>,
    storm: Option<ResMut<Storm>>,
    game: Option<Res<crate::match_flow::GameMatch>>,
    mut rng: ResMut<crate::GameRng>,
    mut moons: Query<&mut DirectionalLight, With<Moon>>,
    mut mpc: ResMut<RainMpc>,
) {
    let (Some(mut storm), Some(game)) = (storm, game) else { return };
    if storm.curves.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    let second = game.state.elapsed_in_progress_time;
    if second != storm.last_second {
        // Out of timings: 60 s more after the last.
        if storm.current >= storm.timings.len() as i32 - 1 {
            let from = storm.timings.last().map_or(0, |t| t.0);
            let (n, min, max) = (storm.curves.len(), storm.min_between, storm.max_between);
            let mut rand01 = || rng.0.frand();
            let more = lightning_timings(from, from + 60, n, min, max, &mut rand01);
            storm.timings.extend(more);
        }
        let next = (storm.current + 1) as usize;
        if let Some(&(at, curve)) = storm.timings.get(next) {
            if at <= second {
                storm.current = next as i32;
                storm.active = Some((curve, now, 1.0, false));
                info!("weather: lightning at {second} s (curve {curve})");
            }
        }
        storm.last_second = second;
    }
    let Some((curve, start, prev, thunder)) = storm.active else { return };
    let c = &storm.curves[curve];
    let end = c.keys.last().map_or(0.0, |k| k.time);
    let Ok(mut moon) = moons.single_mut() else { return };
    let t = now - start;
    if t >= end {
        storm.active = None;
        moon.illuminance = storm.base;
        mpc.lightning = 0.0;
        return;
    }
    let v = c.eval(t);
    moon.illuminance = v * storm.base;
    let (lo, hi) = c.keys.iter().fold((f32::MAX, f32::MIN), |(lo, hi), k| (lo.min(k.value), hi.max(k.value)));
    let lightning = ((v - lo) / hi.max(f32::EPSILON)).clamp(0.0, 1.0);
    if mpc.lightning != lightning {
        mpc.lightning = lightning;
    }
    let mut thunder = thunder;
    if !thunder && prev <= storm.threshold && storm.threshold < v {
        thunder = true;
        commands.trigger(crate::audio::PlaySound { cue: "ThunderCue".into(), at: None });
    }
    storm.active = Some((curve, start, v, thunder));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_rolls_under_its_chance() {
        assert!(rains(0.5, 0.5));
        assert!(!rains(0.5, 0.51));
    }

    #[test]
    fn lightning_is_drawn_min_apart_plus_up_to_max() {
        let mut vals = [0.0, 0.0, 0.99, 0.999].into_iter().cycle();
        let mut rand01 = || vals.next().unwrap();
        let t = lightning_timings(5, 100, 3, 15, 40, &mut rand01);
        // (curve 0, +0) at 5; (curve 2, +40) at 20 + 40 = 60; then from 75...
        assert_eq!(t[0], (5, 0));
        assert_eq!(t[1], (60, 2));
        assert!(t.windows(2).all(|w| w[1].0 >= w[0].0 + 15));
        assert!(t.last().unwrap().0 + 15 >= 100);
    }
}
