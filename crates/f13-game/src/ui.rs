//! The counselor's HUD from the game's own UMG widgets (`ui/*.json`, `f13-export ui`): CounselorHUDWidget's tree
//! (the timer, the inventory, the stamina ring...) and SCBreathWidget while she hides, laid out as Slate does
//! (`f13_data::umg`) on the window scaled from the 1920×1080 design, in the game's textures and font.
//!
//! What the game's code does at runtime is redone here by name: the visibilities it sets before anything shows
//! (`RUNTIME_VISIBILITY`), the timer's text, the stamina ring's fill (StaminaBarMaterial, `shaders/radial_fill.wgsl`),
//! the inventory's item widgets (SC_InventoryItem_Widget, the items' ImageOn icons), the breath bars and the
//! interaction dial (SCInteractMinigame_Widget: the progress circle, the needle, the pips as arcs of the ring, the
//! mouse button of the next pip). The HUD is rebuilt when the window's size or her pockets change.
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
    // The dial's pip template (the pips are drawn as arcs).
    ("Image_0", Vis::Collapsed),
];

/// The pips' colors: waiting (the template pip's tint), hit, missed.
const PIP_WAITING: [f32; 4] = [0.854993, 0.030713, 0.017642, 1.0];
const PIP_HIT: [f32; 4] = [0.552012, 0.527115, 0.434154, 1.0];
const PIP_MISSED: [f32; 4] = [0.854993, 0.030713, 0.017642, 0.2];

/// The keys ActionMapWidgets show (the game picks a key glyph or name from the input settings); the others (gamepad
/// item cycling) are hidden.
const ACTION_KEYS: &[(&str, &str)] = &[
    ("ItemOneActionMap", "1"),
    ("ItemTwoActionMap", "2"),
    ("ItemThreeActionMap", "3"),
    ("ActionMapWidget_103", "Space"),
    // The dial's pip keys (pip key 0 / 1).
    ("Interact1", "RMB"),
    ("Interact2", "LMB"),
];

