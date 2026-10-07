//! The counselor's HUD from the game's own UMG widgets (`ui/*.json`, `f13-export ui`): CounselorHUDWidget's tree
//! (the timer, the inventory, the stamina ring...) and SCBreathWidget while she hides, laid out as Slate does
//! (`f13_data::umg`) on the window scaled from the 1920×1080 design, in the game's textures and font.
//!
//! What the game's code does at runtime is redone here by name: the visibilities it sets before anything shows
//! (`RUNTIME_VISIBILITY`), the timer's text, the stamina ring's fill (StaminaBarMaterial, `shaders/radial_fill.wgsl`),
//! the inventory's item widgets (SC_InventoryItem_Widget, the items' ImageOn icons) and the breath bars. The HUD is
//! rebuilt when the window's size or her pockets change.
//!
//! The counselor HUD has no health or fear meter (the game shows them on her: the limp, the screen's fear effects).
//! F3 shows the debug text instead (player.rs).

use crate::hiding::{Breathing, Hiding};
use crate::interact::Inventory;
use crate::player::Player;
use crate::props::Item;
use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};
use bevy::window::PrimaryWindow;
use f13_data::classes::ClassDb;
use f13_data::umg::{color, margin, Laid, Rect, Vis, Widget};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;

/// The HUD's design size.
const DESIGN: Vec2 = Vec2::new(1920.0, 1080.0);

/// Widgets the game's code hides or shows before the player sees them (gamepad prompts, other modes' widgets,
/// messages that come and go).
const RUNTIME_VISIBILITY: &[(&str, Vis)] = &[
    ("GameTimer", Vis::Visible),
    ("MatchStartText", Vis::Collapsed),
    ("Image_106", Vis::Collapsed),
    ("XPDisplayWidget_81", Vis::Collapsed),
    ("HUDLaggingIconWidget", Vis::Collapsed),
    ("SaveGameThreadActiveIcon_0", Vis::Collapsed),
    ("HUDChatNotificationOverlayWidget_269", Vis::Collapsed),
    ("SweaterAbilityWidget_206", Vis::Collapsed),
    ("ControllerBackground1", Vis::Collapsed),
    ("ControllerBackground2", Vis::Collapsed),
    ("ControllerBackground3", Vis::Collapsed),
    ("ControllerUse1", Vis::Collapsed),
    ("ControllerUse2", Vis::Collapsed),
    ("ControllerUse3", Vis::Collapsed),
    ("WalkieTalkie", Vis::Collapsed),
];

/// The keys ActionMapWidgets show (the game picks a key glyph or name from the input settings); the others (gamepad
/// item cycling) are hidden.
const ACTION_KEYS: &[(&str, &str)] = &[("ItemOneActionMap", "1"), ("ItemTwoActionMap", "2"), ("ItemThreeActionMap", "3"), ("ActionMapWidget_103", "Space")];

/// Shows `key` on an ActionMapWidget (its TextWidget; no glyph image), or hides the widget.
fn action_keys(w: &mut Widget) {
    if w.class.as_deref() == Some("ActionMapWidget") {
        match ACTION_KEYS.iter().find(|(n, _)| *n == w.name) {
            Some((_, key)) => {
                if let Some(t) = w.find_mut("TextWidget") {
                    t.props["Text"] = json!({ "LocalizedString": key });
                }
                // The glyph's place (it sizes the widget where it's autosized), undrawn.
                if let Some(i) = w.find_mut("ImageWidget") {
                    i.props["Brush"] = json!({ "ImageSize": { "X": (18 * key.len()).max(28), "Y": 28 } });
                    i.set_visibility(Vis::Hidden);
                }
            }
            None => w.set_visibility(Vis::Collapsed),
        }
    }
    for c in &mut w.children {
        action_keys(c);
    }
}

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/radial_fill.wgsl");
        app.add_plugins(UiMaterialPlugin::<RadialFill>::default())
            .add_systems(Startup, load_hud)
            .add_systems(Update, (build_hud, (stamina, timer, breath)).chain().run_if(resource_exists::<Hud>));
    }
}

/// The stamina ring: `ring` where `gradient` is under `fill.x`.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct RadialFill {
    #[uniform(0)]
    color: LinearRgba,
    #[uniform(1)]
    fill: Vec4,
    #[texture(2)]
    #[sampler(3)]
    ring: Handle<Image>,
    #[texture(4)]
    #[sampler(5)]
    gradient: Handle<Image>,
}

