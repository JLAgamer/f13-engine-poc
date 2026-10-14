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
    /// The skeleton's and mesh's sockets by name: `attach` is the bone, `transform` the offset from it.
    #[serde(default)]
    pub sockets: HashMap<String, Part>,
    /// Gameplay values (killers: speeds, attack and grab ranges, weapon damage), by Unreal property name.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub values: HashMap<String, f32>,
    /// The body mesh's physics asset: a body per simulated bone and the joints between them (counselors).
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub ragdoll: Vec<RagdollBody>,
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub joints: Vec<RagdollJoint>,
    /// Stat-modified values by property name: [BaseValue, then the StatModifiers by stat].
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub stats: HashMap<String, [f32; 8]>,
    /// Float curves by property name.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub curves: HashMap<String, Vec<CurveKey>>,
    /// Sounds by property name ("SoundCue'Name'" / "SoundWave'Name'").
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub sounds: HashMap<String, String>,
    /// Names the values refer to (the face anim instance's morph targets and eye bones), by property name.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub names: HashMap<String, String>,
    /// The music object's defaults (MusicData, idle times, warning cues; references as "SoundCue'Name'").
    #[serde(default)]
    pub music: serde_json::Value,
    /// Her body parts (SCCharacterBodyPartComponent), by component name (Face, Torso, LeftArm, RightArm, LeftLeg,
    /// RightLeg): the clothes' damage mask each sets.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub body_parts: HashMap<String, BodyPartInfo>,
    /// The character's dynamic cameras (ILLDynamicCameraComponent), by component name.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub cameras: HashMap<String, DynamicCameraInfo>,
    /// The camera shakes her graph plays, by class name: their CDO's properties.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub camera_shakes: HashMap<String, serde_json::Map<String, serde_json::Value>>,
    /// The hair's anim Blueprint: "Node" its AnimDynamics node's settings, "TargetFramerate" / "MaxAlpha" when it
    /// overrides SCHairAnimInstance's.
    #[serde(default)]
    pub hair_dynamics: Option<serde_json::Value>,
    /// Every outfit for this counselor, the default first (the part "Outfit" is it, each slot at its first option).
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub outfits: Vec<Outfit>,
}

/// An outfit Blueprint (ClothingData/<counselor>/*_Outfit): its clothing mesh, skin mask and slots.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Outfit {
    pub class: String,
    pub name: String,
    pub file: String,
    #[serde(default)]
    pub skin_mask: Option<String>,
    #[serde(default)]
    pub slots: Vec<OutfitSlot>,
    /// Its SweaterOutfitMesh (Pamela's sweater on), if any.
    #[serde(default)]
    pub sweater: Option<String>,
    /// Its MenuAnimation's sequence (the menus' preview plays it: SetCurrentOutfit with bUseMenuPose).
    #[serde(default)]
    pub menu_animation: Option<String>,
    /// Its OutfitIcon (export-relative, no extension) and OutfitDescription.
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// A ClothingSlot: the mesh's own material at its SlotIndex and the material instances its MaterialOptions give.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct OutfitSlot {
    pub own: String,
    #[serde(default)]
    pub options: Vec<String>,
    /// Its SlotName, SlotIcon and each option's SwatchIcon (export-relative, no extension).
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub option_icons: Vec<Option<String>>,
    /// The sweater mesh's material the slot's option goes on (GetMaterialIndexFromSlotIndex: the first whose name
    /// contains the slot's SlotMaterialName).
    #[serde(default)]
    pub sweater_own: Option<String>,
}

impl CharacterInfo {
    /// Her outfit with Pamela's sweater on (SCCounselorCharacter::OnRep_WearingSweater: the SweaterOutfit mesh, then
    /// FUN_1403d0660 puts each slot's chosen material on it by GetMaterialIndexFromSlotIndex): the mesh and its
    /// material map. None: her outfit has no sweater mesh.
    pub fn sweater_outfit(&self) -> Option<(String, HashMap<String, String>)> {
        let part = self.parts.iter().find(|p| p.name == "Outfit")?;
        let outfit = self.outfits.iter().find(|o| o.file == part.file)?;
        let file = outfit.sweater.clone()?;
        let mut map = HashMap::new();
        for slot in &outfit.slots {
            let Some(own) = &slot.sweater_own else { continue };
            // The chosen option (CurrentOutfit's): the one the outfit part wears; unmapped, the option is the mesh's
            // own material (the first option's, as the default outfit's).
            let chosen = part.material_map.get(&slot.own).or(slot.options.first()).unwrap_or(&slot.own);
            if chosen != own {
                map.insert(own.clone(), chosen.clone());
            }
        }
        Some((file, map))
    }

