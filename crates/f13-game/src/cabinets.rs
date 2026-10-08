//! Searching cabinets and desks (SCCabinet; SCDoubleCabinet has two drawers, its "2" functions mirroring the first),
//! decompiled:
//! - **Press** on a closed drawer (OnInteract 0x1403505b0, method 1): BeginSpecial on its
//!   UseCabinet_SpecialMoveHandler (Counselor_OpenDrawer, or OpenDrawerLow for desks: AM_normal_stand_drawer_open_mid /
//!   _low). She slides onto the handler's spot, then the montage plays from its Open section (Open -> Idle, Idle
//!   looping).
//! - **UseCabinetStarted** (0x140365070): the drawer is open, she is its interactor, and a timer of the Open section's
//!   length runs (FUN_1403407b0): then the drawer is ready (0x430); if she can't take what it holds
//!   ([`can_take`], FUN_1402df210) it closes at once.
//! - **Ready** (CanInteractWith 0x140339d20: press and hold only while ready and she can take the item; nothing on an
//!   open drawer otherwise): a press closes it, a hold of the interactable's HoldTimeLimit ([`TAKE_HOLD_S`], its
//!   constructor) takes the item.
//! - **Close** (MULTICAST_Close 0x14034e180, take or not): not ready; not taking: an empty drawer stops her montage
//!   and the move at once, else the montage jumps to Close and the move ends after it. Taking: Take, then TakeClose
//!   (the same segment again) when a small item meets full pockets; the item is hers at the montage's OnPickupItem
//!   notify, the selected pocket's item going into the drawer in its place (her pickup, 0x1403a4e30), which closes
//!   on it when the move ends.
//! - **The move ends** (OpenCloseCabinetCompleted 0x140353870; UseCabinetAborted the same): no interactor; a drawer
//!   still holding an item closes (bIsOpen false, AnimatedMesh back to CloseTransform). An emptied drawer stays open,
//!   and an open drawer can't be used again.
//! - **The drawer** (Tick 0x140360f10, while she's the interactor): once her montage is fully blended in, it records her
//!   `target_jnt` bone's component-space pose at InitialTargetBoneTime ([`INITIAL_TARGET_BONE_TIME_S`]) into the
//!   montage (evaluating the montage there), then each frame sets the AnimatedMesh to CloseTransform x (the bone's
//!   pose now relative to that one). With bItemShouldFollowAnimatedMesh the ItemAttachLocation (the item on it) follows
//!   the FollowLocation on the drawer; at BeginPlay (0x1403376c0) it's put there too.
//!
//! Here the bone's poses are sampled from her takes of the montage's sections (the montage plays them full-body once
//! blended in).

use crate::character::PlayAction;
use crate::interact::{Carried, Inventory};
use crate::player::Player;
use crate::props::{InCabinet, Item};
use bevy::prelude::*;
use std::collections::HashMap;

/// The cabinet interactable's HoldTimeLimit (SCCabinet's constructor): holding this long takes the item.
pub const TAKE_HOLD_S: f32 = 1.0;
/// SCCabinet's InitialTargetBoneTime (constructor).
pub const INITIAL_TARGET_BONE_TIME_S: f32 = 0.1;
/// The bone the drawer follows.
const TARGET_BONE: &str = "target_jnt";

#[derive(Component)]
pub struct Cabinet {
    pub slots: Vec<Slot>,
}

/// One drawer: its interactable, special move and moving mesh.
pub struct Slot {
    /// Interact point in the cabinet's space, and its range in cm.
    pub interact: Vec3,
    pub reach: f32,
    /// The special move's spot (UseCabinet_SpecialMoveHandler), cabinet space.
    pub spot: Transform,
    /// Counselor_OpenDrawerLow (desks) rather than Counselor_OpenDrawer.
    pub low: bool,
    /// The AnimatedMesh.
    pub drawer: Option<Entity>,
    /// CloseTransform: the AnimatedMesh's place when closed (cabinet space).
    pub close: Transform,
    /// FollowLocation (on the drawer) and ItemAttachLocation, cabinet space with the drawer closed; None when the item
    /// doesn't follow the drawer.
    pub follow: Option<(Transform, Transform)>,
    pub open: bool,
    /// The Open section is done (0x430): she can close it or take the item.
    pub ready: bool,
    pub interactor: Option<Entity>,
    /// Her target bone's pose at InitialTargetBoneTime (rig space).
    initial: Option<Transform>,
    /// The item in it, and its world transform as spawned (on the ItemAttachLocation).
    pub item: Option<(Entity, Transform)>,
}

