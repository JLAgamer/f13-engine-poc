//! Widget state the HUD's systems share: colours, content alpha (opacity), the widget animations and the entries'
//! hover.

use super::*;

/// A Border's ContentColorAndOpacity alpha: the opacity of everything in it (widget animations play it).
#[derive(Component, PartialEq)]
pub(super) struct ContentAlpha(pub(super) f32);

/// A widget's ColorAndOpacity as widget animations play it: an Image's own colour (over [`ColorBase`]: its brush's
/// tint and the borders' around it), or a user widget's, which multiplies everything in it (over each one's
/// [`TintBase`], its colour before).
#[derive(Component, PartialEq, Clone, Copy)]
pub(super) struct WidgetColor(pub(super) [f32; 4]);

#[derive(Component)]
pub(super) struct ColorBase(pub(super) [f32; 4]);

/// A [`RadialFill`] brush's material colour (its Color vector), which its widget's played ColorAndOpacity tints with
/// its brush's (the hold ring: hidden until its widget's OnInteractable fades it in).
#[derive(Component)]
pub(super) struct RadialTint(pub(super) [f32; 4]);

#[derive(Component)]
pub(super) struct TintBase(pub(super) [f32; 4]);

/// Applies each changed [`WidgetColor`].
pub(super) fn widget_colors(
    mut commands: Commands,
    colors: Query<(Entity, Ref<WidgetColor>, Option<&ColorBase>)>,
    children: Query<&Children>,
    mut nodes: Query<(Option<&mut ImageNode>, Option<&mut TextColor>, Option<&TintBase>)>,
    mut additive: Query<&mut AdditiveBrush>,
    (radial, mut radials): (Query<(&MaterialNode<RadialFill>, &RadialTint)>, ResMut<Assets<RadialFill>>),
) {
    for (e, c, base) in &colors {
        if !c.is_changed() {
            continue;
        }
        if let Some(b) = base {
            // An Image is built with its design colour (and the borders' alpha over it since): only a played one changes.
            if c.is_added() && c.last_changed() == c.added() {
                continue;
            }
            match nodes.get_mut(e) {
                Ok((Some(mut i), _, _)) => i.color = linear(mul(b.0, c.0)),
                Ok((None, Some(mut t), _)) => t.0 = linear(mul(b.0, c.0)),
                _ => {
                    if let Ok(mut a) = additive.get_mut(e) {
                        a.color = mul(b.0, c.0);
                    } else if let Some(mut m) = radial.get(e).ok().and_then(|(m, t)| Some((radials.get_mut(&m.0)?, t))) {
                        m.0.color = LinearRgba::from(linear(mul(mul(b.0, c.0), m.1 .0)));
                    }
                }
            }
            continue;
        }
        for d in children.iter_descendants(e) {
            let Ok((image, text, tb)) = nodes.get_mut(d) else { continue };
            let Some(current) = image.as_ref().map(|i| i.color.to_linear().to_f32_array()).or(text.as_ref().map(|t| t.0.to_linear().to_f32_array())) else { continue };
            let b = match tb {
                Some(b) => b.0,
                None => {
                    commands.entity(d).insert(TintBase(current));
                    current
                }
            };
            if let Some(mut i) = image {
                i.color = linear(mul(b, c.0));
            }
            if let Some(mut t) = text {
                t.0 = linear(mul(b, c.0));
            }
        }
    }
}

/// The alpha a widget's colour was built with, under a Border's ContentAlpha.
#[derive(Component)]
pub(super) struct BaseAlpha(pub(super) f32);

/// Each Border's content opacity on what's in it: all of it when it changes, and anything new in it. Nested Borders
/// multiply (each tints its content: SBorder's ContentScale/ColorAndOpacity), so what's in several takes all of theirs.
pub(super) fn content_alpha(
    mut commands: Commands,
    borders: Query<(Entity, Ref<ContentAlpha>)>,
    alphas: Query<&ContentAlpha>,
    children: Query<&Children>,
    parents: Query<&ChildOf>,
    mut colors: Query<(Option<&mut ImageNode>, Option<&mut TextColor>, Option<&BaseAlpha>)>,
    mut bases: Local<HashMap<Entity, f32>>,
) {
    // The bases taken this pass (their BaseAlpha inserts are deferred: a second Border over the same widget would
    // otherwise take the alpha the first just set as its base).
    bases.clear();
    for (border, alpha) in &borders {
        let changed = alpha.is_changed();
        for e in children.iter_descendants(border) {
            let Ok((image, text, base)) = colors.get_mut(e) else { continue };
            if !changed && (base.is_some() || bases.contains_key(&e)) {
                continue;
            }
            let Some(current) = image.as_ref().map(|i| i.color.alpha()).or(text.as_ref().map(|t| t.0.alpha())) else { continue };
            let base = match base.map(|b| b.0).or(bases.get(&e).copied()) {
                Some(b) => b,
                None => {
                    commands.entity(e).insert(BaseAlpha(current));
                    bases.insert(e, current);
                    current
                }
            };
            let a: f32 = parents.iter_ancestors(e).filter_map(|p| alphas.get(p).ok()).map(|a| a.0).product();
            if let Some(mut i) = image {
                i.color.set_alpha(base * a);
            }
            if let Some(mut t) = text {
                t.0.set_alpha(base * a);
            }
        }
    }
}