    /// The outfit she wears (the part "Outfit"'s mesh) and each slot's option (its material on the part; unmapped, the
    /// slot's own).
    pub fn current_outfit(&self) -> Option<(usize, Vec<usize>)> {
        let part = self.parts.iter().find(|p| p.name == "Outfit")?;
        let i = self.outfits.iter().position(|o| o.file == part.file)?;
        let picks = self.outfits[i]
            .slots
            .iter()
            .map(|s| {
                let worn = part.material_map.get(&s.own).unwrap_or(&s.own);
                s.options.iter().position(|o| o == worn).unwrap_or(0)
            })
            .collect();
        Some((i, picks))
    }

    /// [`choose_outfit`](Self::choose_outfit)'s spec for an outfit (by index) and its slots' options.
    pub fn outfit_spec(&self, outfit: usize, picks: &[usize]) -> Option<String> {
        let o = self.outfits.get(outfit)?;
        Some(format!("{}:{}", o.class, picks.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",")))
    }

    /// Dresses the counselor in another outfit: `spec` its class or name (case-insensitive, a part of it will do),
    /// then optionally `:` and each slot's option index (comma-separated; missing ones are the first). The part
    /// "Outfit" takes its mesh and slot materials, the second skin mask its SkinAlphaMask.
    pub fn choose_outfit(&mut self, spec: &str) -> Result<String, String> {
        let (name, options) = spec.split_once(':').unwrap_or((spec, ""));
        let lower = name.to_lowercase();
        let outfit = self
            .outfits
            .iter()
            .find(|o| o.class.to_lowercase() == lower || o.name.to_lowercase() == lower)
            .or_else(|| self.outfits.iter().find(|o| o.class.to_lowercase().contains(&lower) || o.name.to_lowercase().contains(&lower)))
            .ok_or_else(|| format!("no outfit {name:?} (have {})", self.outfits.iter().map(|o| o.name.as_str()).collect::<Vec<_>>().join(", ")))?
            .clone();
        let picks: Vec<usize> = options.split(',').filter(|s| !s.is_empty()).map(|s| s.trim().parse().map_err(|_| format!("bad option index {s:?}"))).collect::<Result<_, _>>()?;
        let mut map = HashMap::new();
        for (i, slot) in outfit.slots.iter().enumerate() {
            let pick = picks.get(i).copied().unwrap_or(0);
            let Some(material) = slot.options.get(pick).or(slot.options.first()) else { continue };
            if *material != slot.own {
                map.insert(slot.own.clone(), material.clone());
            }
        }
        let part = self.parts.iter_mut().find(|p| p.name == "Outfit").ok_or("no outfit part")?;
        part.file = outfit.file.clone();
        part.material_map = map;
        self.skin_masks.truncate(1);
        if let Some(mask) = &outfit.skin_mask {
            self.skin_masks.push(mask.clone());
        }
        Ok(outfit.name)
    }
}

/// A UCameraShake CDO's oscillation. UCameraShake's defaults: duration 1, blend in and out 0.1; an FOscillator's
/// amplitude and frequency 0, its InitialOffset EOO_OffsetRandom.
pub fn camera_shake(m: &serde_json::Map<String, serde_json::Value>) -> f13_core::dynamic_camera::Shake {
    use f13_core::dynamic_camera::{Oscillator, Shake};
    let f = |k: &str, d: f32| m.get(k).and_then(|v| v.as_f64()).map_or(d, |v| v as f32);
    let osc = |v: Option<&serde_json::Value>| Oscillator {
        amplitude: v.and_then(|v| v.get("Amplitude")).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
        frequency: v.and_then(|v| v.get("Frequency")).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
        random_offset: v.and_then(|v| v.get("InitialOffset")).and_then(|v| v.as_str()) != Some("EOO_OffsetZero"),
    };
    let group = |name: &str, axes: [&str; 3]| {
        let g = m.get(name);
        axes.map(|a| osc(g.and_then(|g| g.get(a))))
    };
    Shake {
        duration: f("OscillationDuration", 1.0),
        blend_in: f("OscillationBlendInTime", 0.1),
        blend_out: f("OscillationBlendOutTime", 0.1),
        rot: group("RotOscillation", ["Pitch", "Yaw", "Roll"]),
        loc: group("LocOscillation", ["X", "Y", "Z"]),
        fov: osc(m.get("FOVOscillation")),
    }
}

