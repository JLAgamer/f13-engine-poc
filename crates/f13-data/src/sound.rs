//! The exporter's `sounds/`: each SoundCue's node graph (`<cue>.json`) and the counselor's footstep tables
//! (`footsteps.json`). Properties a node or attenuation leaves out take Unreal 4.18's defaults.

use serde_json::Value;
use std::path::Path;

/// How a sound fades with distance (FAttenuationSettings). Distances in cm.
#[derive(Clone, Debug, PartialEq)]
pub struct Attenuation {
    pub attenuate: bool,
    pub spatialize: bool,
    /// The sphere's radius at full volume (AttenuationShapeExtents.X).
    pub inner_radius: f32,
    /// Beyond the inner radius, the distance over which it fades out.
    pub falloff: f32,
    pub algorithm: Algorithm,
    /// The level at the end of the falloff for the NaturalSound algorithm (dB).
    pub db_at_max: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
    Linear,
    Logarithmic,
    Inverse,
    LogReverse,
    NaturalSound,
}

impl Default for Attenuation {
    fn default() -> Self {
        Attenuation { attenuate: true, spatialize: true, inner_radius: 400.0, falloff: 3600.0, algorithm: Algorithm::Linear, db_at_max: -60.0 }
    }
}

impl Attenuation {
    fn from_json(v: &Value) -> Self {
        let d = Attenuation::default();
        let f = |k: &str, def: f32| v[k].as_f64().map_or(def, |x| x as f32);
        let algorithm = match v["DistanceAlgorithm"].as_str().unwrap_or("") {
            s if s.ends_with("Logarithmic") => Algorithm::Logarithmic,
            s if s.ends_with("Inverse") => Algorithm::Inverse,
            s if s.ends_with("LogReverse") => Algorithm::LogReverse,
            s if s.ends_with("NaturalSound") => Algorithm::NaturalSound,
            _ => Algorithm::Linear,
        };
        Attenuation {
            attenuate: v["bAttenuate"].as_bool().unwrap_or(d.attenuate),
            spatialize: v["bSpatialize"].as_bool().unwrap_or(d.spatialize),
            inner_radius: v["AttenuationShapeExtents"]["X"].as_f64().map_or(d.inner_radius, |x| x as f32),
            falloff: f("FalloffDistance", d.falloff),
            algorithm,
            db_at_max: f("dBAttenuationAtMax", d.db_at_max),
        }
    }

    /// The volume factor at `distance` cm (UE's FAttenuationSettings::Evaluate for a sphere).
    pub fn factor(&self, distance: f32) -> f32 {
        if !self.attenuate {
            return 1.0;
        }
        if distance <= self.inner_radius {
            return 1.0;
        }
        if self.falloff <= 0.0 {
            return 0.0;
        }
        let a = ((distance - self.inner_radius) / self.falloff).clamp(0.0, 1.0);
        match self.algorithm {
            Algorithm::Linear => 1.0 - a,
            Algorithm::Logarithmic => (0.5 * -(a.max(1e-4)).ln()).clamp(0.0, 1.0),
            Algorithm::Inverse => (0.02 / a.max(1e-4)).clamp(0.0, 1.0),
            Algorithm::LogReverse => (1.0 + 0.5 * (1.0 - a).max(1e-4).ln()).clamp(0.0, 1.0),
            // dBAttenuationAtMax reached at the end of the falloff, silent beyond it.
            Algorithm::NaturalSound => {
                if a >= 1.0 {
                    0.0
                } else {
                    10f32.powf(self.db_at_max.min(0.0) * a / 20.0)
                }
            }
        }
    }
}