impl UiMaterial for RadialFill {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/radial_fill.wgsl".into()
    }
}

/// The HUD's trees and what it was last built for.
#[derive(Resource)]
pub struct Hud {
    hud: Widget,
    breath: Option<Widget>,
    item: Option<Widget>,
    export: PathBuf,
    classes: ClassDb,
    /// Texture name -> PNG (ui/textures.json).
    textures: HashMap<String, String>,
    regular: Handle<Font>,
    bold: Handle<Font>,
    built: Option<(Vec2, Vec<String>)>,
    root: Option<Entity>,
    /// "<user widget class>.<widget name>" -> entity.
    names: HashMap<String, Entity>,
}

/// A progress bar's fill.
#[derive(Component)]
struct Progress {
    fill: Entity,
    kind: String,
    size: Vec2,
}

fn load_hud(mut commands: Commands, export: Res<crate::ExportRoot>, assets: Res<AssetServer>) {
    let dir = &export.0;
    let Some(mut hud) = Widget::load(dir, "CounselorHUDWidget") else {
        warn!("ui: no ui/CounselorHUDWidget.json (run f13-export ui): debug HUD only");
        return;
    };
    let mut breath = Widget::load(dir, "SCBreathWidget");
    for (name, vis) in RUNTIME_VISIBILITY {
        for tree in std::iter::once(&mut hud).chain(breath.as_mut()) {
            if let Some(w) = tree.find_mut(name) {
                w.set_visibility(*vis);
            }
        }
    }
    action_keys(&mut hud);
    if let Some(b) = breath.as_mut() {
        action_keys(b);
    }
    let textures = std::fs::read_to_string(dir.join("ui").join("textures.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    commands.insert_resource(Hud {
        hud,
        breath,
        item: Widget::load(dir, "SC_InventoryItem_Widget"),
        export: dir.clone(),
        classes: ClassDb::load(&dir.join("classes.json")).unwrap_or_default(),
        textures,
        regular: assets.load("ui/fonts/AlteHaasGrotesk_0_Regular.ttf"),
        bold: assets.load("ui/fonts/AlteHaasGrotesk_0_Bold.ttf"),
        built: None,
        root: None,
        names: HashMap::new(),
    });
}

/// The texture a class property names (`Texture2D'Name'`), as an exported PNG.
fn class_texture(hud: &Hud, class: &str, prop: &str) -> Option<String> {
    let v = hud.classes.prop(class, prop).as_str()?;
    let name = v.split('\'').nth(1)?;
    hud.textures.get(name).cloned()
}