impl Slot {
    pub fn new(interact: Vec3, reach: f32, spot: Transform, low: bool, drawer: Option<Entity>, close: Transform, follow: Option<(Transform, Transform)>) -> Self {
        Slot { interact, reach, spot, low, drawer, close, follow, open: false, ready: false, interactor: None, initial: None, item: None }
    }

    /// Her take of a section of the montage.
    pub fn take(&self, section: Section) -> String {
        take_name(self.low, section)
    }

    /// She can search it now.
    pub fn searchable(&self) -> bool {
        !self.open && self.interactor.is_none()
    }
}

/// SCCharacter's check before taking (FUN_1402df210): an item, and not already the item's InventoryMax of its class.
pub fn can_take(item: Option<&Item>, inv: &Inventory, items: &Query<&Item>) -> bool {
    let Some(item) = item else { return false };
    let held = inv.hand.into_iter().chain(inv.pockets.iter().copied()).filter(|&e| items.get(e).is_ok_and(|i| i.class == item.class)).count();
    !(item.inventory_max > 0 && held as u32 >= item.inventory_max)
}

fn take_name(low: bool, section: Section) -> String {
    format!("f_normal_stand_drawer_{}_{}", if low { "low" } else { "mid" }, section.take())
}

/// The montage's sections her takes play.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Open,
    Idle,
    Close,
    Take,
}

impl Section {
    fn name(self) -> &'static str {
        match self {
            Section::Open => "Open",
            Section::Idle => "Idle",
            Section::Close => "Close",
            Section::Take => "Take",
        }
    }

    fn take(self) -> &'static str {
        match self {
            Section::Open => "open",
            Section::Idle => "idle",
            Section::Close => "close",
            Section::Take => "pickup_prop",
        }
    }
}

/// She's searching a drawer: her montage's time and section, and the move's timers.
#[derive(Component)]
pub struct CabinetUse {
    pub cabinet: Entity,
    pub slot: usize,
    pub section: Section,
    /// Seconds into the montage.
    t: f32,
    /// UseCabinetStarted's timer to ready.
    ready_in: Option<f32>,
    /// The move's end (FUN_14043e9e0's timer).
    end_in: Option<f32>,
    picked: bool,
    /// TakeClose follows Take (a small item swapped for her selected one).
    take_close: bool,
}

/// Closes the drawer she's at (MULTICAST_Close), taking its item or not.
#[derive(Event)]
pub struct CloseCabinet {
    pub player: Entity,
    pub take: bool,
}

pub struct CabinetsPlugin;

impl Plugin for CabinetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DrawerPoses>()
            .add_observer(close_cabinet)
            .add_observer(cabinet_move_ended)
            .add_systems(Update, link_items);
    }
}

/// Each spawned item goes in its drawer (SCCabinet::SpawnItem / OnRep_ContainedItem), where it was put.
fn link_items(items: Query<(Entity, &InCabinet, &Transform), Added<InCabinet>>, mut cabinets: Query<&mut Cabinet>) {
    for (e, inc, xf) in &items {
        if let Ok(mut c) = cabinets.get_mut(inc.0) {
            if let Some(s) = c.slots.get_mut(inc.1) {
                s.item = Some((e, *xf));
            }
        }
    }
}