/// A SoundCue node (the ones f13-game plays; anything else passes its first child through).
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Attenuation { settings: Option<Attenuation>, child: Option<Box<Node>> },
    /// Random volume and pitch multipliers in these ranges.
    Modulator { volume: (f32, f32), pitch: (f32, f32), child: Option<Box<Node>> },
    Random { weights: Vec<f32>, children: Vec<Node> },
    /// Every child, each at its input volume.
    Mixer { volumes: Vec<f32>, children: Vec<Node> },
    Looping { child: Option<Box<Node>> },
    /// USoundNodeDelay: its child starts after a random delay in [DelayMin, DelayMax] s.
    Delay { range: (f32, f32), child: Option<Box<Node>> },
    /// USoundNodeModulatorContinuous: volume and pitch from the sound's parameters, while it plays.
    ModulatorContinuous { volume: ParamMap, pitch: ParamMap, child: Option<Box<Node>> },
    /// USoundNodeParamCrossFade (`param` Some: by that parameter) and USoundNodeDistanceCrossFade (None: by the
    /// listener's distance, cm): each child's volume by its input's band.
    CrossFade { param: Option<String>, bands: Vec<CrossFadeBand>, children: Vec<Node> },
    /// A wave file (under sounds/), its volume and pitch, and whether it loops.
    Wave { file: Option<String>, volume: f32, pitch: f32, looping: bool },
    Other { kind: String, children: Vec<Node> },
}