/// A PNG's size from its header.
fn png_size(path: &std::path::Path) -> Option<Vec2> {
    use std::io::Read;
    let mut head = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    let w = u32::from_be_bytes(head[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(head[20..24].try_into().ok()?);
    Some(Vec2::new(w as f32, h as f32))
}

fn build_hud(
    mut commands: Commands,
    mut hud: ResMut<Hud>,
    window: Single<&Window, With<PrimaryWindow>>,
    player: Option<Single<Option<&Inventory>, With<Player>>>,
    items: Query<&Item>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<RadialFill>>,
) {
    let size = window.size();
    if size.x < 1.0 || size.y < 1.0 {
        return;
    }
    let pockets: Vec<String> = player.as_ref().and_then(|p| **p).map_or(vec![], |inv| inv.pockets.iter().filter_map(|&e| items.get(e).ok()).map(|i| i.class.clone()).collect());
    if hud.built.as_ref().is_some_and(|(s, p)| *s == size && *p == pockets) {
        return;
    }
    if let Some(root) = hud.root.take() {
        commands.entity(root).despawn();
    }
    // The window in design units: scaled to fit 1920×1080, the rest added along the longer side.
    let scale = (size.x / DESIGN.x).min(size.y / DESIGN.y);
    let virt = size / scale;
    let mut tree = hud.hud.clone();
    fill_inventory(&hud, &mut tree, &pockets);
    let root = commands.spawn((Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, Pickable::IGNORE)).id();
    let mut names = HashMap::new();
    let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, names: &mut names };
    let full = Rect { x: 0.0, y: 0.0, w: virt.x, h: virt.y };
    let laid = tree.layout(full);
    b.spawn(&laid, root, full, "CounselorHUDWidget");
    if let Some(breath) = &hud.breath {
        let laid = breath.layout(full);
        let e = b.spawn(&laid, root, full, "SCBreathWidget");
        b.commands.entity(e).insert(Visibility::Hidden);
        names.insert("SCBreathWidget".into(), e);
    }
    hud.names = names;
    hud.root = Some(root);
    hud.built = Some((size, pockets));
}

/// Puts an item widget in the inventory's Grid for each pocket item (SCInventory_Widget does it as items are
/// picked up): its ImageOn icon, the count hidden (one item each); ObjectText names the first.
fn fill_inventory(hud: &Hud, tree: &mut Widget, pockets: &[String]) {
    if let Some(text) = tree.find_mut("ObjectText") {
        match pockets.first() {
            Some(class) => {
                let name = hud.classes.display_name(class);
                text.props["Text"] = json!({ "LocalizedString": name });
                text.set_visibility(Vis::Visible);
            }
            None => text.set_visibility(Vis::Collapsed),
        }
    }
    let (Some(item), Some(grid)) = (&hud.item, tree.find_mut("Grid")) else { return };
    grid.children.clear();
    for (i, class) in pockets.iter().enumerate() {
        let mut w = item.clone();
        w.name = format!("Item{i}");
        w.slot_kind = "HorizontalBoxSlot".into();
        w.slot = json!({});
        let icon = class_texture(hud, class, "ImageOn").or_else(|| class_texture(hud, class, "HUDPickupIcon"));
        if let Some(on) = w.find_mut("Image_On") {
            match &icon {
                Some(file) => {
                    let size = png_size(&hud.export.join(file)).unwrap_or(Vec2::splat(64.0));
                    on.props["Brush"] = json!({ "ImageSize": { "X": size.x, "Y": size.y }, "ResourceObject": { "File": file } });
                }
                None => on.set_visibility(Vis::Collapsed),
            }
        }
        for hidden in ["Image_Off", "TextBlock_0"] {
            if let Some(h) = w.find_mut(hidden) {
                h.set_visibility(Vis::Collapsed);
            }
        }
        grid.children.push(w);
    }
}

struct Builder<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    assets: &'a AssetServer,
    materials: &'a mut Assets<RadialFill>,
    hud: &'a Hud,
    scale: f32,
    /// Inside a scale box: its scale (texts are the only thing scaled).
    text_scale: f32,
    names: &'a mut HashMap<String, Entity>,
}

fn linear(c: [f32; 4]) -> Color {
    Color::linear_rgba(c[0], c[1], c[2], c[3])
}

fn mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