/// An ILLDynamicCameraComponent as exported: its MovementBasedSettings, bUse2DVelocity and its spring arm's own
/// A body part's gore (SCCharacterBodyPartComponent): the material parameter it sets (TextureParameterName), its
/// mask (LimbUVTexture; .png path), the masks by the damage's weapon class (CombatDamageGoreInfo), and the body's
/// material elements (ParentMaterialElementIndices).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct BodyPartInfo {
    #[serde(default)]
    pub parameter: Option<String>,
    #[serde(default)]
    pub mask: Option<String>,
    #[serde(default)]
    pub weapon_masks: HashMap<String, String>,
    #[serde(default)]
    pub elements: Vec<u32>,
    /// DetachLimb's: the bone her body hides ("None": the part's attach socket), the gore cap on her body (a
    /// skeletal mesh following her pose) at its socket, and the blood effect (a particle system) at its socket.
    #[serde(default)]
    pub bone_to_hide: Option<String>,
    #[serde(default)]
    pub gore_cap: Option<String>,
    #[serde(default)]
    pub gore_cap_socket: Option<String>,
    #[serde(default)]
    pub blood_effect: Option<String>,
    #[serde(default)]
    pub emitter_socket: Option<String>,
    /// Shown after the detach (the combat dismember): the limb's own mesh (on the body's skeleton), and its gore cap
    /// (a static mesh) at its socket.
    #[serde(default)]
    pub limb: Option<String>,
    #[serde(default)]
    pub limb_cap: Option<String>,
    #[serde(default)]
    pub limb_cap_at: Option<Part>,
    /// The limb mesh's own physics asset's bodies.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub limb_bodies: Vec<RagdollBody>,
    /// ShowLimb's mesh swaps (SCLimbMeshSwapAnimNotify's LimbSwapName -> mesh).
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub mesh_swaps: HashMap<String, String>,
}

/// properties.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct DynamicCameraInfo {
    #[serde(default)]
    pub settings: Vec<serde_json::Map<String, serde_json::Value>>,
    #[serde(default = "yes", rename = "Use2DVelocity")]
    pub use_2d_velocity: bool,
    #[serde(default)]
    pub spring_arm: serde_json::Map<String, serde_json::Value>,
}

fn yes() -> bool {
    true
}

impl DynamicCameraInfo {
    /// Its FCameraBehaviorSettings, the struct's defaults where a field isn't exported.
    pub fn settings(&self) -> Vec<f13_core::dynamic_camera::Settings> {
        use f13_core::dynamic_camera::{BlendFunc, Settings};
        self.settings
            .iter()
            .map(|m| {
                let d = Settings::default();
                let f = |k: &str, v: f32| m.get(k).and_then(|x| x.as_f64()).map_or(v, |x| x as f32);
                let b = |k: &str, v: bool| m.get(k).and_then(|x| x.as_bool()).unwrap_or(v);
                let v3 = |k: &str, v: glam::Vec3| {
                    m.get(k).map_or(v, |x| {
                        let c = |a: &str| x.get(a).and_then(|y| y.as_f64()).unwrap_or(0.0) as f32;
                        glam::Vec3::new(c("X"), c("Y"), c("Z"))
                    })
                };
                Settings {
                    minimum_character_speed: f("MinimumCharacterSpeed", d.minimum_character_speed),
                    blend_exponent: f("BlendExponent", d.blend_exponent),
                    blend_func: m.get("BlendFunc").and_then(|x| x.as_str()).map_or(d.blend_func, BlendFunc::from_name),
                    blend_in_delay: f("BlendInDelay", d.blend_in_delay),
                    blend_in_time: f("BlendInTime", d.blend_in_time),
                    target_arm_length: f("TargetArmLength", d.target_arm_length),
                    socket_offset: v3("SocketOffset", d.socket_offset),
                    target_offset: v3("TargetOffset", d.target_offset),
                    use_pawn_control_rotation: b("bUsePawnControlRotation", d.use_pawn_control_rotation),
                    inherit_pitch: b("bInheritPitch", d.inherit_pitch),
                    inherit_yaw: b("bInheritYaw", d.inherit_yaw),
                    inherit_roll: b("bInheritRoll", d.inherit_roll),
                    enable_camera_lag: b("bEnableCameraLag", d.enable_camera_lag),
                    enable_camera_rotation_lag: b("bEnableCameraRotationLag", d.enable_camera_rotation_lag),
                    use_camera_lag_substepping: b("bUseCameraLagSubstepping", d.use_camera_lag_substepping),
                    camera_lag_speed: f("CameraLagSpeed", d.camera_lag_speed),
                    camera_rotation_lag_speed: f("CameraRotationLagSpeed", d.camera_rotation_lag_speed),
                    camera_lag_max_time_step: f("CameraLagMaxTimeStep", d.camera_lag_max_time_step),
                    camera_lag_max_distance: f("CameraLagMaxDistance", d.camera_lag_max_distance),
                    field_of_view: f("FieldOfView", d.field_of_view),
                }
            })
            .collect()
    }