impl Node {
    fn from_json(v: &Value) -> Option<Node> {
        if v.is_null() {
            return None;
        }
        let p = &v["Props"];
        let f = |k: &str, d: f32| p[k].as_f64().map_or(d, |x| x as f32);
        let children: Vec<Node> = v["Children"].as_array().into_iter().flatten().filter_map(Node::from_json).collect();
        let first = || children.first().cloned().map(Box::new);
        Some(match v["Type"].as_str().unwrap_or("") {
            "Attenuation" => {
                let settings = if p["bOverrideAttenuation"].as_bool() == Some(true) {
                    Some(Attenuation::from_json(&p["AttenuationOverrides"]))
                } else {
                    v.get("Settings").map(Attenuation::from_json)
                };
                Node::Attenuation { settings, child: first() }
            }
            "Modulator" => Node::Modulator { volume: (f("VolumeMin", 0.95), f("VolumeMax", 1.05)), pitch: (f("PitchMin", 0.95), f("PitchMax", 1.05)), child: first() },
            "Random" => {
                let weights = p["Weights"].as_array().map(|w| w.iter().map(|x| x.as_f64().unwrap_or(1.0) as f32).collect()).unwrap_or_default();
                Node::Random { weights, children }
            }
            "Mixer" => {
                let volumes = p["InputVolume"].as_array().map(|w| w.iter().map(|x| x.as_f64().unwrap_or(1.0) as f32).collect()).unwrap_or_default();
                Node::Mixer { volumes, children }
            }
            "Looping" => Node::Looping { child: first() },
            "Delay" => Node::Delay { range: (f("DelayMin", 0.0), f("DelayMax", 0.0)), child: first() },
            "ModulatorContinuous" => Node::ModulatorContinuous {
                volume: ParamMap::from_json(&p["VolumeModulationParams"]),
                pitch: ParamMap::from_json(&p["PitchModulationParams"]),
                child: first(),
            },
            "ParamCrossFade" | "DistanceCrossFade" => Node::CrossFade {
                param: (v["Type"] == "ParamCrossFade").then(|| p["ParamName"].as_str().unwrap_or("").to_string()),
                bands: p["CrossFadeInput"].as_array().into_iter().flatten().map(CrossFadeBand::from_json).collect(),
                children,
            },
            "WavePlayer" => Node::Wave {
                file: v["Wave"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
                volume: v["WaveVolume"].as_f64().unwrap_or(1.0) as f32,
                pitch: v["WavePitch"].as_f64().unwrap_or(1.0) as f32,
                looping: p["bLooping"].as_bool().unwrap_or(false) || v["WaveLooping"].as_bool().unwrap_or(false),
            },
            other => Node::Other { kind: other.to_string(), children },
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub volume: f32,
    pub pitch: f32,
    pub attenuation: Option<Attenuation>,
    pub root: Option<Node>,
    /// Its SoundClassObject.
    pub sound_class: Option<String>,
}

/// One voice a cue plays: the wave, its volume and pitch, whether it loops.
#[derive(Clone, Debug, PartialEq)]
pub struct Voice {
    pub file: String,
    pub volume: f32,
    pub pitch: f32,
    pub looping: bool,
    /// Seconds to wait before it starts (its Delay nodes').
    pub delay: f32,
    /// What changes its volume and pitch while it plays (continuous modulators, cross-fades).
    pub mods: Vec<VoiceMod>,
    /// Its cue's sound class.
    pub class: Option<String>,
}

/// FModulatorContinuousParams (its constructor: Default 1, inputs 0..1, outputs 0..1, MPM_Normal).
#[derive(Clone, Debug, PartialEq)]
pub struct ParamMap {
    pub name: String,
    pub default: f32,
    pub min_in: f32,
    pub max_in: f32,
    pub min_out: f32,
    pub max_out: f32,
    /// MPM_Abs: the parameter's magnitude; MPM_Direct: the parameter itself.
    pub abs: bool,
    pub direct: bool,
}

impl ParamMap {
    fn from_json(v: &Value) -> ParamMap {
        let f = |k: &str, d: f32| v[k].as_f64().map_or(d, |x| x as f32);
        let mode = v["ParamMode"].as_str().unwrap_or("MPM_Normal");
        ParamMap {
            name: v["ParameterName"].as_str().filter(|n| *n != "None").unwrap_or("").to_string(),
            default: f("Default", 1.0),
            min_in: f("MinInput", 0.0),
            max_in: f("MaxInput", 1.0),
            min_out: f("MinOutput", 0.0),
            max_out: f("MaxOutput", 1.0),
            abs: mode == "MPM_Abs",
            direct: mode == "MPM_Direct",
        }
    }

    /// FModulatorContinuousParams::GetValue: the parameter (else Default), its magnitude for MPM_Abs, clamped to the
    /// input range and mapped onto the output range (MPM_Direct: as is).
    pub fn value(&self, param: Option<f32>) -> f32 {
        let mut x = param.filter(|_| !self.name.is_empty()).unwrap_or(self.default);
        if self.direct {
            return x;
        }
        if self.abs {
            x = x.abs();
        }
        let gradient = if self.max_in <= self.min_in { 0.0 } else { (self.max_out - self.min_out) / (self.max_in - self.min_in) };
        self.min_out + (x.clamp(self.min_in, self.max_in) - self.min_in) * gradient
    }
}

/// A cross-fade input (FDistanceDatum): its fade-in and fade-out bands and its volume.
#[derive(Clone, Debug, PartialEq)]
pub struct CrossFadeBand {
    pub fade_in: (f32, f32),
    pub fade_out: (f32, f32),
    pub volume: f32,
}

impl CrossFadeBand {
    fn from_json(v: &Value) -> CrossFadeBand {
        let f = |k: &str, d: f32| v[k].as_f64().map_or(d, |x| x as f32);
        CrossFadeBand {
            fade_in: (f("FadeInDistanceStart", 0.0), f("FadeInDistanceEnd", 0.0)),
            fade_out: (f("FadeOutDistanceStart", 0.0), f("FadeOutDistanceEnd", 0.0)),
            volume: f("Volume", 1.0),
        }
    }

    /// The input's volume at `x` (USoundNodeDistanceCrossFade::ParseNodes): rising across the fade-in band, falling
    /// across the fade-out band, full between them, silent outside. (An empty band, start = end, counts as full:
    /// Unreal divides by zero there.)
    pub fn gain(&self, x: f32) -> f32 {
        let ramp = |(a, b): (f32, f32)| if b > a { (x - a) / (b - a) } else { 1.0 };
        if x >= self.fade_in.0 && x <= self.fade_in.1 {
            self.volume * ramp(self.fade_in)
        } else if x >= self.fade_out.0 && x <= self.fade_out.1 {
            self.volume * (1.0 - ramp(self.fade_out))
        } else if x >= self.fade_in.1 && x <= self.fade_out.0 {
            self.volume
        } else {
            0.0
        }
    }
}

/// A playing voice's change by the sound's parameters (`param` None: the listener's distance, cm).
#[derive(Clone, Debug, PartialEq)]
pub enum VoiceMod {
    Volume(ParamMap),
    Pitch(ParamMap),
    Band { param: Option<String>, band: CrossFadeBand },
}

impl VoiceMod {
    /// (volume, pitch) multipliers for these parameters and listener distance (cm).
    pub fn factors(&self, params: &dyn Fn(&str) -> Option<f32>, distance_cm: f32) -> (f32, f32) {
        match self {
            VoiceMod::Volume(m) => (m.value(params(&m.name)), 1.0),
            VoiceMod::Pitch(m) => (1.0, m.value(params(&m.name))),
            // USoundNodeParamCrossFade: a missing parameter reads 0.
            VoiceMod::Band { param: Some(p), band } => (band.gain(params(p).unwrap_or(0.0)), 1.0),
            VoiceMod::Band { param: None, band } => (band.gain(distance_cm), 1.0),
        }
    }
}

impl Cue {
    pub fn load(dir: &Path, name: &str) -> Result<Cue, crate::LoadError> {
        let path = dir.join(format!("{name}.json"));
        let text = std::fs::read_to_string(&path).map_err(crate::LoadError::Io)?;
        let v: Value = serde_json::from_str(&text).map_err(crate::LoadError::Json)?;
        Ok(Cue {
            volume: v["Volume"].as_f64().unwrap_or(1.0) as f32,
            pitch: v["Pitch"].as_f64().unwrap_or(1.0) as f32,
            attenuation: v.get("Attenuation").map(Attenuation::from_json),
            root: Node::from_json(&v["Root"]),
            sound_class: v["SoundClass"].as_str().map(str::to_string),
        })
    }

    /// The voices one play of the cue starts, and its attenuation (an Attenuation node's, else the cue's).
    /// `rand` returns uniform numbers in [0, 1).
    pub fn play(&self, rand: &mut dyn FnMut() -> f32) -> (Vec<Voice>, Option<Attenuation>) {
        let mut voices = Vec::new();
        let mut att = None;
        if let Some(root) = &self.root {
            walk(root, self.volume, self.pitch, false, 0.0, &[], rand, &mut voices, &mut att);
        }
        for v in &mut voices {
            v.class = self.sound_class.clone();
        }
        (voices, att.or_else(|| self.attenuation.clone()))
    }
}

#[allow(clippy::too_many_arguments)]
fn walk(n: &Node, volume: f32, pitch: f32, looping: bool, delay: f32, mods: &[VoiceMod], rand: &mut dyn FnMut() -> f32, out: &mut Vec<Voice>, att: &mut Option<Attenuation>) {
    let lerp = |(a, b): (f32, f32), t: f32| a + (b - a) * t;
    match n {
        Node::Attenuation { settings, child } => {
            if att.is_none() {
                *att = settings.clone();
            }
            if let Some(c) = child {
                walk(c, volume, pitch, looping, delay, mods, rand, out, att);
            }
        }
        Node::Modulator { volume: v, pitch: p, child } => {
            let (vm, pm) = (lerp(*v, rand()), lerp(*p, rand()));
            if let Some(c) = child {
                walk(c, volume * vm, pitch * pm, looping, delay, mods, rand, out, att);
            }
        }
        Node::Random { weights, children } => {
            if children.is_empty() {
                return;
            }
            let w = |i: usize| weights.get(i).copied().unwrap_or(1.0).max(0.0);
            let total: f32 = (0..children.len()).map(w).sum();
            let mut pick = rand() * total;
            let mut chosen = children.len() - 1;
            for i in 0..children.len() {
                if pick < w(i) {
                    chosen = i;
                    break;
                }
                pick -= w(i);
            }
            walk(&children[chosen], volume, pitch, looping, delay, mods, rand, out, att);
        }
        Node::Mixer { volumes, children } => {
            for (i, c) in children.iter().enumerate() {
                walk(c, volume * volumes.get(i).copied().unwrap_or(1.0), pitch, looping, delay, mods, rand, out, att);
            }
        }
        Node::Delay { range, child } => {
            // USoundNodeDelay::ParseNodes: DelayMax + (DelayMin - DelayMax) x FRand, at least 0.
            let d = (range.1 + (range.0 - range.1) * rand()).max(0.0);
            if let Some(c) = child {
                walk(c, volume, pitch, looping, delay + d, mods, rand, out, att);
            }
        }
        Node::ModulatorContinuous { volume: v, pitch: p, child } => {
            if let Some(c) = child {
                let m: Vec<VoiceMod> = mods.iter().cloned().chain([VoiceMod::Volume(v.clone()), VoiceMod::Pitch(p.clone())]).collect();
                walk(c, volume, pitch, looping, delay, &m, rand, out, att);
            }
        }
        Node::CrossFade { param, bands, children } => {
            for (i, c) in children.iter().enumerate() {
                let band = bands.get(i).cloned().unwrap_or(CrossFadeBand { fade_in: (0.0, 0.0), fade_out: (0.0, 0.0), volume: 0.0 });
                let m: Vec<VoiceMod> = mods.iter().cloned().chain([VoiceMod::Band { param: param.clone(), band }]).collect();
                walk(c, volume, pitch, looping, delay, &m, rand, out, att);
            }
        }
        Node::Looping { child } => {
            if let Some(c) = child {
                walk(c, volume, pitch, true, delay, mods, rand, out, att);
            }
        }
        Node::Wave { file, volume: wv, pitch: wp, looping: wl } => {
            if let Some(file) = file {
                out.push(Voice { file: file.clone(), volume: volume * wv, pitch: pitch * wp, looping: looping || *wl, delay, mods: mods.to_vec(), class: None });
            }
        }
        Node::Other { children, .. } => {
            if let Some(c) = children.first() {
                walk(c, volume, pitch, looping, delay, mods, rand, out, att);
            }
        }
    }
}

/// The counselor's footstep cues by surface type (Base_Counselor_AnimBP's switches).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Footsteps {
    /// (gender, gait: "run" / "walk" / "land") -> cue per surface type (None: none for that surface).
    pub tables: Vec<(String, String, Vec<Option<String>>)>,
    /// GetFootstepInWater: deep water's cue, then shallow water's.
    pub in_water: Vec<String>,
}

impl Footsteps {
    /// The counselor's (footsteps.json).
    pub fn load(dir: &Path) -> Result<Footsteps, crate::LoadError> {
        Self::load_file(dir, "footsteps.json")
    }

    /// An animation Blueprint's tables (footsteps.json: the counselor's, footsteps_jason.json: Jason's).
    pub fn load_file(dir: &Path, name: &str) -> Result<Footsteps, crate::LoadError> {
        let text = std::fs::read_to_string(dir.join(name)).map_err(crate::LoadError::Io)?;
        let v: Value = serde_json::from_str(&text).map_err(crate::LoadError::Json)?;
        let mut tables = Vec::new();
        for f in ["GetFootstepSound", "GetLandingSound"] {
            for t in v[f].as_array().into_iter().flatten() {
                let surfaces = t["Surfaces"].as_array().into_iter().flatten().map(|s| s.as_str().map(str::to_string)).collect();
                tables.push((t["Gender"].as_str().unwrap_or("").to_string(), t["Gait"].as_str().unwrap_or("").to_string(), surfaces));
            }
        }
        let in_water = v["GetFootstepInWater"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_string)).collect();
        Ok(Footsteps { tables, in_water })
    }

    /// The cue for a surface type; none (or out of range): the default surface's, as SCAnimInstance's footstep
    /// handler asks again for surface 0.
    pub fn cue(&self, gender: &str, gait: &str, surface: u8) -> Option<&str> {
        let (_, _, t) = self.tables.iter().find(|(g, a, _)| g == gender && a == gait)?;
        t.get(surface as usize).and_then(|c| c.as_deref()).or_else(|| t.first()?.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_attenuation() {
        let a = Attenuation { inner_radius: 300.0, falloff: 500.0, ..Default::default() };
        assert_eq!(a.factor(100.0), 1.0);
        assert_eq!(a.factor(550.0), 0.5);
        assert_eq!(a.factor(900.0), 0.0);
    }

    #[test]
    fn engine_rpm_bands_and_pitch() {
        // V6Engine's idle and middle bands, and its pitch by RPM (MPM_Abs, 20..2400 -> 1.0..1.1).
        let idle = CrossFadeBand { fade_in: (0.0, 0.0), fade_out: (500.0, 800.0), volume: 1.0 };
        let mid = CrossFadeBand { fade_in: (10.0, 100.0), fade_out: (2000.0, 2400.0), volume: 0.6 };
        assert_eq!(idle.gain(0.0), 1.0);
        assert_eq!(idle.gain(650.0), 0.5);
        assert_eq!(idle.gain(900.0), 0.0);
        assert!((mid.gain(55.0) - 0.3).abs() < 1e-6);
        assert_eq!(mid.gain(1000.0), 0.6);
        assert_eq!(mid.gain(5.0), 0.0);
        let pitch = ParamMap { name: "RPM".into(), default: 1.0, min_in: 20.0, max_in: 2400.0, min_out: 1.0, max_out: 1.1, abs: true, direct: false };
        assert_eq!(pitch.value(Some(-2400.0)), 1.1);
        assert_eq!(pitch.value(Some(0.0)), 1.0);
        // An unnamed map reads its Default: full volume.
        let none = ParamMap { name: String::new(), ..pitch.clone() };
        assert_eq!(none.value(Some(5.0)), none.min_out + (1.0f32.clamp(20.0, 2400.0) - 20.0) * (0.1 / 2380.0));
    }

    #[test]
    fn a_random_cue_plays_one_wave_with_the_modulator_range() {
        let v: Value = serde_json::json!({
            "Volume": 2.0, "Pitch": 1.0,
            "Root": {"Type": "Modulator", "Props": {"VolumeMin": 0.5, "VolumeMax": 0.5}, "Children": [
                {"Type": "Random", "Props": {"Weights": [1.0, 3.0]}, "Children": [
                    {"Type": "WavePlayer", "Props": {}, "Wave": "waves/a.ogg", "Children": []},
                    {"Type": "WavePlayer", "Props": {}, "Wave": "waves/b.ogg", "Children": []}
                ]}
            ]}
        });
        let cue = Cue { volume: 2.0, pitch: 1.0, attenuation: None, root: Node::from_json(&v["Root"]), sound_class: None };
        // rand: 0 for the modulator's volume and pitch, then 0.5 * 4 = 2 > 1: the second wave.
        let mut seq = [0.0, 0.0, 0.5].into_iter();
        let (voices, _) = cue.play(&mut || seq.next().unwrap_or(0.0));
        assert_eq!(voices.len(), 1);
        assert_eq!(voices[0].file, "waves/b.ogg");
        assert!((voices[0].volume - 1.0).abs() < 1e-6);
        assert!((voices[0].pitch - 0.95).abs() < 1e-6);
    }
}
