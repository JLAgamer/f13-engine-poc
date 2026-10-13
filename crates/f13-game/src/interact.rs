//! Counselor interactions: picking up items (one large item in the hands, up to `MaxSmallItems` small ones in the
//! pockets), opening cabinets, installing car parts and getting into the driver's seat (usable once the parts
//! installed from outside are in; the keys are used at the ignition from inside, see vehicle.rs).
//!
//! Controls: E interact (hold for repairs), G drop the held item, 1-3 use the small item in that pocket
//! (first aid: heals when its use animation ends; the counselor can't move meanwhile).
//! Repairs are the game's hold timer and pip minigame (f13-core::minigame): hold E; when the needle is in a pip,
//! press its key (right mouse = key 0, left mouse = key 1, the game's MG_InteractSkill1/2). Letting go cancels.
//! Cabinets: see cabinets.rs.

use crate::player::{Player, PlayerCamera, PlayerTuning};
use crate::doors::Door;
use crate::windows::{Barricade, Window, WindowAction};
use crate::match_flow::ClassScore;
use crate::props::{Cabinet, InCabinet, Item, Vehicle};
use crate::vehicle::Driving;
use bevy::prelude::*;
use f13_core::minigame::{HoldInteraction, MinigameEvent};

/// Counselor `MaxSmallItems` default (constructor).
/// Interaction distance is measured from about chest height.
pub(crate) const CHEST_M: f32 = 1.0;
/// ILLInteractableManagerComponent's constructor (0x1402524a0): its scoring weights (choose_target).
const DISTANCE_WEIGHT: f32 = 0.9;
const VIEW_WEIGHT: f32 = 0.7;
const CHARACTER_ROTATION_WEIGHT: f32 = 0.5;
const MOVEMENT_DIRECTION_WEIGHT: f32 = 0.8;
const BEST_INTERACTABLE_STABILITY_SCORE: f32 = 0.1;
/// ILLInteractableComponent's constructor: MaxYawOffset and MaxPitchOffset (degrees).
const DEFAULT_MAX_YAW: f32 = 15.0;
const DEFAULT_MAX_PITCH: f32 = 90.0;
/// APawn's BaseEyeHeight (no counselor Blueprint sets it): her eyes above her capsule's centre (GetPawnViewLocation),
/// where the interactables' occlusion traces start.
pub(crate) const BASE_EYE_HEIGHT_M: f32 = 0.64;

/// One interactable component's range test settings (ILLInteractableComponent, FUN_14022e940): where it is (actor
/// space; GetInteractionLocation is its location), its yaw and pitch limits (bUseYawLimit / bUsePitchLimit:
/// MaxYawOffset / MaxPitchOffset) and whether it checks for occlusion (bCheckForOcclusion; SCInteractComponent's
/// constructor turns it on).
#[derive(Clone, Debug)]
pub struct InteractLimit {
    pub at: Transform,
    pub yaw: Option<f32>,
    pub pitch: Option<f32>,
    pub occlusion: bool,
    /// Its occlusion trace runs on Visibility (RayTraceChannel ECC_Visibility: SCInteractComponents), else WorldDynamic.
    pub visibility: bool,
}

/// An actor's interactable components, by name.
#[derive(Component, Clone, Default)]
pub struct InteractLimits(pub std::collections::HashMap<String, InteractLimit>);

impl InteractLimits {
    pub fn of(classes: &f13_data::classes::ClassDb, class: &str) -> Self {
        let Some(info) = classes.get(class) else { return Self::default() };
        Self(
            info.components
                .iter()
                .filter(|c| classes.is_child_of(&c.kind, "ILLInteractableComponent"))
                .map(|c| {
                    let p = &c.props;
                    let f = |k: &str, d: f32| p[k].as_f64().map_or(d, |v| v as f32);
                    let limit = InteractLimit {
                        at: crate::to_transform(&c.local.unwrap_or(f13_data::level::Xform::IDENTITY)),
                        yaw: p["bUseYawLimit"].as_bool().unwrap_or(false).then(|| f("MaxYawOffset", DEFAULT_MAX_YAW)),
                        pitch: p["bUsePitchLimit"].as_bool().unwrap_or(false).then(|| f("MaxPitchOffset", DEFAULT_MAX_PITCH)),
                        occlusion: p["bCheckForOcclusion"].as_bool().unwrap_or(classes.is_child_of(&c.kind, "SCInteractComponent")),
                        visibility: p["RayTraceChannel"].as_str() == Some("ECC_Visibility"),
                    };
                    (c.name.clone(), limit)
                })
                .collect(),
        )
    }
}

impl InteractLimit {
    /// The range test's facing part: the direction from the interactable to her (`to_her`) within its limits of its
    /// facing, and her view within them of facing back at it (the manager's bUseCameraRotation).
    pub fn facing(&self, owner: &GlobalTransform, to_her: Vec3, view: Vec3) -> bool {
        if self.yaw.is_none() && self.pitch.is_none() {
            return true;
        }
        let forward = (owner.rotation() * self.at.rotation * Vec3::X).normalize_or(Vec3::X);
        // Yaw and pitch (degrees) of a direction, as Unreal's rotator of it.
        let yaw = |v: Vec3| v.z.atan2(v.x).to_degrees();
        let pitch = |v: Vec3| v.y.atan2(Vec2::new(v.x, v.z).length()).to_degrees();
        let off = |a: f32| ((a + 180.0).rem_euclid(360.0) - 180.0).abs();
        let within = |v: Vec3, f: Vec3| self.yaw.is_none_or(|l| off(yaw(v) - yaw(f)) <= l) && self.pitch.is_none_or(|l| off(pitch(v) - pitch(f)) <= l);
        within(to_her, forward) && within(view, -forward)
    }
}

#[derive(Component, Default, Debug)]
pub struct Inventory {
    pub hand: Option<Entity>,
    /// Her small items (SmallItemInventory), at most `max_small` (MaxSmallItems).
    pub pockets: Vec<Entity>,
    /// Special items (SpecialItemInventory: the map).
    pub special: Vec<Entity>,
    pub max_small: usize,
    /// SmallItemIndex: the selected pocket. Like the game's it can point past the last one (then none is).
    pub selected: usize,
    /// The selected small item, to be used this frame (G, or 1-3 once its pocket is selected).
    pub use_small: Option<Entity>,
}

impl Inventory {
    /// GetCurrentSmallItem: the selected pocket's item.
    pub fn current_small(&self) -> Option<Entity> {
        self.pockets.get(self.selected).copied()
    }
}

/// Class of the first aid spray.
pub const FIRST_AID: &str = "BP_FirstAid_C";
/// The door barricade's class (its score event).
const BARRICADE: &str = "Barricade_C";

/// The counselor is using a pocket item (playing its use animation).
#[derive(Component)]
pub struct UsingItem {
    pub item: Entity,
    pub left: f32,
}

/// An item someone is holding or carrying in a pocket.
#[derive(Component)]
pub struct Carried;

/// Takes the first carried item of one of `classes` out of the inventory (hands first, then pockets).
pub fn take_part(inv: &mut Inventory, items: &Query<&Item>, classes: &[String]) -> Option<Entity> {
    let fits = |e: &Entity| items.get(*e).is_ok_and(|it| classes.contains(&it.class));
    let used = inv.hand.filter(fits).or_else(|| inv.pockets.iter().copied().find(fits))?;
    if inv.hand == Some(used) {
        inv.hand = None;
    }
    inv.pockets.retain(|&p| p != used);
    Some(used)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Item(Entity),
    /// A cabinet's drawer (cabinets.rs).
    Cabinet(Entity, usize),
    Repair(Entity, usize),
    /// A car's seat: the driver's (none) or a passenger's.
    Seat(Entity, Option<usize>),
    Door(Entity),
    Window(Entity),
    Hide(Entity),
    /// A destroyed fuse box to repair (power.rs).
    FuseBox(Entity),
    /// The phone box, to put its fuse in; the phone, to call the police (police.rs).
    PhoneBox(Entity),
    Phone(Entity),
    /// A radio to turn on or off, or (held while it plays) to change its song (radio.rs).
    Radio(Entity),
    /// The CB radio, to call Tommy Jarvis (tommy.rs).
    CbRadio(Entity),
}

impl Interaction {
    /// What she'd use, for logs (testing).
    pub(crate) fn target_name(&self) -> String {
        format!("{:?}", self.target)
    }

    /// She'd use the CB radio (testing).
    pub(crate) fn on_cb_radio(&self) -> bool {
        matches!(self.target, Some(Target::CbRadio(_)))
    }
}

/// A counselor's interaction: what she'd use, and the hold in progress.
#[derive(Component, Default)]
pub struct Interaction {
    target: Option<Target>,
    /// The hold interaction (repair) in progress.
    pub hold: Option<HoldInteraction>,
    /// A hold finished while E is still down: its release isn't a press.
    hold_done: bool,
    /// How long she's held the drop key (small_items).
    drop_held: Option<f32>,
    /// How long she's held E on a playing radio (its press or, past HoldTimeLimit, its hold).
    radio_held: Option<f32>,
}

impl Interaction {
    /// The counselor is busy with a started hold interaction (can't walk).
    pub fn busy(&self) -> bool {
        self.hold.as_ref().is_some_and(|h| h.started)
    }
}

#[derive(Component)]
struct PromptText;

/// The counselor opening or closing a door (its special move's montage: her `f_` take, timed by the shared montage's
/// `m_` take): the leaf swings during the montage's OpenCloseDoor driver window; she stands while it plays.
#[derive(Component)]
pub struct DoorMove {
    door: Entity,
    t: f32,
    /// The OpenCloseDoor window (s) and the montage's length.
    swing: (f32, f32),
    /// Its DoorRotation curve (the leaf follows it in the window).
    curve: Option<f13_core::curve::RichCurve>,
    length: f32,
    swung: bool,
    /// Played on its own root motion: where it plays from and where it leaves her.
    root: Option<(Transform, Transform)>,
}

