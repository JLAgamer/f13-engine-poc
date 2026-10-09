//! Interaction prompts (SCInteractWidget): the widget an interactable shows the local player at its interaction
//! point while it's highlighted, with its InteractionIconWidgetClass in its IconBox.
//!
//! SCInteractComponent (decompiled): its InteractionWidgetClass (the constructor 0x14032e320's SCInteractionWidget,
//! or a class's own: the doors' SCInteractionDoorWidget) is created for the locally controlled character
//! (FUN_140336ad0, gated by FUN_140339c80) as it starts being highlighted or becomes the best interactable; the
//! manager's events then drive it:
//! - OnStartHighlight (vtable 0x528, FUN_14035ee40): ShowWidget, its OnShowPreview (the small ring growing in).
//! - OnBecameBest (FUN_1403395f0): OnIsInteractable, its OnInteractable (the key's button and ring) and the icon's
//!   ShowIcons (AnimHide reversed: the icon fades in).
//! - OnLostBest (FUN_1403396e0): OnNotInteractable (highlighted) or OnNotInteractableOrHighlighted: OnInteractable
//!   reversed and the icon's HideIcons.
//! - OnStopHighlight (vtable 0x530, FUN_14035fa30): HideWidget (OnShowPreview reversed, the icons hidden) or, while
//!   it was the best, OnNotInteractableOrHighlighted; its tick (0x140362d70) cleans the widget up 10 s later unless
//!   it's highlighted again.
//! UpdatePosition (0x14044a900) puts it each frame at the interaction point projected to the screen, centred on its
//! desired size (x -0.5), and collapses it when the point doesn't project.
//!
//! Highlighted (the manager's tick FUN_140279100): CanInteractWith (vtable 0x500) and the highlight test (vtable 0x538,
//! FUN_140240fb0): bCanHighlight, the character's location within HighlightDistance of the interaction point (in 3D,
//! or flat with HighlightZDistance apart in height when bUseSeperateZBounds), within YawHighlightLimit of its facing
//! when bUseYawHighlightLimit, and not occluded when bCheckForOcclusion. (bMustFaceBestInteractable, which would also
//! need it in front of the view, is off.) Each lister (her interactions, Jason's placed kills) fills [`Prompts`].

use super::*;
use crate::player::PlayerCamera;

/// SCInteractComponent's tick: a widget no longer highlighted is cleaned up this long after (0x41200000 = 10 s).
const CLEANUP_S: f32 = 10.0;

/// UpdatePosition's centring (DAT_1420c33dc): the widget's desired size times this.
const CENTRE: f32 = -0.5;

/// Who listed a prompt: each lister replaces its own every frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PromptSource {
    Counselor,
    PlacedKill,
}

/// An interactable's prompt as the local player sees it now.
#[derive(Clone, Debug)]
pub struct Prompt {
    pub source: PromptSource,
    /// The interactable (an entity and which of its interactables).
    pub id: (Entity, u8),
    /// GetInteractionLocation (world).
    pub at: Vec3,
    /// Its InteractionWidgetClass and InteractionIconWidgetClass (without `_C`).
    pub widget: String,
    pub icon: Option<String>,
    pub highlighted: bool,
    pub best: bool,
}

/// The prompts listed this frame (highlighted or best); one missing is neither.
#[derive(Resource, Default)]
pub struct Prompts(Vec<Prompt>);

impl Prompts {
    /// Replaces `source`'s prompts with `list`.
    pub fn set(&mut self, source: PromptSource, list: impl IntoIterator<Item = Prompt>) {
        self.0.retain(|p| p.source != source);
        self.0.extend(list);
    }
}

/// A prompt's widget on screen.
struct Shown {
    root: Entity,
    names: HashMap<String, Entity>,
    widget: String,
    icon: Option<String>,
    /// Its desired size (design units).
    size: Vec2,
    at: Vec3,
    highlighted: bool,
    best: bool,
    /// IconsVisible.
    icons: bool,
    /// Its animations played (user widget class, animation, start, reversed); the last of each kept.
    anims: Vec<(String, String, f32, bool)>,
    /// When it stopped being highlighted (its cleanup timer).
    stopped: Option<f32>,
}

