//! His kit: the grab kills on his four kill keys, the kills and weapons the export lists, and the loadout picked
//! in the lobby (his weapon, his grab kills).

use super::*;

/// His grab kills on the four kill keys (SCKillerCharacter's GrabKills: UseRightGrabKill 0, UseBottomGrabKill 1,
/// UseLeftGrabKill 2, UseTopGrabKill 3, used by FUN_1403ffa80; a click while holding her uses slot 1), as indexes into
/// [`KILLS`]. InitFromPlayerState (0x1403fb9f0) spawns each slot's kill from the player's picks (PlayerState
/// PickedGrabKills, from SCCharacterSelectionsSaveGame's killer profile; this port has no save) or else his
/// GrabKillClasses in order, the first empty one taking the weapon's DefaultGrabKill and ending the list: every
/// Jason keeps Jason_Base's GrabKillClasses[0] (Choke_GrabKill) and empties 1-3, so the choke on the right, his
/// weapon's kill on the bottom (and the click: J2's pickaxe throat hack), nothing on the left or top. From his
/// export (killers/<name>.json Names: GrabKillClass0..3, DefaultGrabKill), set as he loads; J2's until then. Each
/// slot the player picked (PickedGrabKills: his loadout, lobby.rs JasonLoadout) takes the pick instead.
pub fn grab_kill_slots() -> [Option<usize>; 4] {
    let slots = *GRAB_KILL_SLOTS.read().unwrap_or_else(|e| e.into_inner());
    slots.unwrap_or_else(|| {
        let k = |c: &str| kill_index(c);
        [k("Choke_GrabKill"), k("Pickaxe_ThroatHack_GrabKill"), None, None]
    })
}

pub(super) static GRAB_KILL_SLOTS: std::sync::RwLock<Option<[Option<usize>; 4]>> = std::sync::RwLock::new(None);
pub(super) static DEFAULT_SLOTS: std::sync::RwLock<Option<[Option<usize>; 4]>> = std::sync::RwLock::new(None);

/// His picks (kill classes, "" none) over his defaults.
pub fn set_grab_kill_picks(picks: &[String]) {
    *LAST_PICKS.write().unwrap_or_else(|e| e.into_inner()) = picks.to_vec();
    let Some(defaults) = *DEFAULT_SLOTS.read().unwrap_or_else(|e| e.into_inner()) else { return };
    let mut slots = defaults;
    for (i, p) in picks.iter().take(4).enumerate() {
        if let Some(k) = kill_index(p.trim_end_matches("_C")) {
            slots[i] = Some(k);
        }
    }
    *GRAB_KILL_SLOTS.write().unwrap_or_else(|e| e.into_inner()) = Some(slots);
}

/// A kill's index in [`kills`] by its class (without "_C").
pub fn kill_index(class: &str) -> Option<usize> {
    kills().iter().position(|k| k.class == class)
}

/// A kill by its take's name's end (`--kill curbstomp`).
pub fn kill_by_name(name: &str) -> Option<usize> {
    kills().iter().position(|k| k.his.ends_with(name))
}

/// His slots from his killer JSON's names (see [`grab_kill_slots`]); a kill this port doesn't have stays empty.
pub(crate) fn slots_from(names: &serde_json::Value) -> [Option<usize>; 4] {
    let index = |class: &str| kill_index(class.trim_end_matches("_C"));
    let mut slots = [None; 4];
    for (i, slot) in slots.iter_mut().enumerate() {
        match names[format!("GrabKillClass{i}")].as_str().unwrap_or("") {
            "" => {
                *slot = names["DefaultGrabKill"].as_str().and_then(index);
                break;
            }
            class => *slot = index(class),
        }
    }
    slots
}

/// The weapon whose kills the AI picks among (with the hand kills): his slots' weapon kill's (J2's pickaxe until set).
pub(super) fn own_weapon() -> &'static str {
    grab_kill_slots().iter().flatten().map(|&k| kills()[k].weapon.as_str()).find(|w| !w.is_empty()).unwrap_or("PickAxe_C")
}