/// The counselor working a window (its special moves' montages, her takes): opening and closing move the pane in the
/// montage's WindowMoveDriver window; climbing through and diving carry her from her side to the landing over the
/// take (its root motion, here a straight line); a dive breaks the glass at its WindowBreakDriver and ends in the
/// landing (f_normal_dive_land_01). She can't do anything else meanwhile.
#[derive(Component)]
pub struct WindowMove {
    pub(crate) window: Entity,
    pub(crate) kind: WindowMoveKind,
    t: f32,
    from: Vec3,
    to: Vec3,
    effect_done: bool,
    /// Played on its own root motion: where it plays from and where it leaves her.
    root: Option<(Transform, Transform)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowMoveKind {
    Open,
    Close,
    Vault,
    Dive,
    Land,
}

impl WindowMoveKind {
    /// Her take, its length, and when its effect happens (the pane moving, the glass breaking).
    fn take(self) -> (&'static str, f32, f32) {
        match self {
            WindowMoveKind::Open => ("f_normal_stand_window_up", 0.97, 0.2),
            WindowMoveKind::Close => ("f_normal_stand_window_down", 0.97, 0.3),
            WindowMoveKind::Vault => ("f_normal_stand_window_climb_01", 2.53, 0.0),
            WindowMoveKind::Dive => ("f_normal_stand_window_dive", 0.57, 0.2),
            WindowMoveKind::Land => ("f_normal_dive_land_01", 3.53, 0.0),
        }
    }
}

fn start_window_move(commands: &mut Commands, player: Entity, window: Entity, kind: WindowMoveKind, from: Vec3, to: Vec3) {
    commands.entity(player).insert((WindowMove { window, kind, t: 0.0, from, to, effect_done: false, root: None }, crate::character::PlayAction(kind.take().0.into())));
}

/// Through the window on her take's own root motion, from its special move spot: where the take plays from and
/// where it leaves her.
fn window_root_move(window: Entity, kind: WindowMoveKind, root: (Transform, Transform)) -> (WindowMove, crate::character::PlayAction) {
    let m = WindowMove { window, kind, t: 0.0, from: root.0.translation, to: root.1.translation, effect_done: false, root: Some(root) };
    (m, crate::character::PlayAction(kind.take().0.into()))
}

fn window_moves(
    mut commands: Commands,
    time: Res<Time>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    mut players: Query<(Entity, &mut WindowMove, &mut Transform), With<Player>>,
    mut windows: Query<&mut Window>,
    mut rigs: Query<(&ChildOf, &mut crate::character::Rig)>,
) {
    for (pe, mut m, mut xf) in &mut players {
        m.t += time.delta_secs();
        let (take, length, effect_at) = m.kind.take();
        let length = model.as_ref().and_then(|c| c.length(take)).unwrap_or(length);
        match m.root {
            // Her take's root motion carries her; she stays where it plays from.
            Some((origin, _)) => *xf = origin,
            None if matches!(m.kind, WindowMoveKind::Vault | WindowMoveKind::Dive) => {
                let a = (m.t / length).clamp(0.0, 1.0);
                xf.translation = m.from.lerp(m.to, a);
            }
            None => {}
        }
        if !m.effect_done && m.t >= effect_at {
            m.effect_done = true;
            if let Ok(mut w) = windows.get_mut(m.window) {
                match m.kind {
                    WindowMoveKind::Open => w.open = true,
                    WindowMoveKind::Close => w.open = false,
                    WindowMoveKind::Dive if !w.open => w.destroyed = true,
                    _ => {}
                }
            }
        }
        if m.t >= length {
            // Where the take left her.
            if let Some((_, end)) = m.root {
                *xf = end;
                if let Some((_, mut rig)) = rigs.iter_mut().find(|(c, _)| c.parent() == pe) {
                    rig.snap();
                }
            }
            if m.kind == WindowMoveKind::Dive {
                // On to the landing, where she came down.
                let w = m.window;
                let land = WindowMoveKind::Land.take().0;
                match (m.root, model.as_ref()) {
                    (Some((_, end)), Some(c)) if c.root_motion(land).is_some() => {
                        commands.entity(pe).insert(window_root_move(w, WindowMoveKind::Land, c.place(land, end)));
                    }
                    _ => {
                        let to = m.to;
                        start_window_move(&mut commands, pe, w, WindowMoveKind::Land, to, to);
                    }
                }
            } else {
                commands.entity(pe).remove::<(WindowMove, crate::character::PlayAction)>();
            }
        }
    }
}

/// She's repairing (her take of the repair's montage): its start, then its cycle while she works, then its stop
/// (done) or cancel (let go).
#[derive(Component)]
pub struct RepairAnim {
    base: String,
    part: &'static str,
    t: f32,
}

/// Her take of a repair's part ("start", "cycle", "stop", "cancel"): `{base}_{part}`, except the phone box's
/// (enter, cycle, exit, fail).
fn repair_take(base: &str, part: &str) -> String {
    // The CB radio's montage (AM_normal_stand_use_CB_radio): Enter, Idle (looping) and Exit sections, the exit both
    // for a done call and a stopped one.
    if base == crate::tommy::RADIO_TAKE {
        return match part {
            "start" => "m_normal_stand_use_radio_enter",
            "cycle" => "m_normal_stand_use_radio_cycle_01",
            _ => "f_normal_stand_use_radio_exit",
        }
        .to_string();
    }
    let part = match (base == crate::police::PHONE_BOX_TAKE, part) {
        (true, "start") => "enter",
        (true, "stop") => "exit",
        (true, "cancel") => "fail",
        _ => part,
    };
    format!("{base}_{part}")
}

fn repair_start(base: &str) -> (RepairAnim, crate::character::PlayAction) {
    (RepairAnim { base: base.to_string(), part: "start", t: 0.0 }, crate::character::PlayAction(repair_take(base, "start")))
}

/// On a special move's spot (world), facing its forward (Unreal +X; she faces +Z), on her floor.
pub(crate) fn standing_on(player: &Transform, at: Transform) -> Transform {
    let (yaw, _, _) = at.rotation.to_euler(EulerRot::YXZ);
    Transform { translation: Vec3::new(at.translation.x, player.translation.y, at.translation.z), rotation: Quat::from_rotation_y(yaw + std::f32::consts::FRAC_PI_2), ..*player }
}

/// SCSpecialMove_SoloMontage's InterpolationTime (its constructor; no counselor move overrides it).
pub const INTERPOLATION_TIME_S: f32 = 0.15;

/// A special move's way onto its spot (special_moves.json: SCSpecialMove_SoloMontage's InterpolationTime,
/// MoveThreshold, bSweepTestOnLerp; its constructor's 0.15 s, 15 cm and true when a move isn't listed).
#[derive(Clone, Copy, Debug)]
pub struct MoveParams {
    pub interp_time: f32,
    pub threshold_m: f32,
    pub sweep: bool,
}

impl Default for MoveParams {
    fn default() -> Self {
        MoveParams { interp_time: INTERPOLATION_TIME_S, threshold_m: 0.15, sweep: true }
    }
}

/// The special moves' parameters, and which move each actor class's special move components start (their
/// SpecialMove property).
#[derive(Resource, Default)]
pub struct SpecialMoves {
    moves: std::collections::HashMap<String, MoveParams>,
    by_component: std::collections::HashMap<(String, String), String>,
    /// The same by the component's parent (a repair's special move hangs off the repair component).
    by_parent: std::collections::HashMap<(String, String), String>,
}

impl SpecialMoves {
    /// The move `component` of `owner_class` starts: its parameters.
    pub fn of(&self, owner_class: &str, component: &str) -> MoveParams {
        self.by_component.get(&(owner_class.to_string(), component.to_string())).and_then(|m| self.moves.get(m)).copied().unwrap_or_default()
    }