/// A widget animation (UWidgetAnimation): its length, each Border's content alpha curve, each widget's
/// ColorAndOpacity curves (red, green, blue, alpha), its RenderTransform's scale curves (X, Y) and its canvas slot's
/// Offsets curves (left, top, right, bottom), by widget name.
pub(super) struct WidgetAnim {
    pub(super) length: f32,
    pub(super) tracks: Vec<(String, f13_core::curve::RichCurve)>,
    pub(super) colors: Vec<(String, [f13_core::curve::RichCurve; 4])>,
    pub(super) scales: Vec<(String, [f13_core::curve::RichCurve; 2])>,
    pub(super) offsets: Vec<(String, [f13_core::curve::RichCurve; 4])>,
    /// A Border's BrushColor curves (red, green, blue, alpha).
    pub(super) brush_colors: Vec<(String, [f13_core::curve::RichCurve; 4])>,
}

/// Loads a widget class's animations' ContentColorAndOpacity alpha tracks, ColorAndOpacity tracks and RenderTransform
/// scale tracks (ui/<class>.json "Animations").
pub(super) fn load_anims(dir: &std::path::Path, class: &str) -> HashMap<String, WidgetAnim> {
    let Some(v) = std::fs::read_to_string(dir.join(format!("ui/{class}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return HashMap::new() };
    let Some(anims) = v["Animations"].as_object() else { return HashMap::new() };
    anims
        .iter()
        .map(|(name, a)| {
            let tracks = a["Tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["Property"] == "ContentColorAndOpacity")
                .map(|t| (t["Widget"].as_str().unwrap_or("").to_string(), f13_data::character::ue_rich_curve(&t["Sections"][0]["Channels"]["AlphaCurve"])))
                .collect();
            let colors = a["Tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["Property"] == "ColorAndOpacity")
                .map(|t| {
                    let ch = &t["Sections"][0]["Channels"];
                    let c = |k: &str| f13_data::character::ue_rich_curve(&ch[k]);
                    (t["Widget"].as_str().unwrap_or("").to_string(), [c("RedCurve"), c("GreenCurve"), c("BlueCurve"), c("AlphaCurve")])
                })
                .collect();
            let scales = a["Tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["Property"] == "RenderTransform" && t["Sections"][0]["Channels"]["Scale"].is_object())
                .map(|t| {
                    let ch = &t["Sections"][0]["Channels"];
                    (t["Widget"].as_str().unwrap_or("").to_string(), [f13_data::character::ue_rich_curve(&ch["Scale"]), f13_data::character::ue_rich_curve(&ch["Scale[1]"])])
                })
                .collect();
            let offsets = a["Tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["Property"] == "Offsets")
                .map(|t| {
                    let ch = &t["Sections"][0]["Channels"];
                    let c = |k: &str| f13_data::character::ue_rich_curve(&ch[k]);
                    (t["Widget"].as_str().unwrap_or("").to_string(), [c("LeftCurve"), c("TopCurve"), c("RightCurve"), c("BottomCurve")])
                })
                .collect();
            let brush_colors = a["Tracks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|t| t["Property"] == "BrushColor")
                .map(|t| {
                    let ch = &t["Sections"][0]["Channels"];
                    let c = |k: &str| f13_data::character::ue_rich_curve(&ch[k]);
                    (t["Widget"].as_str().unwrap_or("").to_string(), [c("RedCurve"), c("GreenCurve"), c("BlueCurve"), c("AlphaCurve")])
                })
                .collect();
            (name.clone(), WidgetAnim { length: a["Length"].as_f64().unwrap_or(0.0) as f32, tracks, colors, scales, offsets, brush_colors })
        })
        .collect()
}

/// The widget animations playing (UUserWidget::PlayAnimation): on which user widget (its widgets' "<owner>.<widget>"
/// names), which, since when, reversed. Each keeps its last state when over (KeepState), so it stays listed.
#[derive(Resource, Default)]
pub struct PlayingAnims(pub(super) Vec<(String, String, f32, bool)>);

impl PlayingAnims {
    pub(crate) fn play(&mut self, owner: &str, anim: &str, now: f32, reverse: bool) {
        self.0.retain(|(o, a, _, _)| !(o == owner && a == anim));
        self.0.push((owner.to_string(), anim.to_string(), now, reverse));
    }
}

/// The lobby menus' list buttons as the mouse enters and leaves them (lobby.rs gives them Interaction): each
/// button's Blueprint plays its Highlight on hover and reversed as it leaves (Counselor_Outfit_Button and
/// Counselor_Swatch_Button: Border_0's BrushColor; Counselor_Slot_Button: HighlightBorder's), 0.15 s long; here its end
/// colour (start colour on leaving) at once. (The perk and emote entries take the menu's highlight instead.)
pub(super) fn entry_hover(
    hud: Res<Hud>,
    mut loaded: Local<HashMap<String, HashMap<String, WidgetAnim>>>,
    changed: Query<(Entity, Ref<Interaction>, &Name), Changed<Interaction>>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut colors: Query<&mut WidgetColor>,
) {
    for (e, i, n) in &changed {
        // (A rebuilt tree's new button starts at None.)
        if i.is_added() {
            continue;
        }
        let class = if n.starts_with("Outfit_") {
            "Counselor_Outfit_Button"
        } else if n.starts_with("Swatch_") {
            "Counselor_Swatch_Button"
        } else if n.starts_with("Slot_Button_0") {
            "Counselor_Slot_Button"
        } else {
            continue;
        };
        let anims = loaded.entry(class.to_string()).or_insert_with(|| load_anims(&hud.export, class));
        let Some(anim) = anims.get("Highlight") else { continue };
        let t = if *i == Interaction::None { 0.0 } else { anim.length };
        for (widget, curves) in anim.colors.iter().chain(&anim.brush_colors) {
            let Some(d) = children.iter_descendants(e).find(|d| names.get(*d).is_ok_and(|x| x.as_str() == widget)) else { continue };
            if let Ok(mut c) = colors.get_mut(d) {
                c.set_if_neq(WidgetColor(curves.each_ref().map(|k| if k.keys.is_empty() { 1.0 } else { k.eval(t) })));
            }
        }
    }
}

/// Plays the widget animations: each track's Border content alpha at the animation's time (backwards when reversed).
pub(super) fn widget_anims(mut commands: Commands, time: Res<Time>, hud: Res<Hud>, playing: Res<PlayingAnims>, mut borders: Query<&mut ContentAlpha>, mut colors: Query<&mut WidgetColor>, (mut nodes, mut xforms): (Query<&mut Node>, Query<&mut UiTransform>)) {
    let now = time.elapsed_secs();
    for (owner, name, start, reverse) in &playing.0 {
        let Some(anim) = hud.anims.get(owner).and_then(|a| a.get(name)) else { continue };
        let t = (now - start).clamp(0.0, anim.length);
        let x = if *reverse { anim.length - t } else { t };
        for (widget, curve) in &anim.tracks {
            if let Some(mut a) = hud.names.get(&format!("{owner}.{widget}")).and_then(|&e| borders.get_mut(e).ok()) {
                a.set_if_neq(ContentAlpha(curve.eval(x)));
            }
        }
        // (A Border's WidgetColor is its BrushColor: see Builder::spawn. A channel without keys or a default leaves its
        // part as it is: MatchEnd_Badges' AnimIn has BadgeImage's colour without its alpha.)
        for (widget, curves) in anim.colors.iter().chain(&anim.brush_colors) {
            if let Some(mut c) = hud.names.get(&format!("{owner}.{widget}")).and_then(|&e| colors.get_mut(e).ok()) {
                c.set_if_neq(WidgetColor(curves.each_ref().map(|k| if k.keys.is_empty() { 1.0 } else { k.eval(x) })));
            }
        }
        // Its RenderTransform's scale (Counselor_Menu's HoverLeftArrow / HoverRightArrow).
        for (widget, [sx, sy]) in &anim.scales {
            let Some(&e) = hud.names.get(&format!("{owner}.{widget}")) else { continue };
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
        // A canvas slot's offsets (point anchors at the canvas's top left: position and size).
        for (widget, curves) in &anim.offsets {
            if let Some(mut n) = hud.names.get(&format!("{owner}.{widget}")).and_then(|&e| nodes.get_mut(e).ok()) {
                let [l, t, w, h] = curves.each_ref().map(|k| Val::Px(k.eval(x) * hud.scale));
                if n.left != l || n.top != t || n.width != w || n.height != h {
                    (n.left, n.top, n.width, n.height) = (l, t, w, h);
                }
            }
        }
    }
}