    /// Its spring arm: USpringArmComponent's constructor (ProbeSize 12, bDoCollisionTest) unless the template sets them.
    pub fn spring_arm(&self) -> f13_core::dynamic_camera::SpringArm {
        let a = &self.spring_arm;
        f13_core::dynamic_camera::SpringArm::new(
            a.get("ProbeSize").and_then(|x| x.as_f64()).map_or(12.0, |x| x as f32),
            a.get("bDoCollisionTest").and_then(|x| x.as_bool()).unwrap_or(true),
        )
    }
}

/// A RichCurve key as the exporter writes it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct CurveKey {
    pub time: f32,
    pub value: f32,
    pub arrive_tangent: f32,
    pub leave_tangent: f32,
    /// "linear", "constant" or "cubic".
    pub interp: String,
}

impl CharacterInfo {
    /// A stat-modified value, if the export has it.
    pub fn stat(&self, name: &str) -> Option<f13_core::stats::StatModifiedValue> {
        let v = self.stats.get(name)?;
        let mut modifiers = [0.0; f13_core::stats::STAT_COUNT];
        modifiers.copy_from_slice(&v[1..]);
        Some(f13_core::stats::StatModifiedValue { base: v[0], modifiers })
    }

    /// A float curve, if the export has it.
    pub fn curve(&self, name: &str) -> Option<f13_core::curve::RichCurve> {
        self.curves.get(name).map(|k| rich_curve(k))
    }
}

/// A physics asset body: its bone's largest shape ("Sphyl" capsule: radius, length between the sphere centres along
/// the shape's Z; "Sphere"; "Box": radius is half its smallest extent), relative to the bone, in cm.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RagdollBody {
    pub bone: String,
    pub shape: String,
    pub radius: f32,
    #[serde(default)]
    pub length: f32,
    #[serde(default)]
    pub transform: Option<Xform>,
    /// A box's full X, Y, Z extents (cm, Unreal).
    #[serde(default)]
    pub extents: Option<[f32; 3]>,
}

/// A physics constraint: `bone1` (the child) swings relative to `bone2` within the limits (degrees; 0 locked,
/// 180 free), measured between the joint's frames.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RagdollJoint {
    pub bone1: String,
    pub bone2: String,
    pub swing1: f32,
    pub swing2: f32,
    pub twist: f32,
    /// The joint's frames (X and Y axes, Unreal bone space): on `bone1`, and on `bone2`. Swing1 turns X toward Y,
    /// Swing2 toward Z.
    #[serde(default = "x_axis")]
    pub pri1: [f32; 3],
    #[serde(default = "y_axis")]
    pub sec1: [f32; 3],
    #[serde(default = "x_axis")]
    pub pri2: [f32; 3],
    #[serde(default = "y_axis")]
    pub sec2: [f32; 3],
}

fn x_axis() -> [f32; 3] {
    [1.0, 0.0, 0.0]
}