    /// The move of the special move component under `parent` (a repair component) of `owner_class`.
    pub fn of_child(&self, owner_class: &str, parent: &str) -> MoveParams {
        self.by_parent.get(&(owner_class.to_string(), parent.to_string())).and_then(|m| self.moves.get(m)).copied().unwrap_or_default()
    }
}

fn load_special_moves(mut commands: Commands, export: Res<crate::ExportRoot>) {
    let mut s = SpecialMoves::default();
    match std::fs::read_to_string(export.0.join("special_moves.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) {
        Some(v) => {
            for (name, m) in v.as_object().into_iter().flatten() {
                let f = |k: &str, d: f32| m[k].as_f64().map_or(d, |x| x as f32);
                s.moves.insert(
                    name.clone(),
                    MoveParams { interp_time: f("InterpolationTime", INTERPOLATION_TIME_S), threshold_m: f("MoveThreshold", 15.0) / 100.0, sweep: m["SweepTestOnLerp"].as_bool().unwrap_or(true) },
                );
            }
        }
        None => warn!("no special_moves.json (re-export with f13-export levels): the special moves' constructor defaults"),
    }
    if let Ok(classes) = f13_data::classes::ClassDb::load(&export.0.join("classes.json")) {
        for (class, info) in &classes.classes {
            for c in &info.components {
                if let Some(m) = c.props["SpecialMove"].as_str() {
                    s.by_component.insert((class.clone(), c.name.clone()), m.to_string());
                    if let Some(parent) = &c.parent {
                        s.by_parent.insert((class.clone(), parent.clone()), m.to_string());
                    }
                }
            }
        }
    }
    commands.insert_resource(s);
}

/// SCSpecialMove_SoloMontage's way onto its spot (its tick 0x140448320 with bInterpToInteraction, which every counselor
/// interaction move keeps): her transform goes from where she stood toward the spot over its InterpolationTime
/// (location lerped, rotation by normalised lerp), her capsule swept there each frame when bSweepTestOnLerp (stopped
/// where it's blocked). At the end she's set on the spot (swept); blocked, she's put there anyway when within its
/// MoveThreshold (XY), else the move is cancelled (CancelSpecialMove). Then the move begins (DestinationReached: its
/// montage and its camera). A take played on its own root motion is then set where its first frame stands on the
/// spot (`then`: Unreal keeps the actor on the spot and applies the root's motion from there).
#[derive(Component)]
pub struct MoveToSpot {
    from: Transform,
    to: Transform,
    then: Option<Transform>,
    t: f32,
    params: MoveParams,
    begin: Option<Box<dyn FnOnce(&mut World) + Send + Sync>>,
}

/// Starts a special move from `to`: she slides there, then `begin` runs.
pub fn move_to_spot(commands: &mut Commands, player: Entity, from: Transform, to: Transform, params: MoveParams, begin: impl FnOnce(&mut World) + Send + Sync + 'static) {
    commands.entity(player).insert(MoveToSpot { from, to, then: None, t: 0.0, params, begin: Some(Box::new(begin)) });
}

/// The same for a take on its own root motion: she slides onto `spot`, then stands at `origin` (where the take's
/// first frame stands on the spot).
#[allow(clippy::too_many_arguments)]
pub fn move_to_rooted_spot(commands: &mut Commands, player: Entity, from: Transform, spot: Transform, origin: Transform, params: MoveParams, begin: impl FnOnce(&mut World) + Send + Sync + 'static) {
    commands.entity(player).insert(MoveToSpot { from, to: spot, then: Some(origin), t: 0.0, params, begin: Some(Box::new(begin)) });
}

fn moves_to_spots(
    mut commands: Commands,
    time: Res<Time>,
    collision: Res<crate::collision::WorldCollision>,
    dynamic: Res<crate::collision::DynamicColliders>,
    mut players: Query<(Entity, &mut Transform, &mut MoveToSpot, &mut Interaction), With<Player>>,
) {
    let world = collision.with(&dynamic);
    let sweep = |from: Vec3, to: Vec3| f13_core::collision::capsule_sweep(&world, from, to, crate::player::CAPSULE_RADIUS_M, crate::player::CAPSULE_HALF_HEIGHT_M);
    for (pe, mut xf, mut m, mut interaction) in &mut players {
        m.t += time.delta_secs();
        let a = (m.t / m.params.interp_time.max(1e-4)).clamp(0.0, 1.0);
        if a >= 1.0 {
            let blocked = m.params.sweep && sweep(xf.translation, m.to.translation).is_some();
            let near = Vec2::new(xf.translation.x - m.to.translation.x, xf.translation.z - m.to.translation.z).length() <= m.params.threshold_m;
            commands.entity(pe).remove::<MoveToSpot>();
            if blocked && !near {
                info!(
                    "special move cancelled: blocked on the way to its spot ({:.2}, {:.2}, {:.2}), stopped at ({:.2}, {:.2}, {:.2}) from ({:.2}, {:.2}, {:.2})",
                    m.to.translation.x, m.to.translation.y, m.to.translation.z, xf.translation.x, xf.translation.y, xf.translation.z, m.from.translation.x, m.from.translation.y, m.from.translation.z
                );
                interaction.hold = None;
                interaction.target = None;
                continue;
            }
            *xf = m.then.unwrap_or(m.to);
            if let Some(begin) = m.begin.take() {
                commands.queue(begin);
            }
            continue;
        }
        let (from, to) = (m.from.rotation, m.to.rotation);
        let to = if from.dot(to) < 0.0 { -to } else { to };
        let mut next = Transform { translation: m.from.translation.lerp(m.to.translation, a), rotation: (from * (1.0 - a) + to * a).normalize(), ..m.to };
        if m.params.sweep {
            if let Some(hit) = sweep(xf.translation, next.translation) {
                next.translation = xf.translation.lerp(next.translation, hit);
            }
        }
        *xf = next;
    }
}

/// A repair take playing on its own root motion (the montage's: Unreal's default RootMotionFromMontagesOnly): she
/// stays at `origin` while its root carries her mesh, and is left where it got to when it ends or is cut off. The
/// phone box's enter (42.6 cm towards the box) and exit (back), the gas can's cancel (56 cm back).
#[derive(Component)]
struct RepairRoot {
    name: String,
    origin: Transform,
    t: f32,
}

#[allow(clippy::type_complexity)]
fn repair_anims(
    mut commands: Commands,
    time: Res<Time>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    mut players: Query<(Entity, &mut Transform, &mut RepairAnim, Option<&mut RepairRoot>), With<Player>>,
    loose: Query<Entity, (With<RepairRoot>, Without<RepairAnim>)>,
) {
    // Taken off her repair by something else (a car seat, a grab): that places her.
    for e in &loose {
        commands.entity(e).remove::<RepairRoot>();
    }
    let dt = time.delta_secs();
    let len = |n: &str| model.as_ref().and_then(|m| m.length(n)).unwrap_or(1.0);
    for (pe, mut xf, mut r, root) in &mut players {
        r.t += dt;
        let name = repair_take(&r.base, r.part);
        // Only while she still stands at its origin: anything else that moved her (a move onto another spot) places her.
        let rooted = root.as_ref().filter(|rr| rr.origin.translation == xf.translation).map(|rr| rr.name.clone());
        if let Some(model) = model.as_ref() {
            let settle = |rr: &RepairRoot| model.root_at(&rr.name, rr.t).map(|m| crate::character::flat(Transform::from_matrix(rr.origin.to_matrix() * m.to_matrix())));
            match root {
                Some(mut rr) if rr.name == name => rr.t += dt,
                other => {
                    if let Some(at) = other.as_deref().filter(|_| rooted.is_some()).and_then(settle) {
                        *xf = at;
                    }
                    if other.is_some() {
                        commands.entity(pe).remove::<RepairRoot>();
                    }
                    if model.root_motion(&name).is_some() {
                        let (origin, _) = model.place(&name, *xf);
                        *xf = origin;
                        commands.entity(pe).insert(RepairRoot { name: name.clone(), origin, t: dt });
                    }
                }
            }
        }
        let over = r.t >= len(&name);
        match r.part {
            "start" if over => {
                r.part = "cycle";
                r.t = 0.0;
                commands.entity(pe).insert(crate::character::PlayAction(repair_take(&r.base, "cycle")));
            }
            "stop" | "cancel" if over => {
                // Where its root motion left her.
                // Where its root motion left her (she's stood at its origin while it played).
                if let Some((_, last)) = model.as_ref().filter(|_| rooted.as_ref() == Some(&name)).and_then(|m| m.root_motion(&name)) {
                    *xf = crate::character::flat(Transform::from_matrix(xf.to_matrix() * last.to_matrix()));
                }
                commands.entity(pe).remove::<(RepairAnim, RepairRoot, crate::character::PlayAction)>();
            }
            _ => {}
        }
    }
}

/// Ends her repair take: its stop (done) or cancel (let go).
fn end_repair_anim(commands: &mut Commands, player: Entity, base: Option<&String>, done: bool) {
    // Let go while she's still sliding onto the spot: the move never begins.
    commands.entity(player).remove::<MoveToSpot>();
    let Some(base) = base else { return };
    let part = if done { "stop" } else { "cancel" };
    commands.entity(player).insert((RepairAnim { base: base.clone(), part, t: 0.0 }, crate::character::PlayAction(repair_take(base, part))));
}

/// She's playing a door's lock or unlock take while she holds E on it.
#[derive(Component)]
pub struct LockAnim;

/// Her take of a door montage: opening or closing, from inside or outside.
/// The door's special move spot for each of her door takes (the door Blueprints' SpecialMoveHandlers).
fn door_spot(anim: &str) -> &'static str {
    match anim {
        "f_normal_stand_door_open_inside_forward" => "OpenDoorInside_SpecialMoveHandler",
        "f_normal_stand_door_open_outside_forward" => "OpenDoorOutside_SpecialMoveHandler",
        "f_normal_stand_door_close_inside_forward" => "CloseDoorInsideForward_SpecialMoveHandler",
        "f_normal_stand_door_close_outside_forward" => "CloseDoorOutsideForward_SpecialMoveHandler",
        "f_normal_stand_door_lock" => "LockDoorInside_SpecialMoveHandler",
        _ => "",
    }
}

/// Her take at a door played from its special move spot (upright at her height), on its own root motion: where it plays
/// from and where it leaves her. None without the spot or the take's root.
fn door_root(door: &Door, gxf: &GlobalTransform, at_y: f32, anim: &str, model: Option<&crate::character::CounselorCharacter>) -> Option<(Transform, Transform)> {
    let spot = door.spots.get(door_spot(anim))?;
    let model = model.filter(|m| m.root_motion(anim).is_some())?;
    let mut at = crate::hiding::HidingSpot::world(gxf, spot);
    at.translation.y = at_y;
    Some(model.place(anim, at))
}

fn door_anim(opening: bool, inside: bool) -> &'static str {
    match (opening, inside) {
        (true, true) => "f_normal_stand_door_open_inside_forward",
        (true, false) => "f_normal_stand_door_open_outside_forward",
        (false, true) => "f_normal_stand_door_close_inside_forward",
        (false, false) => "f_normal_stand_door_close_outside_forward",
    }
}

/// She's moving a door's barricade (Counselor_Barricade_Pulldown / PushUp): her take, the barricade going up or down
/// as its UseBarricade driver window ends.
#[derive(Component)]
pub struct BarricadeMove {
    pub(crate) door: Entity,
    t: f32,
    /// The montage's UseBarricade driver window: the plank follows her "Barricade" curve in it, and the barricade
    /// toggles at its end.
    driver: (f32, f32),
    curve: Option<f13_core::curve::RichCurve>,
    length: f32,
    flipped: bool,
}

/// Her takes pulling a barricade down and pushing it up.
const BARRICADE_DOWN: &str = "f_normal_stand_barricade_pulldown";
const BARRICADE_UP: &str = "f_normal_stand_barricade_pushup";

fn barricade_moves(
    mut commands: Commands,
    time: Res<Time>,
    mut players: Query<(Entity, &mut BarricadeMove), With<Player>>,
    mut barricades: Query<&mut Barricade>,
    mut transforms: Query<&mut Transform, Without<Player>>,
) {
    for (pe, mut m) in &mut players {
        m.t += time.delta_secs();
        // UseBarricadeDriverTick.
        if let (Some(c), false) = (&m.curve, m.flipped) {
            if (m.driver.0..m.driver.1).contains(&m.t) {
                let plank = barricades.iter().find(|b| b.door == Some(m.door)).and_then(|b| b.plank_at(crate::windows::PLANK_PITCH * c.eval(m.t).clamp(0.0, 1.0)));
                if let Some((e, at)) = plank {
                    if let Ok(mut t) = transforms.get_mut(e) {
                        *t = at;
                    }
                }
            }
        }
        if !m.flipped && m.t >= m.driver.1 {
            m.flipped = true;
            if let Some(mut b) = barricades.iter_mut().find(|b| b.door == Some(m.door)) {
                flip_barricade(&mut commands, &mut b, pe);
            }
        }
        if m.t >= m.length {
            commands.entity(pe).remove::<(BarricadeMove, crate::character::PlayAction)>();
        }
    }
}

fn flip_barricade(commands: &mut Commands, b: &mut Barricade, who: Entity) {
    b.enabled = !b.enabled;
    if b.enabled {
        commands.trigger(ClassScore::new(who, BARRICADE, "BarricadeDoorScoreEvent"));
    }
    info!("door {}", if b.enabled { "barricaded" } else { "unbarricaded" });
}

/// Swings the door at the montage's window, and lets her go at its end (where its root motion took her).
fn door_moves(
    mut commands: Commands,
    time: Res<Time>,
    mut players: Query<(Entity, &mut DoorMove, &mut Transform), With<Player>>,
    mut doors: Query<(&mut Door, &GlobalTransform)>,
    mut rigs: Query<(&ChildOf, &mut crate::character::Rig)>,
) {
    for (pe, mut m, mut xf) in &mut players {
        m.t += time.delta_secs();
        if let Some((origin, end)) = m.root {
            *xf = if m.t >= m.length { end } else { origin };
            if m.t >= m.length {
                if let Some((_, mut rig)) = rigs.iter_mut().find(|(c, _)| c.parent() == pe) {
                    rig.snap();
                }
            }
        }
        if !m.swung && m.t >= m.swing.0 {
            m.swung = true;
            if let Ok((mut door, gxf)) = doors.get_mut(m.door) {
                let what = door.press(gxf, xf.translation);
                door.drive(m.curve.clone(), m.t, m.swing.1);
                info!("door {what}");
            }
        }
        if m.t >= m.length {
            commands.entity(pe).remove::<(DoorMove, crate::character::PlayAction)>();
        }
    }
}

/// NativeSpecialComplete / NativeSpecialAborted: however her door move ends, the view goes back to her.
fn door_move_ended(_: On<Remove, DoorMove>, mut commands: Commands) {
    commands.trigger(crate::interaction_camera::StopInteractionCamera { killer: false });
}

/// The same for a window's open or close move (its vault and dive handlers have no camera).
fn window_move_ended(_: On<Remove, WindowMove>, mut commands: Commands) {
    commands.trigger(crate::interaction_camera::StopInteractionCamera { killer: false });
}

pub struct InteractPlugin;

impl Plugin for InteractPlugin {
    fn build(&self, app: &mut App) {
        app.add_observer(door_move_ended)
            .add_observer(window_move_ended)
            .add_systems(Startup, (spawn_prompt, load_special_moves))
            .add_systems(
            Update,
            (add_inventory, crate::windows::link_barricades, choose_target, act.run_if(crate::net::simulating), crate::cabinets::cabinet_uses.run_if(crate::net::simulating), crate::cabinets::follow_items, moves_to_spots.run_if(crate::net::simulating), door_moves.run_if(crate::net::simulating), window_moves.run_if(crate::net::simulating), barricade_moves.run_if(crate::net::simulating), repair_anims.run_if(crate::net::simulating), small_items.run_if(crate::net::simulating), dropping.run_if(crate::net::simulating), use_item.run_if(crate::net::simulating), update_prompt)
                .chain()
                .before(crate::doors::swing_doors),
        )
        .add_systems(Update, (crate::doors::swing_doors, crate::windows::update_window_visuals, crate::windows::place_planks));
    }
}

fn spawn_prompt(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(18.0), ..default() },
        // Under the HUD's timer (the inventory takes the bottom left).
        Node { position_type: PositionType::Absolute, top: Val::Percent(20.0), left: Val::Px(12.0), ..default() },
        PromptText,
    ));
}

