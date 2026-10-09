//! The counselor's HUD from the game's own UMG widgets (`ui/*.json`, `f13-export ui`): CounselorHUDWidget's tree
//! (the timer, the inventory, the stamina ring...) and SCBreathWidget while she hides, laid out as Slate does
//! (`f13_data::umg`) on the window scaled from the 1920×1080 design, in the game's textures and font.
//!
//! What the game's code does at runtime is redone here by name: the visibilities it sets before anything shows
//! (`RUNTIME_VISIBILITY`), the timer's text, the stamina ring's fill (StaminaBarMaterial, `shaders/radial_fill.wgsl`),
//! the inventory's item widgets (SC_InventoryItem_Widget, the items' ImageOn icons), the breath bars and the
//! interaction dial (SCInteractMinigame_Widget: the progress circle, the needle, the pips as arcs of the ring, the
//! mouse button of the next pip), the police timer while they're on their way (ShowPoliceTimer, PoliceTimerText), the
//! minimap's noise rings (SC_Noise_Widget), the XP lines (XPDisplayWidget, XPLineWidget). The HUD is rebuilt when the
//! window's size or her pockets change, the police timer shows or hides, or she scores.
//!
//! The help texts (CounselorHelpTextWidget, KillerHelpTextWidget) never show: their Construct jumps to its end and
//! nothing else calls ShowHelpText (no native caller), so Border_0's content stays at its alpha 0.
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
    ("MatchStartText", Vis::Collapsed),
    ("Image_106", Vis::Collapsed),
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

/// The keys ActionMapWidgets show for their PrimaryAction (the game picks a key glyph or name from the input
/// settings: DefaultInput.ini's keyboard mappings here); the others (the gamepad's item cycling and powers, the
/// mouse wheel's item cycling) are hidden.
const ACTION_KEYS: &[(&str, &str)] = &[
    ("CSLR_SmallItem_1", "1"),
    ("CSLR_SmallItem_2", "2"),
    ("CSLR_SmallItem_3", "3"),
    ("CSLR_HoldBreath", "Space"),
    ("CSLR_UseAbility", "Q"),
    // The dial's pip keys (pip key 0 / 1).
    ("MG_InteractSkill1", "RMB"),
    ("MG_InteractSkill2", "LMB"),
    ("GLB_Wiggle", "E"),
    ("GLB_Map", "M"),
    // Jason's.
    ("KLR_Grab", "RMB"),
    ("KLR_Morph_PC", "1"),
    ("KLR_Sense_PC", "2"),
    ("KLR_Shift_PC", "3"),
    ("KLR_Stalk_PC", "4"),
    ("CMBT_KLR_GrabKillRight", "1"),
    ("CMBT_KLR_GrabKillBottom", "2"),
    ("CMBT_KLR_GrabKillLeft", "3"),
    ("CMBT_KLR_GrabKillTop", "4"),
    ("KLR_Morph_Activate", "LMB"),
    ("KLR_Morph_Cancel", "1"),
    ("KLR_KnifeThrow", "MMB"),
    ("KLR_KnifeCancel", "RMB"),
    ("KLR_PlaceTrap", "R"),
];