impl Builder<'_, '_, '_> {
    fn px(&self, v: f32) -> Val {
        Val::Px(v * self.scale)
    }

    fn node(&self, r: Rect, parent: Rect) -> Node {
        Node {
            position_type: PositionType::Absolute,
            left: self.px(r.x - parent.x),
            top: self.px(r.y - parent.y),
            width: self.px(r.w),
            height: self.px(r.h),
            ..default()
        }
    }

    /// Spawns the laid-out widget under `parent` (whose rectangle is `parent_rect`); `owner` is the user widget it's
    /// in, naming it. Returns its entity.
    fn spawn(&mut self, laid: &Laid, parent: Entity, parent_rect: Rect, owner: &str) -> Entity {
        let w = laid.widget;
        let r = laid.rect;
        let vis = if w.visibility() == Vis::Visible { Visibility::Inherited } else { Visibility::Hidden };
        let e = self.commands.spawn((self.node(r, parent_rect), vis, Pickable::IGNORE, ChildOf(parent), Name::new(w.name.clone()))).id();
        self.names.insert(format!("{owner}.{}", w.name), e);
        let p = &w.props;
        match w.kind.as_str() {
            "Image" => self.brush(e, &p["Brush"], color(&p["ColorAndOpacity"], [1.0; 4])),
            "Border" => self.brush(e, &p["Background"], color(&p["BrushColor"], [1.0; 4])),
            "TextBlock" => {
                let font = if p["Font"]["TypefaceFontName"].as_str() == Some("Bold") { self.hud.bold.clone() } else { self.hud.regular.clone() };
                let justify = match p["Justification"].as_str().unwrap_or("") {
                    "ETextJustify::Center" => Justify::Center,
                    "ETextJustify::Right" => Justify::Right,
                    _ => Justify::Left,
                };
                let mut t = self.commands.entity(e);
                t.insert((
                    Text::new(w.text()),
                    TextFont { font: font.into(), font_size: FontSize::Px(w.font_px() * self.scale * self.text_scale), ..default() },
                    TextColor(linear(color(&p["ColorAndOpacity"], [1.0; 4]))),
                    // Text sizes are estimates: overflow the box rather than wrap.
                    TextLayout { justify, linebreak: LineBreak::NoWrap },
                ));
                t.entry::<Node>().and_modify(|mut n| n.height = Val::Auto);
                let shadow = color(&p["ShadowColorAndOpacity"], [0.0; 4]);
                if shadow[3] > 0.0 {
                    let o = &p["ShadowOffset"];
                    let off = Vec2::new(o["X"].as_f64().unwrap_or(1.0) as f32, o["Y"].as_f64().unwrap_or(1.0) as f32) * self.scale;
                    t.insert(TextShadow { offset: off, color: linear(shadow) });
                }
            }
            "ProgressBar" => {
                let s = &p["WidgetStyle"];
                self.brush(e, &s["BackgroundImage"], [1.0; 4]);
                let fill = self.commands.spawn((Node { position_type: PositionType::Absolute, ..default() }, Pickable::IGNORE, ChildOf(e))).id();
                self.brush(fill, &s["FillImage"], color(&p["FillColorAndOpacity"], [1.0; 4]));
                let kind = p["BarFillType"].as_str().unwrap_or("LeftToRight").rsplit(':').next().unwrap_or("").to_string();
                let progress = Progress { fill, kind, size: Vec2::new(r.w, r.h) * self.scale };
                set_progress(self.commands, fill, &progress, p["Percent"].as_f64().unwrap_or(0.0) as f32);
                self.commands.entity(e).insert(progress);
            }
            _ => {}
        }
        let owner = w.class.as_deref().unwrap_or(owner).to_string();
        // A scale box fits its content (ScaleToFit).
        let outer = self.text_scale;
        if w.kind == "ScaleBox" {
            if let Some(c) = w.children.first() {
                let (dw, dh) = c.desired();
                if dw > 0.0 && dh > 0.0 {
                    self.text_scale *= (r.w / dw).min(r.h / dh);
                }
            }
        }
        for c in &laid.children {
            self.spawn(c, e, r, &owner);
        }
        self.text_scale = outer;
        e
    }

    /// Draws a Slate brush on `e`, tinted: its texture (a box brush 9-sliced by its margin), a UI material's first
    /// texture (the stamina ring's two as [`RadialFill`]), or a plain color.
    fn brush(&mut self, e: Entity, b: &Value, tint: [f32; 4]) {
        if b["DrawAs"].as_str() == Some("ESlateBrushDrawType::NoDrawType") {
            return;
        }
        let c = mul(color(&b["TintColor"], [1.0; 4]), tint);
        if c[3] <= 0.0 {
            return;
        }
        let res = &b["ResourceObject"];
        let textures: Vec<&str> = res["Textures"].as_array().map_or(vec![], |t| t.iter().filter_map(|t| t.as_str()).collect());
        if textures.len() == 2 && res["Scalars"]["Alpha"].is_number() {
            let v = res["Vectors"]["Color"].as_array().map_or([1.0; 4], |a| [0, 1, 2, 3].map(|i| a.get(i).and_then(|x| x.as_f64()).unwrap_or(1.0) as f32));
            let m = self.materials.add(RadialFill {
                color: LinearRgba::from(linear(mul(c, v))),
                fill: Vec4::new(res["Scalars"]["Alpha"].as_f64().unwrap_or(1.0) as f32, 0.0, 0.0, 0.0),
                ring: self.assets.load(textures[0].to_string()),
                gradient: self.assets.load(textures[1].to_string()),
            });
            self.commands.entity(e).insert(MaterialNode(m));
            return;
        }
        let file = res["File"].as_str().or(textures.first().copied());
        match file {
            Some(file) => {
                let mut image = ImageNode::new(self.assets.load(file.to_string())).with_color(linear(c));
                if b["DrawAs"].as_str() == Some("ESlateBrushDrawType::Box") {
                    let m = margin(&b["Margin"], [0.0; 4]);
                    let (sx, sy) = (b["ImageSize"]["X"].as_f64().unwrap_or(32.0) as f32, b["ImageSize"]["Y"].as_f64().unwrap_or(32.0) as f32);
                    image.image_mode = NodeImageMode::Sliced(TextureSlicer {
                        border: BorderRect { min_inset: Vec2::new(m[0] * sx, m[1] * sy), max_inset: Vec2::new(m[2] * sx, m[3] * sy) },
                        ..default()
                    });
                }
                self.commands.entity(e).insert(image);
            }
            None if res.is_null() => {
                self.commands.entity(e).insert(BackgroundColor(linear(c)));
            }
            None => {}
        }
    }
}