fn add_inventory(mut commands: Commands, players: Query<Entity, (With<Player>, Without<Inventory>)>, model: Option<Res<crate::character::CounselorCharacter>>) {
    let Some(model) = model else { return };
    for p in &players {
        // MaxSmallItems (her class's; the constructor's 3 for an older export).
        let max_small = model.info.values.get("MaxSmallItems").map_or(3, |&v| v as usize);
        commands.entity(p).insert((Inventory { max_small, ..default() }, Interaction::default()));
    }
}

fn has_part(inv: &Inventory, carried: &Query<&Item, With<Carried>>, parts: &[String]) -> bool {
    inv.hand.iter().chain(inv.pockets.iter()).filter_map(|&e| carried.get(e).ok()).any(|i| parts.contains(&i.class))
}

/// ILLInteractableManagerComponent's tick (FUN_140279100; weights from its constructor 0x1402524a0): of the
/// interactables in range (3D distance from her capsule's centre to the interaction point within its DistanceLimit),
/// the highest score = ViewWeight x (view . dir) + DistanceWeight x (1 - flat distance / DistanceLimit) +
/// CharacterRotationWeight x (facing . dir) + MovementDirectionWeight x (moving direction . dir, when moving), plus
/// BestInteractableStabilityScore for the current one; dir is flat from her to the point
/// (bIgnoreHeightInDistanceComparison). In range (FUN_14022e940): within DistanceLimit, the facing limits
/// ([`InteractLimit::facing`]), and with bCheckForOcclusion not occluded (IsOccluded 0x14022f300: a line trace on its
/// RayTraceChannel from her eyes (her view point, BaseEyeHeight up; RayTraceZOffset 0) to the interaction point,
/// ignoring her and the interactable's actor). The trace runs against what blocks its channel: WorldDynamic, or
/// Visibility for SCInteractComponents (the beds' and such: window glass doesn't block it), by each level placement's
/// and collider's own response to it (f13-export collision lists where the two channels differ).
#[allow(clippy::too_many_arguments)]
/// The interactable manager's score of an interactable at `point` (reach in cm) for a character at `centre` looking
/// along `look`, facing `facing` and moving along `moving` (flat, unit): its weighted view, distance, rotation and
/// movement terms, plus the stability bonus for the current best one.
pub(crate) fn score(centre: Vec3, point: Vec3, reach_cm: f32, look: Vec3, facing: Vec3, moving: Option<Vec3>, current: bool) -> f32 {
    let flat = (point - centre).with_y(0.0);
    let dir = flat.normalize_or_zero();
    let score = VIEW_WEIGHT * look.dot(dir)
        + DISTANCE_WEIGHT * (1.0 - flat.length() * 100.0 / reach_cm.max(1.0))
        + CHARACTER_ROTATION_WEIGHT * facing.dot(dir)
        + moving.map_or(0.0, |m| MOVEMENT_DIRECTION_WEIGHT * m.dot(dir));
    if current { score + BEST_INTERACTABLE_STABILITY_SCORE } else { score }
}

fn choose_target(
    mut players: Query<(Entity, &Transform, &Inventory, &Player, Option<&crate::cabinets::CabinetUse>, &mut Interaction), (Without<Driving>, Without<crate::hiding::Hiding>)>,
    pads: crate::controls::Pads,
    items: Query<(Entity, &Item, &GlobalTransform, Option<&InCabinet>), Without<Carried>>,
    carried: Query<&Item, With<Carried>>,
    cabinets: Query<(Entity, &Cabinet, &GlobalTransform)>,
    (vehicles, seated): (Query<(Entity, &Vehicle, &GlobalTransform)>, Query<&Driving>),
    doors: Query<(Entity, &Door, &GlobalTransform)>,
    windows: Query<(Entity, &Window, &GlobalTransform)>,
    barricades: Query<&Barricade>,
    (spots, fuse_boxes): (Query<(Entity, &crate::hiding::HidingSpot, &GlobalTransform)>, Query<(Entity, &crate::power::FuseBox)>),
    (phone_boxes, phones, radios, cb_radios): (Query<(Entity, &crate::police::PhoneBox)>, Query<(Entity, &crate::police::Phone)>, Query<(Entity, &crate::radio::Radio)>, Query<(Entity, &crate::tommy::CbRadio)>),
    (limits, globals, collision, dynamic, parents, no_world_dynamic, (names, actor_tris, no_visibility, (not_visibility, not_world_dynamic))): (
        Query<&InteractLimits>,
        Query<&GlobalTransform>,
        Option<Res<crate::collision::WorldCollision>>,
        Res<crate::collision::DynamicColliders>,
        Query<&ChildOf>,
        Query<(), With<crate::collision::NoWorldDynamic>>,
        (Query<&Name>, Option<Res<crate::collision::ActorTris>>, Query<(), With<crate::collision::NoVisibility>>, (Option<Res<crate::collision::NotVisibility>>, Option<Res<crate::collision::NotWorldDynamic>>)),
    ),
) {
    for (pe, xf, inv, p, searching, mut interaction) in &mut players {
    if interaction.busy() {
        continue; // keep the target while repairing
    }
    // At a drawer: it's the only thing to use, once it's ready (CanInteractWith: nothing on it before).
    if let Some(u) = searching {
        let ready = cabinets.get(u.cabinet).is_ok_and(|(_, c, _)| c.slots.get(u.slot).is_some_and(|s| s.ready && s.interactor == Some(pe)));
        let target = ready.then_some(Target::Cabinet(u.cabinet, u.slot));
        if target != interaction.target {
            interaction.hold = None;
        }
        interaction.target = target;
        continue;
    }
    let centre = xf.translation + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M;
    let look = { let (yaw, pitch) = pads.look(pe); PlayerCamera { yaw, pitch }.look() };
    let facing = xf.rotation * Vec3::Z;
    // Her velocity is in Bevy's XZ plane.
    let moving = (p.velocity.length_squared() > 0.0).then(|| Vec3::new(p.velocity.x, 0.0, p.velocity.y).normalize_or_zero());
    let current = interaction.target;
    let mut best: Option<(f32, Target)> = None;
    let eyes = centre + Vec3::Y * BASE_EYE_HEIGHT_M;
    // The owner's own colliders (or its parts').
    let of_owner = |e: Entity, owner: Entity| {
        let mut at = e;
        for _ in 0..8 {
            if at == owner {
                return true;
            }
            match parents.get(at) {
                Ok(p) => at = p.parent(),
                Err(_) => return false,
            }
        }
        false
    };
    let mut consider = |owner: Entity, component: &str, point: Vec3, reach_cm: f32, t: Target| {
        if point.distance(centre) * 100.0 > reach_cm {
            return;
        }
        if let (Some(limit), Ok(ogxf)) = (limits.get(owner).ok().and_then(|l| l.0.get(component)), globals.get(owner)) {
            if !limit.facing(ogxf, centre - point, look) {
                return;
            }
            if let Some(world) = collision.as_ref().filter(|_| limit.occlusion) {
                // Its channel's blockers: dynamic colliders and level placements by their response to it.
                let passes = |e: Entity| if limit.visibility { no_visibility.contains(e) } else { no_world_dynamic.contains(e) };
                let (grid, lists) = world.without(&dynamic, |e| passes(e) || of_owner(e, owner) || e == pe);
                // A level actor's own meshes are in the static grid.
                let own = names.get(owner).ok().and_then(|n| actor_tris.as_ref()?.0.get(n.as_str()));
                let glass = if limit.visibility { not_visibility.as_ref().map(|g| &g.0) } else { not_world_dynamic.as_ref().map(|g| &g.0) };
                let level = crate::collision::Excluding(grid, own);
                if f13_core::collision::segment_hit(&(&crate::collision::Excluding(&level, glass), lists.as_slice()), eyes, point, 1.0).is_some() {
                    return;
                }
            }
        }
        let score = score(centre, point, reach_cm, look, facing, moving, current == Some(t));
        if best.is_none_or(|(bs, _)| score > bs) {
            best = Some((score, t));
        }
    };
    for (e, item, gxf, in_cab) in &items {
        // In a drawer: the drawer hands it over.
        if in_cab.is_some() {
            continue;
        }
        let at = limits.get(e).ok().and_then(|l| l.0.get("Interactable")).map_or(gxf.translation(), |l| gxf.transform_point(l.at.translation));
        consider(e, "Interactable", at, item.reach, Target::Item(e));
    }
    for (e, cab, gxf) in &cabinets {
        for (i, s) in cab.slots.iter().enumerate() {
            if s.searchable() {
                consider(e, if i == 0 { "Interactable" } else { "Interactable2" }, gxf.transform_point(s.interact), s.reach, Target::Cabinet(e, i));
            }
        }
    }
    for (e, v, gxf) in &vehicles {
        for (i, r) in v.repairs.iter().enumerate() {
            if !r.done && has_part(inv, &carried, &r.parts) {
                consider(e, &r.name, gxf.transform_point(r.at), r.reach, Target::Repair(e, i));
            }
        }
        if v.repaired() {
            // The seats are SCInteractComponents like any other (their range test and location are the base's); a
            // taken one isn't offered.
            let free = |seat: Option<usize>| !seated.iter().any(|d| d.vehicle == e && d.seat == seat);
            if free(None) {
                consider(e, "DriverSeat", gxf.transform_point(v.driver_seat), v.seat_reach, Target::Seat(e, None));
            }
            for (i, s) in v.passengers.iter().enumerate() {
                if free(Some(i)) {
                    consider(e, "PassengerSeat", gxf.transform_point(s.at), s.reach, Target::Seat(e, Some(i)));
                }
            }
        }
    }
    for (e, d, gxf) in &doors {
        let (press, hold) = d.can_interact(gxf, xf.translation, barricade_of(&barricades, e));
        if press || hold {
            consider(e, "InteractComponent", gxf.transform_point(d.interact), d.reach, Target::Door(e));
        }
    }
    for (e, w, gxf) in &windows {
        consider(e, "Indoor Interactable", gxf.transform_point(w.interact), w.reach, Target::Window(e));
    }
    for (e, s, gxf) in &spots {
        if s.usable() {
            consider(e, "Interactable", gxf.transform_point(s.interact), s.reach, Target::Hide(e));
        }
    }
    for (e, f) in &fuse_boxes {
        if f.destroyed {
            consider(e, "Interactable", f.interact, f.reach, Target::FuseBox(e));
        }
    }
    for (e, b) in &phone_boxes {
        if b.broken || (!b.repaired && has_part(inv, &carried, &b.parts)) {
            consider(e, "Interactable", b.interact, b.reach, Target::PhoneBox(e));
        }
    }
    for (e, p) in &phones {
        if p.enabled {
            consider(e, "Interactable", p.interact, p.reach, Target::Phone(e));
        }
    }
    // (SCCBRadio::CanInteractWith: nobody on it and not called yet.)
    for (e, r) in &cb_radios {
        if !r.called {
            consider(e, "Interactable", r.interact, r.reach, Target::CbRadio(e));
        }
    }
    for (e, r) in &radios {
        if r.usable() {
            consider(e, "Interactable", r.interact, r.reach, Target::Radio(e));
        }
    }
    let target = best.map(|(_, t)| t);
    if target != interaction.target {
        interaction.hold = None;
    }
    interaction.target = target;
    }
}