/// Shows `key` on an ActionMapWidget (its TextWidget; no glyph image), or hides the widget.
fn action_keys(w: &mut Widget) {
    if matches!(w.class.as_deref(), Some("ActionMapWidget" | "MinigameActionMapWidget")) {
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
        embedded_asset!(app, "shaders/minimap.wgsl");
        app.add_plugins((UiMaterialPlugin::<RadialFill>::default(), UiMaterialPlugin::<Minimap>::default()))
            .add_systems(Startup, load_hud)
            .add_systems(Update, (build_hud, (stamina, timer, breath, dial, minimap, jason_hud), hide_for_jason).chain().run_if(resource_exists::<Hud>));
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

/// The minimap's map (see `shaders/minimap.wgsl`).
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct Minimap {
    #[uniform(0)]
    view: Vec4,
    #[uniform(1)]
    dir: Vec4,
    #[texture(2)]
    #[sampler(3)]
    map: Handle<Image>,
}

impl UiMaterial for Minimap {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/minimap.wgsl".into()
    }
}

/// The level's minimap (its SCMiniMapVolume): the map texture (PNG), the area it covers (Unreal cm: min x, min y,
/// max x, max y) and its DefaultMinimapZoom, taken as the share of the map across the minimap (PLACEHOLDER: the
/// native widget's use of it isn't read).
struct MapInfo {
    file: String,
    area: [f32; 4],
    zoom: f32,
}

/// CircleMask's MinimapRadius (the RetainerBox's effect material).
const MINIMAP_RADIUS: f32 = 0.39;

/// The HUD's trees and what it was last built for.
#[derive(Resource)]
pub struct Hud {
    hud: Widget,
    /// Jason's (KillerHUDWidget), shown instead when he's played.
    killer: Option<Widget>,
    breath: Option<Widget>,
    dial: Option<Widget>,
    item: Option<Widget>,
    export: PathBuf,
    classes: ClassDb,
    /// Texture name -> PNG (ui/textures.json).
    textures: HashMap<String, String>,
    regular: Handle<Font>,
    bold: Handle<Font>,
    built: Option<(Vec2, Vec<String>, bool)>,
    root: Option<Entity>,
    /// "<user widget class>.<widget name>" -> entity.
    names: HashMap<String, Entity>,
    map: Option<MapInfo>,
}

/// A progress bar's fill.
#[derive(Component)]
struct Progress {
    fill: Entity,
    kind: String,
    size: Vec2,
}

fn load_hud(mut commands: Commands, export: Res<crate::ExportRoot>, assets: Res<AssetServer>, level: Res<crate::Level>) {
    let dir = &export.0;
    let Some(mut hud) = Widget::load(dir, "CounselorHUDWidget") else {
        warn!("ui: no ui/CounselorHUDWidget.json (run f13-export ui): debug HUD only");
        return;
    };
    let mut breath = Widget::load(dir, "SCBreathWidget");
    let mut dial = Widget::load(dir, "SCInteractMinigame_Widget");
    for (name, vis) in RUNTIME_VISIBILITY {
        for tree in std::iter::once(&mut hud).chain(breath.as_mut()).chain(dial.as_mut()) {
            if let Some(w) = tree.find_mut(name) {
                w.set_visibility(*vis);
            }
        }
    }
    for tree in std::iter::once(&mut hud).chain(breath.as_mut()).chain(dial.as_mut()) {
        action_keys(tree);
    }
    let textures: HashMap<String, String> = std::fs::read_to_string(dir.join("ui").join("textures.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let map = level.layouts.iter().flat_map(|l| &l.actors).find(|a| a.class == "SCMiniMapVolume").and_then(|a| {
        let name = a.props["MapTexture"]["AssetPathName"].as_str()?.rsplit('.').next()?.to_string();
        let b = a.bounds?;
        Some(MapInfo { file: textures.get(&name)?.clone(), area: [b[0], b[1], b[3], b[4]], zoom: a.props["DefaultMinimapZoom"].as_f64().unwrap_or(0.2) as f32 })
    });
    let mut killer = Widget::load(dir, "KillerHUDWidget");
    if let Some(k) = killer.as_mut() {
        // Shown by things not ported: the Morph map, the rage overlay, help texts, XP.
        for name in [
            "Map_Overylay",
            "RageBorder",
            "KillerHelpTextWidget",
            "XPDisplayWidget_81",
            "Reticle",
            "SaveGameThreadActiveIcon_0",
            "HUDLaggingIconWidget",
            "HUDActiveMicWidget",
            "WalkieSpeakingPlayerListWidget",
            "HUDChatNotificationOverlayWidget",
        ] {
            if let Some(w) = k.find_mut(name) {
                w.set_visibility(Vis::Collapsed);
            }
        }
        // Faded in by his HUD's Blueprint as the match starts: shown.
        for name in ["KillerAbilityWidget_521", "KillerKnifeWidget_77", "KillerTrapWidget_184", "RageWidget_190", "ClockBorder", "txtTimer"] {
            if let Some(w) = k.find_mut(name) {
                let c = &mut w.props["ColorAndOpacity"];
                if c["SpecifiedColor"].is_object() {
                    c["SpecifiedColor"]["A"] = json!(1.0);
                } else {
                    c["A"] = json!(1.0);
                }
            }
        }
        action_keys(k);
    }
    commands.insert_resource(Hud {
        hud,
        killer,
        breath,
        dial,
        item: Widget::load(dir, "SC_InventoryItem_Widget"),
        export: dir.clone(),
        classes: ClassDb::load(&dir.join("classes.json")).unwrap_or_default(),
        textures,
        regular: assets.load("ui/fonts/AlteHaasGrotesk_0_Regular.ttf"),
        bold: assets.load("ui/fonts/AlteHaasGrotesk_0_Bold.ttf"),
        built: None,
        root: None,
        names: HashMap::new(),
        map,
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
    mut maps: ResMut<Assets<Minimap>>,
    play: Option<Res<crate::jason_player::PlayJason>>,
) {
    let size = window.size();
    let jason = play.is_some() && hud.killer.is_some();
    if size.x < 1.0 || size.y < 1.0 {
        return;
    }
    let pockets: Vec<String> = player.as_ref().and_then(|p| **p).map_or(vec![], |inv| inv.pockets.iter().filter_map(|&e| items.get(e).ok()).map(|i| i.class.clone()).collect());
    if hud.built.as_ref().is_some_and(|(s, p, j)| *s == size && *p == pockets && *j == jason) {
        return;
    }
    if let Some(root) = hud.root.take() {
        commands.entity(root).despawn();
    }
    // The window in design units: scaled to fit 1920×1080, the rest added along the longer side.
    let scale = (size.x / DESIGN.x).min(size.y / DESIGN.y);
    let virt = size / scale;
    let mut tree = if jason { hud.killer.clone().unwrap_or_else(|| hud.hud.clone()) } else { hud.hud.clone() };
    if !jason {
        fill_inventory(&hud, &mut tree, &pockets);
    }
    let root = commands.spawn((Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, Pickable::IGNORE)).id();
    let mut names = HashMap::new();
    let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, names: &mut names };
    let full = Rect { x: 0.0, y: 0.0, w: virt.x, h: virt.y };
    let laid = tree.layout(full);
    b.spawn(&laid, root, full, if jason { "KillerHUDWidget" } else { "CounselorHUDWidget" });
    if let Some(breath) = &hud.breath {
        let laid = breath.layout(full);
        let e = b.spawn(&laid, root, full, "SCBreathWidget");
        b.commands.entity(e).insert(Visibility::Hidden);
        b.names.insert("SCBreathWidget".into(), e);
    }
    if let Some(dial) = &hud.dial {
        let laid = dial.layout(full);
        let e = b.spawn(&laid, root, full, "SCInteractMinigame_Widget");
        b.commands.entity(e).insert(Visibility::Hidden);
        b.names.insert("SCInteractMinigame_Widget".into(), e);
    }
    // The map behind the minimap's icons and frame, in its RetainerBox.
    if let (Some(map), Some((_, &e))) = (&hud.map, names.iter().find(|(k, _)| k.ends_with(".RetainerBox_0"))) {
        let m = maps.add(Minimap { view: Vec4::new(0.5, 0.5, map.zoom, MINIMAP_RADIUS), dir: Vec4::new(0.0, -1.0, 0.0, 0.0), map: assets.load(map.file.clone()) });
        commands.entity(e).insert(MaterialNode(m));
    }
    hud.names = names;
    hud.root = Some(root);
    hud.built = Some((size, pockets, jason));
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
        // Its RenderTransform: scale (a negative one mirrors: the minimap's view cone) and angle (degrees, clockwise).
        let rt = &p["RenderTransform"];
        if rt.is_object() {
            let s = |k: &str| rt["Scale"][k].as_f64().unwrap_or(1.0) as f32;
            let angle = rt["Angle"].as_f64().unwrap_or(0.0) as f32;
            self.commands.entity(e).insert(UiTransform { scale: Vec2::new(s("X"), s("Y")), rotation: Rot2::degrees(angle), ..default() });
        }
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
        if res["Parent"].as_str() == Some("ProgressCircle") && !textures.is_empty() {
            // The dial's progress circle: its ring filled clockwise from the top.
            let ring: Handle<Image> = self.assets.load(textures[0].to_string());
            let m = self.materials.add(RadialFill { color: LinearRgba::from(linear(c)), fill: Vec4::new(0.0, 0.0, 1.0, 0.0), ring: ring.clone(), gradient: ring });
            self.commands.entity(e).insert(MaterialNode(m));
            return;
        }
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

/// Keeps the minimap on her: her place on the map, turned with the camera, the compass's N kept on north.
fn minimap(
    mut commands: Commands,
    hud: Res<Hud>,
    player: Option<Single<&Transform, With<Player>>>,
    camera: Option<Single<&GlobalTransform, With<crate::player::PlayerCamera>>>,
    nodes: Query<&MaterialNode<Minimap>>,
    mut maps: ResMut<Assets<Minimap>>,
) {
    let (Some(map), Some(player), Some(camera)) = (&hud.map, player, camera) else { return };
    let Some(&e) = hud.names.iter().find(|(k, _)| k.ends_with(".RetainerBox_0")).map(|(_, e)| e) else { return };
    let Ok(node) = nodes.get(e) else { return };
    let [x0, y0, x1, y1] = map.area;
    // Unreal x, y (cm) from Bevy x, z (m).
    let (x, y) = (player.translation.x * 100.0, player.translation.z * 100.0);
    let uv = Vec2::new((x - x0) / (x1 - x0), (y - y0) / (y1 - y0));
    let f = camera.forward();
    let dir = Vec2::new(f.x, f.z).normalize_or(Vec2::NEG_Y);
    if let Some(mut m) = maps.get_mut(&node.0) {
        m.view = Vec4::new(uv.x, uv.y, map.zoom, MINIMAP_RADIUS);
        m.dir = Vec4::new(dir.x, dir.y, 0.0, 0.0);
    }
    // North (the map's up) clockwise from the minimap's up.
    if let Some((_, &compass)) = hud.names.iter().find(|(k, _)| k.ends_with(".ComapssPanel")) {
        commands.entity(compass).insert(UiTransform { rotation: Rot2::radians((-dir.x).atan2(-dir.y)), ..default() });
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

/// The player became Jason (the host's `jason`): her HUD goes.
fn hide_for_jason(hud: Res<Hud>, play: Option<Res<crate::jason_player::PlayJason>>, mut vis: Query<&mut Visibility>) {
    // Without his HUD exported, none.
    if let (Some(root), Some(_), None) = (hud.root, play, &hud.killer) {
        if let Ok(mut v) = vis.get_mut(root) {
            v.set_if_neq(Visibility::Hidden);
        }
    }
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
    let Some(&e) = hud.names.get("SCCounselorTimerWidget.txtTimer").or_else(|| hud.names.get("SCKillerTimerWidget.txtTimer")) else { return };
    let Ok(mut text) = texts.get_mut(e) else { return };
    let left = game.state.remaining_time.max(0);
    let s = format!("{:02}:{:02}", left / 60, left % 60);
    if text.0 != s {
        text.0 = s;
    }
}

/// Jason's HUD: his knives and traps left, each ability's unlock bar filling and its icon's glow while it's ready
/// (its charge bar as it recharges), and his rage building.
#[allow(clippy::type_complexity)]
fn jason_hud(
    mut commands: Commands,
    hud: Res<Hud>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<(&crate::jason::Jason, Option<&crate::senses::Knowledge>)>,
    mut texts: Query<&mut Text>,
    bars: Query<&Progress>,
    mut vis: Query<&mut Visibility>,
) {
    use f13_core::killer_abilities::{Ability, Blockers};
    if play.is_none() || hud.built.as_ref().is_none_or(|b| !b.2) {
        return;
    }
    let Some((j, know)) = jasons.iter().next() else { return };
    let mut set_text = |name: &str, s: String| {
        if let Some(mut t) = hud.names.get(name).and_then(|&e| texts.get_mut(e).ok()) {
            if t.0 != s {
                t.0 = s;
            }
        }
    };
    set_text("KillerKnifeWidget.TextBlock_1", format!("{:02}", j.knives));
    set_text("KillerTrapWidget.TextBlock_1", format!("{:02}", j.traps));
    let Some(k) = know else { return };
    let a = &k.abilities;
    let mut bar = |name: &str, percent: f32| {
        if let Some(p) = hud.names.get(name).and_then(|&e| bars.get(e).ok()) {
            set_progress(&mut commands, p.fill, p, percent);
        }
    };
    for ability in Ability::ALL {
        let n = format!("{ability:?}");
        bar(&format!("KillerAbilityWidget.{n}UnlockBar"), a.unlock_fraction(ability));
        bar(&format!("KillerAbilityWidget.{n}Bar"), if a.is_unlocked(ability) { a.recharge_fraction(ability) } else { 0.0 });
        let ready = a.is_available(ability, &Blockers::default());
        if let Some(mut v) = hud.names.get(&format!("KillerAbilityWidget.Icon{n}_Glow")).and_then(|&e| vis.get_mut(e).ok()) {
            v.set_if_neq(if ready { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
    let rage = if a.rage_active { 1.0 } else { (a.rage_timer / a.tuning.rage_unlock_time.max(1.0)).clamp(0.0, 1.0) };
    bar("RageWidget.ProgressBar_41", rage);
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

/// The dial's pip arcs (on its Root canvas), for the pips they were made for.
#[derive(Component)]
struct DialPips {
    pips: Vec<(f32, f32)>,
    arcs: Vec<Entity>,
}

/// The interaction dial while a hold interaction runs (repairs; doors and windows have no pips): progress, the
/// needle, the pips by state, the next pip's mouse button.
fn dial(
    mut commands: Commands,
    hud: Res<Hud>,
    interaction: Option<Res<crate::interact::Interaction>>,
    tuning: Res<crate::player::PlayerTuning>,
    mut vis: Query<&mut Visibility>,
    mut transforms: Query<&mut UiTransform>,
    nodes: Query<&MaterialNode<RadialFill>>,
    pips: Query<&DialPips>,
    mut materials: ResMut<Assets<RadialFill>>,
    assets: Res<AssetServer>,
) {
    use f13_core::minigame::PipState;
    let Some(&root) = hud.names.get("SCInteractMinigame_Widget") else { return };
    let hold = interaction.as_ref().and_then(|i| i.hold.as_ref()).filter(|h| h.started);
    if let Ok(mut v) = vis.get_mut(root) {
        v.set_if_neq(if hold.is_some() { Visibility::Inherited } else { Visibility::Hidden });
    }
    let Some(h) = hold else { return };
    let progress = h.progress(&tuning.minigame).clamp(0.0, 1.0);
    let name = |n: &str| hud.names.get(&format!("SCInteractMinigame_Widget.{n}")).copied();
    // The progress circle.
    if let Some(m) = name("Progress_Image").and_then(|e| nodes.get(e).ok()) {
        if materials.get(&m.0).is_some_and(|m| (m.fill.x - progress).abs() > 1e-3) {
            if let Some(mut m) = materials.get_mut(&m.0) {
                m.fill.x = progress;
            }
        }
    }
    // The needle: the tick at 12 o'clock turned with the progress.
    if let Some(e) = name("ProgressTick") {
        let rotation = Rot2::degrees(progress * 360.0);
        match transforms.get_mut(e) {
            Ok(mut t) => t.rotation = rotation,
            Err(_) => {
                commands.entity(e).insert(UiTransform { rotation, ..default() });
            }
        }
    }
    // The pips: arcs of the ring, made again when the layout changes.
    let Some(canvas) = name("Root") else { return };
    let layout: Vec<(f32, f32)> = h.pips.iter().map(|p| (p.position / 360.0, (p.position + p.width) / 360.0)).collect();
    let current = pips.get(canvas).ok().filter(|d| d.pips == layout).map(|d| d.arcs.clone());
    let arcs = match current {
        Some(arcs) => arcs,
        None => {
            if let Ok(old) = pips.get(canvas) {
                for &a in &old.arcs {
                    commands.entity(a).despawn();
                }
            }
            let ring = hud.dial.as_ref().and_then(|d| d.find("Background")).and_then(|b| b.props["Brush"]["ResourceObject"]["File"].as_str()).map(String::from);
            let Some(ring) = ring else { return };
            let ring: Handle<Image> = assets.load(ring);
            let arcs: Vec<Entity> = layout
                .iter()
                .map(|&(start, end)| {
                    let m = materials.add(RadialFill { color: LinearRgba::from(linear(PIP_WAITING)), fill: Vec4::new(end, start, 1.0, 0.0), ring: ring.clone(), gradient: ring.clone() });
                    commands
                        .spawn((
                            Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() },
                            MaterialNode(m),
                            Pickable::IGNORE,
                            ChildOf(canvas),
                        ))
                        .id()
                })
                .collect();
            commands.entity(canvas).insert(DialPips { pips: layout, arcs: arcs.clone() });
            return;
        }
    };
    for (p, &a) in h.pips.iter().zip(&arcs) {
        let c = match p.state {
            PipState::Hit => PIP_HIT,
            PipState::Missed => PIP_MISSED,
            _ => PIP_WAITING,
        };
        let c = LinearRgba::from(linear(c));
        if let Some(m) = nodes.get(a).ok().map(|n| n.0.clone()) {
            if materials.get(&m).is_some_and(|m| m.color != c) {
                if let Some(mut m) = materials.get_mut(&m) {
                    m.color = c;
                }
            }
        }
    }
    // The next pip's button in the middle.
    let next = h.pips.iter().find(|p| matches!(p.state, PipState::Waiting | PipState::Active)).map(|p| p.key);
    for (n, key) in [("Interact1", 0u8), ("Interact2", 1u8)] {
        if let Some(mut v) = name(n).and_then(|e| vis.get_mut(e).ok()) {
            v.set_if_neq(if next == Some(key) { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
}