/// Sizes a progress bar's fill to `percent` along its fill type.
fn set_progress(commands: &mut Commands, fill: Entity, p: &Progress, percent: f32) {
    let t = percent.clamp(0.0, 1.0);
    let (w, h) = (p.size.x, p.size.y);
    let (left, top, fw, fh) = match p.kind.as_str() {
        "RightToLeft" => (w * (1.0 - t), 0.0, w * t, h),
        "FillFromCenter" => (w * (1.0 - t) / 2.0, h * (1.0 - t) / 2.0, w * t, h * t),
        "TopToBottom" => (0.0, 0.0, w, h * t),
        "BottomToTop" => (0.0, h * (1.0 - t), w, h * t),
        _ => (0.0, 0.0, w * t, h),
    };
    commands.entity(fill).entry::<Node>().and_modify(move |mut n| {
        n.left = Val::Px(left);
        n.top = Val::Px(top);
        n.width = Val::Px(fw);
        n.height = Val::Px(fh);
    });
}

/// The stamina ring's fill: her stamina.
fn stamina(
    hud: Res<Hud>,
    player: Option<Single<&Player>>,
    tuning: Res<crate::player::PlayerTuning>,
    nodes: Query<&MaterialNode<RadialFill>>,
    mut materials: ResMut<Assets<RadialFill>>,
) {
    let (Some(player), Some(&e)) = (player, hud.names.get("SCStamina_Widget.ProgressBar")) else { return };
    let Ok(node) = nodes.get(e) else { return };
    let t = (player.counselor.stamina / tuning.counselor.stamina_max).clamp(0.0, 1.0);
    // Only touched when it changes (a changed material is uploaded again).
    if materials.get(&node.0).is_some_and(|m| (m.fill.x - t).abs() > 1e-3) {
        if let Some(mut m) = materials.get_mut(&node.0) {
            m.fill.x = t;
        }
    }
}

/// The match timer (mm:ss left).
fn timer(hud: Res<Hud>, game: Res<crate::match_flow::GameMatch>, mut texts: Query<&mut Text>) {
    let Some(&e) = hud.names.get("SCCounselorTimerWidget.txtTimer") else { return };
    let Ok(mut text) = texts.get_mut(e) else { return };
    let left = game.state.remaining_time.max(0);
    let s = format!("{:02}:{:02}", left / 60, left % 60);
    if text.0 != s {
        text.0 = s;
    }
}

/// The breath widget while she hides: both bars at her breath.
fn breath(
    mut commands: Commands,
    hud: Res<Hud>,
    player: Option<Single<(Option<&Breathing>, Has<Hiding>), With<Player>>>,
    tuning: Option<Res<crate::hiding::BreathTuning>>,
    mut vis: Query<&mut Visibility>,
    bars: Query<&Progress>,
) {
    let Some(&root) = hud.names.get("SCBreathWidget") else { return };
    let (breathing, hiding) = player.map_or((None, false), |p| *p);
    let shown = hiding && breathing.is_some();
    if let Ok(mut v) = vis.get_mut(root) {
        v.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
    }
    let (Some(b), Some(t)) = (breathing, tuning) else { return };
    let percent = b.breath.current / t.0.max;
    for name in ["SCBreathWidget.ProgressBar_121", "SCBreathWidget.ProgressBar_231"] {
        if let Some(p) = hud.names.get(name).and_then(|&e| bars.get(e).ok()) {
            set_progress(&mut commands, p.fill, p, percent);
        }
    }
}