/// Starts the drawer's special move: she slides onto its spot, then her montage begins (UseCabinetStarted).
pub fn search(commands: &mut Commands, player: Entity, from: Transform, cabinet: Entity, slot: usize, spot: Transform, params: crate::interact::MoveParams) {
    let at = spot.translation;
    crate::interact::move_to_spot(commands, player, from, spot, params, move |w| {
        let take = {
            let Some(mut cab) = w.get_mut::<Cabinet>(cabinet) else { return };
            let Some(s) = cab.slots.get_mut(slot) else { return };
            s.open = true;
            s.interactor = Some(player);
            s.take(Section::Open)
        };
        let open = w.get_resource::<crate::character::CounselorCharacter>().and_then(|m| m.info.animations.get(&take)).and_then(|a| a.section("Open"));
        let ready_in = open.map_or(0.0, |(a, b)| b - a);
        w.entity_mut(player).insert((CabinetUse { cabinet, slot, section: Section::Open, t: 0.0, ready_in: Some(ready_in), end_in: None, picked: false, take_close: false }, PlayAction(take)));
        // NativeDestinationReached: the handler takes the view (bTakeCameraControl: the constructor's true) to its
        // CabinetCam.
        let handler = if slot == 0 { "UseCabinet_SpecialMoveHandler" } else { "UseCabinet_SpecialMoveHandler2" };
        w.trigger(crate::interaction_camera::PlayInteractionCamera { actor: cabinet, handler });
        info!("searching a cabinet from ({:.2}, {:.2}, {:.2})", at.x, at.y, at.z);
    });
}

/// OpenCloseCabinetCompleted / UseCabinetAborted: however her drawer move ends, the view goes back to her.
fn cabinet_move_ended(_: On<Remove, CabinetUse>, mut commands: Commands) {
    commands.trigger(crate::interaction_camera::StopInteractionCamera);
}

/// MULTICAST_Close: not ready any more; the montage to Close or Take, or (empty, not taking) the move ends.
fn close_cabinet(
    ev: On<CloseCabinet>,
    mut players: Query<(&mut CabinetUse, &Inventory)>,
    mut cabinets: Query<&mut Cabinet>,
    items: Query<&Item>,
    model: Option<Res<crate::character::CounselorCharacter>>,
) {
    let Ok((mut u, inv)) = players.get_mut(ev.player) else { return };
    let Ok(mut cab) = cabinets.get_mut(u.cabinet) else { return };
    let Some(slot) = cab.slots.get_mut(u.slot) else { return };
    slot.ready = false;
    u.ready_in = None;
    if !ev.take && slot.item.is_none() {
        u.end_in = Some(0.0);
        return;
    }
    let section = if ev.take { Section::Take } else { Section::Close };
    let bounds = model.as_ref().and_then(|m| m.info.animations.get(&slot.take(section))).and_then(|a| a.section(section.name()));
    let (start, end) = bounds.unwrap_or((0.0, 0.0));
    u.section = section;
    u.t = start;
    u.end_in = Some(end - start);
    // A small, not special item with her pockets full: TakeClose after Take (SetNextSection), its length added.
    let small = slot.item.and_then(|(e, _)| items.get(e).ok()).is_some_and(|i| !i.large && !i.special);
    if ev.take && small && inv.pockets.len() >= inv.max_small {
        let take_close = model.as_ref().and_then(|m| m.info.animations.get(&slot.take(section))).and_then(|a| a.section("TakeClose"));
        if let Some((a, b)) = take_close {
            u.take_close = true;
            u.end_in = Some(end - start + b - a);
        }
    }
}