/// The grab kills' KillBounds by [`KILLS`] index (DoesKillVolumeFit): the box's place in the kill's root
/// space (its component transform, Bevy) and half size (BoxExtent, m); and their ContextKillHUDIcon (its PNG):
/// KillerGrabWidget's icons (GetGrabKillIcon(slot, true)).
#[derive(Resource, Default)]
pub struct GrabKillInfo {
    pub bounds: Vec<Option<(Transform, Vec3)>>,
    pub icons: Vec<Option<String>>,
}

/// Which of his four grab kill slots he can use as he holds her (IsGrabKillAvailable 0x1403f5500): bGrabKillsActive
/// and the slot's kill's KillBounds fitting where he stands (SCDynamicContextKill::DoesKillVolumeFit 0x140379c40: an
/// overlap on WorldDynamic of the box, the kill attached at his feet facing his way (ReattachKill 0x140396600),
/// blocked by anything but him, her and the kill). A kill key on a slot that isn't (FUN_1403ffa80) does nothing.
/// bGrabKillsActive: FinishGrabCounselor (0x1403eb560) sets it once his grab montage's enter has played out
/// (UpdateGrabState 0x140324b40: the rest of its section and the GrabSuccess section; a timer for that long, its
/// lambda the guarded write at 0x1403eaf64), so none is available during the grab's enter take. Not ported:
/// IsGrabKillAvailable's other conditions (her vt+0xe90; slot 1's controller flag 0x3a8).
#[derive(Resource, Default, Debug, PartialEq)]
pub struct GrabKillsAvailable(pub [bool; 4]);

/// A grab kill: the weapon it needs (its WeaponClass; "" for a hand kill), his and her paired montage's take (their
/// special moves' ContextMontage), the kill class (without "_C": its camera and KillBounds) and its KillInfo's
/// KillName. The export's grab_kills.json (f13-export GrabKills: Kill_Select_Widget's NonWeaponGrabKills and
/// WeaponGrabKills, the kills the menu offers), else [`builtin_kills`].
#[derive(Clone, Debug)]
pub struct Kill {
    pub weapon: String,
    pub his: String,
    pub hers: String,
    pub class: String,
    pub name: String,
    /// Its FriendlyName and Description (the menus' picker), and its ContextKillHUDIcon (a texture name).
    pub title: String,
    pub description: String,
    pub icon: String,
}

pub(super) static KILL_TABLE: std::sync::OnceLock<Vec<Kill>> = std::sync::OnceLock::new();

/// The grab kills (see [`Kill`]).
pub fn kills() -> &'static [Kill] {
    KILL_TABLE.get_or_init(builtin_kills)
}

/// Reads grab_kills.json and weapons.json (before anything asks for [`kills`] or [`weapons`]).
pub fn load_kills(export: &std::path::Path) {
    load_weapons(export);
    let v: Option<serde_json::Value> = std::fs::read_to_string(export.join("grab_kills.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    let list: Vec<Kill> = v
        .as_ref()
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|k| {
            let s = |n: &str| k[n].as_str().map(str::to_string);
            Some(Kill {
                weapon: s("Weapon")?,
                his: s("HisTake")?,
                hers: s("HersTake")?,
                class: s("Class")?.trim_end_matches("_C").to_string(),
                name: s("KillName")?,
                title: s("Name").unwrap_or_default(),
                description: s("Description").unwrap_or_default(),
                icon: s("Icon").unwrap_or_default(),
            })
        })
        .collect();
    if list.is_empty() {
        warn!("no grab_kills.json (re-export: f13-export grabkills): the pickaxe's, the choke and the weapons' defaults only");
        return;
    }
    let _ = KILL_TABLE.set(list);
}

/// A Jason weapon (weapons.json: f13-export Weapons): its class, FriendlyName, ThumbnailImage (.png), ItemMesh3P
/// (.glb), WeaponStats.Damage, DefaultGrabKill, and its NeutralAttack / ForwardAttack takes (the light and heavy
/// swings: the pickaxe and axe two-handed, the pig splitter, shears, machete and fire axe one-handed, the spear's
/// lunge).
#[derive(Clone, Debug)]
pub struct WeaponInfo {
    pub class: String,
    pub name: String,
    pub image: String,
    pub mesh: String,
    pub damage: f32,
    pub default_grab_kill: String,
    pub light: String,
    pub heavy: String,
}

pub(super) static WEAPON_TABLE: std::sync::OnceLock<Vec<WeaponInfo>> = std::sync::OnceLock::new();

/// The weapons (see [`WeaponInfo`]); none before weapons.json.
pub fn weapons() -> &'static [WeaponInfo] {
    WEAPON_TABLE.get_or_init(Vec::new)
}