#[allow(clippy::too_many_arguments)]
fn act(
    mut commands: Commands,
    pads: crate::controls::Pads,
    time: Res<Time>,
    tuning: Res<PlayerTuning>,
    mut rng: ResMut<crate::GameRng>,
    // Held or in a trap, E wiggles (the stun's WiggleInputComponent over hers).
    mut players: Query<(Entity, &mut Transform, &mut Inventory, &mut Player, Has<MoveToSpot>, &mut Interaction), (Without<Driving>, Without<crate::hiding::Hiding>, Without<crate::jason::Held>, Without<crate::items::Trapped>)>,
    (items, item_xfs): (Query<&Item>, Query<&GlobalTransform, With<Item>>),
    (mut spots, model): (Query<(&mut crate::hiding::HidingSpot, &GlobalTransform)>, Option<Res<crate::character::CounselorCharacter>>),
    (cabinets, cabinet_xfs): (Query<&Cabinet>, Query<&GlobalTransform, With<Cabinet>>),
    mut vehicles: Query<&mut Vehicle>,
    mut doors: Query<(&mut Door, &GlobalTransform)>,
    (windows, mut barricades, (mut phone_boxes, mut phones, mut game, match_tuning), (names, special_moves)): (Query<(&mut Window, &GlobalTransform)>, Query<&mut Barricade>, (Query<&mut crate::police::PhoneBox>, Query<&mut crate::police::Phone>, Option<ResMut<crate::match_flow::GameMatch>>, Option<Res<crate::match_flow::MatchTuning>>), (Query<&Name>, Option<Res<SpecialMoves>>)),
    (mut visibility, mut fuse_boxes, mut grids, vehicle_xfs, radios): (Query<&mut Visibility>, Query<&mut crate::power::FuseBox>, Option<ResMut<crate::power::PowerGrids>>, Query<&GlobalTransform, With<Vehicle>>, Query<&crate::radio::Radio>),
    (mut cb_radios, mut hunter): (Query<&mut crate::tommy::CbRadio>, ResMut<crate::tommy::HunterTimer>),
) {
    use crate::controls::Button;
    for (player_entity, mut player_xf, mut inv, mut p, sliding, mut interaction) in &mut players {
    let (player_xf, inv, p) = (&mut *player_xf, &mut *inv, &mut *p);
    let Some(target) = interaction.target else { continue };
    // A special move component's way onto its spot (its owner's class: the actor's Name).
    let class_of = |owner: Entity| names.get(owner).map_or(String::new(), |n| n.as_str().to_string());
    let moves = special_moves.as_deref();
    let mv = |owner: Entity, component: &str| moves.map_or(MoveParams::default(), |m| m.of(&class_of(owner), component));
    // On her way onto a move's spot: only the hold that started it goes on.
    if sliding && !interaction.hold.as_ref().is_some_and(|h| h.started) {
        continue;
    }
    match target {
        Target::Item(e) if pads.just_pressed(player_entity, Button::Interact) => {
            let Ok(item) = items.get(e) else { continue };
            if item.large {
                if let Some(old) = inv.hand.take() {
                    drop_at(&mut commands, &mut visibility, old, player_xf);
                }
                inv.hand = Some(e);
            } else {
                let at = item_xfs.get(e).map_or(player_xf.translation, |g| g.translation());
                take_small(&mut commands, &mut visibility, inv, item, e, Swap::Drop(at));
            }
            info!("picked up {}", item.name);
            commands.trigger(ClassScore::new(player_entity, &item.class, "PickupScoreEvent"));
            commands.entity(e).insert(Carried).remove::<InCabinet>();
            if let Ok(mut v) = visibility.get_mut(e) {
                *v = Visibility::Hidden;
            }
            interaction.target = None;
        }
        Target::Cabinet(e, i) => {
            let (Ok(cab), Ok(gxf)) = (cabinets.get(e), cabinet_xfs.get(e)) else { continue };
            let Some(slot) = cab.slots.get(i) else { continue };
            if interaction.hold_done {
                interaction.hold_done = pads.pressed(player_entity, Button::Interact);
                continue;
            }
            // Closed: a press searches it (its special move).
            if slot.searchable() {
                if pads.just_pressed(player_entity, Button::Interact) {
                    let at = crate::hiding::HidingSpot::world(gxf, &slot.spot);
                    let to = Transform { translation: at.translation.with_y(player_xf.translation.y), rotation: at.rotation, ..*player_xf };
                    let handler = if i == 0 { "UseCabinet_SpecialMoveHandler" } else { "UseCabinet_SpecialMoveHandler2" };
                    crate::cabinets::search(&mut commands, player_entity, *player_xf, e, i, to, mv(e, handler));
                    interaction.target = None;
                }
                continue;
            }
            if !slot.ready || slot.interactor != Some(player_entity) {
                continue;
            }
            // Ready: a press closes it; holding takes the item.
            let item = slot.item.and_then(|(it, _)| items.get(it).ok());
            let takeable = crate::cabinets::can_take(item, inv, &items);
            let started = interaction.hold.as_ref().is_some_and(|h| h.started);
            if pads.just_released(player_entity, Button::Interact) && !started {
                interaction.hold = None;
                commands.trigger(crate::cabinets::CloseCabinet { player: player_entity, take: false });
                continue;
            }
            if !pads.pressed(player_entity, Button::Interact) || !takeable {
                interaction.hold = None;
                continue;
            }
            let h = interaction.hold.get_or_insert_with(|| HoldInteraction::new(crate::cabinets::TAKE_HOLD_S, false));
            let mut events = Vec::new();
            h.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            if events.contains(&MinigameEvent::Completed) {
                commands.trigger(crate::cabinets::CloseCabinet { player: player_entity, take: true });
                interaction.hold = None;
                interaction.hold_done = true;
            }
        }
        Target::Repair(v, i) => {
            let Ok(mut vehicle) = vehicles.get_mut(v) else { continue };
            if !pads.pressed(player_entity, Button::Interact) {
                if interaction.hold.take().is_some_and(|h| h.started) {
                    info!("repair cancelled");
                    end_repair_anim(&mut commands, player_entity, vehicle.repairs[i].anim.as_ref(), false);
                }
                continue;
            }
            let r = &vehicle.repairs[i];
            let hold = interaction.hold.get_or_insert_with(|| HoldInteraction::new(r.hold_time.unwrap_or(tuning.repair_hold_time), r.has_pips));
            let mut events = Vec::new();
            for (button, key) in [(Button::Secondary, 0), (Button::Primary, 1)] {
                if pads.just_pressed(player_entity, button) {
                    hold.press(key, &tuning.minigame, &mut events);
                }
            }
            hold.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            let mut completed = false;
            for e in events {
                match e {
                    MinigameEvent::Started => {
                        info!("repair started: {} pips, {:.1}s at x{:.2}", hold.pips.len(), hold.hold_time, tuning.minigame.speed_mod.value(&p.counselor.stats, false));
                        // On its special move's spot, facing its forward (Unreal +X; she faces +Z), on her floor.
                        let to = match (r.stand, vehicle_xfs.get(v)) {
                            (Some(stand), Ok(vxf)) => standing_on(player_xf, vxf.mul_transform(stand).compute_transform()),
                            _ => *player_xf,
                        };
                        if let Some(base) = r.anim.clone() {
                            let params = moves.map_or(MoveParams::default(), |m| m.of_child(&class_of(v), &r.name));
                            move_to_spot(&mut commands, player_entity, *player_xf, to, params, move |w| {
                                w.entity_mut(player_entity).insert(repair_start(&base));
                            });
                        }
                    }
                    MinigameEvent::Hit => info!("pip hit (bonus x{:.1})", hold.bonus),
                    MinigameEvent::Missed => info!("pip missed"),
                    MinigameEvent::NotifyKiller => warn!("repair failed noisily: Jason would be notified"),
                    MinigameEvent::Completed => completed = true,
                }
            }
            if !completed {
                continue;
            }
            let parts = vehicle.repairs[i].parts.clone();
            let Some(used) = take_part(inv, &items, &parts) else { continue };
            commands.entity(used).despawn();
            vehicle.repairs[i].done = true;
            end_repair_anim(&mut commands, player_entity, vehicle.repairs[i].anim.as_ref(), true);
            info!("{}: {} installed", vehicle.class, vehicle.repairs[i].name);
            // SCDriveableVehicle::Repair (0x14043e4e0): AppliedPart for each part fitted while it isn't repaired yet (the
            // last too; AppliedLastPart is StartVehicle's: vehicle.rs).
            crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: player_entity, prop: "VO:AppliedPart" }));
            commands.trigger(ClassScore::new(player_entity, &vehicle.class, "PartRepairedScoreEvent"));
            if vehicle.repairs.iter().all(|r| r.done) {
                commands.trigger(ClassScore::new(player_entity, &vehicle.class, "VehicleRepairedScoreEvent"));
            }
            interaction.hold = None;
            interaction.target = None;
        }
        Target::FuseBox(f) => {
            // Repairing the fuse box: a hold with the minigame (its HoldTimeLimit), like a car part.
            if !pads.pressed(player_entity, Button::Interact) {
                if interaction.hold.take().is_some_and(|h| h.started) {
                    info!("fuse box repair cancelled");
                }
                continue;
            }
            let Ok(mut fuse) = fuse_boxes.get_mut(f) else { continue };
            let hold = interaction.hold.get_or_insert_with(|| HoldInteraction::new(fuse.hold_time, fuse.has_pips));
            let mut events = Vec::new();
            for (button, key) in [(Button::Secondary, 0), (Button::Primary, 1)] {
                if pads.just_pressed(player_entity, button) {
                    hold.press(key, &tuning.minigame, &mut events);
                }
            }
            hold.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            if events.contains(&MinigameEvent::Started) {
                info!("fuse box repair started: {} pips, {:.1}s", hold.pips.len(), hold.hold_time);
            }
            if !events.contains(&MinigameEvent::Completed) {
                continue;
            }
            if let Some(g) = grids.as_mut() {
                crate::power::set_destroyed(&mut fuse, false, g);
            }
            commands.trigger(ClassScore::new(player_entity, &fuse.class, "RepairScoreEvent"));
            interaction.hold = None;
            interaction.target = None;
        }
        Target::PhoneBox(b) => {
            // Putting the fuse in (its FuseRepair) or repairing it after Jason broke it (its Interactable): a hold
            // with the repair minigame, on its special move's spot.
            let Ok(mut pb) = phone_boxes.get_mut(b) else { continue };
            if !pb.needs_work() {
                interaction.hold = None;
                interaction.target = None;
                continue;
            }
            let fixing = pb.repaired;
            if !pads.pressed(player_entity, Button::Interact) {
                if interaction.hold.take().is_some_and(|h| h.started) {
                    info!("phone box repair cancelled");
                    end_repair_anim(&mut commands, player_entity, Some(&crate::police::PHONE_BOX_TAKE.to_string()), false);
                }
                continue;
            }
            let hold = interaction.hold.get_or_insert_with(|| HoldInteraction::new(pb.hold_time, if fixing { pb.fix_pips } else { pb.has_pips }));
            let mut events = Vec::new();
            for (button, key) in [(Button::Secondary, 0), (Button::Primary, 1)] {
                if pads.just_pressed(player_entity, button) {
                    hold.press(key, &tuning.minigame, &mut events);
                }
            }
            hold.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            if events.contains(&MinigameEvent::Started) {
                info!("phone box {} started: {} pips, {:.1}s", if fixing { "repair" } else { "fuse" }, hold.pips.len(), hold.hold_time);
                let to = pb.stand.map_or(*player_xf, |stand| standing_on(player_xf, stand));
                move_to_spot(&mut commands, player_entity, *player_xf, to, mv(b, "InteractSpecialMove"), move |w| {
                    w.entity_mut(player_entity).insert(repair_start(crate::police::PHONE_BOX_TAKE));
                });
            }
            if !events.contains(&MinigameEvent::Completed) {
                continue;
            }
            if !fixing {
                let parts = pb.parts.clone();
                let Some(used) = take_part(inv, &items, &parts) else { continue };
                commands.entity(used).despawn();
            }
            crate::police::repair_phone_box(&mut pb, &mut phones);
            end_repair_anim(&mut commands, player_entity, Some(&crate::police::PHONE_BOX_TAKE.to_string()), true);
            info!("phone box {}: the phone works", if fixing { "repaired" } else { "fused" });
            commands.trigger(ClassScore::new(player_entity, &pb.class, "RepairScoreEvent"));
            interaction.hold = None;
            interaction.target = None;
        }
        Target::Radio(r) => {
            // OnInteract: a press turns it on or off; while it plays (CanInteractWith 3: press and hold), the press
            // comes on letting go before its HoldTimeLimit and the hold, at it, changes the song. Either is her
            // Counselor_UseRadio move from its spot.
            let Ok(radio) = radios.get(r) else { continue };
            if !radio.usable() {
                interaction.target = None;
                continue;
            }
            let held = pads.pressed(player_entity, Button::Interact);
            let change = match interaction.radio_held {
                _ if !radio.playing => pads.just_pressed(player_entity, Button::Interact).then_some(false),
                None => {
                    if pads.just_pressed(player_entity, Button::Interact) {
                        interaction.radio_held = Some(0.0);
                    }
                    None
                }
                Some(t) if held => {
                    let t = t + time.delta_secs();
                    interaction.radio_held = Some(t);
                    (t >= radio.hold_limit).then_some(true)
                }
                Some(_) => Some(false),
            };
            let Some(change) = change else { continue };
            interaction.radio_held = None;
            if !change && !radio.playing {
                commands.trigger(ClassScore::new(player_entity, &radio.class, "TurnRadioOnScoreEvent"));
            }
            info!("using a radio ({})", if change { "next song" } else if radio.playing { "off" } else { "on" });
            let to = standing_on(player_xf, radio.use_spot);
            move_to_spot(&mut commands, player_entity, *player_xf, to, mv(r, "CounselorUse_SpecialMoveHandler"), move |w| {
                w.entity_mut(player_entity).insert((crate::radio::RadioUse::new(r, change), crate::character::PlayAction(crate::radio::USE_TAKE.into())));
            });
            interaction.target = None;
        }
        Target::CbRadio(r) => {
            // Calling for Tommy: the radio's hold with its minigame (bShowMinigameProperties), her CB radio take.
            let Ok(mut radio) = cb_radios.get_mut(r) else { continue };
            if radio.called || !pads.pressed(player_entity, Button::Interact) {
                if interaction.hold.take().is_some_and(|h| h.started) {
                    info!("tommy: the CB radio call stopped");
                    end_repair_anim(&mut commands, player_entity, Some(&crate::tommy::RADIO_TAKE.to_string()), false);
                }
                if radio.called {
                    interaction.target = None;
                }
                continue;
            }
            let hold = interaction.hold.get_or_insert_with(|| HoldInteraction::new(radio.hold_time, true));
            let mut events = Vec::new();
            for (button, key) in [(Button::Secondary, 0), (Button::Primary, 1)] {
                if pads.just_pressed(player_entity, button) {
                    hold.press(key, &tuning.minigame, &mut events);
                }
            }
            hold.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            if events.contains(&MinigameEvent::Started) {
                info!("tommy: calling on the CB radio: {:.1}s", hold.hold_time);
                commands.entity(player_entity).insert(repair_start(crate::tommy::RADIO_TAKE));
            }
            if !events.contains(&MinigameEvent::Completed) {
                continue;
            }
            end_repair_anim(&mut commands, player_entity, Some(&crate::tommy::RADIO_TAKE.to_string()), true);
            crate::tommy::call_hunter(&mut commands, &mut radio, &mut hunter, player_entity);
            interaction.hold = None;
            interaction.target = None;
        }
        Target::Phone(ph) => {
            // Calling the police: a plain hold (no pips) on the phone's spot.
            let Ok(mut phone) = phones.get_mut(ph) else { continue };
            // Its box broken under her (Jason): the call is cut off.
            if !phone.enabled {
                if interaction.hold.take().is_some_and(|h| h.started) {
                    info!("phone call cut off");
                    end_repair_anim(&mut commands, player_entity, Some(&crate::police::PHONE_TAKE.to_string()), false);
                }
                interaction.target = None;
                continue;
            }
            if !pads.pressed(player_entity, Button::Interact) {
                if interaction.hold.take().is_some_and(|h| h.started) {
                    info!("phone call cancelled");
                    end_repair_anim(&mut commands, player_entity, Some(&crate::police::PHONE_TAKE.to_string()), false);
                }
                continue;
            }
            let hold = interaction.hold.get_or_insert_with(|| HoldInteraction::new(phone.hold_time, false));
            let mut events = Vec::new();
            hold.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            if events.contains(&MinigameEvent::Started) {
                info!("calling the police: {:.1}s", hold.hold_time);
                let to = phone.stand.map_or(*player_xf, |stand| standing_on(player_xf, stand));
                move_to_spot(&mut commands, player_entity, *player_xf, to, mv(ph, "UsePhone_SpecialMove"), move |w| {
                    w.entity_mut(player_entity).insert(repair_start(crate::police::PHONE_TAKE));
                });
            }
            if !events.contains(&MinigameEvent::Completed) {
                continue;
            }
            end_repair_anim(&mut commands, player_entity, Some(&crate::police::PHONE_TAKE.to_string()), true);
            crate::police::call_police(&mut commands, &mut phone, game.as_deref_mut(), match_tuning.as_deref(), player_entity);
            interaction.hold = None;
            interaction.target = None;
        }
        Target::Door(d) => {
            let Ok((mut door, gxf)) = doors.get_mut(d) else { continue };
            if interaction.hold_done {
                interaction.hold_done = pads.pressed(player_entity, Button::Interact);
                continue;
            }
            let barricade = barricades.iter().find(|b| b.door == Some(d)).map(|b| b.enabled);
            let (press, hold) = door.can_interact(gxf, player_xf.translation, barricade);
            // Press = let go before the hold starts; hold = keep holding for the door's hold time.
            let started = interaction.hold.as_ref().is_some_and(|h| h.started);
            if pads.just_released(player_entity, Button::Interact) && !started && press {
                interaction.hold = None;
                if door.locked {
                    info!("door locked");
                    crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: d, prop: "LockedDoorSound" }));
                    continue;
                }
                // Her open or close montage; the leaf moves in its driver window (DoorMove).
                let anim = door_anim(!door.open, Door::is_inside(gxf, player_xf.translation));
                let timing = model.as_ref().and_then(|m| m.info.animations.get(&anim.replacen("f_", "m_", 1)));
                match timing {
                    Some(t) => {
                        let swing = t.window("Driver: OpenCloseDoor").unwrap_or((0.0, 0.5));
                        let root = door_root(&door, gxf, player_xf.translation.y, anim, model.as_deref());
                        // She slides onto the move's spot (its take's start), then the take plays and its handler takes
                        // the camera (doors' open and close moves have bTakeCameraControl).
                        let spot = door.spots.get(door_spot(anim)).map(|spot| {
                            let at = crate::hiding::HidingSpot::world(gxf, spot);
                            Transform { translation: at.translation.with_y(player_xf.translation.y), rotation: at.rotation, ..*player_xf }
                        });
                        let to = spot.unwrap_or(*player_xf);
                        let origin = root.map(|r| r.0);
                        let m = DoorMove { door: d, t: 0.0, swing, curve: t.curve("DoorRotation"), length: t.length, swung: false, root };
                        let begin = move |w: &mut World| {
                            w.entity_mut(player_entity).insert((m, crate::character::PlayAction(anim.into())));
                            w.trigger(crate::interaction_camera::PlayInteractionCamera { actor: d, handler: door_spot(anim), killer: false });
                        };
                        match origin {
                            Some(origin) => move_to_rooted_spot(&mut commands, player_entity, *player_xf, to, origin, mv(d, door_spot(anim)), begin),
                            None => move_to_spot(&mut commands, player_entity, *player_xf, to, mv(d, door_spot(anim)), begin),
                        }
                    }
                    None => {
                        let what = door.press(gxf, player_xf.translation);
                        info!("door {what}");
                    }
                }
                continue;
            }
            let barricaded_door = barricades.iter().any(|b| b.door == Some(d));
            if !pads.pressed(player_entity, Button::Interact) || !hold {
                if interaction.hold.take().is_some_and(|h| h.started) && !barricaded_door {
                    commands.entity(player_entity).remove::<(LockAnim, crate::character::PlayAction, MoveToSpot)>();
                }
                continue;
            }
            let h = interaction.hold.get_or_insert_with(|| HoldInteraction::new(door.hold_time, false));
            let mut events = Vec::new();
            h.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            // Locking or unlocking (not a barricade): her take while she holds.
            if events.contains(&MinigameEvent::Started) && !barricaded_door {
                let anim = if door.locked { "f_normal_stand_door_unlock" } else { "f_normal_stand_door_lock" };
                // On the lock's special move spot (the take stays in place).
                let spot = if door.locked {
                    if Door::is_inside(gxf, player_xf.translation) { "UnlockDoorInside_SpecialMoveHandler" } else { "UnlockDoorOutside_SpecialMoveHandler" }
                } else {
                    door_spot(anim)
                };
                let to = door.spots.get(spot).map_or(*player_xf, |s| {
                    let at = crate::hiding::HidingSpot::world(gxf, s);
                    Transform { translation: Vec3::new(at.translation.x, player_xf.translation.y, at.translation.z), rotation: at.rotation, ..*player_xf }
                });
                move_to_spot(&mut commands, player_entity, *player_xf, to, mv(d, spot), move |w| {
                    w.entity_mut(player_entity).insert((LockAnim, crate::character::PlayAction(anim.into())));
                });
            }
            if events.contains(&MinigameEvent::Completed) {
                commands.entity(player_entity).remove::<(LockAnim, crate::character::PlayAction)>();
                match barricades.iter_mut().find(|b| b.door == Some(d)) {
                    // A barricade only moves while the door is shut (OnUsedBarricade): her take, the barricade moving
                    // in its UseBarricade window.
                    Some(mut b) if !door.open => {
                        let anim = if b.enabled { BARRICADE_UP } else { BARRICADE_DOWN };
                        match model.as_ref().and_then(|m| m.info.animations.get(anim)) {
                            Some(t) => {
                                let driver = t.window("Driver: UseBarricade").unwrap_or((t.length * 0.5, t.length * 0.5));
                                commands.entity(player_entity).insert((
                                    BarricadeMove { door: d, t: 0.0, driver, curve: t.curve("Barricade"), length: t.length, flipped: false },
                                    crate::character::PlayAction(anim.into()),
                                ));
                            }
                            None => flip_barricade(&mut commands, &mut b, player_entity),
                        }
                    }
                    Some(_) => info!("door open (can't barricade)"),
                    None => info!("door {}", door.toggle_lock()),
                }
                interaction.hold = None;
                interaction.hold_done = true;
                interaction.target = None;
            }
        }
        Target::Window(w) => {
            let Ok((window, gxf)) = windows.get(w) else { continue };
            let w_entity = w;
            if interaction.hold_done {
                interaction.hold_done = pads.pressed(player_entity, Button::Interact);
                continue;
            }
            let started = interaction.hold.as_ref().is_some_and(|h| h.started);
            if pads.just_released(player_entity, Button::Interact) && !started {
                interaction.hold = None;
                let action = window.press_action(p.counselor.sprinting, p.counselor.wounded);
                match action {
                    WindowAction::Open => {
                        start_window_move(&mut commands, player_entity, w, WindowMoveKind::Open, player_xf.translation, player_xf.translation);
                        // Its handler for her side takes the view (bTakeCameraControl).
                        let handler = if Window::is_inside(gxf, player_xf.translation) { "CounselorOpenInside" } else { "CounselorOpenOutside" };
                        commands.trigger(crate::interaction_camera::PlayInteractionCamera { actor: w, handler, killer: false });
                    }
                    _ => {
                        // Through the window (her climb or dive take carries her over), taking any damage.
                        let damage = window.damage(action);
                        let landing = Window::landing(gxf, window.interact, player_xf.translation);
                        let kind = if action == WindowAction::Dive { WindowMoveKind::Dive } else { WindowMoveKind::Vault };
                        // From the side she's on: its special move spot, her take's root motion carrying her over.
                        let side = if Window::is_inside(gxf, player_xf.translation) { 0 } else { 1 };
                        let spot = if kind == WindowMoveKind::Dive { window.dive[side] } else { window.vault[side] };
                        let take = kind.take().0;
                        match (spot, model.as_ref()) {
                            (Some(spot), Some(c)) if c.root_motion(take).is_some() => {
                                let mut at = crate::hiding::HidingSpot::world(gxf, &spot);
                                at.translation.y = player_xf.translation.y;
                                let root = c.place(take, at);
                                let component = match (kind == WindowMoveKind::Dive, side) {
                                    (true, 0) => "CounselorDiveInside",
                                    (true, _) => "CounselorDiveOutside",
                                    (false, 0) => "CounselorVaultInside",
                                    (false, _) => "CounselorVaultOutside",
                                };
                                move_to_rooted_spot(&mut commands, player_entity, *player_xf, at, root.0, mv(w, component), move |w| {
                                    w.entity_mut(player_entity).insert(window_root_move(w_entity, kind, root));
                                });
                            }
                            _ => start_window_move(&mut commands, player_entity, w, kind, player_xf.translation, landing),
                        }
                        if damage > 0.0 {
                            p.counselor.take_damage(f13_core::health::Hit { amount: damage, ..Default::default() }, &tuning.counselor);
                        }
                        p.velocity = Vec2::ZERO;
                        info!("window {action:?}: {damage:.0} damage, health {:.0}", p.counselor.health);
                        interaction.target = None;
                        continue;
                    }
                }
                info!("window {action:?}");
                continue;
            }
            if !pads.pressed(player_entity, Button::Interact) || !window.can_hold() {
                interaction.hold = None;
                continue;
            }
            let h = interaction.hold.get_or_insert_with(|| HoldInteraction::new(0.5, false));
            let mut events = Vec::new();
            h.tick(time.delta_secs(), &p.counselor.stats, &tuning.minigame, &mut rng.0, &mut events);
            if events.contains(&MinigameEvent::Completed) {
                start_window_move(&mut commands, player_entity, w, WindowMoveKind::Close, player_xf.translation, player_xf.translation);
                let handler = if Window::is_inside(gxf, player_xf.translation) { "CounselorCloseInside" } else { "CounselorCloseOutside" };
                commands.trigger(crate::interaction_camera::PlayInteractionCamera { actor: w, handler, killer: false });
                info!("window Close");
                interaction.hold = None;
                interaction.hold_done = true;
            }
        }
        Target::Hide(s) if pads.just_pressed(player_entity, Button::Interact) && p.counselor.alive() => {
            let (Ok((mut spot, gxf)), Some(model)) = (spots.get_mut(s), model.as_ref()) else { continue };
            if spot.usable() {
                crate::hiding::enter(&mut commands, player_entity, player_xf, p, s, &mut spot, gxf, model, time.elapsed_secs(), |c| mv(s, c));
            }
            interaction.target = None;
        }
        Target::Seat(v, seat) if pads.just_pressed(player_entity, Button::Interact) => {
            let right = seat.and_then(|i| vehicles.get(v).ok().and_then(|veh| veh.passengers.get(i).map(|s| s.right))).unwrap_or(false);
            let d = match seat {
                Some(i) => Driving::ride(v, i, right),
                None => Driving::enter(v),
            };
            let take = d.seat_move.map(|m| m.take()).unwrap_or(crate::vehicle::DRIVE_TAKE);
            commands.entity(player_entity).remove::<RepairAnim>().insert((d, crate::character::PlayAction(take.into())));
            info!("got in the {}", if seat.is_some() { "passenger's seat" } else { "driver's seat" });
            interaction.target = None;
        }
        _ => {}
    }
    }
}