/// Each drawer's montage: sections, timers, the item taken at OnPickupItem, the drawer following her bone, the move's
/// end.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cabinet_uses(
    mut commands: Commands,
    time: Res<Time>,
    model: Option<Res<crate::character::CounselorCharacter>>,
    export: Res<crate::ExportRoot>,
    mut poses: ResMut<DrawerPoses>,
    mut players: Query<(Entity, &mut CabinetUse, &mut Inventory, &Transform, Has<crate::jason::Held>), With<Player>>,
    mut cabinets: Query<&mut Cabinet>,
    mut drawers: Query<&mut Transform, (With<crate::props::Drawer>, Without<Player>)>,
    items: Query<&Item>,
    mut visibility: Query<&mut Visibility>,
) {
    let Some(model) = model else { return };
    let dt = time.delta_secs();
    for (pe, mut u, mut inv, pxf, held) in &mut players {
        let Ok(mut cab) = cabinets.get_mut(u.cabinet) else {
            commands.entity(pe).remove::<(CabinetUse, PlayAction)>();
            continue;
        };
        let Some(slot) = cab.slots.get_mut(u.slot) else { continue };
        let low = slot.low;
        let anim = |section: Section| model.info.animations.get(&take_name(low, section));
        let bounds = |section: Section| anim(section).and_then(|a| a.section(section.name()));
        let before = u.t;
        u.t += dt;
        // Open runs into Idle, which loops.
        if u.section == Section::Open && bounds(Section::Open).is_some_and(|(_, end)| u.t >= end) {
            u.section = Section::Idle;
        }
        if u.section == Section::Idle {
            if let Some((start, end)) = bounds(Section::Idle) {
                if u.t >= end {
                    u.t = start + (u.t - end) % (end - start).max(1e-3);
                }
            }
        }
        commands.entity(pe).insert(PlayAction(slot.take(u.section)));

        // The ready timer; a drawer she can't take from closes.
        if let Some(r) = u.ready_in.as_mut() {
            *r -= dt;
            if *r <= 0.0 {
                u.ready_in = None;
                slot.ready = true;
                let item = slot.item.and_then(|(e, _)| items.get(e).ok());
                if !can_take(item, &inv, &items) {
                    commands.trigger(CloseCabinet { player: pe, take: false });
                }
            }
        }

        // Take into TakeClose: the same segment again.
        if u.section == Section::Take && u.take_close && bounds(Section::Take).is_some_and(|(_, end)| u.t >= end) {
            if let Some((start, _)) = anim(Section::Take).and_then(|a| a.section("TakeClose")) {
                u.t = start;
                u.take_close = false;
                commands.entity(pe).insert(crate::character::RestartAction);
            }
        }

        // OnPickupItem: the item is hers.
        if u.section == Section::Take && !u.picked {
            if let Some((at, _)) = anim(Section::Take).and_then(|a| a.window("OnPickupItem")) {
                if before < at && u.t >= at {
                    u.picked = true;
                    if let Some((e, rest)) = slot.item.take() {
                        if let Ok(item) = items.get(e) {
                            if item.large {
                                if let Some(old) = inv.hand.take() {
                                    crate::interact::drop_at(&mut commands, &mut visibility, old, pxf);
                                }
                                inv.hand = Some(e);
                            } else {
                                crate::interact::take_small(&mut commands, &mut visibility, &mut inv, item, e, crate::interact::Swap::Cabinet(u.cabinet, u.slot, rest));
                            }
                            info!("took {} from a cabinet", item.name);
                            commands.trigger(crate::match_flow::ClassScore::new(&item.class, "PickupScoreEvent"));
                        }
                        commands.entity(e).insert(Carried).remove::<InCabinet>();
                        if let Ok(mut v) = visibility.get_mut(e) {
                            *v = Visibility::Hidden;
                        }
                    }
                }
            }
        }

        // The drawer follows her target bone once the montage is fully blended in.
        let blended = anim(Section::Open).and_then(|a| a.blend_in).unwrap_or(0.0);
        if u.t >= blended || u.section != Section::Open {
            let open = slot.take(Section::Open);
            let initial = slot.initial.or_else(|| poses.pose(&export.0, &model.info, &open, INITIAL_TARGET_BONE_TIME_S));
            slot.initial = initial;
            let take = slot.take(u.section);
            let local_t = u.t - bounds(u.section).map_or(0.0, |(start, _)| start);
            let current = poses.pose(&export.0, &model.info, &take, local_t);
            if let (Some(initial), Some(current), Some(mut d)) = (initial, current, slot.drawer.and_then(|d| drawers.get_mut(d).ok())) {
                *d = Transform::from_matrix(slot.close.to_matrix() * initial.to_matrix().inverse() * current.to_matrix());
            }
        }

        // The move ends: by its timer, grabbed out of it (UseCabinetAborted), or with the montage.
        if let Some(e) = u.end_in.as_mut() {
            *e -= dt;
        }
        if held || u.end_in.is_some_and(|e| e <= 0.0) {
            slot.interactor = None;
            slot.initial = None;
            slot.ready = false;
            if slot.item.is_some() {
                slot.open = false;
                if let Some(mut d) = slot.drawer.and_then(|d| drawers.get_mut(d).ok()) {
                    *d = slot.close;
                }
            }
            commands.entity(pe).remove::<(CabinetUse, PlayAction)>();
            info!("done with the cabinet ({})", if slot.open { "left open" } else { "closed" });
        }
    }
}