/// Shows its action's key on an ActionMapWidget (its TextWidget; no glyph image), or hides the widget.
fn action_keys(w: &mut Widget) {
    if matches!(w.class.as_deref(), Some("ActionMapWidget" | "MinigameActionMapWidget")) {
        let action = w.props["PrimaryAction"].as_str().unwrap_or_default().to_string();
        match ACTION_KEYS.iter().find(|(a, _)| *a == action) {
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
            .init_resource::<PlayingAnims>()
            .init_resource::<MapScreenUi>()
            .add_systems(Startup, load_hud)
            .add_systems(Update, (build_hud, (stamina, timer, breath, dial, minimap, map_icons, killer_map_icons, (map_screen, counselor_map_icons).chain(), jason_hud, noise_meter, xp_lines), widget_anims, fear_widgets, content_alpha, widget_colors, hide_for_jason).chain().run_if(resource_exists::<Hud>));
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
/// max x, max y) and the share of the map across the minimap. The minimap's InitializeMinimap (0x14029c050) takes
/// the volume's DefaultMinimapZoom as its MinimapZoom and, with bAutoAdjustSize (ILLMinimap's constructor turns it
/// on), sizes the map to MinimapZoom x 0.1 x the volume's size (cm) in widget units, kept centred on her: across the
/// minimap widget (CounselorMinimapWidget's slot, [`MINIMAP_SIZE`]) that's this share of the map.
struct MapInfo {
    file: String,
    area: [f32; 4],
    zoom: f32,
    /// Jason's map (the volume's KillerMapTexture).
    killer_file: Option<String>,
}

/// SC_MiniMap_Widget's size in CounselorMinimapWidget (its slot's 328 x 328).
const MINIMAP_SIZE: f32 = 328.0;

/// CircleMask's MinimapRadius (the RetainerBox's effect material).
const MINIMAP_RADIUS: f32 = 0.39;

/// The minimap's RetainerBox (Jason's HUD has none: his trap counter has a RetainerBox_0 of its own).
const MINIMAP_RETAINER: &str = "SC_MiniMap_Widget.RetainerBox_0";

/// The HUD's trees and what it was last built for.
#[derive(Resource)]
pub struct Hud {
    hud: Widget,
    /// Jason's (KillerHUDWidget), shown instead when he's played.
    killer: Option<Widget>,
    breath: Option<Widget>,
    dial: Option<Widget>,
    item: Option<Widget>,
    /// Her objectives and map screen (ObjectivesWidget) and the objective rows its FindObjectives adds.
    objectives: Option<Widget>,
    car_objective: Option<Widget>,
    police_objective: Option<Widget>,
    export: PathBuf,
    classes: ClassDb,
    /// Texture name -> PNG (ui/textures.json).
    textures: HashMap<String, String>,
    regular: Handle<Font>,
    bold: Handle<Font>,
    /// Built for: the window size, her pockets, Jason's HUD, the police timer and the game timer shown, the score
    /// lines shown.
    built: Option<(Vec2, Vec<String>, bool, (bool, bool), usize)>,
    root: Option<Entity>,
    /// "<user widget class>.<widget name>" -> entity.
    names: HashMap<String, Entity>,
    map: Option<MapInfo>,
    /// Design units to pixels, as last built.
    scale: f32,
    noise: Option<NoiseWidget>,
    xp: Option<XpLines>,
    /// The widget animations of the user widgets whose code plays them, by class.
    anims: HashMap<String, HashMap<String, WidgetAnim>>,
    /// SCCounselorTimerWidget's TimeToShow (its class's default; SCTimerWidget's constructor leaves it 0).
    time_to_show: i32,
    /// KillerAbilityWidget's IconUsableColor, IconUnusableColor and IconLockedColor (its defaults).
    ability_icon_colors: [[f32; 4]; 3],
}

/// XPDisplayWidget's lines: an XPLineWidget per score shown (AddNewXPLine), MaxAmountOfSlots of them (Construct adds
/// that many empty ones; each new line removes the first), each faded in by its FadeIn, out by its FadeOut 5 s later.
struct XpLines {
    line: Widget,
    slots: usize,
    /// FadeIn and FadeOut: Border_0's content alpha, and their lengths.
    fade_in: (f13_core::curve::RichCurve, f32),
    fade_out: (f13_core::curve::RichCurve, f32),
    /// The textures its SetIconImage sets, by name.
    textures: HashMap<String, String>,
}

/// XPLineWidget's Delay between its FadeIn and its FadeOut, s.
const XP_LINE_HOLD_S: f32 = 5.0;

/// XPLineWidget's SetIconImage: the icon of each EScoreEventCategory (none: the template's stays).
const XP_ICONS: &[(&str, &str)] = &[
    ("Offense", "Img_Hud_Counselor_XP_Icon_Offense_01"),
    ("Defense", "Img_Hud_Counselor_XP_Icon_Defense_01"),
    ("Fixer", "Img_Hud_Counselor_XP_Icon_Fixer_01"),
    ("Escaped", "Img_Hud_Counselor_XP_Icon_Escaped_01"),
    ("TeamEscaped", "Img_Hud_Counselor_XP_Icon_TeamEscaped_01"),
    ("Reinforcements", "Img_Hud_Counselor_XP_Icon_Reinforcements_01"),
    ("CompletedMatch", "Img_Hud_Counselor_XP_Icon_MatchComplete_01"),
    ("TimeBonus", "Img_Hud_Counselor_XP_Icon_TimeBonus_01"),
    ("Demasking", "Img_Hud_Counselor_XP_Icon_Demasking_01"),
    ("Kill", "Img_Hud_Jason_XP_Icon_CounselorKill_01"),
    ("NoSurvivors", "Img_Hud_Jason_XP_Icon_TeamKilled"),
    ("Resourceful", "Img_Hud_Jason_XP_Icon_EnvironmentContextKill_01"),
    ("Versatile", "Img_Hud_Jason_XP_Icon_GrabKill_01"),
    ("Seeker", "Img_Hud_Jason_XP_Icon_Seeker_01"),
    ("DemolitionMan", "Img_Hud_Counselor_XP_IconTemp_01"),
    ("BoatKill", "Img_Hud_Jason_XP_Icon_BoatKill_01"),
    ("Butcher", "Img_Hud_Jason_XP_Icon_WeaponSwingKill_01"),
    ("Trapper", "Img_HUD_Jason_XP_Icon_Counselor_Trapped"),
    ("PartyCrasher", "Img_HUD_Jason_XP_Icon_Shutdown_Electricity"),
    ("DefeatedJason", "Img_Hud_Counselor_XP_Icon_JasonKilled_01"),
];

impl XpLines {
    fn load(dir: &std::path::Path) -> Option<Self> {
        let line = Widget::load(dir, "XPLineWidget")?;
        let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("ui/XPLineWidget.json")).ok()?).ok()?;
        let display: Value = std::fs::read_to_string(dir.join("ui/XPDisplayWidget.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        // An animation's Border_0 content alpha channel.
        let anim = |name: &str| {
            let a = &v["Animations"][name];
            let track = a["Tracks"].as_array()?.iter().find(|t| t["Widget"] == "Border_0" && t["Property"] == "ContentColorAndOpacity")?;
            Some((f13_data::character::ue_rich_curve(&track["Sections"][0]["Channels"]["AlphaCurve"]), a["Length"].as_f64()? as f32))
        };
        Some(XpLines {
            line,
            slots: display["Defaults"]["MaxAmountOfSlots"].as_u64().unwrap_or(0) as usize,
            fade_in: anim("FadeIn")?,
            fade_out: anim("FadeOut")?,
            textures: v["Textures"].as_object().map(|t| t.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default(),
        })
    }

    /// A line's Border_0 content alpha `t` s after AddNewXPLine: FadeIn, then FadeOut after the delay (both keep their
    /// last state).
    fn alpha(&self, t: f32) -> f32 {
        if t < XP_LINE_HOLD_S {
            self.fade_in.0.eval(t.min(self.fade_in.1))
        } else {
            self.fade_out.0.eval((t - XP_LINE_HOLD_S).min(self.fade_out.1))
        }
    }
}

/// SC_Noise_Widget's defaults: its rings' start sizes, the pulse's time and curve.
struct NoiseWidget {
    ring1_start: Vec2,
    ring2_start: Vec2,
    ping_time: f32,
    curve: f13_core::curve::RichCurve,
}

impl NoiseWidget {
    fn load(dir: &std::path::Path) -> Option<Self> {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("ui/SC_Noise_Widget.json")).ok()?).ok()?;
        let d = v.get("Defaults")?;
        let v2 = |n: &str| Vec2::new(d[n]["X"].as_f64().unwrap_or(0.0) as f32, d[n]["Y"].as_f64().unwrap_or(0.0) as f32);
        Some(NoiseWidget {
            ring1_start: v2("Ring1StartSize"),
            ring2_start: v2("Ring2StartSize"),
            ping_time: d["PingTime"].as_f64().unwrap_or(0.0) as f32,
            curve: f13_data::character::ue_rich_curve(&d["PingCurve"]["FloatCurve"]),
        })
    }
}

/// A progress bar's fill: a box clipping its full-size image to the filled part (SProgressBar's PushClip).
#[derive(Component)]
struct Progress {
    fill: Entity,
    image: Entity,
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
        let zoom = a.props["DefaultMinimapZoom"].as_f64().unwrap_or(0.2) as f32;
        let killer = a.props["KillerMapTexture"]["AssetPathName"].as_str().and_then(|p| p.rsplit('.').next()).and_then(|n| textures.get(n).cloned());
        Some(MapInfo { file: textures.get(&name)?.clone(), area: [b[0], b[1], b[3], b[4]], zoom: MINIMAP_SIZE / (zoom * 0.1 * (b[3] - b[0])).max(1.0), killer_file: killer })
    });
    let mut killer = Widget::load(dir, "KillerHUDWidget");
    if let Some(k) = killer.as_mut() {
        // Shown by things not ported: the rage overlay; the help text never shows (see the module docs).
        for name in [
            "RageBorder",
            "KillerHelpTextWidget",
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
        objectives: Widget::load(dir, "ObjectivesWidget"),
        car_objective: Widget::load(dir, "Objective_Car_Widget"),
        police_objective: Widget::load(dir, "Objective_Police_Widget"),
        export: dir.clone(),
        classes: ClassDb::load(&dir.join("classes.json")).unwrap_or_default(),
        textures,
        regular: assets.load("ui/fonts/AlteHaasGrotesk_0_Regular.ttf"),
        bold: assets.load("ui/fonts/AlteHaasGrotesk_0_Bold.ttf"),
        built: None,
        root: None,
        names: HashMap::new(),
        map,
        scale: 1.0,
        anims: ["SCCounselorTimerWidget", "SCBreathWidget", "KillerHUDWidget", "ObjectivesWidget"].iter().map(|c| (c.to_string(), load_anims(dir, c))).collect(),
        ability_icon_colors: {
            let d = std::fs::read_to_string(dir.join("ui/KillerAbilityWidget.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_default();
            ["IconUsableColor", "IconUnusableColor", "IconLockedColor"].map(|k| color(&d["Defaults"][k], [1.0; 4]))
        },
        time_to_show: std::fs::read_to_string(dir.join("ui/SCCounselorTimerWidget.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["Defaults"]["TimeToShow"].as_i64())
            .unwrap_or(0) as i32,
        xp: XpLines::load(dir).or_else(|| {
            warn!("ui: no XPLineWidget animations (re-export with f13-export ui): no XP lines");
            None
        }),
        noise: NoiseWidget::load(dir).or_else(|| {
            warn!("ui: SC_Noise_Widget has no defaults (re-export with f13-export ui): no noise meter");
            None
        }),
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
    game: Option<Res<crate::match_flow::GameMatch>>,
) {
    let size = window.size();
    let jason = play.is_some() && hud.killer.is_some();
    // ShowPoliceTimer while they're on their way (HidePoliceTimer once they're here).
    let police = game.as_ref().is_some_and(|g| g.state.police_time_remaining > 0);
    // ShowGameTimer while the time left is within TimeToShow (see `timer`).
    let game_timer = game.as_ref().is_some_and(|g| (0..=hud.time_to_show).contains(&g.state.remaining_time));
    if size.x < 1.0 || size.y < 1.0 {
        return;
    }
    let pockets: Vec<String> = player.as_ref().and_then(|p| **p).map_or(vec![], |inv| inv.pockets.iter().filter_map(|&e| items.get(e).ok()).map(|i| i.class.clone()).collect());
    let selected = player.as_ref().and_then(|p| **p).map_or(0, |inv| inv.selected);
    // Rebuilt when they or the selection change.
    let pocket_key: Vec<String> = pockets.iter().cloned().chain([format!("#{selected}")]).collect();
    // His score lines on his HUD, hers on hers.
    let score_lines = |g: &crate::match_flow::GameMatch| if jason { g.killer_score_lines().to_vec() } else { g.score_lines().to_vec() };
    let scores = game.as_ref().map_or(vec![], |g| score_lines(g));
    let lines = scores.len();
    if hud.built.as_ref().is_some_and(|(s, p, j, c, l)| *s == size && *p == pocket_key && *j == jason && *c == (police, game_timer) && *l == lines) {
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
        fill_inventory(&hud, &mut tree, &pockets, selected);
    }
    fill_xp_lines(&hud, &mut tree, &scores);
    if police {
        if let Some(w) = tree.find_mut("PoliceTimer") {
            w.set_visibility(Vis::Visible);
        }
    }
    if game_timer {
        if let Some(w) = tree.find_mut("GameTimer") {
            w.set_visibility(Vis::Visible);
        }
    }
    let root = commands.spawn((Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, Pickable::IGNORE)).id();
    let mut names = HashMap::new();
    let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, content: [1.0; 4], names: &mut names };
    let full = Rect { x: 0.0, y: 0.0, w: virt.x, h: virt.y };
    let laid = tree.layout(full);
    b.spawn(&laid, root, full, if jason { "KillerHUDWidget" } else { "CounselorHUDWidget" });
    // Her breath meter and interaction dial are hers (Jason's HUD has neither).
    if let Some(breath) = hud.breath.as_ref().filter(|_| !jason) {
        let laid = breath.layout(full);
        let e = b.spawn(&laid, root, full, "SCBreathWidget");
        b.commands.entity(e).insert(Visibility::Hidden);
        b.names.insert("SCBreathWidget".into(), e);
    }
    if let Some(dial) = hud.dial.as_ref().filter(|_| !jason) {
        let laid = dial.layout(full);
        let e = b.spawn(&laid, root, full, "SCInteractMinigame_Widget");
        b.commands.entity(e).insert(Visibility::Hidden);
        b.names.insert("SCInteractMinigame_Widget".into(), e);
    }
    // The map behind the minimap's icons and frame, in its RetainerBox.
    if let (Some(map), Some((_, &e))) = (&hud.map, names.iter().find(|(k, _)| k.as_str() == MINIMAP_RETAINER)) {
        let m = maps.add(Minimap { view: Vec4::new(0.5, 0.5, map.zoom, MINIMAP_RADIUS), dir: Vec4::new(0.0, -1.0, 0.0, 0.0), map: assets.load(map.file.clone()) });
        commands.entity(e).insert(MaterialNode(m));
    }
    hud.names = names;
    hud.root = Some(root);
    hud.scale = scale;
    hud.built = Some((size, pocket_key, jason, (police, game_timer), lines));
}

/// Puts an item widget in the inventory's Grid for each pocket item (SCInventory_Widget does it as items are
/// picked up): its ImageOn icon, the count hidden (one item each); ObjectText names the first.
/// SCInventory_Widget's InventoryChanged: an SC_InventoryItem_Widget per small item, Active for the selected one
/// (SmallItemIndex: ImageOn shown, else ImageOff), and ObjectText the selected item's name. Not here: the uses left
/// (NumUses - TimesUsed, shown above 1).
fn fill_inventory(hud: &Hud, tree: &mut Widget, pockets: &[String], selected: usize) {
    if let Some(text) = tree.find_mut("ObjectText") {
        match pockets.get(selected) {
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
        let active = i == selected;
        for (widget, prop, shown) in [("Image_On", "ImageOn", active), ("Image_Off", "ImageOff", !active)] {
            let icon = class_texture(hud, class, prop).or_else(|| class_texture(hud, class, "HUDPickupIcon"));
            if let Some(image) = w.find_mut(widget) {
                match icon.filter(|_| shown) {
                    Some(file) => {
                        let size = png_size(&hud.export.join(&file)).unwrap_or(Vec2::splat(64.0));
                        image.props["Brush"] = json!({ "ImageSize": { "X": size.x, "Y": size.y }, "ResourceObject": { "File": file } });
                        image.set_visibility(Vis::Visible);
                    }
                    None => image.set_visibility(Vis::Hidden),
                }
            }
        }
        if let Some(h) = w.find_mut("TextBlock_0") {
            h.set_visibility(Vis::Collapsed);
        }
        grid.children.push(w);
    }
}

/// XPDisplayWidget's box: its last MaxAmountOfSlots lines (empty ones first, as its Construct adds them), each with
/// its text and its category's icon.
fn fill_xp_lines(hud: &Hud, tree: &mut Widget, scores: &[(String, String, f32)]) {
    let (Some(xp), Some(stack)) = (&hud.xp, tree.find_mut("MainXPDisplayBox")) else { return };
    stack.children.clear();
    let shown = &scores[scores.len().saturating_sub(xp.slots)..];
    for i in 0..xp.slots {
        let mut w = xp.line.clone();
        w.name = format!("XPLine{i}");
        // Its widgets named by line (XPLine<i>.Border_0) for the fades.
        w.class = Some(format!("XPLine{i}"));
        w.slot_kind = "VerticalBoxSlot".into();
        w.slot = json!({});
        let score = (i + shown.len()).checked_sub(xp.slots).and_then(|k| shown.get(k));
        if let Some(text) = w.find_mut("XPDescriptionAndAmouint") {
            text.props["Text"] = json!({ "LocalizedString": score.map_or("", |s| s.0.as_str()) });
        }
        let icon = score.and_then(|s| XP_ICONS.iter().find(|(c, _)| *c == s.1)).and_then(|(_, t)| xp.textures.get(*t));
        if let (Some(file), Some(image)) = (icon, w.find_mut("XPIcon")) {
            image.props["Brush"]["ResourceObject"] = json!({ "File": file });
        }
        stack.children.push(w);
    }
}

/// A Border's ContentColorAndOpacity alpha: the opacity of everything in it (widget animations play it).
#[derive(Component, PartialEq)]
struct ContentAlpha(f32);

/// A widget's ColorAndOpacity as widget animations play it: an Image's own colour (over [`ColorBase`]: its brush's
/// tint and the borders' around it), or a user widget's, which multiplies everything in it (over each one's
/// [`TintBase`], its colour before).
#[derive(Component, PartialEq, Clone, Copy)]
struct WidgetColor([f32; 4]);

#[derive(Component)]
struct ColorBase([f32; 4]);

#[derive(Component)]
struct TintBase([f32; 4]);

/// Applies each changed [`WidgetColor`].
fn widget_colors(
    mut commands: Commands,
    colors: Query<(Entity, Ref<WidgetColor>, Option<&ColorBase>)>,
    children: Query<&Children>,
    mut nodes: Query<(Option<&mut ImageNode>, Option<&mut TextColor>, Option<&TintBase>)>,
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
                _ => {}
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
struct BaseAlpha(f32);

/// Each Border's content opacity on what's in it: all of it when it changes, and anything new in it.
fn content_alpha(
    mut commands: Commands,
    borders: Query<(Entity, Ref<ContentAlpha>)>,
    children: Query<&Children>,
    mut colors: Query<(Option<&mut ImageNode>, Option<&mut TextColor>, Option<&BaseAlpha>)>,
) {
    for (border, alpha) in &borders {
        let changed = alpha.is_changed();
        for e in children.iter_descendants(border) {
            let Ok((image, text, base)) = colors.get_mut(e) else { continue };
            if !changed && base.is_some() {
                continue;
            }
            let Some(current) = image.as_ref().map(|i| i.color.alpha()).or(text.as_ref().map(|t| t.0.alpha())) else { continue };
            let base = match base {
                Some(b) => b.0,
                None => {
                    commands.entity(e).insert(BaseAlpha(current));
                    current
                }
            };
            if let Some(mut i) = image {
                i.color.set_alpha(base * alpha.0);
            }
            if let Some(mut t) = text {
                t.0.set_alpha(base * alpha.0);
            }
        }
    }
}

/// A widget animation (UWidgetAnimation): its length, each Border's content alpha curve and each widget's
/// ColorAndOpacity curves (red, green, blue, alpha), by widget name.
struct WidgetAnim {
    length: f32,
    tracks: Vec<(String, f13_core::curve::RichCurve)>,
    colors: Vec<(String, [f13_core::curve::RichCurve; 4])>,
}

/// Loads a widget class's animations' ContentColorAndOpacity alpha tracks and ColorAndOpacity tracks
/// (ui/<class>.json "Animations").
fn load_anims(dir: &std::path::Path, class: &str) -> HashMap<String, WidgetAnim> {
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
            (name.clone(), WidgetAnim { length: a["Length"].as_f64().unwrap_or(0.0) as f32, tracks, colors })
        })
        .collect()
}

/// The widget animations playing (UUserWidget::PlayAnimation): on which user widget (its widgets' "<owner>.<widget>"
/// names), which, since when, reversed. Each keeps its last state when over (KeepState), so it stays listed.
#[derive(Resource, Default)]
pub struct PlayingAnims(Vec<(String, String, f32, bool)>);

impl PlayingAnims {
    fn play(&mut self, owner: &str, anim: &str, now: f32, reverse: bool) {
        self.0.retain(|(o, a, _, _)| !(o == owner && a == anim));
        self.0.push((owner.to_string(), anim.to_string(), now, reverse));
    }
}

/// Plays the widget animations: each track's Border content alpha at the animation's time (backwards when reversed).
fn widget_anims(time: Res<Time>, hud: Res<Hud>, playing: Res<PlayingAnims>, mut borders: Query<&mut ContentAlpha>, mut colors: Query<&mut WidgetColor>) {
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
        for (widget, curves) in &anim.colors {
            if let Some(mut c) = hud.names.get(&format!("{owner}.{widget}")).and_then(|&e| colors.get_mut(e).ok()) {
                c.set_if_neq(WidgetColor(curves.each_ref().map(|k| k.eval(x))));
            }
        }
    }
}

/// CounselorHUDWidget's UpdateFearOpacity (its tick, once the match is in progress and MatchStart has played: 5 s):
/// CounselorFearWidgets (the inventory, the stamina ring; designed at alpha 0, kept at 0 through MatchStart) takes
/// SCHUDWidget's FearOpacity. PLACEHOLDER: what writes FearOpacity natively wasn't found (MODLOG "Played Jason fixes"):
/// full opacity stands in. (The minimap takes it too unless NervesOfSteel; left as built.)
fn fear_widgets(hud: Res<Hud>, game: Option<Res<crate::match_flow::GameMatch>>, mut colors: Query<&mut WidgetColor>) {
    let Some(&e) = hud.names.get("CounselorHUDWidget.CounselorFearWidgets_33") else { return };
    let Ok(mut c) = colors.get_mut(e) else { return };
    let started = game.is_some_and(|g| g.state.state == f13_core::game_mode::MatchState::InProgress && g.state.elapsed_in_progress_time >= MATCH_START_S);
    c.set_if_neq(WidgetColor([1.0, 1.0, 1.0, if started { 1.0 } else { 0.0 }]));
}

/// CounselorHUDWidget's MatchStart animation's length (s).
const MATCH_START_S: i32 = 5;

/// XPDisplayWidget's lines fading: each line's Border_0 content by its FadeIn / FadeOut (an empty line stays at its
/// design alpha, 0).
fn xp_lines(
    time: Res<Time>,
    hud: Res<Hud>,
    game: Option<Res<crate::match_flow::GameMatch>>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    mut borders: Query<&mut ContentAlpha>,
) {
    let (Some(xp), Some(game)) = (&hud.xp, game) else { return };
    let scores = if play.is_some() && hud.killer.is_some() { game.killer_score_lines() } else { game.score_lines() };
    let shown = &scores[scores.len().saturating_sub(xp.slots)..];
    let now = time.elapsed_secs();
    for i in 0..xp.slots {
        let Some(mut a) = hud.names.get(&format!("XPLine{i}.Border_0")).and_then(|&e| borders.get_mut(e).ok()) else { continue };
        let Some(score) = (i + shown.len()).checked_sub(xp.slots).and_then(|k| shown.get(k)) else { continue };
        a.set_if_neq(ContentAlpha(xp.alpha(now - score.2)));
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
    /// Inside borders: their ContentColorAndOpacity's colour, multiplied (it tints everything in them).
    content: [f32; 4],
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
            "Image" => {
                // Its ColorAndOpacity can be played (WidgetColor), over its brush's tint and the borders' around it.
                let cao = color(&p["ColorAndOpacity"], [1.0; 4]);
                self.commands.entity(e).insert((ColorBase(mul(color(&p["Brush"]["TintColor"], [1.0; 4]), self.content)), WidgetColor(cao)));
                self.brush(e, &p["Brush"], mul(cao, self.content));
            }
            "Border" => self.brush(e, &p["Background"], mul(color(&p["BrushColor"], [1.0; 4]), self.content)),
            "TextBlock" => {
                let font = if p["Font"]["TypefaceFontName"].as_str() == Some("Bold") { self.hud.bold.clone() } else { self.hud.regular.clone() };
                let justify = match p["Justification"].as_str().unwrap_or("") {
                    "ETextJustify::Center" => Justify::Center,
                    "ETextJustify::Right" => Justify::Right,
                    _ => Justify::Left,
                };
                let mut t = self.commands.entity(e);
                // Its ColorAndOpacity (the text's colour) can be played too (WidgetColor), under the borders' tint.
                t.insert((ColorBase(self.content), WidgetColor(color(&p["ColorAndOpacity"], [1.0; 4]))));
                t.insert((
                    Text::new(w.text()),
                    TextFont { font: font.into(), font_size: FontSize::Px(w.font_px() * self.scale * self.text_scale), ..default() },
                    TextColor(linear(mul(color(&p["ColorAndOpacity"], [1.0; 4]), self.content))),
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
                let fill = self.commands.spawn((Node { position_type: PositionType::Absolute, overflow: Overflow::clip(), ..default() }, Pickable::IGNORE, ChildOf(e))).id();
                let size = Vec2::new(r.w, r.h) * self.scale;
                let image = self
                    .commands
                    .spawn((Node { position_type: PositionType::Absolute, width: Val::Px(size.x), height: Val::Px(size.y), ..default() }, Pickable::IGNORE, ChildOf(fill)))
                    .id();
                self.brush(image, &s["FillImage"], color(&p["FillColorAndOpacity"], [1.0; 4]));
                let kind = p["BarFillType"].as_str().unwrap_or("LeftToRight").rsplit(':').next().unwrap_or("").to_string();
                let progress = Progress { fill, image, kind, size };
                set_progress(self.commands, fill, &progress, p["Percent"].as_f64().unwrap_or(0.0) as f32);
                self.commands.entity(e).insert(progress);
            }
            _ => {}
        }
        // A user widget's ColorAndOpacity tints what's in it.
        if w.class.is_some() && p["ColorAndOpacity"].is_object() {
            self.commands.entity(e).insert(WidgetColor(color(&p["ColorAndOpacity"], [1.0; 4])));
        }
        let owner = w.class.as_deref().unwrap_or(owner).to_string();
        // A scale box fits its content (ScaleToFit).
        let outer = self.text_scale;
        let outer_content = self.content;
        if w.kind == "Border" && p["ContentColorAndOpacity"].is_object() {
            // Its content's colour tints what's in it; its opacity is played by widget animations (ContentAlpha).
            let c = color(&p["ContentColorAndOpacity"], [1.0; 4]);
            self.content = mul(self.content, [c[0], c[1], c[2], 1.0]);
            self.commands.entity(e).insert(ContentAlpha(c[3]));
        }
        if w.kind == "ScaleBox" {
            if let Some(c) = w.children.first() {
                let (dw, dh) = c.desired();
                // (A collapsed box is laid out at no size: its texts keep their scale, unseen.)
                if dw > 0.0 && dh > 0.0 && r.w > 0.0 && r.h > 0.0 {
                    self.text_scale *= (r.w / dw).min(r.h / dh);
                }
            }
        }
        for c in &laid.children {
            self.spawn(c, e, r, &owner);
        }
        self.text_scale = outer;
        self.content = outer_content;
        e
    }

    /// Draws a Slate brush on `e`, tinted: its texture (a box brush 9-sliced by its margin), a UI material's first
    /// texture (the stamina ring's two as [`RadialFill`]), or a plain color.
    fn brush(&mut self, e: Entity, b: &Value, tint: [f32; 4]) {
        if b["DrawAs"].as_str() == Some("ESlateBrushDrawType::NoDrawType") {
            return;
        }
        // Drawn even when transparent (a widget animation can show it).
        let c = mul(color(&b["TintColor"], [1.0; 4]), tint);
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
#[allow(clippy::too_many_arguments)]
fn minimap(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut images: Query<&mut ImageNode>,
    hud: Res<Hud>,
    player: Option<Single<&Transform, With<Player>>>,
    camera: Option<Single<&GlobalTransform, With<crate::player::PlayerCamera>>>,
    nodes: Query<&MaterialNode<Minimap>>,
    mut maps: ResMut<Assets<Minimap>>,
) {
    let (Some(map), Some(player), Some(camera)) = (&hud.map, player, camera) else { return };
    // His map's image (MapImage): the level's KillerMapTexture in place of MiniMap_Mat_Inst's (SCMiniMapBase's
    // vt+0x4c8 from InitializeMinimap 0x14029c050), across SC_KillerMap_Widget (the click maps across it).
    if let (Some(file), Some(mut img)) = (&map.killer_file, hud.names.get("SC_KillerMap_Widget.Image_0").and_then(|&e| images.get_mut(e).ok())) {
        let handle = asset_server.load(file.clone());
        if img.image != handle {
            img.image = handle;
        }
    }
    let Some(&e) = hud.names.iter().find(|(k, _)| k.as_str() == MINIMAP_RETAINER).map(|(_, e)| e) else { return };
    let Ok(node) = nodes.get(e) else { return };
    let uv = map_uv(map.area, player.translation);
    let f = camera.forward();
    let dir = map_dir(map.area, *f).normalize_or(Vec2::NEG_Y);
    if let Some(mut m) = maps.get_mut(&node.0) {
        m.view = Vec4::new(uv.x, uv.y, map.zoom, MINIMAP_RADIUS);
        m.dir = Vec4::new(dir.x, dir.y, 0.0, 0.0);
    }
    // North (the map's up) clockwise from the minimap's up.
    if let Some((_, &compass)) = hud.names.iter().find(|(k, _)| k.ends_with(".ComapssPanel")) {
        commands.entity(compass).insert(UiTransform { rotation: Rot2::radians((-dir.x).atan2(-dir.y)), ..default() });
    }
}

/// Where a place (Bevy, m) is on the level's map texture: across it the world's Y from the volume's minimum, down
/// it the world's X from its maximum (ILLMinimap's TopLeft = (max X, min Y): PositionMinimap 0x1402a4420 offsets the
/// map by the pawn's Y across and X down; GetClickWorldLocationForSlot 0x140297e70 the same way back).
pub(crate) fn map_uv(area: [f32; 4], p: Vec3) -> Vec2 {
    let [x0, y0, x1, y1] = area;
    let (x, y) = (p.x * 100.0, p.z * 100.0);
    Vec2::new((y - y0) / (y1 - y0), (x1 - x) / (x1 - x0))
}

/// A world direction (Bevy) in the map texture's UV space (its scale: the map's size).
fn map_dir(area: [f32; 4], d: Vec3) -> Vec2 {
    let [x0, y0, x1, y1] = area;
    Vec2::new(d.z / (y1 - y0), -d.x / (x1 - x0))
}

/// Something the minimap shows (its class's SCMinimapIconComponent): the icon widget counselors see (DefaultIcon)
/// and Jason's (KillerIcon); the minimap picks one by whose it is (its icon getter), none when that one is unset.
/// Items get none: SCItem's constructor, its BeginPlay (0x140373ee0) and its pickup (vtable slot 0x640, 0x1403901f0)
/// all SetVisibility(false) its MinimapIcon, and only SCImposterOutfit::OnShowIcon turns one on (no item Blueprint
/// does); the minimap shows only visible icon components (UpdateObjectLocations). Nor do icons not visible from the
/// start (bStartVisible).
#[derive(Component, Clone, Debug)]
pub struct MapIcon {
    pub counselor: Option<String>,
    pub killer: Option<String>,
}

impl MapIcon {
    pub fn of(classes: &ClassDb, class: &str) -> Option<MapIcon> {
        if classes.is_child_of(class, "SCItem") {
            return None;
        }
        let c = classes.get(class)?.components.iter().find(|c| classes.is_child_of(&c.kind, "ILLMinimapIconComponent") || c.kind == "SCMinimapIconComponent")?;
        if c.props["bStartVisible"].as_bool() == Some(false) {
            return None;
        }
        let widget = |k: &str| c.props[k].as_str().map(|s| s.trim_end_matches("_C").to_string());
        let icon = MapIcon { counselor: widget("DefaultIcon"), killer: widget("KillerIcon") };
        (icon.counselor.is_some() || icon.killer.is_some()).then_some(icon)
    }
}

/// An icon drawn on the minimap, for the entity it shows.
#[derive(Component)]
struct MapIconNode;

/// An icon widget's image and size (design units): its first Image, laid out about the icon's point.
fn icon_image(dir: &std::path::Path, widget: &str) -> Option<(String, Vec2, [f32; 4])> {
    fn find<'a>(l: &'a f13_data::umg::Laid<'a>) -> Option<(String, Vec2, [f32; 4])> {
        if l.widget.kind == "Image" {
            let b = &l.widget.props["Brush"];
            if let Some(file) = b["ResourceObject"]["File"].as_str() {
                return Some((file.to_string(), Vec2::new(l.rect.w, l.rect.h), color(&b["TintColor"]["SpecifiedColor"], [1.0; 4])));
            }
        }
        l.children.iter().find_map(find)
    }
    let w = Widget::load(dir, widget)?;
    let laid = w.layout(f13_data::umg::Rect::default());
    find(&laid)
}

/// His own icon on his map: Jason_Base's MinimapIconComponent's KillerIcon.
const PLAYER_ARROW: &str = "PlayerArrow_Widget";

/// Jason's map's icons (ILLMinimap::UpdateObjectLocations 0x1402ae080 on SC_KillerMap_Widget, whose map isn't centred
/// on him or turned): each MapIcon's KillerIcon, and his own (PlayerArrow_Widget), in its IconPanel where its place
/// falls across the map, at the angle its widget's UpdateMapWidget leaves (SetRenderAngle). They take the map's
/// opacity (MapFade).
#[allow(clippy::too_many_arguments)]
fn killer_map_icons(
    commands: Commands,
    hud: Res<Hud>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    window: Single<&Window, With<PrimaryWindow>>,
    assets: Res<AssetServer>,
    icons: Query<(Entity, &MapIcon, &GlobalTransform)>,
    jasons: Query<(Entity, &GlobalTransform), With<crate::jason::Jason>>,
    (panels, tints): (Query<&ComputedNode>, Query<&WidgetColor>),
    nodes: Query<(&mut Node, &mut ImageNode, &mut UiTransform), With<MapIconNode>>,
    mut placed: Local<(Option<Entity>, HashMap<Entity, Entity>)>,
    mut images: Local<HashMap<String, Option<(String, Vec2, [f32; 4])>>>,
) {
    let (Some(_), Some(map)) = (play, &hud.map) else { return };
    let Some(&panel) = hud.names.get("SC_KillerMap_Widget.IconPanel") else { return };
    let tint = hud.names.get("KillerHUDWidget.SC_KillerMap_Widget").and_then(|&e| tints.get(e).ok()).map_or([1.0; 4], |c| c.0);
    let shown: Vec<(Entity, String, GlobalTransform)> =
        icons.iter().filter_map(|(e, i, g)| Some((e, i.killer.clone()?, *g))).chain(jasons.iter().map(|(e, g)| (e, PLAYER_ARROW.to_string(), *g))).collect();
    place_map_icons(commands, &hud, map, panel, tint, &window, &assets, &panels, nodes, &mut placed, &mut images, shown);
}

/// Her map screen's icons (UpdateObjectLocations on CounselorMap_Widget, which isn't turned: FUN_140430e20 turns a map
/// only with bCanEverRotate, which only her corner minimap sets): each MapIcon's DefaultIcon in its IconPanel where
/// its place falls across the map. And her place: PLACEHOLDER (the user's account of the game): PlayerArrow_Widget,
/// as Jason's map shows him; her MinimapIconComponent has no DefaultIcon and the map no PlayerViewIcon.
#[allow(clippy::too_many_arguments)]
fn counselor_map_icons(
    commands: Commands,
    hud: Res<Hud>,
    screen: Res<MapScreenUi>,
    window: Single<&Window, With<PrimaryWindow>>,
    assets: Res<AssetServer>,
    icons: Query<(Entity, &MapIcon, &GlobalTransform)>,
    her: Query<(Entity, &GlobalTransform), With<Player>>,
    panels: Query<&ComputedNode>,
    nodes: Query<(&mut Node, &mut ImageNode, &mut UiTransform), With<MapIconNode>>,
    mut placed: Local<(Option<Entity>, HashMap<Entity, Entity>)>,
    mut images: Local<HashMap<String, Option<(String, Vec2, [f32; 4])>>>,
) {
    let Some(map) = &hud.map else { return };
    let Some(&panel) = screen.names.get("CounselorMap_Widget.IconPanel") else { return };
    let shown: Vec<(Entity, String, GlobalTransform)> =
        icons.iter().filter_map(|(e, i, g)| Some((e, i.counselor.clone()?, *g))).chain(her.iter().map(|(e, g)| (e, PLAYER_ARROW.to_string(), *g))).collect();
    place_map_icons(commands, &hud, map, panel, [1.0; 4], &window, &assets, &panels, nodes, &mut placed, &mut images, shown);
}

/// Her objectives and map screen as built: its root, its widgets by "<class>.<name>", and what it was built for.
#[derive(Resource, Default)]
pub struct MapScreenUi {
    root: Option<Entity>,
    names: HashMap<String, Entity>,
    built: Option<(Vec2, Vec<bool>)>,
    /// When it opened (its AnimIn and BorderIn play from then).
    opened: Option<f32>,
}

/// The objective rows' state (UpdateObjectives, checked every 0.2 s by ObjectivesWidget's CheckObjectives): per row
/// in FindObjectives' order, its lines' IsCompleted.
fn objective_rows(
    vehicles: &Query<&crate::props::Vehicle>,
    boxes: &Query<&crate::police::PhoneBox>,
    phones: &Query<&crate::police::Phone>,
    game: Option<&crate::match_flow::GameMatch>,
    escaped: Option<&crate::vehicle::Escaped>,
) -> Vec<(&'static str, Vec<bool>)> {
    let mut rows = Vec::new();
    // FindPhone: the phone (Objective_Police_Widget): the box repaired (IsPhoneBoxRepaired), the police called
    // (IsPoliceCalled), on the scene (called and PoliceTimeRemaining at 0); its escape line is never ticked.
    if !phones.is_empty() {
        let repaired = boxes.iter().any(|b| b.repaired);
        let called = phones.iter().any(|p| p.called);
        let here = called && game.is_some_and(|g| g.state.police_time_remaining <= 0 && g.state.police_time_remaining > -1);
        rows.push(("police", vec![repaired, called, here, false]));
    }
    // Find4DoorCar, Find2DoorCar (GetCarVehicle(bFourDoor): the 4-seat Chevette_BP, the 2-seat Chevette_2Seat_BP):
    // the hood's and gas tank's repairs, the first start (bFirstStartComplete), escaped in it.
    for four in [true, false] {
        let Some(v) = vehicles.iter().find(|v| v.class.contains("2Seat") != four) else { continue };
        let done = |name: &str| v.repairs.iter().any(|r| r.name == name && r.done);
        let gone = escaped.is_some_and(|e| e.how.contains(&format!("({})", v.class.trim_end_matches("_C"))));
        rows.push((if four { "car4" } else { "car2" }, vec![done("HoodRepair"), done("GasTankRepair"), v.started, gone]));
    }
    rows
}

/// An objective row as its widgets' Construct and bindings leave it: each ObjectivesHeader's icon and text, each
/// ObjectivesLineWidget's text (ObjectiveText) and its tick (BGImg and Check visible when IsCompleted).
fn fill_objective_row(w: &mut Widget, done: &[bool], four: Option<bool>) {
    fn walk(w: &mut Widget, lines: &mut std::slice::Iter<'_, bool>) {
        match w.class.as_deref() {
            Some("ObjectivesHeader") => {
                let (text, icon) = (w.props["ObjectiveHeaderText"].clone(), w.props["ObjectiveIcon"]["File"].clone());
                if let Some(t) = w.find_mut("txtObjectiveHeader") {
                    t.props["Text"] = text;
                }
                if let (Some(i), Some(file)) = (w.find_mut("imgIcon"), icon.as_str()) {
                    i.props["Brush"]["ResourceObject"] = json!({ "File": file });
                }
            }
            Some("ObjectivesLineWidget") => {
                let text = w.props["ObjectiveText"].clone();
                let done = lines.next().copied().unwrap_or(false);
                if let Some(t) = w.find_mut("ObjectiveTxt") {
                    t.props["Text"] = text;
                }
                for n in ["BGImg", "Check"] {
                    if let Some(i) = w.find_mut(n) {
                        i.set_visibility(if done { Vis::Visible } else { Vis::Hidden });
                    }
                }
            }
            _ => {}
        }
        for c in &mut w.children {
            walk(c, lines);
        }
    }
    // Objective_Car_Widget: the 4 or 2 seat header by Is4DoorCar; CrossOutImage and the destroyed texts stay hidden
    // (no car is destroyed here).
    if let Some(four) = four {
        for (n, shown) in [("4SeaterHeader", four), ("2SeaterHeader", !four)] {
            if let Some(h) = w.find_mut(n) {
                h.set_visibility(if shown { Vis::Visible } else { Vis::Collapsed });
            }
        }
    }
    walk(w, &mut done.iter());
}

/// GLB_Map for her (items.rs `MapScreen`): CounselorHUDWidget's OnShowCounselorMap pushes ObjectivesWidget and plays
/// HideHUD; closing it, ShowHUD. ObjectivesWidget opens on its objectives page (Construct: OpenObjectives: MainSwitcher
/// 0, ObjectivesButtonSwitcher 1), its rows from FindObjectives, CounselorMap_Widget shown with the Map (or the
/// Preparedness perk: GetVisibility_0), else NoMapImage and NoMapText (GetVisibility_NoMap). The map is its
/// MapTexture whole (unturned, and drawn as UI: no lighting).
#[allow(clippy::too_many_arguments)]
fn map_screen(
    mut commands: Commands,
    hud: Res<Hud>,
    mut ui: ResMut<MapScreenUi>,
    open: Option<Res<crate::items::MapScreen>>,
    window: Single<&Window, With<PrimaryWindow>>,
    assets: Res<AssetServer>,
    mut materials: ResMut<Assets<RadialFill>>,
    player: Query<&Inventory, With<Player>>,
    items: Query<&Item>,
    (vehicles, boxes, phones, game, escaped): (
        Query<&crate::props::Vehicle>,
        Query<&crate::police::PhoneBox>,
        Query<&crate::police::Phone>,
        Option<Res<crate::match_flow::GameMatch>>,
        Option<Res<crate::vehicle::Escaped>>,
    ),
    mut vis: Query<&mut Visibility>,
    (time, mut borders, mut colors): (Res<Time>, Query<&mut ContentAlpha>, Query<&mut WidgetColor>),
) {
    let open = open.is_some_and(|o| o.0) && hud.objectives.is_some();
    let now = time.elapsed_secs();
    // Its Construct's AnimIn (the nav and action bars, the backdrop dimmed to half) and OpenObjectives' BorderIn (the
    // page): 0.15 s fades from when it opened.
    match (open, ui.opened) {
        (true, None) => ui.opened = Some(now),
        (false, Some(_)) => ui.opened = None,
        _ => {}
    }
    if let (Some(at), Some(anims)) = (ui.opened, hud.anims.get("ObjectivesWidget")) {
        for anim in ["AnimIn", "BorderIn"].iter().filter_map(|n| anims.get(*n)) {
            let x = (now - at).clamp(0.0, anim.length);
            for (widget, curve) in &anim.tracks {
                if let Some(mut a) = ui.names.get(&format!("ObjectivesWidget.{widget}")).and_then(|&e| borders.get_mut(e).ok()) {
                    a.set_if_neq(ContentAlpha(curve.eval(x)));
                }
            }
            for (widget, curves) in &anim.colors {
                if let Some(mut c) = ui.names.get(&format!("ObjectivesWidget.{widget}")).and_then(|&e| colors.get_mut(e).ok()) {
                    c.set_if_neq(WidgetColor(curves.each_ref().map(|k| k.eval(x))));
                }
            }
        }
    }
    let size = window.size();
    let has_map = player.single().is_ok_and(|inv| inv.special.iter().any(|&e| items.get(e).is_ok_and(|i| i.kind == crate::props::ItemKind::Map)));
    let rows = objective_rows(&vehicles, &boxes, &phones, game.as_deref(), escaped.as_deref());
    let key: Vec<bool> = [open, has_map].into_iter().chain(rows.iter().flat_map(|(_, d)| d.iter().copied())).collect();
    if ui.built.as_ref().is_some_and(|(s, k)| *s == size && *k == key) {
        return;
    }
    ui.built = Some((size, key));
    if let Some(root) = ui.root.take() {
        commands.entity(root).despawn();
    }
    ui.names.clear();
    // HideHUD / ShowHUD: her HUD under it.
    if let Some(Ok(mut v)) = hud.root.map(|r| vis.get_mut(r)) {
        v.set_if_neq(if open { Visibility::Hidden } else { Visibility::Inherited });
    }
    if !open || size.x < 1.0 || size.y < 1.0 {
        return;
    }
    let Some(mut tree) = hud.objectives.clone() else { return };
    // Its key prompts: the page tabs' CSLR_ObjectivePrev / Next are the gamepad's shoulders only (hidden).
    action_keys(&mut tree);
    // The switchers' active pages only.
    for (n, active) in [("MainSwitcher", 0), ("ObjectivesButtonSwitcher", 1), ("StatusSwitcher", 0)] {
        if let Some(w) = tree.find_mut(n) {
            if active < w.children.len() {
                let keep = w.children.swap_remove(active);
                w.children = vec![keep];
            }
        }
    }
    if let Some(list) = tree.find_mut("ObjectivesVerticleBox") {
        list.children.clear();
        for (i, (kind, done)) in rows.iter().enumerate() {
            let template = if *kind == "police" { &hud.police_objective } else { &hud.car_objective };
            let Some(mut row) = template.clone() else { continue };
            row.name = format!("Objective{i}");
            row.slot_kind = "VerticalBoxSlot".into();
            row.slot = json!({});
            fill_objective_row(&mut row, done, (*kind != "police").then_some(*kind == "car4"));
            list.children.push(row);
        }
    }
    for (n, shown) in [("CounselorMap_Widget_554", has_map), ("NoMapImage", !has_map), ("NoMapText", !has_map)] {
        if let Some(w) = tree.find_mut(n) {
            w.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
        }
    }
    // Its Map image: MiniMap_Mat_Inst, whose Texture SCMiniMapBase sets to the volume's MapTexture (0x140430e20).
    if let (Some(map), Some(img)) = (&hud.map, tree.find_mut("Map")) {
        img.props["Brush"] = json!({ "ImageSize": { "X": 730.0, "Y": 730.0 }, "ResourceObject": { "File": map.file } });
    }
    let scale = (size.x / DESIGN.x).min(size.y / DESIGN.y);
    let virt = size / scale;
    let root = commands.spawn((Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, GlobalZIndex(10), Pickable::IGNORE)).id();
    let mut names = HashMap::new();
    let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, content: [1.0; 4], names: &mut names };
    let full = Rect { x: 0.0, y: 0.0, w: virt.x, h: virt.y };
    let laid = tree.layout(full);
    b.spawn(&laid, root, full, "ObjectivesWidget");
    ui.names = names;
    ui.root = Some(root);
}

/// Puts each (thing, icon widget, place) in a map's IconPanel where its place falls across the whole map.
#[allow(clippy::too_many_arguments)]
fn place_map_icons(
    mut commands: Commands,
    hud: &Hud,
    map: &MapInfo,
    panel: Entity,
    tint: [f32; 4],
    window: &Window,
    assets: &AssetServer,
    panels: &Query<&ComputedNode>,
    mut nodes: Query<(&mut Node, &mut ImageNode, &mut UiTransform), With<MapIconNode>>,
    placed: &mut (Option<Entity>, HashMap<Entity, Entity>),
    images: &mut HashMap<String, Option<(String, Vec2, [f32; 4])>>,
    shown: Vec<(Entity, String, GlobalTransform)>,
) {
    let Ok(computed) = panels.get(panel) else { return };
    if placed.0 != Some(panel) {
        *placed = (Some(panel), HashMap::new());
    }
    let size = computed.size() * computed.inverse_scale_factor();
    let scale = (window.size().x / DESIGN.x).min(window.size().y / DESIGN.y);
    let dir = hud.export.clone();
    let mut seen = Vec::new();
    for (e, widget, g) in shown {
        let Some((file, isize, brush)) = images.entry(widget.clone()).or_insert_with(|| icon_image(&dir, &widget)).clone() else { continue };
        let uv = map_uv(map.area, g.translation());
        let s = isize * scale;
        let (left, top) = (Val::Px(uv.x * size.x - s.x / 2.0), Val::Px(uv.y * size.y - s.y / 2.0));
        // Its yaw in Unreal (X north on the map, Y east): clockwise on screen. Each icon widget's UpdateMapWidget may
        // change it: the cabins', cars', phone's, fuse boxes', camps', traps' and police's set it to (0, 0, 0); only
        // PlayerArrow_Widget's keeps it.
        let f = g.rotation() * Vec3::Z;
        let rotation = if widget == PLAYER_ARROW { Rot2::radians(f.z.atan2(f.x)) } else { Rot2::IDENTITY };
        let color = linear(mul(brush, tint));
        seen.push(e);
        match placed.1.get(&e).and_then(|&n| nodes.get_mut(n).ok()) {
            Some((mut n, mut img, mut t)) => {
                n.left = left;
                n.top = top;
                img.color = color;
                t.rotation = rotation;
            }
            None => {
                let n = commands
                    .spawn((
                        Node { position_type: PositionType::Absolute, left, top, width: Val::Px(s.x), height: Val::Px(s.y), ..default() },
                        ImageNode { color, ..ImageNode::new(assets.load(file)) },
                        UiTransform { rotation, ..default() },
                        MapIconNode,
                        Pickable::IGNORE,
                        ChildOf(panel),
                    ))
                    .id();
                placed.1.insert(e, n);
            }
        }
    }
    placed.1.retain(|src, n| {
        let keep = seen.contains(src);
        if !keep {
            commands.entity(*n).despawn();
        }
        keep
    });
}

/// The minimap's icons: each MapIcon's widget where its place falls on the map (the shader's mapping turned
/// around), hidden outside the map's circle.
#[allow(clippy::too_many_arguments)]
fn map_icons(
    mut commands: Commands,
    hud: Res<Hud>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    window: Single<&Window, With<PrimaryWindow>>,
    assets: Res<AssetServer>,
    icons: Query<(Entity, &MapIcon, &GlobalTransform)>,
    maps: Query<(&MaterialNode<Minimap>, &ComputedNode)>,
    all_maps: Res<Assets<Minimap>>,
    mut nodes: Query<(&mut UiTransform, &mut Visibility), With<MapIconNode>>,
    mut placed: Local<(Option<Entity>, HashMap<Entity, Entity>)>,
    mut images: Local<HashMap<String, Option<(String, Vec2, [f32; 4])>>>,
) {
    let Some(map) = &hud.map else { return };
    let Some(&retainer) = hud.names.iter().find(|(k, _)| k.as_str() == MINIMAP_RETAINER).map(|(_, e)| e) else { return };
    let Ok((node, computed)) = maps.get(retainer) else { return };
    let Some(m) = all_maps.get(&node.0) else { return };
    // A rebuilt HUD took the old icons with it.
    if placed.0 != Some(retainer) {
        *placed = (Some(retainer), HashMap::new());
    }
    let size = computed.size() * computed.inverse_scale_factor();
    let scale = (window.size().x / DESIGN.x).min(window.size().y / DESIGN.y);
    let f = m.dir.truncate().truncate().normalize_or(Vec2::NEG_Y);
    let r = Vec2::new(-f.y, f.x);
    let jason = play.is_some();
    let dir = hud.export.clone();
    for (e, icon, gxf) in &icons {
        let Some(widget) = (if jason { &icon.killer } else { &icon.counselor }) else { continue };
        let Some((file, isize, brush)) = images.entry(widget.clone()).or_insert_with(|| icon_image(&dir, widget)).clone() else { continue };
        let p = gxf.translation();
        let uv = map_uv(map.area, p);
        let delta = (uv - m.view.truncate().truncate()) / m.view.z;
        let d = Vec2::new(delta.dot(r), -delta.dot(f));
        let shown = d.length() <= m.view.w;
        let s = isize * scale;
        // Placed by its transform, not its layout: layout positions snap to whole pixels while the map under it
        // scrolls smoothly, which shook the icons.
        let at = Val2::px((0.5 + d.x) * size.x - s.x / 2.0, (0.5 + d.y) * size.y - s.y / 2.0);
        let vis = if shown { Visibility::Inherited } else { Visibility::Hidden };
        match placed.1.get(&e).and_then(|&n| nodes.get_mut(n).ok()) {
            Some((mut t, mut v)) => {
                t.translation = at;
                v.set_if_neq(vis);
            }
            None => {
                let n = commands
                    .spawn((
                        Node { position_type: PositionType::Absolute, left: Val::Px(0.0), top: Val::Px(0.0), width: Val::Px(s.x), height: Val::Px(s.y), ..default() },
                        UiTransform { translation: at, ..default() },
                        ImageNode { color: linear(brush), ..ImageNode::new(assets.load(file)) },
                        vis,
                        MapIconNode,
                        Pickable::IGNORE,
                        ChildOf(retainer),
                    ))
                    .id();
                placed.1.insert(e, n);
            }
        }
    }
    // Icons whose thing is gone.
    placed.1.retain(|&src, n| {
        let keep = icons.contains(src);
        if !keep {
            commands.entity(*n).try_despawn();
        }
        keep
    });
}

/// Sizes a progress bar's fill to `percent` along its fill type; its image stays put, clipped.
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
    commands.entity(p.image).entry::<Node>().and_modify(move |mut n| {
        n.left = Val::Px(-left);
        n.top = Val::Px(-top);
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
fn timer(
    time: Res<Time>,
    hud: Res<Hud>,
    game: Res<crate::match_flow::GameMatch>,
    mut texts: Query<&mut Text>,
    mut playing: ResMut<PlayingAnims>,
    mut shown: Local<(bool, bool)>,
) {
    let Some(&e) = hud.names.get("SCCounselorTimerWidget.txtTimer").or_else(|| hud.names.get("SCKillerTimerWidget.txtTimer")) else { return };
    let Ok(mut text) = texts.get_mut(e) else { return };
    let left = game.state.remaining_time.max(0);
    let s = format!("{:02}:{:02}", left / 60, left % 60);
    if text.0 != s {
        text.0 = s;
    }
    // SCTimerWidget's tick (FUN_140434db0): the game timer shows (ShowGameTimer: Fade Game Timer In) while the time
    // left is within TimeToShow, and hides (the animation reversed) outside it; the police timer while their
    // countdown runs. (Jason's timer widget plays its own FadeIn on ClockBorder's colour: not ported.)
    if hud.names.contains_key("SCCounselorTimerWidget.GameTimer") {
        let now = time.elapsed_secs();
        let remaining = game.state.remaining_time;
        let within = (0..=hud.time_to_show).contains(&remaining);
        if within != shown.0 {
            shown.0 = within;
            playing.play("SCCounselorTimerWidget", "Fade Game Timer In", now, !within);
        }
        let police = game.state.police_time_remaining >= 1;
        if police != shown.1 {
            shown.1 = police;
            playing.play("SCCounselorTimerWidget", "Fade Police Timer In", now, !police);
        }
    }
    // The police's countdown (PoliceTimerText), while it runs.
    let police = game.state.police_time_remaining;
    if let Some(mut text) = hud.names.get("SCCounselorTimerWidget.txtPoliceTimer").and_then(|&e| texts.get_mut(e).ok()).filter(|_| police > 0) {
        let s = format!("{:02}:{:02}", police / 60, police % 60);
        if text.0 != s {
            text.0 = s;
        }
    }
}

/// Jason's HUD: his knives and traps left, each ability's unlock bar filling and its icon's glow while it's ready
/// (its charge bar as it recharges), and his rage building.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
fn jason_hud(
    mut commands: Commands,
    time: Res<Time>,
    hud: Res<Hud>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<(&crate::jason::Jason, Option<&crate::senses::Knowledge>, &Transform)>,
    her: Query<(&Player, &Transform, Has<crate::jason::Held>, Has<Hiding>), Without<crate::jason::Jason>>,
    tuning: Option<Res<crate::jason::KillerTuning>>,
    (mut texts, mut icon_colors): (Query<&mut Text>, Query<&mut WidgetColor>),
    bars: Query<&Progress>,
    mut vis: Query<&mut Visibility>,
    mut playing: ResMut<PlayingAnims>,
    mut shown: Local<(bool, bool, f32)>,
    (mut morph_map, map_nodes): (Option<ResMut<crate::jason_player::MorphMap>>, Query<(&ComputedNode, &UiGlobalTransform)>),
    mut map_shown: Local<bool>,
    (grab_info, grab_available, asset_server, mut images): (
        Option<Res<crate::jason::GrabKillInfo>>,
        Option<Res<crate::jason::GrabKillsAvailable>>,
        Res<AssetServer>,
        Query<&mut ImageNode>,
    ),
) {
    use f13_core::killer_abilities::{Ability, Blockers};
    if play.is_none() || hud.built.as_ref().is_none_or(|b| !b.2) {
        return;
    }
    let Some((j, know, jxf)) = jasons.iter().next() else { return };
    let now = time.elapsed_secs();
    // GetCombatVisibility: in combat stance or a kill, these hide.
    let combat = j.combat || matches!(j.state, crate::jason::JasonState::Killing { .. });
    for name in ["KillerAbilityWidget_521", "GrabIconWidget_121", "RageWidget_190", "KillerKnifeWidget_77", "KillerTrapWidget_184"] {
        if let Some(mut v) = hud.names.get(&format!("KillerHUDWidget.{name}")).and_then(|&e| vis.get_mut(e).ok()) {
            v.set_if_neq(if combat { Visibility::Hidden } else { Visibility::Inherited });
        }
    }
    // KillerGrabWidget while he holds her (Get_CanvasPanel_64_Visibility_0: GetGrabbedCounselor valid): Icon1-4 are
    // slots 0-3 (right, bottom, left, top: Get_Icon*_Brush, GetGrabKillIcon(slot, true): the kill's
    // ContextKillHUDIcon), IconUsableColor when IsGrabKillAvailable, else IconLockedColor (GetIconColor).
    // His abilities' diamond gives way to it (SCKillerAbilityWidget::GetWidgetVisibility 0x14042dee0: collapsed while
    // he holds someone).
    for (name, shown) in [("KillerGrabWidget.CanvasPanel_64", j.holding()), ("KillerAbilityWidget.CanvasPanel_64", !j.holding())] {
        if let Some(mut v) = hud.names.get(name).and_then(|&e| vis.get_mut(e).ok()) {
            v.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
    if j.holding() {
        const USABLE: [f32; 4] = [0.854993, 0.030713, 0.017642, 0.498039];
        const LOCKED: [f32; 4] = [0.039546, 0.039546, 0.039546, 1.0];
        for (slot, &kill) in crate::jason::GRAB_KILL_SLOTS.iter().enumerate() {
            let icon = hud.names.get(&format!("KillerGrabWidget.Icon{}", slot + 1)).copied();
            // An empty slot: no brush (Get_Icon*_Brush's NoDrawType).
            if let Some(mut v) = icon.and_then(|e| vis.get_mut(e).ok()) {
                v.set_if_neq(if kill.is_some() { Visibility::Inherited } else { Visibility::Hidden });
            }
            let Some(kill) = kill else { continue };
            let Some(mut img) = icon.and_then(|e| images.get_mut(e).ok()) else { continue };
            if let Some(file) = grab_info.as_ref().and_then(|i| i.icons[kill].as_ref()) {
                let handle = asset_server.load(file.clone());
                if img.image != handle {
                    img.image = handle;
                }
            }
            let c = linear(if grab_available.as_ref().is_some_and(|a| a.0[slot]) { USABLE } else { LOCKED });
            if img.color != c {
                img.color = c;
            }
        }
    }
    // His map (Map_Overylay): open (bIsMapEnabled); MorphUse / MorphCancel and the map's header while it's out for a
    // Morph (Get_MorphUse_Visibility_0, Get_HeaderBracket_Visibility_0); MapClose when open otherwise.
    if let Some(m) = morph_map.as_deref_mut() {
        let set = |vis: &mut Query<&mut Visibility>, name: &str, on: bool| {
            if let Some(mut v) = hud.names.get(name).and_then(|&e| vis.get_mut(e).ok()) {
                v.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
            }
        };
        // UpdateMapVisibility: bShowMorphMap (Jason_Base's SetMapStatus: the map shown, either map) changed -> MapFade,
        // reversed as it's put away (the overlay's widgets are transparent until it plays).
        if m.open != *map_shown {
            *map_shown = m.open;
            playing.play("KillerHUDWidget", "MapFade", now, !m.open);
        }
        for n in ["MorphUse", "MorphUseText", "MorphCancel", "MorphCancelText"] {
            set(&mut vis, &format!("KillerHUDWidget.{n}"), m.morph);
        }
        for n in ["MapClose", "MapCloseText"] {
            set(&mut vis, &format!("KillerHUDWidget.{n}"), m.open && !m.morph);
        }
        for n in ["HeaderBar", "HeaderBracket", "TextBlock_0"] {
            set(&mut vis, &format!("SC_KillerMap_Widget.{n}"), m.morph);
        }
        // GetClickWorldLocationForSlot (0x140297e70): the click's place across the map widget (clamped 0..1); world X
        // down from TopLeft.X, Y across from TopLeft.Y, at his height.
        if let Some(click) = m.click.take() {
            let rect = hud.names.get("KillerHUDWidget.SC_KillerMap_Widget").and_then(|&e| map_nodes.get(e).ok());
            if let (Some((node, gxf)), Some(map)) = (rect, &hud.map) {
                let scale = node.inverse_scale_factor();
                let size = node.size() * scale;
                let top_left = gxf.translation * scale - size / 2.0;
                let uv = ((click - top_left) / size.max(Vec2::splat(1.0))).clamp(Vec2::ZERO, Vec2::ONE);
                let [x0, y0, x1, y1] = map.area;
                let world = Vec3::new((x1 - uv.y * (x1 - x0)) / 100.0, jxf.translation.y, (y0 + uv.x * (y1 - y0)) / 100.0);
                info!("Morph map: clicked ({:.2}, {:.2}) of {:.0}x{:.0} at {:.0} -> ({:.0}, {:.0})", uv.x, uv.y, size.x, size.y, top_left, world.x, world.z);
                m.target = Some(world);
            }
        }
    }
    let (grab_shown, reticle, next_check) = &mut *shown;
    // UpdateGrabIcon, every 0.5 s: IsCharacterInGrabRange (a living counselor, not held or hidden, within
    // MaxGrabDistance and MinGrabAngle of where he faces) shows the grab icon (ShowGrabIcon, then PulseGrabIcon),
    // else hides it (ShowGrabIcon reversed).
    if now >= *next_check {
        *next_check = now + 0.5;
        let in_range = tuning.as_ref().is_some_and(|t| {
            her.iter().any(|(p, xf, held, hiding)| {
                let d = xf.translation - jxf.translation;
                let facing = jxf.rotation * Vec3::Z;
                p.counselor.alive() && !held && !hiding && d.length() < t.grab_max_m && facing.dot(d.normalize_or_zero()) > t.grab_angle_deg.to_radians().cos()
            })
        });
        if in_range != *grab_shown {
            *grab_shown = in_range;
            playing.play("KillerHUDWidget", "ShowGrabIcon", now, !in_range);
            if !in_range {
                playing.0.retain(|(o, a, _, _)| !(o == "KillerHUDWidget" && a == "PulseGrabIcon"));
            }
        }
    }
    if *grab_shown && !playing.0.iter().any(|(o, a, _, _)| o == "KillerHUDWidget" && a == "PulseGrabIcon") {
        if playing.0.iter().any(|(o, a, s, _)| o == "KillerHUDWidget" && a == "ShowGrabIcon" && now - s >= 0.5) {
            playing.play("KillerHUDWidget", "PulseGrabIcon", now, false);
        }
    }
    // PulseGrabIcon loops.
    if let Some(p) = playing.0.iter_mut().find(|(o, a, _, _)| o == "KillerHUDWidget" && a == "PulseGrabIcon") {
        if now - p.2 >= 1.0 {
            p.2 += 1.0;
        }
    }
    // The knife's reticle while he aims (PressKnifeThrow -> OnShowKnifeReticle: ReticleFade; let go or put away,
    // reversed).
    let aiming = matches!(j.state, crate::jason::JasonState::Throwing { release: None, .. });
    if aiming != *reticle {
        *reticle = aiming;
        playing.play("KillerHUDWidget", "ReticleFade", now, !aiming);
    }
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
        // GetCurrentRechargeTimePercent (0x1403ede80): while it runs, Sense's and Shift's bars empty as they go
        // (1 - ActiveSenseTimer / SenseDuration, 1 - the Shift's elapsed time / ShiftTime) and Stalk's is
        // StalkActiveTimer / StalkActiveTime; else the recharge, all clamped to 0..1.
        let amount = match ability {
            Ability::Sense if k.sensing(now) => (k.sense_until - now) / k.sense_length.max(1e-3),
            Ability::Shift => match &j.state {
                crate::jason::JasonState::Shifting { shift } => 1.0 - shift.elapsed / tuning.as_ref().map_or(4.0, |t| t.shift_time).max(1e-3),
                _ => a.recharge_fraction(ability),
            },
            Ability::Stalk => match &j.stalk {
                Some(s) => s.timer / tuning.as_ref().map_or(20.0, |t| t.stalk_active_time).max(1e-3),
                None => a.recharge_fraction(ability),
            },
            _ => a.recharge_fraction(ability),
        };
        bar(&format!("KillerAbilityWidget.{n}Bar"), amount.clamp(0.0, 1.0));
        let ready = a.is_available(ability, &Blockers::default());
        // Get<ability>IconColor (FUN_140424da0): locked, IconLockedColor; unlocked, IconUsableColor while it's
        // available, else IconUnusableColor.
        let [usable, unusable, locked] = hud.ability_icon_colors;
        let c = if !a.is_unlocked(ability) { locked } else if ready { usable } else { unusable };
        if let Some(mut w) = hud.names.get(&format!("KillerAbilityWidget.Icon{n}")).and_then(|&e| icon_colors.get_mut(e).ok()) {
            w.set_if_neq(WidgetColor(c));
        }
        if let Some(mut v) = hud.names.get(&format!("KillerAbilityWidget.Icon{n}_Glow")).and_then(|&e| vis.get_mut(e).ok()) {
            v.set_if_neq(if ready { Visibility::Inherited } else { Visibility::Hidden });
        }
    }
    let rage = if a.rage_active { 1.0 } else { (a.rage_timer / a.tuning.rage_unlock_time.max(1.0)).clamp(0.0, 1.0) };
    bar("RageWidget.ProgressBar_41", rage);
}

/// The breath widget while she hides: both bars at her breath.
#[allow(clippy::too_many_arguments)]
fn breath(
    mut commands: Commands,
    time: Res<Time>,
    hud: Res<Hud>,
    player: Option<Single<(Option<&Breathing>, Has<Hiding>), With<Player>>>,
    tuning: Option<Res<crate::hiding::BreathTuning>>,
    mut vis: Query<&mut Visibility>,
    bars: Query<&Progress>,
    mut playing: ResMut<PlayingAnims>,
    mut was_shown: Local<bool>,
) {
    let Some(&root) = hud.names.get("SCBreathWidget") else { return };
    let (breathing, hiding) = player.map_or((None, false), |p| *p);
    let shown = hiding && breathing.is_some();
    // Its Construct (the game creates it as she hides): TransitionIn.
    if shown && !*was_shown {
        playing.play("SCBreathWidget", "TransitionIn", time.elapsed_secs(), false);
    }
    *was_shown = shown;
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

/// SC_Noise_Widget's pulse (its Tick, UpdateRings, BeginPulse, EndPulse): the time to the next ping, the pulse's
/// time, whether the rings are out, their end size; the rings' centres (pixels) on the HUD it was built for.
#[derive(Default)]
struct NoisePulse {
    next: f32,
    current: f32,
    active: bool,
    end: Vec2,
    root: Option<Entity>,
    centres: HashMap<Entity, Vec2>,
}

/// The minimap's noise rings: while she moves, a ping every 5-15 s over her noise level (GetNoiseLevel); each ping
/// grows both rings from their start sizes to 10 x her noise along PingCurve over PingTime, then hides them (0.1).
fn noise_meter(
    time: Res<Time>,
    hud: Res<Hud>,
    player: Option<Single<&Player>>,
    values: Option<Res<crate::senses::SenseValues>>,
    mut rng: ResMut<crate::GameRng>,
    mut nodes: Query<&mut Node>,
    mut s: Local<NoisePulse>,
) {
    let (Some(w), Some(p)) = (hud.noise.as_ref(), player) else { return };
    let rings: Vec<Entity> = ["SC_Noise_Widget.Ring1", "SC_Noise_Widget.Ring2"].iter().filter_map(|n| hud.names.get(*n).copied()).collect();
    if rings.len() < 2 {
        return;
    }
    if s.root != hud.root {
        s.root = hud.root;
        s.centres.clear();
    }
    for &e in &rings {
        if !s.centres.contains_key(&e) {
            let Ok(n) = nodes.get(e) else { return };
            let px = |v: Val| if let Val::Px(x) = v { x } else { 0.0 };
            s.centres.insert(e, Vec2::new(px(n.left) + px(n.width) / 2.0, px(n.top) + px(n.height) / 2.0));
        }
    }
    let mut set = |e: Entity, size: Vec2, centres: &HashMap<Entity, Vec2>| {
        if let (Ok(mut n), Some(c)) = (nodes.get_mut(e), centres.get(&e)) {
            let size = size * hud.scale;
            n.left = Val::Px(c.x - size.x / 2.0);
            n.top = Val::Px(c.y - size.y / 2.0);
            n.width = Val::Px(size.x);
            n.height = Val::Px(size.y);
        }
    };
    let dt = time.delta_secs();
    s.next -= dt;
    // UpdateRings.
    s.current = (s.current + dt).clamp(0.0, w.ping_time);
    let c = w.curve.eval(if w.ping_time > 0.0 { s.current / w.ping_time } else { 1.0 });
    set(rings[0], w.ring1_start.lerp(s.end, c), &s.centres);
    set(rings[1], w.ring2_start.lerp(s.end, c), &s.centres);
    if s.current == w.ping_time {
        // EndPulse.
        s.active = false;
        set(rings[0], Vec2::splat(0.1), &s.centres);
        set(rings[1], Vec2::splat(0.1), &s.centres);
    }
    // BeginPulse, while she moves.
    if s.next <= 0.0 && !s.active && p.velocity.length() > 0.0 {
        let noise = values.as_ref().map_or(0.0, |v| {
            let c = &p.counselor;
            f13_core::perception::noise_level(&v.noise, &c.stats, c.stance, c.sprinting, c.running, true)
        });
        s.next = (5.0 + 10.0 * rng.0.frand()) / noise.max(0.001);
        s.active = true;
        set(rings[0], w.ring1_start, &s.centres);
        set(rings[1], w.ring2_start, &s.centres);
        s.end = Vec2::splat(10.0) * noise;
        s.current = 0.0;
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