pub fn weapon_info(class: &str) -> Option<&'static WeaponInfo> {
    weapons().iter().find(|w| w.class == class)
}

pub(super) fn load_weapons(export: &std::path::Path) {
    let v: Option<serde_json::Value> = std::fs::read_to_string(export.join("weapons.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    let list: Vec<WeaponInfo> = v
        .as_ref()
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|w| {
            let s = |n: &str| w[n].as_str().map(str::to_string);
            Some(WeaponInfo {
                class: s("Class")?,
                name: s("Name").unwrap_or_default(),
                image: s("Image").unwrap_or_default(),
                mesh: s("Mesh").unwrap_or_default(),
                damage: w["Damage"].as_f64().unwrap_or(20.0) as f32,
                default_grab_kill: s("DefaultGrabKill").unwrap_or_default().trim_end_matches("_C").to_string(),
                light: s("LightTake").unwrap_or_default(),
                heavy: s("HeavyTake").unwrap_or_default(),
            })
        })
        .collect();
    if list.is_empty() {
        warn!("no weapons.json (re-export: f13-export weapons): every Jason swings the pickaxe's way");
    }
    let _ = WEAPON_TABLE.set(list);
}

/// The weapon his match uses (InitFromPlayerState: the PlayerState's picked weapon when valid, else his WeaponClass)
/// and a count of its changes ([`apply_weapon`]).
pub(super) static WEAPON: std::sync::RwLock<Option<&'static WeaponInfo>> = std::sync::RwLock::new(None);
pub(super) static WEAPON_CHANGES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// His own: his killer JSON's Menu (WeaponClass, AllowedWeaponClasses) and Names (the grab kill defaults), and the
/// last picks.
pub(super) static OWN: std::sync::RwLock<Option<(String, Vec<String>, serde_json::Value)>> = std::sync::RwLock::new(None);
pub(super) static LAST_PICKS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

/// The light and heavy swings' takes (his weapon's; the pickaxe's before weapons.json).
pub(crate) fn attack_take() -> &'static str {
    WEAPON.read().unwrap_or_else(|e| e.into_inner()).map(|w| w.light.as_str()).filter(|t| !t.is_empty()).unwrap_or(ATTACK)
}

pub(crate) fn heavy_take() -> &'static str {
    WEAPON.read().unwrap_or_else(|e| e.into_inner()).map(|w| w.heavy.as_str()).filter(|t| !t.is_empty()).unwrap_or(HEAVY_ATTACK)
}

/// The weapon he carries now (None: no weapons.json).
pub fn current_weapon() -> Option<&'static WeaponInfo> {
    *WEAPON.read().unwrap_or_else(|e| e.into_inner())
}

/// A Jason's weapon for a pick ("" or one he isn't allowed: his own).
pub fn weapon_for(own: &str, allowed: &[String], pick: &str) -> String {
    if !pick.is_empty() && (pick == own || allowed.iter().any(|a| a == pick)) && weapon_info(pick).is_some() {
        pick.to_string()
    } else {
        own.to_string()
    }
}