fn y_axis() -> [f32; 3] {
    [0.0, 1.0, 0.0]
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
    /// The reference pose (bone space, Unreal), from the mesh's bind pose.
    #[serde(default)]
    pub local: Option<crate::level::Xform>,
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
    /// Notify states with a duration (e.g. `AttackTraceNotify`, when a weapon swing can hit) and named notifies (zero
    /// length, e.g. `shouldRagdoll`), in seconds.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub windows: Vec<Window>,
    /// The montage's float curves (e.g. "Barricade"), keyed in seconds into it.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub curves: BTreeMap<String, Vec<CurveKey>>,
    /// The montage's sections (CompositeSections), in seconds into it.
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub sections: Vec<Window>,
    /// The montage's BlendIn time, when it sets one.
    #[serde(default)]
    pub blend_in: Option<f32>,
    /// The sequence's AuthoredSyncMarkers (LeftPlant / RightPlant on the cycles), by time (start = end).
    #[serde(default, deserialize_with = "crate::level::null_as_empty")]
    pub sync_markers: Vec<Window>,
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

    /// The montage's section called `name`: its start and end.
    pub fn section(&self, name: &str) -> Option<(f32, f32)> {
        self.sections.iter().find(|w| w.name == name).map(|w| (w.start, w.end))
    }

    /// The montage's float curve called `name`.
    pub fn curve(&self, name: &str) -> Option<f13_core::curve::RichCurve> {
        self.curves.get(name).map(|k| rich_curve(k))
    }
}

/// Exported RichCurve keys as an f13-core curve.
pub fn rich_curve(keys: &[CurveKey]) -> f13_core::curve::RichCurve {
    use f13_core::curve::{Interp, Key};
    let keys = keys
        .iter()
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
    f13_core::curve::RichCurve { keys }
}

/// An FRichCurve as Unreal serializes it (`{"Keys": [{"InterpMode": "RCIM_Cubic", "Time", "Value", "ArriveTangent",
/// "LeaveTangent"}]}`) as an f13-core curve.
pub fn ue_rich_curve(v: &serde_json::Value) -> f13_core::curve::RichCurve {
    use f13_core::curve::{Interp, Key};
    let f = |k: &serde_json::Value, n: &str| k[n].as_f64().unwrap_or(0.0) as f32;
    let keys = v["Keys"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|k| Key {
            time: f(k, "Time"),
            value: f(k, "Value"),
            interp: match k["InterpMode"].as_str().unwrap_or("RCIM_Linear") {
                "RCIM_Constant" => Interp::Constant,
                "RCIM_Cubic" => Interp::Cubic,
                _ => Interp::Linear,
            },
            arrive_tangent: f(k, "ArriveTangent"),
            leave_tangent: f(k, "LeaveTangent"),
        })
        .collect::<Vec<_>>();
    // No keys: FRichCurve::Eval gives its DefaultValue (MAX_flt when unset).
    if keys.is_empty() {
        if let Some(d) = v["DefaultValue"].as_f64().filter(|d| d.abs() < 1e30) {
            return f13_core::curve::RichCurve { keys: vec![Key { time: 0.0, value: d as f32, interp: Interp::Constant, arrive_tangent: 0.0, leave_tangent: 0.0 }] };
        }
    }
    f13_core::curve::RichCurve { keys }
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

    #[test]
    fn choose_outfit_swaps_the_part() {
        let mut info: CharacterInfo = serde_json::from_str(
            r#"{"Name":"A","Body":"b.glb","MeshTransform":{"T":[0,0,0],"R":[0,0,0,1],"S":[1,1,1]},"CapsuleHalfHeight":88,"Parts":[{"Name":"Outfit","File":"default.glb","Attach":"MasterPose"}],"SkinMasks":["face","default_mask"],
            "Bones":[],"Animations":{},"Outfits":[
              {"Class":"A_Default_Outfit","Name":"Default","File":"default.glb","Slots":[{"Own":"M","Options":["M","M2"]}]},
              {"Class":"A_Swimwear_Outfit","Name":"Swimwear","File":"swim.glb","SkinMask":"swim_mask","Slots":[{"Own":"S","Options":["S1","S2"]}]}]}"#,
        )
        .unwrap();
        assert_eq!(info.current_outfit(), Some((0, vec![0])));
        assert_eq!(info.choose_outfit("swim:1").unwrap(), "Swimwear");
        assert_eq!(info.current_outfit(), Some((1, vec![1])));
        assert_eq!(info.outfit_spec(1, &[1]).as_deref(), Some("A_Swimwear_Outfit:1"));
        assert_eq!(info.parts[0].file, "swim.glb");
        assert_eq!(info.parts[0].material_map.get("S").map(String::as_str), Some("S2"));
        assert_eq!(info.skin_masks, ["face", "swim_mask"]);
        assert_eq!(info.choose_outfit("Default").unwrap(), "Default");
        assert!(info.parts[0].material_map.is_empty());
        assert_eq!(info.skin_masks, ["face"]);
        assert!(info.choose_outfit("nope").is_err());
    }
}