fn barricade_of(barricades: &Query<&Barricade>, door: Entity) -> Option<bool> {
    barricades.iter().find(|b| b.door == Some(door)).map(|b| b.enabled)
}

/// Puts an item back in the world at the counselor's feet.
pub(crate) fn drop_at(commands: &mut Commands, visibility: &mut Query<&mut Visibility>, item: Entity, player: &Transform) {
    let forward = player.rotation * Vec3::Z;
    drop_on(commands, visibility, item, player.translation + forward * 0.5 + Vec3::Y * 0.05);
}

/// Puts a carried item down at `at`.
pub(crate) fn drop_on(commands: &mut Commands, visibility: &mut Query<&mut Visibility>, item: Entity, at: Vec3) {
    commands.entity(item).remove::<(Carried, ChildOf, crate::items::InHand)>().insert(Transform::from_translation(at));
    crate::net_world::shout(commands, crate::net_world::Shout::Dropped(crate::audio::Dropped { item, at }));
    if let Ok(mut v) = visibility.get_mut(item) {
        *v = Visibility::Inherited;
    }
}

/// Her take of DropLargeItemMontage (AM_normal_stand_drop_item_right) and of DropSmallItemMontage (_left).
const DROP_LARGE_TAKE: &str = "f_normal_stand_item_drop_right";
const DROP_SMALL_TAKE: &str = "f_normal_stand_item_drop_left";

