//! A counselor's character as the exporter writes it (`counselors/<Blueprint>.json`): body mesh, the parts that follow
//! it (outfit, face, hair), the shared skeleton's bones and the animations, plus the `.anim` files themselves.

use crate::level::Xform;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CharacterInfo {
    pub name: String,
    /// The body glTF (CharacterMesh0), relative to the export directory.
    pub body: String,
    /// CharacterMesh0 relative to the capsule's centre.
    pub mesh_transform: Xform,
    pub capsule_half_height: f32,
    /// Capsule radius (cm; ACharacter's 34 unless the Blueprint sets it).
    #[serde(default = "default_radius")]
    pub capsule_radius: f32,
    pub parts: Vec<Part>,
    /// The two skin masks the game multiplies to hide the body under the outfit (counselor's own, outfit's).
    #[serde(default)]
    pub skin_masks: Vec<String>,
    pub bones: Vec<Bone>,
    pub animations: BTreeMap<String, AnimInfo>,
    /// Gameplay values (killers: speeds, attack and grab ranges, weapon damage), by Unreal property name.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub values: HashMap<String, f32>,
}

fn default_radius() -> f32 {
    34.0
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Part {
    pub name: String,
    /// Empty for an attach point only (a socket other parts hang off).
    #[serde(default)]
    pub file: String,
    /// "MasterPose": the part's skin uses the body's bones of the same names. Otherwise the body bone (or another
    /// part's name) its root hangs off, at `transform`.
    pub attach: String,
    #[serde(default)]
    pub transform: Option<Xform>,
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub material_map: HashMap<String, String>,
}

impl Part {
    pub fn master_pose(&self) -> bool {
        self.attach == "MasterPose"
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Bone {
    pub name: String,
    pub parent: i32,
    /// "Animation" (translation from the animation) or "Skeleton" (from the mesh's bind pose).
    pub retarget: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AnimInfo {
    pub file: String,
    pub length: f32,
    pub fps: f32,
    pub frames: u32,
    pub root_motion: bool,
    /// The root keeps its animated translation (paired kills: both characters play around one origin).
    #[serde(default)]
    pub keep_root: bool,
    /// Notify states with a duration (e.g. `AttackTraceNotify`, when a weapon swing can hit), in seconds.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub windows: Vec<Window>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Window {
    pub name: String,
    pub start: f32,
    pub end: f32,
}

impl AnimInfo {
    /// The first notify window called `name`.
    pub fn window(&self, name: &str) -> Option<(f32, f32)> {
        self.windows.iter().find(|w| w.name == name).map(|w| (w.start, w.end))
    }
}

impl CharacterInfo {
    pub fn load(path: &Path) -> Result<Self, crate::LoadError> {
        let text = std::fs::read_to_string(path).map_err(crate::LoadError::Io)?;
        serde_json::from_str(&text).map_err(crate::LoadError::Json)
    }
}

/// One bone's samples, converted to glTF space (metres, Y up): `rotation` per frame and, for bones whose translation
/// comes from the animation, `translation` per frame.
#[derive(Debug, Clone, PartialEq)]
pub struct BoneTrack {
    pub bone: String,
    pub rotation: Vec<[f32; 4]>,
    pub translation: Option<Vec<[f32; 3]>>,
}

/// A sampled animation: `frames` evenly spaced samples from 0 to the sequence's length.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimData {
    pub frames: usize,
    pub tracks: Vec<BoneTrack>,
}

impl AnimData {
    pub fn load(path: &Path) -> Result<Self, crate::LoadError> {
        let bytes = std::fs::read(path).map_err(crate::LoadError::Io)?;
        Self::parse(&bytes).ok_or_else(|| crate::LoadError::Invalid(format!("{}: not a valid .anim file", path.display())))
    }

    /// The exporter's `.anim` format (tools/f13-export/Counselor.cs).
    pub fn parse(b: &[u8]) -> Option<Self> {
        let mut r = Reader { b, at: 0 };
        if r.take(4)? != b"F13A" || r.u32()? != 1 {
            return None;
        }
        let _fps = r.f32()?;
        let frames = r.u32()? as usize;
        let count = r.u32()?;
        let mut tracks = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let len = r.u16()? as usize;
            let bone = String::from_utf8(r.take(len)?.to_vec()).ok()?;
            let flags = r.take(1)?[0];
            let mut rotation = Vec::with_capacity(frames);
            if flags & 1 != 0 {
                for _ in 0..frames {
                    let q = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
                    rotation.push(Xform { t: [0.0; 3], r: q, s: [1.0; 3] }.to_gltf_space().rotation);
                }
            }
            let translation = if flags & 2 != 0 {
                let mut t = Vec::with_capacity(frames);
                for _ in 0..frames {
                    let v = [r.f32()?, r.f32()?, r.f32()?];
                    t.push(Xform { t: v, ..Xform::IDENTITY }.to_gltf_space().translation);
                }
                Some(t)
            } else {
                None
            };
            tracks.push(BoneTrack { bone, rotation, translation });
        }
        Some(AnimData { frames, tracks })
    }
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.at..self.at + n)?;
        self.at += n;
        Some(s)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(frames: u32, tracks: &[(&str, u8)]) -> Vec<u8> {
        let mut b = b"F13A".to_vec();
        b.extend(1u32.to_le_bytes());
        b.extend(30f32.to_le_bytes());
        b.extend(frames.to_le_bytes());
        b.extend((tracks.len() as u32).to_le_bytes());
        for (name, flags) in tracks {
            b.extend((name.len() as u16).to_le_bytes());
            b.extend(name.as_bytes());
            b.push(*flags);
            for f in 0..frames {
                // A quarter turn about Unreal Z.
                let h = std::f32::consts::FRAC_PI_4;
                for v in [0.0, 0.0, h.sin(), h.cos()] {
                    b.extend((v + f as f32 * 0.0).to_le_bytes());
                }
            }
            if flags & 2 != 0 {
                for f in 0..frames {
                    for v in [100.0f32, 200.0, 300.0 + f as f32] {
                        b.extend(v.to_le_bytes());
                    }
                }
            }
        }
        b
    }

    #[test]
    fn parses_and_converts_to_gltf_space() {
        let a = AnimData::parse(&file(2, &[("root", 3), ("spine_01", 1)])).unwrap();
        assert_eq!(a.frames, 2);
        assert_eq!(a.tracks.len(), 2);
        let root = &a.tracks[0];
        assert_eq!(root.bone, "root");
        // Unreal Z is glTF Y: a turn about Z becomes one about Y, angle negated (handedness).
        let q = root.rotation[0];
        assert!(q[0].abs() < 1e-6 && q[2].abs() < 1e-6 && (q[1] + std::f32::consts::FRAC_PI_4.sin()).abs() < 1e-6, "{q:?}");
        assert_eq!(root.translation.as_ref().unwrap()[1], [1.0, 3.01, 2.0]);
        assert!(a.tracks[1].translation.is_none());
    }

    #[test]
    fn rejects_truncated_files() {
        let mut b = file(2, &[("root", 3)]);
        b.truncate(b.len() - 1);
        assert!(AnimData::parse(&b).is_none());
        assert!(AnimData::parse(b"F13B").is_none());
    }
}