/// His weapon for the match ("" his own; one he isn't allowed: his own), his default slots following its DefaultGrabKill.
pub fn set_weapon(pick: &str) {
    let own = OWN.read().unwrap_or_else(|e| e.into_inner()).clone();
    let Some((own_class, allowed, names)) = own else { return };
    let class = weapon_for(&own_class, &allowed, pick);
    let Some(w) = weapon_info(&class) else { return };
    let changed = current_weapon().is_none_or(|c| c.class != w.class);
    *WEAPON.write().unwrap_or_else(|e| e.into_inner()) = Some(w);
    let mut names = names;
    names["DefaultGrabKill"] = serde_json::Value::String(format!("{}_C", w.default_grab_kill));
    *DEFAULT_SLOTS.write().unwrap_or_else(|e| e.into_inner()) = Some(slots_from(&names));
    let picks = LAST_PICKS.read().unwrap_or_else(|e| e.into_inner()).clone();
    set_grab_kill_picks(&picks);
    if changed {
        info!("Jason: his weapon {}", w.name);
        WEAPON_CHANGES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Whether his own weapon and grab kill defaults are known (his killer JSON read: [`load_knife_look`]).
pub fn own_ready() -> bool {
    OWN.read().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// A loadout to apply once his own defaults are known (killer_profile.rs JasonLoadout: the match's Jason player's).
#[derive(Resource, Clone, Debug)]
pub struct PendingLoadout(pub crate::killer_profile::JasonLoadout);

pub(super) fn apply_pending_loadout(mut commands: Commands, pending: Option<Res<PendingLoadout>>) {
    let Some(p) = pending.filter(|_| own_ready()) else { return };
    p.0.apply();
    commands.remove_resource::<PendingLoadout>();
}

/// A rig's weapon as shown ([`WEAPON_CHANGES`] when its mesh was last set).
#[derive(Component)]
pub(super) struct WeaponShown(u32);

/// As his weapon changes: his swings' lengths and windows, his damage (its WeaponStats.Damage x his modifiers), and the
/// mesh in his hand (the match's Jason's rig: its Weapon part's scene).
#[allow(clippy::type_complexity)]
pub(super) fn apply_weapon(
    mut commands: Commands,
    assets: Res<AssetServer>,
    killer: Option<Res<KillerCharacter>>,
    tuning: Option<ResMut<KillerTuning>>,
    rigs: Query<(Entity, &ChildOf, &crate::character::Rig, Option<&WeaponShown>)>,
    jasons: Query<(), With<Jason>>,
    mut tuned: Local<u32>,
) {
    let (Some(killer), Some(w)) = (killer, current_weapon()) else { return };
    let changes = WEAPON_CHANGES.load(std::sync::atomic::Ordering::Relaxed);
    if let Some(mut t) = tuning.filter(|_| *tuned != changes) {
        *tuned = changes;
        let info = &killer.0.info;
        let v = |k: &str, d: f32| info.values.get(k).copied().unwrap_or(d);
        let fresh = KillerTuning::new(info);
        t.attack_length = fresh.attack_length;
        t.attack_window = fresh.attack_window;
        t.heavy_length = fresh.heavy_length;
        t.heavy_window = fresh.heavy_window;
        t.damage = w.damage * v("DamageCharacterModifier", 1.0);
        t.world_damage = w.damage * v("DamageWorldModifier", 1.0);
    }
    let Some(index) = killer.0.info.parts.iter().position(|p| p.name == "Weapon") else { return };
    for (rig_entity, parent, rig, shown) in &rigs {
        if shown.is_some_and(|s| s.0 == changes) || !jasons.contains(parent.parent()) {
            continue;
        }
        let Some(part) = rig.part(index) else { continue };
        if !w.mesh.is_empty() {
            commands.entity(part).despawn_related::<Children>().insert(bevy::world_serialization::WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(w.mesh.clone()))));
        }
        commands.entity(rig_entity).insert(WeaponShown(changes));
    }
}

/// The kills before grab_kills.json: the pickaxe's, the choke, and the other Jasons' weapons' DefaultGrabKill.
pub(super) fn builtin_kills() -> Vec<Kill> {
    [
        ("PickAxe_C", "cinematic_pickaxe_throathack", "cinematic_pickaxe_throathack", "Pickaxe_ThroatHack_GrabKill", "Pickaxe Throat Hack"),
        ("PickAxe_C", "cinematic_pickaxe_head_canopener", "cinematic_pickaxe_head_canopener", "Pickaxe_Canopener_GrabKill", "Pickaxe Can Opener"),
        ("PickAxe_C", "cinematic_pickaxe_curbstomp", "cinematic_pickaxe_curbstomp", "PickAxe_CurbStomp_GrabKill", "Pickaxe Curb Stomp"),
        ("PickAxe_C", "cinematic_pickaxe_sweeptheleg", "cinematic_pickaxe_sweeptheleg", "PickAxe_SweepTheLeg_GrabKill", "Pickaxe Sweep The Leg"),
        ("PickAxe_C", "cinematic_pickaxe_toppopper", "cinematic_pickaxe_toppopper", "PickAxe_TopPopper_GrabKill", "Pickaxe Top Popper"),
        ("", "cinematic_hand_choke", "cinematic_hand_choke", "Choke_GrabKill", "Choke"),
        ("Axe_C", "cinematic_axe_crotchchop", "cinematic_axe_crotchchop", "Axe_CrotchChop_GrabKill", "Crotch Chop"),
        ("PigSplitter_C", "cinematic_pigsplitter_TheJarvis", "cinematic_pigsplitter_TheJarvis", "PigSplitter_TheJarvis_GrabKill", "TheJarvis"),
        ("Shears_C", "cinematic_shears_lastbreath", "cinematic_shears_lastbreath", "Shears_LastBreath_GrabKill", "Last Breath"),
        ("Spear_C", "cinematic_m_spear_head_knockdownstab_01", "cinematic_spear_head_knockdownstab_01", "Spear_KnockDown_GrabKill", "Spear Knockdown"),
        ("Machete_C", "cinematic_machete_eviscerate_stomach", "cinematic_machete_eviscerate_stomach", "Machete_Eviscerate_GrabKill", "Machete Eviscerate"),
        ("FireAxe_C", "cinematic_fireaxe_doubletap", "cinematic_fireaxe_doubletap", "Fireaxe_Doubletap_GrabKill", "Double Tap"),
    ]
    .into_iter()
    .map(|(w, his, hers, class, name)| Kill { weapon: w.into(), his: his.into(), hers: hers.into(), class: class.into(), name: name.into(), title: name.into(), description: String::new(), icon: String::new() })
    .collect()
}

/// IsGrabKillAvailable for each slot while he holds her (see [`GrabKillsAvailable`]).
#[allow(clippy::type_complexity)]
pub(super) fn grab_kills_available(
    info: Option<Res<GrabKillInfo>>,
    killer: Option<Res<KillerCharacter>>,
    mut available: ResMut<GrabKillsAvailable>,
    jasons: Query<(&Jason, &Transform)>,
    (collision, dynamic, no_world_dynamic, not_world_dynamic): (
        Option<Res<crate::collision::WorldCollision>>,
        Res<crate::collision::DynamicColliders>,
        Query<(), With<crate::collision::NoWorldDynamic>>,
        Option<Res<crate::collision::NotWorldDynamic>>,
    ),
) {
    // bGrabKillsActive: his grab's enter played out.
    let active = |j: &Jason| matches!(j.state, JasonState::Holding { t } if t >= killer.as_ref().and_then(|k| k.length(GRAB_ENTER)).unwrap_or(0.0));
    let Some((_, xf)) = jasons.iter().find(|(j, _)| j.holding() && active(j)) else {
        available.set_if_neq(GrabKillsAvailable([false; 4]));
        return;
    };
    let origin = Transform::from_matrix(xf.to_matrix() * Transform::from_rotation(crate::character::FACING).to_matrix());
    let fits = grab_kill_slots().map(|k| {
        // No kill in the slot: never available.
        let Some(k) = k else { return false };
        let (Some(world), Some((local, half))) = (collision.as_ref(), info.as_ref().and_then(|i| i.bounds[k])) else { return true };
        let w = origin.mul_transform(local);
        let b = f13_core::collision::Obb { centre: w.translation, axes: [w.rotation * Vec3::X, w.rotation * Vec3::Y, w.rotation * Vec3::Z], half: half * w.scale.abs() };
        let (grid, lists) = world.without(&dynamic, |e| no_world_dynamic.contains(e));
        let level = crate::collision::Excluding(grid, not_world_dynamic.as_ref().map(|g| &g.0));
        !f13_core::collision::box_blocked(&(&level, lists.as_slice()), &b)
    });
    if available.set_if_neq(GrabKillsAvailable(fits)) {
        info!("Jason: grab kills that fit (right, bottom, left, top): {fits:?}");
    }
}