/// She's dropping an item (bDroppingLargeItem / bDroppingSmallItem): her drop take, seconds in.
#[derive(Component)]
pub struct Dropping {
    large: bool,
    t: f32,
}

/// The drop take: at its DropLargeItem / DropSmallItem notify (SCCounselorAnimInstance::OnDropLargeItem /
/// OnDropSmallItem) the item in her hands, or the selected small item, goes down; the take ends the drop.
fn dropping(
    mut commands: Commands,
    time: Res<Time>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    mut players: Query<(Entity, &Transform, &mut Inventory, &mut Dropping)>,
    mut visibility: Query<&mut Visibility>,
) {
    let Some(model) = model else { return };
    for (pe, xf, mut inv, mut d) in &mut players {
        let take = if d.large { DROP_LARGE_TAKE } else { DROP_SMALL_TAKE };
        let info = model.info.animations.get(take);
        let at = info.and_then(|a| a.window(if d.large { "DropLargeItem" } else { "DropSmallItem" })).map_or(0.0, |w| w.0);
        let before = d.t;
        d.t += time.delta_secs();
        if before < at && d.t >= at || at <= 0.0 && before == 0.0 {
            let item = if d.large { inv.hand.take() } else { inv.current_small() };
            if let Some(item) = item {
                inv.pockets.retain(|&x| x != item);
                drop_at(&mut commands, &mut visibility, item, xf);
            }
        }
        if d.t >= info.map_or(0.0, |a| a.length) {
            commands.entity(pe).remove::<(Dropping, crate::character::PlayAction)>();
        }
    }
}

/// Where the selected small item goes when a small item is taken with her pockets full.
pub(crate) enum Swap {
    /// Dropped where the new item lay (its OnDropped at the picked item's location).
    Drop(Vec3),
    /// Left in the drawer the new item came from (the cabinet's ContainedItem), where that one rested.
    Cabinet(Entity, usize, Transform),
}

/// Her pickup (0x1403a4e30, at the pickup's OnPickupItem): a special item goes to her special items; a small one, with
/// her pockets full, takes the selected pocket's place (that item is dropped where the new one lay, or left in its
/// drawer), and is selected.
pub(crate) fn take_small(commands: &mut Commands, visibility: &mut Query<&mut Visibility>, inv: &mut Inventory, item: &Item, e: Entity, swap: Swap) {
    if item.special {
        inv.special.push(e);
        return;
    }
    if inv.pockets.len() >= inv.max_small {
        if let Some(old) = inv.current_small() {
            inv.pockets.retain(|&x| x != old);
            match swap {
                Swap::Drop(at) => drop_on(commands, visibility, old, at),
                Swap::Cabinet(cabinet, slot, rest) => {
                    commands.entity(old).remove::<(Carried, ChildOf, crate::items::InHand)>().insert((rest, crate::props::InCabinet(cabinet, slot)));
                    if let Ok(mut v) = visibility.get_mut(old) {
                        *v = Visibility::Inherited;
                    }
                }
            }
            info!("swapped a small item out of her pockets");
        }
    }
    inv.pockets.push(e);
    inv.selected = inv.pockets.len() - 1;
}