impl Shown {
    fn play(&mut self, owner: &str, anim: &str, now: f32, reverse: bool) {
        self.anims.retain(|(o, a, _, _)| !(o == owner && a == anim));
        self.anims.push((owner.to_string(), anim.to_string(), now, reverse));
    }

    /// ShowIcons / HideIcons: the icon's AnimHide reversed / forward.
    fn icons(&mut self, show: bool, now: f32) {
        if let Some(icon) = self.icon.clone() {
            self.play(&icon, "AnimHide", now, show);
        }
        self.icons = show;
    }
}

#[derive(Resource, Default)]
pub(super) struct PromptUi {
    shown: HashMap<(Entity, u8), Shown>,
    /// Each class's animations.
    anims: HashMap<String, HashMap<String, WidgetAnim>>,
    scale: f32,
}

/// Builds `widget` with `icon` in its IconBox: its tree, laid out at its desired size.
fn build(b: &mut Builder, dir: &std::path::Path, widget: &str, icon: Option<&str>) -> Option<(Entity, Vec2)> {
    let mut tree = Widget::load(dir, widget)?;
    if let (Some(icon), Some(box_)) = (icon.and_then(|i| Widget::load(dir, i).map(|t| (i, t))), tree.find_mut("IconBox")) {
        // The Blueprint's Construct: CreateWidget(InteractionIconWidgetClass), AddChildToHorizontalBox(IconBox).
        box_.children.push(Widget {
            kind: format!("{}_C", icon.0),
            name: "InteractionIconWidget".into(),
            class: Some(icon.0.to_string()),
            props: json!({}),
            slot_kind: "HorizontalBoxSlot".into(),
            slot: json!({}),
            children: vec![icon.1],
        });
    }
    action_keys(&mut tree);
    let (w, h) = tree.desired();
    let rect = Rect { x: 0.0, y: 0.0, w, h };
    let root = b
        .commands
        .spawn((Node { position_type: PositionType::Absolute, width: Val::Px(w * b.scale), height: Val::Px(h * b.scale), ..default() }, GlobalZIndex(5), Visibility::Hidden, Pickable::IGNORE))
        .id();
    let laid = tree.layout(rect);
    b.spawn(&laid, root, rect, widget);
    Some((root, Vec2::new(w, h)))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prompts(
    mut commands: Commands,
    hud: Res<Hud>,
    mut ui: ResMut<PromptUi>,
    list: Res<Prompts>,
    time: Res<Time>,
    window: Single<&Window, With<PrimaryWindow>>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<RadialFill>>,
    camera: Query<(&Camera, &GlobalTransform), With<PlayerCamera>>,
    mut nodes: Query<(&mut Node, &mut Visibility)>,
    (mut colors, mut alphas, mut xforms): (Query<&mut WidgetColor>, Query<&mut ContentAlpha>, Query<&mut UiTransform>),
) {
    let now = time.elapsed_secs();
    let size = window.size();
    if size.x < 1.0 || size.y < 1.0 {
        return;
    }
    let scale = (size.x / DESIGN.x).min(size.y / DESIGN.y);
    let ui = &mut *ui;
    if scale != ui.scale {
        // Rebuilt at the new size.
        for (_, s) in ui.shown.drain() {
            commands.entity(s.root).despawn();
        }
        ui.scale = scale;
    }
    // The manager's events, from last frame's flags to this frame's.
    let mut seen = Vec::new();
    for p in &list.0 {
        seen.push(p.id);
        let (highlight, best) = (p.highlighted, p.best);
        if !ui.shown.contains_key(&p.id) {
            if !highlight && !best {
                continue;
            }
            let mut names = HashMap::new();
            let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, content: [1.0; 4], names: &mut names };
            let Some((root, wsize)) = build(&mut b, &hud.export, &p.widget, p.icon.as_deref()) else {
                warn!("ui: no ui/{}.json (re-export with f13-export ui): no prompt", p.widget);
                continue;
            };
            for class in std::iter::once(&p.widget).chain(&p.icon) {
                if !ui.anims.contains_key(class) {
                    ui.anims.insert(class.clone(), load_anims(&hud.export, class));
                }
            }
            ui.shown.insert(
                p.id,
                Shown { root, names, widget: p.widget.clone(), icon: p.icon.clone(), size: wsize, at: p.at, highlighted: false, best: false, icons: false, anims: vec![], stopped: None },
            );
        }
        let s = ui.shown.get_mut(&p.id).expect("built");
        s.at = p.at;
        update(s, highlight, best, now);
    }
    for (id, s) in ui.shown.iter_mut() {
        if !seen.contains(id) {
            update(s, false, false, now);
        }
    }
    // Cleaned up 10 s after it stopped being highlighted.
    ui.shown.retain(|_, s| {
        let keep = s.stopped.is_none_or(|t| now - t < CLEANUP_S);
        if !keep {
            commands.entity(s.root).despawn();
        }
        keep
    });
    // UpdatePosition.
    let cam = camera.iter().find(|(c, _)| c.is_active);
    for s in ui.shown.values() {
        let Ok((mut node, mut vis)) = nodes.get_mut(s.root) else { continue };
        let at = cam.and_then(|(c, g)| c.world_to_viewport(g, s.at).ok());
        match at {
            Some(px) => {
                let left = px.x + s.size.x * scale * CENTRE;
                let top = px.y + s.size.y * scale * CENTRE;
                if node.left != Val::Px(left) || node.top != Val::Px(top) {
                    node.left = Val::Px(left);
                    node.top = Val::Px(top);
                }
                vis.set_if_neq(Visibility::Inherited);
            }
            None => {
                vis.set_if_neq(Visibility::Hidden);
            }
        }
    }
    // Their animations.
    for s in ui.shown.values() {
        for (owner, name, start, reverse) in &s.anims {
            let Some(anim) = ui.anims.get(owner).and_then(|a| a.get(name)) else { continue };
            let t = (now - start).clamp(0.0, anim.length);
            let x = if *reverse { anim.length - t } else { t };
            let named = |w: &str| s.names.get(&format!("{owner}.{w}")).copied();
            for (w, curve) in &anim.tracks {
                if let Some(mut a) = named(w).and_then(|e| alphas.get_mut(e).ok()) {
                    a.set_if_neq(ContentAlpha(curve.eval(x)));
                }
            }
            for (w, curves) in &anim.colors {
                if let Some(mut c) = named(w).and_then(|e| colors.get_mut(e).ok()) {
                    c.set_if_neq(WidgetColor(curves.each_ref().map(|k| k.eval(x))));
                }
            }
            for (w, [sx, sy]) in &anim.scales {
                let Some(e) = named(w) else { continue };
                let v = Vec2::new(sx.eval(x), sy.eval(x));
                match xforms.get_mut(e) {
                    Ok(mut t) => {
                        if t.scale != v {
                            t.scale = v;
                        }
                    }
                    Err(_) => {
                        commands.entity(e).insert(UiTransform { scale: v, ..default() });
                    }
                }
            }
        }
    }
}