/// Items in drawers ride on them: the ItemAttachLocation is put on the FollowLocation, which moves with the drawer.
pub(crate) fn follow_items(cabinets: Query<(&Cabinet, &GlobalTransform)>, drawers: Query<&Transform, (With<crate::props::Drawer>, Without<Item>)>, mut items: Query<&mut Transform, (With<Item>, Without<Carried>)>) {
    for (cab, gxf) in &cabinets {
        let cab_m = gxf.to_matrix();
        for slot in &cab.slots {
            let (Some((item, rest)), Some((follow, attach))) = (slot.item, slot.follow) else { continue };
            let drawer = slot.drawer.and_then(|d| drawers.get(d).ok()).copied().unwrap_or(slot.close);
            // Where the attach point is now, relative to where the item was spawned on it.
            let moved = drawer.to_matrix() * slot.close.to_matrix().inverse() * follow.to_matrix() * attach.to_matrix().inverse();
            if let Ok(mut t) = items.get_mut(item) {
                *t = Transform::from_matrix(cab_m * moved * cab_m.inverse() * rest.to_matrix());
            }
        }
    }
}

/// Her target bone's component-space pose through each drawer take, sampled per frame (rig space).
#[derive(Resource, Default)]
pub struct DrawerPoses(HashMap<String, Option<(Vec<Transform>, f32)>>);

impl DrawerPoses {
    /// The bone's pose `t` seconds into `take` (held at its ends).
    fn pose(&mut self, export: &std::path::Path, info: &f13_data::character::CharacterInfo, take: &str, t: f32) -> Option<Transform> {
        let entry = self.0.entry(take.to_string()).or_insert_with(|| {
            let a = info.animations.get(take)?;
            let data = f13_data::character::AnimData::load(&export.join(&a.file)).map_err(|e| warn!("{take}: {e}")).ok()?;
            let frames = bone_frames(info, &data, TARGET_BONE);
            if frames.is_none() {
                warn!("{take}: no {TARGET_BONE}");
            }
            frames.map(|f| (f, a.length))
        });
        let (frames, length) = entry.as_ref()?;
        let n = frames.len();
        if n == 0 {
            return None;
        }
        let f = (t / length.max(1e-3)).clamp(0.0, 1.0) * (n - 1) as f32;
        let (i, a) = (f.floor() as usize, f.fract());
        let (p, q) = (frames[i.min(n - 1)], frames[(i + 1).min(n - 1)]);
        Some(Transform { translation: p.translation.lerp(q.translation, a), rotation: p.rotation.slerp(q.rotation, a), scale: p.scale.lerp(q.scale, a) })
    }
}

/// A bone's component-space transform at each frame of an animation: its chain from the root, each bone's sampled
/// rotation (and translation for bones whose translation comes from the animation) over its reference pose.
fn bone_frames(info: &f13_data::character::CharacterInfo, data: &f13_data::character::AnimData, bone: &str) -> Option<Vec<Transform>> {
    let mut chain = Vec::new();
    let mut at = info.bones.iter().position(|b| b.name == bone)? as i32;
    while at >= 0 {
        chain.push(at as usize);
        at = info.bones[at as usize].parent;
    }
    chain.reverse();
    let tracks: HashMap<&str, &f13_data::character::BoneTrack> = data.tracks.iter().map(|t| (t.bone.as_str(), t)).collect();
    Some(
        (0..data.frames.max(1))
            .map(|f| {
                chain.iter().fold(Transform::IDENTITY, |parent, &i| {
                    let b = &info.bones[i];
                    let mut local = b.local.as_ref().map_or(Transform::IDENTITY, crate::to_transform);
                    if let Some(track) = tracks.get(b.name.as_str()) {
                        if let Some(r) = track.rotation.get(f) {
                            local.rotation = Quat::from_array(*r).normalize();
                        }
                        if b.retarget == "Animation" {
                            if let Some(t) = track.translation.as_ref().and_then(|v| v.get(f)) {
                                local.translation = Vec3::from_array(*t);
                            }
                        }
                    }
                    parent * local
                })
            })
            .collect(),
    )
}