/// Her small items' keys (DefaultInput.ini's counselor actions): the mouse wheel selects the next (down) or previous
/// (up) pocket, wrapping (0x1403c0060, 0x1403cbea0; not while one is in use); 1-3 select that pocket and use it
/// (SERVER_OnUseSmallItem 0x1403ce550, if it holds one); G uses the selected one (CSLR_SmallItem). Z drops
/// (OnDropItemPressed / Released 0x1403c1570, the tick 0x1403dbc70): held for DropItemHoldTime with a large item, that
/// one; released sooner, or with no large item, the selected small item: her drop montage plays
/// (DropLargeItemMontage / DropSmallItemMontage) and the item goes at its DropLargeItem / DropSmallItem notify
/// ([`dropping`]). The press counts (FUN_1403a7500) when she's not in a seat, a water volume or a special move, not
/// interacting, and not already dropping.
#[allow(clippy::type_complexity)]
fn small_items(
    mut commands: Commands,
    time: Res<Time>,
    pads: crate::controls::Pads,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    water: Option<Res<crate::water::Water>>,
    mut players: Query<(Entity, &Transform, &mut Inventory, &mut Interaction, Has<UsingItem>, Has<crate::items::ItemAction>, Has<Dropping>, Has<MoveToSpot>, Has<crate::controls::LocalPlayer>), (With<Player>, Without<Driving>)>,
) {
    use crate::controls::Button;
    for (pe, xf, mut inv, mut interaction, using, acting, dropping, sliding, local) in &mut players {
    let busy = interaction.hold.as_ref().is_some_and(|h| h.started);
    let drop_held = &mut interaction.drop_held;
    inv.use_small = None;
    let count = inv.pockets.len();
    if !using && !acting && count > 0 {
        // The mouse wheel: this machine's counselor.
        if local && scroll.delta.y < 0.0 {
            inv.selected = if inv.selected + 1 >= count { 0 } else { inv.selected + 1 };
        } else if local && scroll.delta.y > 0.0 {
            inv.selected = if inv.selected == 0 { count - 1 } else { inv.selected - 1 };
        }
        if let Some(slot) = [Button::Slot1, Button::Slot2, Button::Slot3].iter().position(|&b| pads.just_pressed(pe, b)) {
            if slot < count {
                inv.selected = slot;
                inv.use_small = inv.current_small();
            }
        }
    }
    if pads.just_pressed(pe, Button::Drop) {
        inv.use_small = inv.current_small();
    }
    // The drop key: not in a water volume (her feet in one) nor during a special move or an interaction.
    let hold_time = model.as_ref().and_then(|m| m.info.values.get("DropItemHoldTime").copied()).unwrap_or(1.5);
    let in_water = water.as_ref().is_some_and(|w| w.surface_at(xf.translation + Vec3::Y * 0.05).is_some());
    let free = !in_water && !busy && !sliding;
    if pads.just_pressed(pe, Button::UseItem) {
        debug!("drop key pressed (dt {:.3}): free {free} (water {in_water}, busy {}, sliding {sliding}), dropping {dropping}", time.delta_secs(), busy);
    }
    if pads.just_pressed(pe, Button::UseItem) && free && !dropping {
        *drop_held = Some(0.0);
    }
    if let Some(held) = drop_held.as_mut() {
        if free && pads.pressed(pe, Button::UseItem) {
            *held += time.delta_secs();
            if *held >= hold_time {
                if inv.hand.is_some() && !dropping {
                    commands.entity(pe).insert((Dropping { large: true, t: 0.0 }, crate::character::PlayAction(DROP_LARGE_TAKE.into())));
                }
                *drop_held = None;
            }
        }
    }
    if pads.just_released(pe, Button::UseItem) {
        debug!("drop key released: held {:?}, free {free}, dropping {dropping}, hand {:?}, pockets {}", *drop_held, inv.hand, inv.pockets.len());
        if let Some(held) = drop_held.take() {
            if free && !dropping && (inv.hand.is_none() || held < hold_time) && !inv.pockets.is_empty() {
                commands.entity(pe).insert((Dropping { large: false, t: 0.0 }, crate::character::PlayAction(DROP_SMALL_TAKE.into())));
            }
        }
    }
    }
}

/// The selected small item used (`Inventory::use_small`): first aid here (items.rs throws firecrackers).
fn use_item(
    mut commands: Commands,
    time: Res<Time>,
    tuning: Res<PlayerTuning>,
    mut players: Query<(Entity, &mut Player, &mut Inventory, Option<&mut UsingItem>), Without<Driving>>,
    items: Query<&Item>,
) {
    for (entity, mut p, mut inv, using) in &mut players {
    if let Some(mut u) = using {
        u.left -= time.delta_secs();
        if u.left > 0.0 {
            continue;
        }
        let healed = p.counselor.heal(tuning.first_aid.health_restored, &tuning.counselor);
        info!("used first aid: +{healed:.0} health");
        commands.trigger(ClassScore::new(entity, FIRST_AID, "UseFirstAidScoreEvent"));
        inv.pockets.retain(|&e| e != u.item);
        commands.entity(u.item).despawn();
        commands.entity(entity).remove::<UsingItem>();
        continue;
    }
    let Some(item) = inv.use_small else { continue };
    if !p.counselor.alive() || items.get(item).map_or(true, |i| i.class != FIRST_AID) {
        continue;
    }
    if p.counselor.health >= tuning.counselor.health.max_health {
        info!("already at full health");
        continue;
    }
    commands.entity(entity).insert(UsingItem { item, left: tuning.first_aid.use_length });
    }
}

fn update_prompt(
    tuning: Res<PlayerTuning>,
    doors: Query<(&Door, &GlobalTransform)>,
    (windows, barricades): (Query<&Window>, Query<&Barricade>),
    player: Option<Single<(&Inventory, Has<Driving>, &Transform, Option<&UsingItem>, &Player, Option<&crate::hiding::Hiding>, &Interaction), With<crate::controls::LocalPlayer>>>,
    items: Query<&Item>,
    (vehicles, phone_boxes, cabinets, radios): (Query<&Vehicle>, Query<&crate::police::PhoneBox>, Query<&Cabinet>, Query<&crate::radio::Radio>),
    mut text: Single<&mut Text, With<PromptText>>,
    play: Option<Res<crate::jason_player::PlayJason>>,
) {
    // Her prompts are hers: none on Jason's screen, nor without her (a host playing Jason has no counselor here).
    let Some(player) = player.filter(|_| play.is_none()) else {
        if !text.0.is_empty() {
            text.0.clear();
        }
        return;
    };
    let (inv, driving, _, using, p, hiding, interaction) = *player;
    let name = |e: Entity| items.get(e).map_or("?".to_string(), |i| i.name.clone());
    // Her prompts only (the pockets and the cars' parts are the game's HUD's: ui.rs).
    let mut lines: Vec<String> = Vec::new();
    if let Some(h) = hiding {
        lines.push(match h.phase {
            crate::hiding::HidePhase::Hidden => "Hiding   [E] Leave   (mouse: look around)".into(),
            crate::hiding::HidePhase::Killed => String::new(),
            _ => "...".into(),
        });
    } else if let Some(u) = using {
        lines.push(format!("Using {}... {:.1}s", name(u.item), u.left.max(0.0)));
    } else if driving {
        lines.push("[Hold E] Start (needs keys)   [W/S/A/D] Drive   [F] Get out".into());
    } else {
        match interaction.target {
            Some(Target::Item(e)) => {
                lines.push(format!("[E] Pick up {}", name(e)));
            }
            Some(Target::Cabinet(c, i)) => match cabinets.get(c).ok().and_then(|c| c.slots.get(i)) {
                Some(s) if s.ready => {
                    let item = s.item.and_then(|(it, _)| items.get(it).ok());
                    lines.push(match item {
                        Some(it) if crate::cabinets::can_take(Some(it), inv, &items) => format!("[E] Close   [Hold E] Take {}", it.name),
                        _ => "[E] Close".into(),
                    });
                }
                _ => lines.push("[E] Search".into()),
            },
            Some(Target::Repair(v, i)) => {
                if let Ok(v) = vehicles.get(v) {
                    match &interaction.hold {
                        Some(h) if h.started => {
                            // The dial is on the HUD (ui.rs).
                            lines.push(format!("Repairing {}: {:.0}%  (RMB/LMB when the needle is in a pip)", v.repairs[i].name, h.progress(&tuning.minigame) * 100.0));
                        }
                        _ => lines.push(format!("[Hold E] Repair: {}", v.repairs[i].name)),
                    }
                }
            }
            Some(Target::Seat(_, None)) => lines.push("[E] Get in (driver)".into()),
            Some(Target::Seat(_, Some(_))) => lines.push("[E] Get in (passenger)".into()),
            Some(Target::Hide(_)) => lines.push("[E] Hide".into()),
            Some(Target::PhoneBox(b)) => {
                let fixing = phone_boxes.get(b).is_ok_and(|b| b.repaired);
                lines.push(match &interaction.hold {
                    Some(h) if h.started => format!("{}: {:.0}%", if fixing { "Repairing the phone box" } else { "Putting the fuse in" }, h.progress(&tuning.minigame) * 100.0),
                    _ if fixing => "[Hold E] Repair the phone box".into(),
                    _ => "[Hold E] Put the fuse in the phone box".into(),
                });
            }
            Some(Target::CbRadio(_)) => lines.push(match &interaction.hold {
                Some(h) if h.started => format!("Calling for help: {:.0}%", h.progress(&tuning.minigame) * 100.0),
                _ => "[Hold E] Call for help on the CB radio".into(),
            }),
            Some(Target::Phone(_)) => lines.push(match &interaction.hold {
                Some(h) if h.started => format!("Calling the police: {:.0}%", h.progress(&tuning.minigame) * 100.0),
                _ => "[Hold E] Call the police".into(),
            }),
            Some(Target::Radio(r)) => lines.push(if radios.get(r).is_ok_and(|r| r.playing) { "[E] Turn off the radio   [Hold E] Change the song".into() } else { "[E] Turn on the radio".into() }),
            Some(Target::FuseBox(_)) => lines.push(match &interaction.hold {
                Some(h) if h.started => format!("Repairing the fuse box: {:.0}%", h.progress(&tuning.minigame) * 100.0),
                _ => "[Hold E] Repair the fuse box".into(),
            }),
            Some(Target::Window(w)) => {
                if let Ok(window) = windows.get(w) {
                    let press = match window.press_action(p.counselor.sprinting, p.counselor.wounded) {
                        WindowAction::Dive => "[E] Dive through",
                        WindowAction::Open => "[E] Open window",
                        _ => "[E] Climb through",
                    };
                    lines.push(if window.can_hold() { format!("{press}   [Hold E] Close") } else { press.to_string() });
                }
            }
            Some(Target::Door(d)) => {
                if let Ok((door, gxf)) = doors.get(d) {
                    let barricade = barricades.iter().find(|b| b.door == Some(d)).map(|b| b.enabled);
                    let (press, hold) = door.can_interact(gxf, player.2.translation, barricade);
                    let mut s = Vec::new();
                    if press {
                        s.push(if door.open { "[E] Close door" } else { "[E] Open door" });
                    }
                    if hold {
                        s.push(match barricade {
                            Some(true) => "[Hold E] Remove barricade",
                            Some(false) => "[Hold E] Barricade",
                            None if door.locked => "[Hold E] Unlock",
                            None => "[Hold E] Lock",
                        });
                    }
                    if barricade == Some(true) && !hold {
                        s.push("Barricaded");
                    }
                    lines.push(s.join("   "));
                }
            }
            None => {}
        }
        if inv.hand.is_some() {
            lines.push("[G] Drop".into());
        }
        if let Some(i) = inv.pockets.iter().position(|&e| items.get(e).is_ok_and(|it| it.class == FIRST_AID)) {
            lines.push(format!("[{}] Use first aid", i + 1));
        }
        use crate::props::ItemKind;
        match inv.hand.and_then(|e| items.get(e).ok()).map(|i| &i.kind) {
            Some(ItemKind::Weapon(_)) => lines.push("[LMB] Swing   [W + LMB] Heavy swing".into()),
            Some(ItemKind::Gun { .. }) => lines.push("[RMB] Aim  [LMB] Fire (while aiming)".into()),
            Some(ItemKind::Trap) => lines.push("[LMB] Place the bear trap".into()),
            _ => {}
        }
        for (i, it) in inv.pockets.iter().filter_map(|&e| items.get(e).ok()).enumerate() {
            match it.kind {
                ItemKind::Firecracker { .. } => lines.push(format!("[{}] Throw firecrackers", i + 1)),
                ItemKind::PocketKnife => lines.push("Pocket knife: frees you when Jason grabs you".into()),
                ItemKind::Map => lines.push("[M] Map".into()),
                ItemKind::WalkieTalkie => lines.push("Walkie-talkie: talks to other players (no use offline)".into()),
                _ => {}
            }
        }
    }
    text.0 = lines.join("\n");
}