/// The manager's events for a prompt going from its last flags to `highlight` / `best`.
fn update(s: &mut Shown, highlight: bool, best: bool, now: f32) {
    let widget = s.widget.clone();
    if highlight && !s.highlighted {
        // OnStartHighlight: ShowWidget.
        s.play(&widget, "OnShowPreview", now, false);
    }
    if best && !s.best {
        // OnBecameBest: OnIsInteractable (unless its icons already show).
        if !s.icons {
            s.play(&widget, "OnInteractable", now, false);
            s.icons(true, now);
        }
    }
    if !best && s.best {
        // OnLostBest: OnNotInteractable / OnNotInteractableOrHighlighted.
        s.play(&widget, "OnInteractable", now, true);
        if s.icons {
            s.icons(false, now);
        }
    }
    if !highlight && s.highlighted {
        // OnStopHighlight: HideWidget (or, still the best, OnNotInteractableOrHighlighted).
        if best {
            s.play(&widget, "OnInteractable", now, true);
        } else {
            s.play(&widget, "OnShowPreview", now, true);
        }
        if s.icons {
            s.icons(false, now);
        }
    }
    if highlight || best {
        s.stopped = None;
    } else if s.highlighted || s.best {
        s.stopped = Some(now);
    }
    s.highlighted = highlight;
    s.best = best;
}
