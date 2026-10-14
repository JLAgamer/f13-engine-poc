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

mod prompts;
pub use prompts::{Prompt, PromptSource, Prompts};

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
    // The interaction prompts' key.
    ("GLB_Interact1", "E"),
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
    // Spectating (SPEC_ToggleMode Space, SPEC_ViewNext / Prev left / right click, SPEC_ToggleHUD H).
    ("SPEC_ToggleMode", "Space"),
    ("SPEC_ViewNext", "LMB"),
    ("SPEC_ViewPrev", "RMB"),
    ("SPEC_ToggleHUD", "H"),
];

/// Shows its action's key on an ActionMapWidget (its TextWidget; no glyph image), or hides the widget.
fn action_keys(w: &mut Widget) {
    // ActionDisplay's Construct: its ActionMapWidget takes its PrimaryAction, its TitleText its Text.
    if w.class.as_deref() == Some("ActionDisplay") {
        let (action, text) = (w.props["PrimaryAction"].clone(), w.props["Text"].clone());
        if let Some(m) = w.find_mut("ActionMapWidget") {
            m.props["PrimaryAction"] = action;
        }
        if let (Some(t), false) = (w.find_mut("TitleText"), text.is_null()) {
            t.props["Text"] = text;
        }
    }
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
        embedded_asset!(app, "shaders/additive.wgsl");
        app.add_plugins((UiMaterialPlugin::<RadialFill>::default(), UiMaterialPlugin::<Minimap>::default(), UiMaterialPlugin::<Additive>::default()))
            .init_resource::<PlayingAnims>()
            .init_resource::<MapScreenUi>()
            .init_resource::<Prompts>()
            .init_resource::<prompts::PromptUi>()
            .add_systems(Startup, load_hud)
            .init_resource::<MatchEndPage>()
            .add_systems(Update, lobby_mute_clicks)
            .add_systems(Update, entry_hover.run_if(resource_exists::<Hud>))
            .add_systems(Update, (match_end_page, build_hud, (stamina, timer, breath, dial, minimap, map_icons, killer_map_icons, (map_screen, counselor_map_icons).chain(), jason_hud, ability_popup, morph_cursor, rage_hud, noise_meter, xp_lines, wiggle_state.pipe(wiggle_widget)), prompts::prompts, spectator_hud, widget_anims, fear_widgets, content_alpha, widget_colors, additive_brushes, hide_for_jason).chain().run_if(resource_exists::<Hud>));
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

/// A brush on M_UI_JasonHud_01 (Jason's ability and rage bars and frames, his notices' icons, his Morph map's cursor): a
/// BLEND_Additive UI material, its graph cooked away and ported from its compiled shader (`shaders/additive.wgsl`): the
/// texture times the brush's tint (twice), its opacity through the Smoke and Grime layers (Brightness scales the smoke),
/// added to what's under it.
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct Additive {
    #[uniform(0)]
    color: LinearRgba,
    /// Brightness, Smoke, Grime.
    #[uniform(1)]
    params: Vec4,
    /// Strech X / Y, Offset X / Y.
    #[uniform(2)]
    coords: Vec4,
    #[texture(3)]
    #[sampler(4)]
    texture: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    smoke: Handle<Image>,
    #[texture(7)]
    #[sampler(8)]
    grime: Handle<Image>,
    /// The scene under it (tonemap.rs SceneCopy): its add is made in Slate's gamma space.
    #[texture(9)]
    #[sampler(10)]
    scene: Handle<Image>,
}

impl UiMaterial for Additive {
    fn fragment_shader() -> ShaderRef {
        "embedded://f13_game/shaders/additive.wgsl".into()
    }

    fn specialize(descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor, _key: bevy::ui_render::prelude::UiMaterialKey<Self>) {
        use bevy::render::render_resource::{BlendComponent, BlendFactor, BlendOperation, BlendState};
        if let Some(Some(target)) = descriptor.fragment.as_mut().and_then(|f| f.targets.get_mut(0)) {
            target.blend = Some(BlendState {
                color: BlendComponent { src_factor: BlendFactor::One, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                alpha: BlendComponent::OVER,
            });
        }
    }
}

/// An additive brush to draw: its texture, its colour (tint, played colour) and the instance's parameters (uniexpr.py's
/// defaults: Brightness 1, Smoke 0, Grime 0, Strech 1, Offset 0).
#[derive(Component, Clone)]
struct AdditiveBrush {
    texture: Handle<Image>,
    color: [f32; 4],
    brightness: f32,
    smoke: f32,
    grime: f32,
    strech: [f32; 2],
    offset: [f32; 2],
}

impl AdditiveBrush {
    fn new(texture: Handle<Image>, color: [f32; 4], scalars: &serde_json::Value) -> Self {
        let f = |k: &str, d: f32| scalars[k].as_f64().map_or(d, |v| v as f32);
        AdditiveBrush {
            texture,
            color,
            brightness: f("Brightness", 1.0),
            smoke: f("Smoke", 0.0),
            grime: f("Grime", 0.0),
            strech: [f("Strech X", 1.0), f("Strech Y", 1.0)],
            offset: [f("Offset X", 0.0), f("Offset Y", 0.0)],
        }
    }
}

/// Gives each new additive brush its material, and keeps a changed one's colour and parameters.
fn additive_brushes(
    mut commands: Commands,
    mut materials: ResMut<Assets<Additive>>,
    brushes: Query<(Entity, Ref<AdditiveBrush>, Option<&MaterialNode<Additive>>)>,
    assets: Res<AssetServer>,
    mut layers: Local<Option<(Handle<Image>, Handle<Image>)>>,
    scene: Option<Res<crate::tonemap::SceneCopy>>,
) {
    // (No tonemapper: the default 1 x 1 image, the linear add.)
    let scene = scene.map(|s| s.0.clone()).unwrap_or_default();
    // The material's own samples (UI_Smoke, Grime): wrapped, as their textures' default address mode.
    let (smoke, grime) = layers
        .get_or_insert_with(|| {
            let wrap = |s: &mut bevy::image::ImageLoaderSettings| {
                use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
                s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::Repeat,
                    address_mode_v: ImageAddressMode::Repeat,
                    mag_filter: ImageFilterMode::Linear,
                    min_filter: ImageFilterMode::Linear,
                    ..default()
                });
            };
            (assets.load_with_settings("SummerCamp/Content/UI/Materials/UI_Smoke.png", wrap), assets.load_with_settings("SummerCamp/Content/UI/Materials/Grime.png", wrap))
        })
        .clone();
    for (e, b, node) in &brushes {
        if !b.is_changed() && node.is_some() {
            continue;
        }
        let c = b.color;
        let color = LinearRgba::new(c[0], c[1], c[2], c[3]);
        let params = Vec4::new(b.brightness, b.smoke, b.grime, 0.0);
        let coords = Vec4::new(b.strech[0], b.strech[1], b.offset[0], b.offset[1]);
        match node {
            Some(n) => {
                if let Some(mut m) = materials.get_mut(&n.0) {
                    m.color = color;
                    m.params = params;
                    m.coords = coords;
                }
            }
            None => {
                let m = materials.add(Additive { color, params, coords, texture: b.texture.clone(), smoke: smoke.clone(), grime: grime.clone(), scene: scene.clone() });
                commands.entity(e).insert(MaterialNode(m));
            }
        }
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
    /// SpectatorHUDWidget: hers once she's out (spectate.rs SpectatorStatus).
    spectator: Option<Widget>,
    breath: Option<Widget>,
    dial: Option<Widget>,
    /// SC_WiggleMinigame_Widget, put in the HUD's OverlayWidget.
    wiggle: Option<Widget>,
    item: Option<Widget>,
    /// Her objectives and map screen (ObjectivesWidget) and the objective rows its FindObjectives adds.
    objectives: Option<Widget>,
    car_objective: Option<Widget>,
    police_objective: Option<Widget>,
    /// His notice as an ability unlocks or rage starts (AbilityAvailable; [`ability_popup`]).
    ability_popup: Option<Widget>,
    /// The Crosshairs software cursor (DefaultEngine.ini's CrosshairsCursor: Morph_Area_Cursor), his Morph map's.
    morph_cursor: Option<Widget>,
    /// The lobby (LobbyWidget) and its player list's rows (LobbyPlayerListItem), with the row's textures and colours.
    lobby: Option<Widget>,
    lobby_row: Option<(Widget, HashMap<String, String>, Value)>,
    /// The counselor menu (Counselor_Menu: lobby.rs CounselorMenu).
    counselor_menu: Option<Widget>,
    /// The audio settings (AudioSettingsMenuWidget: audio_settings.rs AudioMenu).
    audio_settings: Option<Widget>,
    /// The wardrobe (Counselor_Wardrobe_Menu: wardrobe.rs WardrobeMenu) and its lists' buttons (Counselor_Outfit_Button,
    /// Counselor_Swatch_Button).
    wardrobe: Option<Widget>,
    outfit_button: Option<Widget>,
    swatch_button: Option<Widget>,
    /// The emote wheel (Counselor_Emote_Select: emotes.rs EmoteWheel), the counselor menu's emotes (Counselor_Emote_Menu:
    /// emotes.rs EmoteMenu) and its panel's buttons (EmoteButtonLarge).
    emote_wheel: Option<Widget>,
    emote_menu: Option<Widget>,
    emote_button: Option<Widget>,
    /// The perk library (Counselor_Perk_Menu: perks.rs PerkMenu) and its buttons (PerkButtonLarge).
    perk_menu: Option<Widget>,
    perk_button: Option<Widget>,
    /// The map played's definition (ui/map.json: SCMapDefinition's DisplayName and first LobbyThumbnailImages).
    lobby_map: Value,
    /// The match's end (MatchEnd: SCMatchEndLobbyWidget) and its XP lines (MatchEndLine).
    match_end: Option<Widget>,
    match_end_line: Option<Widget>,
    /// The badge screens after it (MatchEnd_Badges), and the badges (ui/badges.json: name, icon, unlock count).
    match_badges: Option<Widget>,
    badges: Value,
    /// The scoreboard after them (ScoreboardWidget), its rows (ScoreboardPlayerListItem), and the characters'
    /// thumbnails (ui/characters.json).
    scoreboard: Option<Widget>,
    scoreboard_row: Option<Widget>,
    characters: Value,
    export: PathBuf,
    classes: ClassDb,
    /// Texture name -> PNG (ui/textures.json).
    textures: HashMap<String, String>,
    regular: Handle<Font>,
    bold: Handle<Font>,
    /// Built for: the window size, her pockets, Jason's HUD, the police timer and the game timer shown, the score
    /// lines shown.
    built: Option<(Vec2, Vec<String>, bool, (bool, bool, bool), usize)>,
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
    // Text widths from the fonts the HUD draws with.
    if let (Ok(regular), Ok(bold)) = (std::fs::read(dir.join("ui/fonts/AlteHaasGrotesk_0_Regular.ttf")), std::fs::read(dir.join("ui/fonts/AlteHaasGrotesk_0_Bold.ttf"))) {
        f13_data::umg::set_fonts(&regular, &bold);
    }
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
        // Shown by things not ported; the help text never shows (see the module docs). (RageBorder, at its alpha 0.005
        // until rage plays RageActive on it, stays: rage_hud.)
        for name in [
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
        wiggle: Widget::load(dir, "SC_WiggleMinigame_Widget").map(|mut w| {
            action_keys(&mut w);
            w
        }),
        item: Widget::load(dir, "SC_InventoryItem_Widget"),
        objectives: Widget::load(dir, "ObjectivesWidget"),
        car_objective: Widget::load(dir, "Objective_Car_Widget"),
        police_objective: Widget::load(dir, "Objective_Police_Widget"),
        ability_popup: Widget::load(dir, "AbilityAvailable"),
        morph_cursor: Widget::load(dir, "Morph_Area_Cursor"),
        lobby: Widget::load(dir, "LobbyWidget").map(|mut w| {
            // Shown by things not ported (the party, chat and lag icons, the map choice and the custom match).
            for name in ["HUDLaggingIconWidget", "SaveGameThreadActiveIcon", "PlayerCardRightWidget", "ChatNotificationOverlayWidget_180", "LobbyPrevMapArrowButtonWidget", "LobbyNextMapArrowButtonWidget", "CustomMatchButton", "CustomServerSettings", "MenuActionBarWidget", "MapFocusHighlight", "MapFocusBorderImage"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            action_keys(&mut w);
            w
        }),
        counselor_menu: Widget::load(dir, "Counselor_Menu").map(|mut w| {
            // Not ported: its 3D preview (CharacterPreviewImage: CounselorCustomizationPreview's render target), perks,
            // ability, clothing and emote buttons, customization points, the locked banner (all unlocked) and its action
            // bar (the keys: lobby.rs).
            // Its header's title (MenuHeaderWidget's construct: TitleText from its Text, "Counselor").
            if let Some(h) = w.find_mut("MenuHeaderWidget") {
                let title = h.props["Text"].clone();
                if let Some(t) = h.find_mut("TitleText").filter(|_| title.is_object()) {
                    t.props["Text"] = title;
                }
            }
            for name in ["AbilityButton", "ClothingButton", "ClothesBorder", "CounselorEmoteButton", "EmotesBorder", "CPBox", "LockedBorder", "ActionBarWidget", "QuickPlayOverlayWidget"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            action_keys(&mut w);
            w
        }),
        audio_settings: Widget::load(dir, "AudioSettingsMenuWidget").map(|mut w| {
            // Not ported: its party status, quick play overlay, save icon and action bar (the keys: audio_settings.rs).
            // Its header's title (MenuHeaderWidget's construct: TitleText from its Text, "Audio Settings").
            if let Some(h) = w.find_mut("MenuHeaderWidget") {
                let title = h.props["Text"].clone();
                if let Some(t) = h.find_mut("TitleText").filter(|_| title.is_object()) {
                    t.props["Text"] = title;
                }
            }
            for name in ["SaveGameThreadActiveIcon_0", "QuickPlayOverlayWidget", "PartyStatusWidget", "MenuActionBarWidget"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            w
        }),
        wardrobe: Widget::load(dir, "Counselor_Wardrobe_Menu").map(|mut w| {
            // Its header's title (MenuHeaderWidget: TitleText its Text, "Clothing").
            if let Some(h) = w.find_mut("MenuHeaderWidget") {
                let title = h.props["Text"].clone();
                if let Some(t) = h.find_mut("TitleText").filter(|_| title.is_object()) {
                    t.props["Text"] = title;
                }
            }
            // Not ported: its action bar and gamepad image (the keys: wardrobe.rs).
            for name in ["ActionBarWidget", "ControllerImage"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            w
        }),
        outfit_button: Widget::load(dir, "Counselor_Outfit_Button"),
        emote_wheel: Widget::load(dir, "Counselor_Emote_Select"),
        emote_menu: Widget::load(dir, "Counselor_Emote_Menu").map(|mut w| {
            // Its header's title (MenuHeaderWidget: TitleText its Text, "Emotes").
            if let Some(h) = w.find_mut("MenuHeaderWidget") {
                let title = h.props["Text"].clone();
                if let Some(t) = h.find_mut("TitleText").filter(|_| title.is_object()) {
                    t.props["Text"] = title;
                }
            }
            // Not ported: its action bar ("More Emotes": the store) and gamepad image (the keys: emotes.rs).
            for name in ["ActionBarWidget", "ControllerImage"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            w
        }),
        emote_button: Widget::load(dir, "EmoteButtonLarge"),
        perk_menu: Widget::load(dir, "Counselor_Perk_Menu").map(|mut w| {
            // Its header's title (MenuHeaderWidget: TitleText its Text, "Perk Select").
            if let Some(h) = w.find_mut("MenuHeaderWidget") {
                let title = h.props["Text"].clone();
                if let Some(t) = h.find_mut("TitleText").filter(|_| title.is_object()) {
                    t.props["Text"] = title;
                }
            }
            // Not ported: rolling (PerkRollButton), customization points (CPBox), selling back (its hint), rarity and the
            // legendary modifier, its action bar (the keys: perks.rs).
            for name in ["ActionBarWidget", "PerkRollButton", "CPBox", "TextBlock_0", "Legendary"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            w
        }),
        perk_button: Widget::load(dir, "PerkButtonLarge"),
        swatch_button: Widget::load(dir, "Counselor_Swatch_Button"),
        lobby_row: Widget::load(dir, "LobbyPlayerListItem").map(|w| {
            let json = |n: &str| std::fs::read_to_string(dir.join(format!("ui/{n}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_default();
            let (v, speaker, ping) = (json("LobbyPlayerListItem"), json("PlayerSpeakIconMuteButton"), json("PlayerPingDisplayWidget"));
            // Its own textures and its speaker's (Img_Speaker_On_01 / _Off_01: Get_SpeakingIcon_Brush).
            let textures = [&v, &speaker].iter().flat_map(|j| j["Textures"].as_object().into_iter().flatten()).filter_map(|(k, f)| Some((k.clone(), f.as_str()?.to_string()))).collect();
            // Its colours, and its ping's (PingGoodColor, PingBadColor, PingVeryBadColor).
            let mut defaults = v["Defaults"].clone();
            for k in ["PingGoodColor", "PingBadColor", "PingVeryBadColor"] {
                defaults[k] = ping["Defaults"][k]["SpecifiedColor"].clone();
            }
            (w, textures, defaults)
        }),
        lobby_map: std::fs::read_to_string(dir.join("ui/map.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default(),
        match_end: Widget::load(dir, "MatchEnd").map(|mut w| {
            // Not ported: the player card, the Continue button (the screen closes as the match does), the level up.
            for name in ["PlayerCardRightWidget", "MenuActionBarWidget_930", "LevelUp"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            w
        }),
        match_end_line: Widget::load(dir, "MatchEndLine"),
        match_badges: Widget::load(dir, "MatchEnd_Badges").map(|mut w| {
            for name in ["PlayerCardRightWidget", "MenuActionBarWidget_930"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            w
        }),
        badges: std::fs::read_to_string(dir.join("ui/badges.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default(),
        scoreboard: Widget::load(dir, "ScoreboardWidget"),
        scoreboard_row: Widget::load(dir, "ScoreboardPlayerListItem"),
        characters: std::fs::read_to_string(dir.join("ui/characters.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default(),
        spectator: Widget::load(dir, "SpectatorHUDWidget").map(|mut w| {
            action_keys(&mut w);
            w
        }),
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
        anims: ["SCCounselorTimerWidget", "SCBreathWidget", "KillerHUDWidget", "ObjectivesWidget", "KillerAbilityWidget", "AbilityAvailable", "RageWidget", "YouDied", "YouSurvived", "SpectatorHUDWidget", "LobbyWidget", "MatchEnd", "MatchEnd_Badges", "Counselor_Menu"].iter().map(|c| (c.to_string(), load_anims(dir, c))).collect(),
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

impl Hud {
    /// SpectatorHUDWidget was exported.
    pub fn has_spectator(&self) -> bool {
        self.spectator.is_some()
    }

    /// Whether the match's end shows as the game's widget (MatchEnd).
    pub fn has_match_end(&self) -> bool {
        self.match_end.is_some()
    }
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
    spectating: Option<Res<crate::spectate::SpectatorStatus>>,
    (lobby, me, client, mut playing, time): (Query<&crate::lobby::NetLobby>, Option<Res<crate::lobby::Me>>, Option<Res<crate::net::NetClient>>, ResMut<PlayingAnims>, Res<Time>),
    (match_tuning, mutes, page, names, ready_focus): (Option<Res<crate::match_flow::MatchTuning>>, Option<Res<crate::voice::VoiceMutes>>, Res<MatchEndPage>, Query<&Name>, Option<Res<crate::lobby::ReadyFocus>>),
    (counselor_menu, counselor_list, counselors, player_tuning, audio_menu, preview_image): (
        Option<Res<crate::lobby::CounselorMenu>>,
        Option<Res<crate::lobby::CounselorList>>,
        Option<Res<crate::character::CounselorCharacter>>,
        Res<crate::player::PlayerTuning>,
        Option<Res<crate::audio_settings::AudioMenu>>,
        Option<Res<crate::counselor_preview::PreviewImage>>,
    ),
    (wardrobe_menu, emote_wheel, emotes, emote_menu): (Option<Res<crate::wardrobe::WardrobeMenu>>, Option<Res<crate::emotes::EmoteWheel>>, Option<Res<crate::emotes::Emotes>>, Option<Res<crate::emotes::EmoteMenu>>),
    (perk_menu, perks, profiles): (Option<Res<crate::perks::PerkMenu>>, Option<Res<crate::perks::Perks>>, Option<Res<crate::wardrobe::CounselorProfiles>>),
) {
    let size = window.size();
    // The lobby while it's open (LobbyWidget, as the game's lobby map shows it).
    let open_lobby = lobby.iter().next().filter(|l| l.open && hud.lobby.is_some());
    let my_id = if client.is_some() { me.as_ref().map(|m| m.id) } else { Some(0) };
    let jason = play.is_some() && hud.killer.is_some();
    // Out of the match (dead or escaped): her spectator HUD instead (the game's HUD swaps to it as she spectates).
    let spectator = !jason && hud.spectator.is_some() && spectating.as_ref().is_some_and(|s| s.out);
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
    let mut pocket_key: Vec<String> = pockets.iter().cloned().chain([format!("#{selected}")]).collect();
    if let Some(w) = &emote_wheel {
        pocket_key.push(format!("wheel:{:?}", w.section));
    }
    // The spectator HUD's next-mode title is set as it's built (its ActionDisplays share their widgets' names).
    let next_mode = spectating.as_ref().map_or("", |s| s.next);
    if spectator {
        pocket_key = vec![format!("spectator:{next_mode}")];
    }
    // The match over: its XP by category (MatchEnd), in place of the HUD.
    let ended = game.as_ref().filter(|g| matches!(g.state.state, f13_core::game_mode::MatchState::WaitingPostMatch | f13_core::game_mode::MatchState::LeavingMap) && hud.match_end.is_some());
    let end_lines = ended.map(|g| {
        let awards = if jason { g.killer_score.awards.clone() } else { g.mine().map_or(vec![], |t| t.score.awards.clone()) };
        match_end_lines(&awards, match_tuning.as_deref())
    });
    // Past it (Continue): a badge earned a screen (OnDisplayUnlock -> MatchEnd_Badges' ShowBadgeUnlock, one at a time).
    let earned = ended.map_or(0, |g| if jason { g.killer_badges.len() } else { g.mine().map_or(0, |t| t.badges.len()) });
    let badge = ended.and_then(|g| {
        let earned = if jason { g.killer_badges.clone() } else { g.mine().map_or(vec![], |t| t.badges.clone()) };
        earned.get(page.0.checked_sub(1)?).cloned()
    }).filter(|_| hud.match_badges.is_some());
    // Then the scoreboard (OnShowScoreboard): everyone, how their match ended.
    let board = ended.filter(|_| page.0 > earned && hud.scoreboard.is_some()).map(|g| scoreboard_rows(g, &names));
    if let (Some(g), Some(lines)) = (ended, &end_lines) {
        pocket_key = lines.iter().map(|(c, xp)| format!("matchend:{c}:{xp}")).collect();
        pocket_key.insert(0, "matchend:".into());
        pocket_key.push(format!("closing:{}", g.state.remaining_time));
        if let Some(b) = &badge {
            pocket_key.insert(0, format!("badge:{b}"));
        }
        if let Some(rows) = &board {
            pocket_key.insert(0, format!("board:{rows:?}"));
        }
    }
    // The counselor menu over the lobby while it's open.
    let menu_class = open_lobby.and(counselor_menu.as_ref()).and_then(|m| counselor_list.as_ref()?.0.get(m.index).cloned()).filter(|_| hud.counselor_menu.is_some());
    if let Some(l) = open_lobby {
        pocket_key = l.members.iter().map(|m| format!("lobby:{}:{}:{}:{:?}:{}:{}:{}:{}:{}", m.id, m.name, m.ready, m.preference, m.ping, m.talked, m.talking, mutes.as_ref().is_some_and(|x| x.0.contains(&m.name)), m.class)).collect();
        pocket_key.push(format!("ready focus {}", ready_focus.as_ref().is_none_or(|f| f.0)));
        pocket_key.push(format!("countdown:{:?}", l.countdown.map(|t| t.ceil() as i32)));
        if let Some(c) = &menu_class {
            let slots = profiles.as_ref().map(|p| p.perks.get(c).cloned().unwrap_or_default());
            pocket_key = vec![format!("counselor_menu:{c}:{slots:?}")];
        }
        if let Some(m) = perk_menu.as_ref().filter(|_| hud.perk_menu.is_some()) {
            pocket_key = vec![format!("perks:{}:{}:{}", m.class, m.slot, m.index)];
        }
        if let Some(m) = emote_menu.as_ref().filter(|_| hud.emote_menu.is_some()) {
            pocket_key = vec![format!("emotes:{}:{:?}:{}:{:?}", m.class, m.slots, m.section, m.list)];
        }
        if let Some(w) = wardrobe_menu.as_ref().filter(|_| hud.wardrobe.is_some()) {
            pocket_key = vec![format!("wardrobe:{}:{}:{:?}:{}", w.class, w.outfit, w.picks, w.row)];
        }
        if let Some(m) = audio_menu.as_ref().filter(|_| hud.audio_settings.is_some()) {
            pocket_key = vec![format!("audio_settings:{}:{:?}", m.focus, m.edit)];
        }
    }
    // His score lines on his HUD, hers on hers.
    let score_lines = |g: &crate::match_flow::GameMatch| if jason { g.killer_score_lines().to_vec() } else { g.score_lines().to_vec() };
    let scores = game.as_ref().map_or(vec![], |g| score_lines(g));
    let lines = scores.len();
    if hud.built.as_ref().is_some_and(|(s, p, j, c, l)| *s == size && *p == pocket_key && *j == jason && *c == (police, game_timer, spectator) && *l == lines) {
        return;
    }
    if let Some(root) = hud.root.take() {
        commands.entity(root).despawn();
    }
    // The window in design units: scaled to fit 1920×1080, the rest added along the longer side.
    let scale = (size.x / DESIGN.x).min(size.y / DESIGN.y);
    let virt = size / scale;
    let mut tree = if let Some(rows) = &board {
        let mut t = hud.scoreboard.clone().unwrap_or_else(|| hud.hud.clone());
        fill_scoreboard(&hud, &mut t, rows);
        t
    } else if let (Some(g), Some(b)) = (ended, &badge) {
        if !hud.built.as_ref().is_some_and(|built| built.1.first() == pocket_key.first()) {
            playing.play("MatchEnd_Badges", "AnimIn", time.elapsed_secs(), false);
        }
        let mut t = hud.match_badges.clone().unwrap_or_else(|| hud.hud.clone());
        fill_badge(&hud, &mut t, b, g.state.remaining_time);
        t
    } else if let (Some(g), Some(lines)) = (ended, &end_lines) {
        // AnimIn as it's shown, the total's TotalAnimIn after the lines (each line DelayIndex * 0.2 s. PLACEHOLDER:
        // the wait before the lines, 1 s: OnUpdateScoreEvents' Delay; DisplayPlayerScoreDelay not read).
        if !hud.built.as_ref().is_some_and(|b| b.1.first().is_some_and(|k| k.starts_with("matchend:"))) {
            let now = time.elapsed_secs();
            playing.play("MatchEnd", "AnimIn", now, false);
            playing.play("MatchEnd", "TotalAnimIn", now + 1.0 + 0.2 * lines.len() as f32, false);
        }
        let mut t = hud.match_end.clone().unwrap_or_else(|| hud.hud.clone());
        fill_match_end(&hud, &mut t, lines, g.state.remaining_time);
        t
    } else if let (Some(m), Some(t), Some(_)) = (&audio_menu, &hud.audio_settings, open_lobby) {
        let mut t = t.clone();
        fill_audio_settings(&mut t, m);
        t
    } else if let (Some(m), Some(t), Some(_), Some(p), Some(pr)) = (&perk_menu, &hud.perk_menu, open_lobby, &perks, &profiles) {
        let mut t = t.clone();
        fill_perk_menu(&hud, &mut t, m, p, pr, counselors.as_deref());
        t
    } else if let (Some(m), Some(t), Some(_), Some(e)) = (&emote_menu, &hud.emote_menu, open_lobby, &emotes) {
        let mut t = t.clone();
        fill_emote_menu(&hud, &mut t, m, e, counselors.as_deref());
        t
    } else if let (Some(w), Some(t), Some(_), Some(c)) = (&wardrobe_menu, &hud.wardrobe, open_lobby, counselors.as_deref()) {
        let mut t = t.clone();
        fill_wardrobe(&hud, &mut t, w, c);
        t
    } else if let (Some(class), Some(_)) = (&menu_class, open_lobby) {
        let mut t = hud.counselor_menu.clone().unwrap_or_else(|| hud.hud.clone());
        fill_counselor_menu(&mut t, class, counselors.as_deref(), &player_tuning);
        fill_perk_slots(&mut t, class, perks.as_deref(), profiles.as_deref());
        t
    } else if let Some(l) = open_lobby {
        // Its contents fade in (AnimIn: LobbyContentsBorder's and LobbyDetailsBorder's content alpha, 0 as built).
        // PLACEHOLDER: played as it opens (the blueprint's caller not read).
        if !hud.built.as_ref().is_some_and(|b| b.1.first().is_some_and(|k| k.starts_with("lobby:"))) {
            playing.play("LobbyWidget", "AnimIn", time.elapsed_secs(), false);
        }
        let mut t = hud.lobby.clone().unwrap_or_else(|| hud.hud.clone());
        fill_lobby(&hud, &mut t, l, my_id, mutes.as_deref(), ready_focus.as_ref().is_none_or(|f| f.0));
        t
    } else if jason {
        hud.killer.clone().unwrap_or_else(|| hud.hud.clone())
    } else if spectator {
        hud.spectator.clone().unwrap_or_else(|| hud.hud.clone())
    } else {
        hud.hud.clone()
    };
    if spectator && !next_mode.is_empty() {
        if let Some(t) = tree.find_mut("SpectateModeDisplay").and_then(|d| d.find_mut("TitleText")) {
            t.props["Text"] = json!({ "LocalizedString": next_mode });
        }
    }
    if !jason && !spectator && open_lobby.is_none() && ended.is_none() {
        fill_inventory(&hud, &mut tree, &pockets, selected);
        // The emote wheel over it (SCInGameHUD's EmoteWidgetInstance, AddToViewport ZOrder 10), centred.
        if let (Some(w), Some(mut wheel), Some(emotes)) = (&emote_wheel, hud.emote_wheel.clone(), &emotes) {
            fill_emote_wheel(&mut wheel, &w.slots, w.section, emotes);
            tree.children.push(Widget {
                kind: "Counselor_Emote_Select_C".into(),
                name: "EmoteWheel".into(),
                class: Some("Counselor_Emote_Select".into()),
                props: json!({}),
                slot_kind: "CanvasPanelSlot".into(),
                slot: json!({ "LayoutData": { "Anchors": { "Minimum": { "X": 0.0, "Y": 0.0 }, "Maximum": { "X": 1.0, "Y": 1.0 } }, "Offsets": { "Left": 0.0, "Top": 0.0, "Right": 0.0, "Bottom": 0.0 } }, "ZOrder": 10 }),
                children: vec![wheel],
            });
        }
    }
    if open_lobby.is_none() && ended.is_none() {
        fill_xp_lines(&hud, &mut tree, &scores);
    }
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
    // OnStartWiggleWidget's: SC_WiggleMinigame_Widget in OverlayWidget (both HUDs), shown while a minigame runs.
    if let (Some(w), Some(overlay)) = (hud.wiggle.clone(), tree.find_mut("OverlayWidget")) {
        let mut node = Widget {
            kind: "SC_WiggleMinigame_Widget_C".into(),
            name: WIGGLE.into(),
            class: Some("SC_WiggleMinigame_Widget".into()),
            props: json!({}),
            slot_kind: "OverlaySlot".into(),
            slot: json!({}),
            children: vec![w],
        };
        node.set_visibility(Vis::Hidden);
        overlay.children.push(node);
    }
    let root = commands.spawn((Node { position_type: PositionType::Absolute, width: Val::Percent(100.0), height: Val::Percent(100.0), ..default() }, Pickable::IGNORE)).id();
    let mut names = HashMap::new();
    let preview = preview_image.map(|p| (p.counselor.clone(), p.wardrobe.clone(), p.emote.clone()));
    let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, content: [1.0; 4], names: &mut names, preview };
    let full = Rect { x: 0.0, y: 0.0, w: virt.x, h: virt.y };
    let laid = tree.layout(full);
    b.spawn(&laid, root, full, if board.is_some() { "ScoreboardWidget" } else if badge.is_some() { "MatchEnd_Badges" } else if ended.is_some() { "MatchEnd" } else if menu_class.is_some() && open_lobby.is_some() { "Counselor_Menu" } else if open_lobby.is_some() { "LobbyWidget" } else if jason { "KillerHUDWidget" } else if spectator { "SpectatorHUDWidget" } else { "CounselorHUDWidget" });
    // Her breath meter and interaction dial are hers (Jason's HUD has neither; nor has the lobby).
    if let Some(breath) = hud.breath.as_ref().filter(|_| !jason && open_lobby.is_none() && ended.is_none()) {
        let laid = breath.layout(full);
        let e = b.spawn(&laid, root, full, "SCBreathWidget");
        b.commands.entity(e).insert(Visibility::Hidden);
        b.names.insert("SCBreathWidget".into(), e);
    }
    if let Some(dial) = hud.dial.as_ref().filter(|_| !jason && open_lobby.is_none() && ended.is_none()) {
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
    hud.built = Some((size, pocket_key, jason, (police, game_timer, spectator), lines));
}

/// The match's XP by category, in the order first scored (SCMatchEndLobbyWidget DisplayPlayerScore 0x1404234e0: a line
/// per categorized score event, its category's name (0x140448fc0) and its total. PLACEHOLDER: the order, the player
/// state's categorized events' own not read). Events without a category (None) aren't shown.
fn match_end_lines(awards: &[f13_core::game_mode::Award], tuning: Option<&crate::match_flow::MatchTuning>) -> Vec<(String, i32)> {
    let mut lines: Vec<(String, i32)> = vec![];
    for a in awards {
        let category = tuning.and_then(|t| t.events.get(&a.event)).map_or("None", |e| e.category.as_str());
        let Some(name) = category_name(category) else { continue };
        match lines.iter_mut().find(|(n, _)| n == name) {
            Some(l) => l.1 += a.xp,
            None => lines.push((name.to_string(), a.xp)),
        }
    }
    lines
}

/// EScoreEventCategory's display names (0x140448fc0).
fn category_name(category: &str) -> Option<&'static str> {
    Some(match category {
        "Offense" => "Offense",
        "Defense" => "Defense",
        "Betrayal" => "Betrayal",
        "Fixer" => "Fixer",
        "Escaped" => "Escaped",
        "TeamEscaped" => "Team Escaped",
        "Reinforcements" => "Reinforcements",
        "CompletedMatch" => "Completed Match",
        "TimeBonus" => "Time Bonus",
        "Demasking" => "Demasking",
        "Kill" => "Kill",
        "NoSurvivors" => "No Survivors",
        "Resourceful" => "Resourceful",
        "Versatile" => "Versatile",
        "Seeker" => "Seeker",
        "DemolitionMan" => "Demolition Man",
        "BoatKill" => "Boat Kill",
        "Butcher" => "Butcher",
        "Trapper" => "Trapper",
        "PartyCrasher" => "Party Crasher",
        "DefeatedJason" => "Defeated Jason!",
        "Escort" => "Escort",
        "Sacrifice" => "Sacrifice",
        _ => return None,
    })
}

/// A lobby row's speaker is its mute button (PlayerSpeakIconMuteButton's MuteButton: a click mutes or unmutes that
/// player here, not for this player's own row; CanBeMuted). Its rows' speakers take clicks (Interaction).
fn lobby_mute_clicks(
    mut commands: Commands,
    speakers: Query<(Entity, &Name), (Without<Interaction>, With<Node>)>,
    clicked: Query<(Entity, &Interaction, &Name), Changed<Interaction>>,
    parents: Query<&ChildOf>,
    names: Query<&Name>,
    lobby: Query<&crate::lobby::NetLobby>,
    (me, client): (Option<Res<crate::lobby::Me>>, Option<Res<crate::net::NetClient>>),
    mutes: Option<ResMut<crate::voice::VoiceMutes>>,
) {
    for (e, n) in &speakers {
        if n.as_str() == "PlayerSpeakIconMuteButton" {
            commands.entity(e).try_insert(Interaction::None);
        }
    }
    let (Some(l), Some(mut mutes)) = (lobby.iter().next().filter(|l| l.open), mutes) else { return };
    let my_id = if client.is_some() { me.map(|m| m.id) } else { Some(0) };
    for (e, i, n) in &clicked {
        if *i != Interaction::Pressed || n.as_str() != "PlayerSpeakIconMuteButton" {
            continue;
        }
        // Its row: LobbyRow<i> among its ancestors.
        let row = parents.iter_ancestors(e).find_map(|a| names.get(a).ok()?.as_str().strip_prefix("LobbyRow")?.parse::<usize>().ok());
        let Some(m) = row.and_then(|r| l.members.get(r)).filter(|m| Some(m.id) != my_id) else { continue };
        if !mutes.0.remove(&m.name) {
            mutes.0.insert(m.name.clone());
        }
        info!("lobby: {} {}", m.name, if mutes.0.contains(&m.name) { "muted" } else { "unmuted" });
    }
}

/// The match end's page: 0 its XP, then a badge each (Continue: MenuSelect, Enter here), back to 0 for the next match.
#[derive(Resource, Default)]
pub struct MatchEndPage(pub usize);

fn match_end_page(mut page: ResMut<MatchEndPage>, keys: Res<ButtonInput<KeyCode>>, game: Option<Res<crate::match_flow::GameMatch>>) {
    let ended = game.as_ref().is_some_and(|g| matches!(g.state.state, f13_core::game_mode::MatchState::WaitingPostMatch | f13_core::game_mode::MatchState::LeavingMap));
    if !ended {
        if page.0 != 0 {
            page.0 = 0;
        }
    } else if keys.just_pressed(KeyCode::Enter) {
        page.0 += 1;
    }
}

/// A scoreboard row: their name, their status text, whether they're Jason and whether they're dead.
type ScoreRow = (String, String, bool, bool);

/// The scoreboard's players: each counselor (her name and SCPlayerListItemWidget GetStatusDisplayText 0x14042d630:
/// "Escaped", her death's DeathScoreboardDisplayText, "Alive"), then Jason ("Killed {0}/{1}": his kills of the
/// counselors). PLACEHOLDERS: the order (the list's own not read), a death's text ("Dead"), Jason's name.
fn scoreboard_rows(g: &crate::match_flow::GameMatch, names: &Query<&Name>) -> Vec<ScoreRow> {
    let mut rows: Vec<ScoreRow> = g
        .counselors
        .iter()
        .map(|(e, t)| {
            let name = names.get(*e).map_or("Counselor".to_string(), |n| n.to_string());
            let status = if t.record.escaped { "Escaped" } else if t.record.dead { "Dead" } else { "Alive" };
            (name, status.to_string(), false, t.record.dead && !t.record.escaped)
        })
        .collect();
    let kills = g.counselors.values().filter(|t| t.record.dead && !t.record.escaped).count();
    rows.push(("Jason".to_string(), format!("Killed {kills}/{}", g.counselors.len()), true, false));
    rows
}

/// ScoreboardWidget filled: a ScoreboardPlayerListItem per player in its ScoreBoardHUDWidget's PlayerVerticalBox,
/// each its AliveDeadPanel, its picked character (the character's ThumbnailImage; its border red for the dead and
/// Jason, else (0.552, 0.527, 0.434, 0.5); the image red for the dead), its name and its status.
fn fill_scoreboard(hud: &Hud, tree: &mut Widget, rows: &[ScoreRow]) {
    // Its header (MenuSmallHeaderWidget: TitleText its Text, TitleBar gone with RemoveLine).
    if let Some(h) = tree.find_mut("MenuSmallHeaderWidget") {
        let (text, remove_line) = (h.props["Text"].clone(), h.props["RemoveLine"].as_bool().unwrap_or(false));
        if let Some(t) = h.find_mut("TitleText") {
            t.props["Text"] = text;
        }
        if let Some(bar) = h.find_mut("TitleBar").filter(|_| remove_line) {
            bar.set_visibility(Vis::Collapsed);
        }
    }
    let Some(row) = &hud.scoreboard_row else { return };
    let Some(list) = tree.find_mut("PlayerVerticalBox") else { return };
    list.children.clear();
    let red = json!({ "R": 0.854993, "G": 0.030713, "B": 0.017642, "A": 1.0 });
    for (i, (name, status, killer, dead)) in rows.iter().enumerate() {
        let mut w = row.clone();
        w.name = format!("ScoreboardRow{i}");
        w.slot_kind = "VerticalBoxSlot".into();
        w.slot = json!({});
        for (n, shown) in [("AliveDeadPanel", true), ("PickedCharacterPanel", true), ("PlayerStatusPanel", true), ("SpectatingPanel", false)] {
            if let Some(x) = w.find_mut(n) {
                x.set_visibility(if shown { Vis::Visible } else { Vis::Collapsed });
            }
        }
        if let (Some(i), Some(file)) = (w.find_mut("CharacterImage"), hud.characters[if *killer { "Killer" } else { "Counselor" }]["Thumbnail"].as_str()) {
            i.props["Brush"] = json!({ "ImageSize": { "X": 32.0, "Y": 32.0 }, "ResourceObject": { "File": file } });
            i.props["ColorAndOpacity"] = if *dead { red.clone() } else { json!({ "R": 1.0, "G": 1.0, "B": 1.0, "A": 1.0 }) };
        }
        if let Some(b) = w.find_mut("CharacterBorderImg") {
            b.props["ColorAndOpacity"] = if *dead || *killer { red.clone() } else { json!({ "R": 0.552012, "G": 0.527115, "B": 0.434154, "A": 0.5 }) };
        }
        if let Some(t) = w.find_mut("PlayerNameText") {
            t.props["Text"] = json!({ "LocalizedString": name });
        }
        if let Some(t) = w.find_mut("StatusText") {
            t.props["Text"] = json!({ "LocalizedString": status });
        }
        if let Some(t) = w.find_mut("PingText") {
            t.props["Text"] = json!({ "LocalizedString": "" });
        }
        list.children.push(w);
    }
}

/// MatchEnd_Badges filled with a badge (GetBadgeInfo at star level 0: its DisplayName, one-star icon and unlock
/// count; BadgeProgress "{c}/{t}", both its unlock count as the blueprint passes them), and the time until the lobby.
fn fill_badge(hud: &Hud, tree: &mut Widget, badge: &str, closing: i32) {
    let b = &hud.badges[badge];
    if let Some(t) = tree.find_mut("BadgeName") {
        t.props["Text"] = json!({ "LocalizedString": b["Name"].as_str().unwrap_or(badge) });
    }
    if let (Some(t), Some(n)) = (tree.find_mut("BadgeProgress"), b["Unlock"].as_i64()) {
        t.props["Text"] = json!({ "LocalizedString": format!("{n}/{n}") });
    }
    if let (Some(i), Some(file)) = (tree.find_mut("BadgeImage"), b["Icon"].as_str()) {
        i.props["Brush"]["ResourceObject"] = json!({ "File": file });
    }
    if let Some(t) = tree.find_mut("TextBlock_358") {
        let c = closing.max(0);
        t.props["Text"] = json!({ "LocalizedString": format!("{:02}:{:02}", c / 60, c % 60) });
    }
}

/// MatchEnd filled: a MatchEndLine per category in ExperienceBox (OnAddMatchEndLine: CatagoryText, "{Num} xp"), the
/// total ("{0} xp": ShowTotal 0x140444550), and the time until the lobby (GetText_0: the travel countdown, mm:ss).
fn fill_match_end(hud: &Hud, tree: &mut Widget, lines: &[(String, i32)], closing: i32) {
    let total: i32 = lines.iter().map(|l| l.1).sum();
    if let Some(t) = tree.find_mut("TotalXPText") {
        t.props["Text"] = json!({ "LocalizedString": format!("{total} xp") });
    }
    if let Some(t) = tree.find_mut("TextBlock_358") {
        let c = closing.max(0);
        t.props["Text"] = json!({ "LocalizedString": format!("{:02}:{:02}", c / 60, c % 60) });
    }
    let Some(row) = &hud.match_end_line else { return };
    let Some(list) = tree.find_mut("ExperienceBox") else { return };
    list.children.clear();
    for (i, (category, xp)) in lines.iter().enumerate() {
        let mut w = row.clone();
        w.name = format!("MatchEndLine{i}");
        w.slot_kind = "VerticalBoxSlot".into();
        w.slot = json!({});
        if let Some(t) = w.find_mut("Catagory") {
            t.props["Text"] = json!({ "LocalizedString": category });
        }
        if let Some(t) = w.find_mut("XPAmount") {
            t.props["Text"] = json!({ "LocalizedString": format!("{xp} xp") });
        }
        list.children.push(w);
    }
}

/// The lobby's player list: a LobbyPlayerListItem per member in LobbyPlayerListWidget's PlayerVerticalBox (its
/// AddPlayerWidget), each its name (PlayerName), its tint (GetPlayerTint: LocalPlayerColor for this player, else
/// RemoteUnreadyPlayerColor to RemoteReadyPlayerColor by bIsReady), its ready box (Get_ReadyBoxImage_Brush:
/// Img_Lobby_ReadyBox_Checked_01 or _Empty_01) and the host's arrow; and the travel countdown (CountdownHorizontalBox,
/// CountdownText); each row's ping and speaker. Not ported: the mute button (Shift + its number mutes here), the
/// connecting spinner, the host's Jason pick.
/// AudioSettingsMenuWidget's options (SettingsOptionSliderWidget, SettingsOptionBooleanWidget: decompiled) as the menu
/// edits them: each TitleText its OptionTitle; a slider's ValueSlider at its value and ValueText "{value x 100}%"
/// (ValueDisplayPercentage, ValueDisplayMax 100, 0 digits), a boolean's ValueText "Yes" / "No" (OnOff unset: its arrows
/// hidden). The focused row (OnSliderFocus / OnButtonFocus) shows its FocusBar, its title in HoverFont (Bold, 24) and
/// HoverColor, and a slider's ValueText (ValueTextFade's end, alpha 0.5: hidden otherwise). Not ported: the focus
/// animations (FocusPulse, FocusBurst, FocusFadeOut) and the fades (their end states shown).
fn fill_audio_settings(tree: &mut Widget, menu: &crate::audio_settings::AudioMenu) {
    use crate::audio_settings::{Field, ROWS};
    let text = |t: &str| json!({ "LocalizedString": t });
    for (i, (name, field)) in ROWS.iter().enumerate() {
        let Some(x) = tree.find_mut(name) else { continue };
        let focused = i == menu.focus;
        let title = x.props["OptionTitle"].clone();
        let hover = x.props["HoverColor"]["SpecifiedColor"].clone();
        if let Some(t) = x.find_mut("TitleText") {
            if title.is_object() {
                t.props["Text"] = title;
            }
            if focused {
                t.props["Font"]["TypefaceFontName"] = json!("Bold");
                t.props["Font"]["Size"] = json!(24);
                if hover.is_object() {
                    t.props["ColorAndOpacity"]["SpecifiedColor"] = hover;
                }
            }
        }
        if let Some(b) = x.find_mut("FocusBar") {
            b.set_visibility(if focused { Vis::Visible } else { Vis::Hidden });
        }
        match *field {
            Field::Volume(v) => {
                let value = menu.edit.volume(v);
                if let Some(s) = x.find_mut("ValueSlider") {
                    s.props["Value"] = json!(value);
                }
                if let Some(t) = x.find_mut("ValueText") {
                    t.props["Text"] = text(&format!("{}%", (value * 100.0).round() as i32));
                    t.props["ColorAndOpacity"]["SpecifiedColor"]["A"] = json!(if focused { 0.5 } else { 0.0 });
                }
            }
            Field::Flag(f) => {
                if let Some(t) = x.find_mut("ValueText") {
                    t.props["Text"] = text(if menu.edit.flag(f) { "Yes" } else { "No" });
                }
                for arrow in ["LeftButton", "RightButton"] {
                    if let Some(a) = x.find_mut(arrow) {
                        a.set_visibility(Vis::Collapsed);
                    }
                }
            }
        }
    }
}

/// Counselor_Menu's PerkButton_Slot_1..3 (PerkButton: SetPerk / ClearPerk): each slot's perk's LoadoutIcon (PerkImage
/// shown), else the empty slot (PerkImage hidden); HighlightImg (its focus) hidden.
fn fill_perk_slots(tree: &mut Widget, class: &str, perks: Option<&crate::perks::Perks>, profiles: Option<&crate::wardrobe::CounselorProfiles>) {
    let slots = profiles.and_then(|p| p.perks.get(class)).cloned().unwrap_or_default();
    for i in 0..3 {
        let Some(b) = tree.find_mut(&format!("PerkButton_Slot_{}", i + 1)) else { continue };
        let icon = slots.get(i).and_then(|p| perks?.0.get(p)).and_then(|d| d.icon.clone());
        if let Some(img) = b.find_mut("PerkImage") {
            match icon {
                Some(icon) => {
                    img.props["Brush"]["ResourceObject"] = json!({ "File": format!("{icon}.png") });
                    img.set_visibility(Vis::Visible);
                }
                None => img.set_visibility(Vis::Hidden),
            }
        }
        if let Some(h) = b.find_mut("HighlightImg") {
            h.set_visibility(Vis::Hidden);
        }
    }
}

/// Counselor_Perk_Menu as perks.rs has it: her names; PerkBox's PerkButtonLarges in rows of ten (SetupPerkList), each
/// its perk's LoadoutIcon, AlreadySelectedCheck on her picked ones, HighlightImg on the highlighted one; PerkName and
/// PerkDescription the highlighted perk's, Increases / Decreases its Positive / NegativeDescription. PLACEHOLDER: no
/// percentages (the rolled modifiers'), no rarity; three rows shown at a time, around the highlighted one (the rows fit
/// above PerkName: the game's library was the player's own rolls), none new (Image_0).
fn fill_perk_menu(hud: &Hud, tree: &mut Widget, m: &crate::perks::PerkMenu, perks: &crate::perks::Perks, profiles: &crate::wardrobe::CounselorProfiles, counselors: Option<&crate::character::CounselorCharacter>) {
    let text = |t: &str| json!({ "LocalizedString": t });
    let name = counselors.map(|c| c.looks(&m.class)).and_then(|l| l.names.get("CharacterName").cloned()).unwrap_or_else(|| m.class.clone());
    let (first, rest) = name.split_once(' ').unwrap_or((name.as_str(), ""));
    let library = perks.library();
    let picked = profiles.picked_perks(&m.class);
    let def = library.get(m.index).and_then(|p| perks.0.get(p));
    let (positive, negative) = (def.and_then(|d| d.positive.clone()).unwrap_or_default(), def.and_then(|d| d.negative.clone()).unwrap_or_default());
    for (n, t) in [
        ("FirstNameText", first.to_string()),
        ("LastNameText", rest.to_string()),
        ("PerkName", def.map_or(String::new(), |d| d.name.clone())),
        ("PerkDescription", def.and_then(|d| d.description.clone()).unwrap_or_default()),
        ("IncreaseDescriptionText", positive.clone()),
        ("DecreaseDescriptionText", negative.clone()),
        ("IncreasePercentText", String::new()),
        ("DecreasePrecentageText", String::new()),
    ] {
        if let Some(x) = tree.find_mut(n) {
            x.props["Text"] = text(&t);
        }
    }
    for (row, shown) in [("Increases", !positive.is_empty()), ("Decreases", !negative.is_empty())] {
        if let Some(x) = tree.find_mut(row) {
            x.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
        }
    }
    let (Some(button), Some(b)) = (&hud.perk_button, tree.find_mut("PerkBox")) else { return };
    let rows = library.len().div_ceil(crate::perks::ROW);
    let first = (m.index / crate::perks::ROW).saturating_sub(1).min(rows.saturating_sub(3));
    b.children = library
        .chunks(crate::perks::ROW)
        .enumerate()
        .skip(first)
        .take(3)
        .map(|(r, row)| {
            let buttons = row
                .iter()
                .enumerate()
                .map(|(k, p)| {
                    let i = r * crate::perks::ROW + k;
                    let mut x = button.clone();
                    x.name = format!("Perk_{i}");
                    x.slot_kind = "HorizontalBoxSlot".into();
                    x.slot = json!({});
                    if let (Some(img), Some(icon)) = (x.find_mut("PerkImage"), perks.0.get(p).and_then(|d| d.icon.clone())) {
                        img.props["Brush"]["ResourceObject"] = json!({ "File": format!("{icon}.png") });
                    }
                    for (n, shown) in [("HighlightImg", i == m.index), ("AlreadySelectedCheck", picked.contains(p)), ("Rarity", false), ("Image_0", false)] {
                        if let Some(h) = x.find_mut(n) {
                            h.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
                        }
                    }
                    x
                })
                .collect();
            Widget::new("HorizontalBox", &format!("PerkRow_{r}"), json!({}), "VerticalBoxSlot", json!({}), buttons)
        })
        .collect();
}

/// Counselor_Emote_Menu as emotes.rs has it: her names; its wheel (CounselorEmoteSelect: her eight, the section
/// highlighted) or, the panel shown (ShowEmoteBox: the wheel hidden), CounselorEmotePanel's EmoteBox of
/// EmoteButtonLarges, rows of nine (PerkHorizontalBox: AddEmote), each its emote's icon, the highlighted one's
/// HighlightImg; EmoteName the highlighted emote's name (OnUpdateHighlightRotation, the panel's highlight).
fn fill_emote_menu(hud: &Hud, tree: &mut Widget, m: &crate::emotes::EmoteMenu, emotes: &crate::emotes::Emotes, counselors: Option<&crate::character::CounselorCharacter>) {
    let text = |t: &str| json!({ "LocalizedString": t });
    let name = counselors.map(|c| c.looks(&m.class)).and_then(|l| l.names.get("CharacterName").cloned()).unwrap_or_else(|| m.class.clone());
    let (first, rest) = name.split_once(' ').unwrap_or((name.as_str(), ""));
    for (n, t) in [("FirstNameText", first), ("LastNameText", rest)] {
        if let Some(x) = tree.find_mut(n) {
            x.props["Text"] = text(t);
        }
    }
    let available = emotes.available();
    let highlighted = match m.list {
        Some(i) => available.get(i).cloned(),
        None => m.slots.get(m.section).cloned(),
    };
    let label = highlighted.and_then(|e| emotes.emotes.get(&e)).map_or(String::new(), |d| d.name.clone());
    if let Some(x) = tree.find_mut("EmoteName") {
        x.props["Text"] = text(&label);
    }
    if let Some(select) = tree.find_mut("CounselorEmoteSelect") {
        select.set_visibility(if m.list.is_some() { Vis::Hidden } else { Vis::Visible });
        fill_emote_wheel(select, &m.slots, Some(m.section), emotes);
    }
    let Some(panel) = tree.find_mut("CounselorEmotePanel") else { return };
    panel.set_visibility(if m.list.is_some() { Vis::Visible } else { Vis::Hidden });
    // (ShowEmoteList's end: the panel faded in from its alpha 0 as built.)
    panel.props["ColorAndOpacity"]["A"] = json!(1.0);
    let (Some(i), Some(button), Some(b)) = (m.list, &hud.emote_button, panel.find_mut("EmoteBox")) else { return };
    b.children = available
        .chunks(9)
        .enumerate()
        .map(|(r, row)| {
            let buttons = row
                .iter()
                .enumerate()
                .map(|(k, e)| {
                    let mut x = button.clone();
                    x.name = format!("Emote_{}", r * 9 + k);
                    x.slot_kind = "HorizontalBoxSlot".into();
                    x.slot = json!({});
                    if let (Some(img), Some(icon)) = (x.find_mut("EmoteImage"), emotes.emotes.get(e).and_then(|d| d.icon.clone())) {
                        img.props["Brush"]["ResourceObject"] = json!({ "File": format!("{icon}.png") });
                    }
                    if let Some(h) = x.find_mut("HighlightImg") {
                        h.set_visibility(if r * 9 + k == i { Vis::Visible } else { Vis::Hidden });
                    }
                    x
                })
                .collect();
            Widget::new("HorizontalBox", &format!("EmoteRow_{r}"), json!({}), "VerticalBoxSlot", json!({}), buttons)
        })
        .collect();
}

/// Counselor_Emote_Select as emotes.rs has it: each EmoteIcon's EmoteImage its emote's EmoteIcon (SetEmoteIcon), the
/// highlight (HLGradient, HLLines, Arrow: GetHighlightVisibility, OnUpdateHighlightRotation) turned to the highlighted
/// section (45 degrees each, about the wheel's centre: their RenderTransformPivot), hidden with none.
fn fill_emote_wheel(tree: &mut Widget, slots: &[String], section: Option<usize>, emotes: &crate::emotes::Emotes) {
    for (i, name) in slots.iter().enumerate().take(8) {
        let icon = emotes.emotes.get(name).and_then(|e| e.icon.clone());
        if let (Some(x), Some(icon)) = (tree.find_mut(&format!("EmoteIcon{i}")).and_then(|x| x.find_mut("EmoteImage")), icon) {
            x.props["Brush"]["ResourceObject"] = json!({ "File": format!("{icon}.png") });
        }
    }
    for name in ["HLGradient", "HLLines", "Arrow"] {
        let Some(x) = tree.find_mut(name) else { continue };
        match section {
            Some(s) => {
                x.set_visibility(Vis::Visible);
                x.props["RenderTransform"] = json!({ "Angle": s as f64 * 45.0 });
            }
            None => x.set_visibility(Vis::Hidden),
        }
    }
}

/// The wardrobe (Counselor_Wardrobe_Menu, SCCounselorWardrobeWidget: decompiled) as wardrobe.rs has it: her names
/// (PreviewClassFirstName / LastName); the OutfitList's Counselor_Outfit_Buttons (each outfit's OutfitIcon, the worn
/// one's SelectedImage); the outfit's slot buttons (their SlotIcon; the slots it lacks collapsed), the focused slot's
/// swatch list (EnableSwatch: its SlotIcon and a Counselor_Swatch_Button per option, its SwatchIcon, the chosen one's
/// SelectedImage); DescriptionText the focused row's (OnOutfitHighlighted / OnSwatchHighlighted's text: the outfit's
/// or slot's name). All unlocked (LockImage collapsed). PLACEHOLDER: the focused slot shown by its button's
/// HighlightBorder (the buttons' focus styles not read).
fn fill_wardrobe(hud: &Hud, tree: &mut Widget, w: &crate::wardrobe::WardrobeMenu, counselors: &crate::character::CounselorCharacter) {
    let looks = counselors.looks(&w.class);
    let text = |t: &str| json!({ "LocalizedString": t });
    let image = |path: &Option<String>| path.as_ref().map(|p| json!({ "File": format!("{p}.png") }));
    let name = looks.names.get("CharacterName").cloned().unwrap_or_else(|| w.class.clone());
    let (first, rest) = name.split_once(' ').unwrap_or((name.as_str(), ""));
    for (n, t) in [("FirstNameText", first), ("LastNameText", rest)] {
        if let Some(x) = tree.find_mut(n) {
            x.props["Text"] = text(t);
        }
    }
    let Some(outfit) = looks.outfits.get(w.outfit) else { return };
    let description = if w.row == 0 { outfit.name.clone() } else { outfit.slots.get(w.row - 1).and_then(|s| s.name.clone()).unwrap_or_default() };
    if let Some(x) = tree.find_mut("DescriptionText") {
        x.props["Text"] = text(&description);
    }
    if let (Some(list), Some(button)) = (tree.find_mut("OutfitList").and_then(|l| l.find_mut("ScrollBox_0")), &hud.outfit_button) {
        list.children = looks
            .outfits
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let mut b = button.clone();
                b.name = format!("Outfit_{i}");
                b.slot_kind = "ScrollBoxSlot".into();
                b.slot = json!({});
                if let (Some(p), Some(r)) = (b.find_mut("PreviewImage"), image(&o.icon)) {
                    p.props["Brush"]["ResourceObject"] = r;
                }
                if let Some(s) = b.find_mut("SelectedImage") {
                    s.set_visibility(if i == w.outfit { Vis::Visible } else { Vis::Hidden });
                }
                if let Some(l) = b.find_mut("LockImage") {
                    l.set_visibility(Vis::Collapsed);
                }
                b
            })
            .collect();
    }
    for k in 0..3 {
        let slot = outfit.slots.get(k);
        if let Some(b) = tree.find_mut(&format!("Slot_Button_0{}", k + 1)) {
            match slot {
                Some(s) => {
                    if let (Some(i), Some(r)) = (b.find_mut("Image_0"), image(&s.icon)) {
                        i.props["Brush"]["ResourceObject"] = r;
                    }
                    if let Some(h) = b.find_mut("HighlightBorder") {
                        h.set_visibility(if w.row == k + 1 { Vis::Visible } else { Vis::Hidden });
                    }
                }
                None => b.set_visibility(Vis::Collapsed),
            }
        }
        let list_name = if k == 0 { "Counselor_Slot_01_Swatch_List_195".to_string() } else { format!("Counselor_Slot_0{}_Swatch_List", k + 1) };
        let Some(list) = tree.find_mut(&list_name) else { continue };
        let Some(s) = slot.filter(|_| w.row == k + 1) else {
            list.set_visibility(Vis::Collapsed);
            continue;
        };
        list.set_visibility(Vis::Visible);
        if let (Some(i), Some(r)) = (list.find_mut("SlotIcon"), image(&s.icon)) {
            i.props["Brush"]["ResourceObject"] = r;
        }
        let pick = w.picks.get(k).copied().unwrap_or(0);
        if let (Some(b), Some(button)) = (list.find_mut("SwatchListBox"), &hud.swatch_button) {
            b.children = (0..s.options.len())
                .map(|i| {
                    let mut sw = button.clone();
                    sw.name = format!("Swatch_{i}");
                    sw.slot_kind = "WrapBoxSlot".into();
                    sw.slot = json!({});
                    if let (Some(img), Some(r)) = (sw.find_mut("Image_0"), image(&s.option_icons.get(i).cloned().flatten())) {
                        img.props["Brush"]["ResourceObject"] = r;
                    }
                    if let Some(x) = sw.find_mut("SelectedImage") {
                        x.set_visibility(if i == pick { Vis::Visible } else { Vis::Hidden });
                    }
                    if let Some(l) = sw.find_mut("LockImage") {
                        l.set_visibility(Vis::Collapsed);
                    }
                    sw
                })
                .collect();
        }
    }
}

/// Counselor_Menu's SetCounselor (decompiled): FirstNameText the counselor's CharacterName's first word, LastNameText the
/// rest; NameText and DescriptionText cleared (they show the focused stat, perk or ability's, not ported); each
/// Stat_Widget's UpdateStat: StatAmount her stat (GetCounselorStatAmount), StatBar at it / 10, TitleName its StatName.
fn fill_counselor_menu(tree: &mut Widget, class: &str, counselors: Option<&crate::character::CounselorCharacter>, tuning: &crate::player::PlayerTuning) {
    use f13_core::stats::Stat;
    let name = counselors.map(|c| c.looks(class)).and_then(|l| l.names.get("CharacterName").cloned()).unwrap_or_else(|| class.to_string());
    let mut words = name.split(' ');
    let first = words.next().unwrap_or("").to_string();
    let rest = words.collect::<Vec<_>>().join(" ");
    let text = |t: &str| json!({ "LocalizedString": t });
    for (w, t) in [("FirstNameText", first), ("LastNameText", rest), ("NameText", String::new()), ("DescriptionText", String::new())] {
        if let Some(x) = tree.find_mut(w) {
            x.props["Text"] = text(&t);
        }
    }
    let stats = tuning.stats_of(class);
    // (Its RepairStat is Intelligence.)
    for (w, stat) in [("ComposureStat", Stat::Composure), ("LuckStat", Stat::Luck), ("RepairStat", Stat::Intelligence), ("SpeedStat", Stat::Speed), ("StaminaStat", Stat::Stamina), ("StealthSTat", Stat::Stealth), ("StrengthStat", Stat::Strength)] {
        let Some(x) = tree.find_mut(w) else { continue };
        let title = x.props["StatName"].clone();
        let v = stats[stat as usize];
        if let Some(t) = x.find_mut("TitleName") {
            if title.is_object() {
                t.props["Text"] = title;
            }
        }
        if let Some(t) = x.find_mut("StatAmount") {
            t.props["Text"] = text(&v.to_string());
        }
        if let Some(b) = x.find_mut("StatBar") {
            b.props["Percent"] = json!(v as f32 / 10.0);
        }
    }
}

fn fill_lobby(hud: &Hud, tree: &mut Widget, lobby: &crate::lobby::NetLobby, me: Option<u64>, mutes: Option<&crate::voice::VoiceMutes>, ready_focus: bool) {
    if let Some(cd) = tree.find_mut("CountdownHorizontalBox") {
        cd.set_visibility(if lobby.countdown.is_some() { Vis::Visible } else { Vis::Hidden });
    }
    if let (Some(t), Some(text)) = (lobby.countdown, tree.find_mut("CountdownText")) {
        text.props["Text"] = json!({ "LocalizedString": format!("{}", t.ceil().max(0.0) as i32) });
    }
    // The map (USCLobbyWidget's map display: its definition's DisplayName and LobbyThumbnailImages).
    if let (Some(name), Some(text)) = (hud.lobby_map["Name"].as_str(), tree.find_mut("MapNameText")) {
        text.props["Text"] = json!({ "LocalizedString": name });
    }
    if let (Some(file), Some(image)) = (hud.lobby_map["Image"].as_str(), tree.find_mut("CurrentMapImage")) {
        image.props["Brush"]["ResourceObject"] = json!({ "File": file });
    }
    // The ready bar: its glow while the ready button has the focus (its Tick: ReadyFocused, which its hover sets and
    // clears: lobby.rs ReadyFocus; on at first, the default focus, as Enter readies here), its text the other way from
    // her (ReadyText, UnreadyText).
    let ready = lobby.members.iter().any(|m| Some(m.id) == me && m.ready);
    if let Some(bar) = tree.find_mut("StartBarMaterialImage") {
        bar.props["Brush"]["ResourceObject"]["Scalars"]["Glow"] = json!(if ready_focus { 1.0 } else { 0.0 });
    }
    if let Some(t) = tree.find_mut("ReadyUnreadyText") {
        t.props["Text"] = json!({ "LocalizedString": if ready { "U N R E A D Y" } else { "R E A D Y" } });
    }
    let Some((row, textures, defaults)) = &hud.lobby_row else { return };
    let Some(list) = tree.find_mut("PlayerVerticalBox") else { return };
    list.children.clear();
    let tint = |k: &str| color(&defaults[k], [1.0; 4]);
    for (i, m) in lobby.members.iter().enumerate() {
        let mut w = row.clone();
        w.name = format!("LobbyRow{i}");
        w.slot_kind = "VerticalBoxSlot".into();
        w.slot = json!({});
        let c = if Some(m.id) == me {
            tint("LocalPlayerColor")
        } else if m.ready {
            tint("RemoteReadyPlayerColor")
        } else {
            tint("RemoteUnreadyPlayerColor")
        };
        // Their picks (Get_HasPlayerFullyLoaded_Visibility: shown once they're in): the counselor button their
        // PickedCounselorClass's thumbnail, Jason's the killer's (CharacterSelectButtonWidget Get_CharacterImage_Brush:
        // GetCharacterThumbnailImage, 58 x 58).
        for (button, file) in [
            ("CounselorButton", hud.characters["Classes"][if m.class.is_empty() { crate::character::DEFAULT_CLASS } else { m.class.as_str() }].as_str()),
            ("JasonButton", hud.characters["Killer"]["Thumbnail"].as_str()),
        ] {
            let Some(b) = w.find_mut(button) else { continue };
            b.set_visibility(Vis::Visible);
            if let (Some(file), Some(image)) = (file, b.find_mut("CharacterImage")) {
                image.props["Brush"] = json!({ "ImageSize": { "X": 58.0, "Y": 58.0 }, "ResourceObject": { "File": file } });
                image.set_visibility(Vis::Visible);
            }
        }
        if let Some(name) = w.find_mut("PlayerName") {
            name.props["Text"] = json!({ "LocalizedString": m.name });
            name.props["ColorAndOpacity"] = json!({ "SpecifiedColor": { "R": c[0], "G": c[1], "B": c[2], "A": c[3] } });
        }
        let ready = if m.ready { "Img_Lobby_ReadyBox_Checked_01" } else { "Img_Lobby_ReadyBox_Empty_01" };
        if let (Some(file), Some(image)) = (textures.get(ready), w.find_mut("ReadyBoxImage")) {
            image.props["Brush"]["ResourceObject"] = json!({ "File": file });
            image.set_visibility(Vis::Visible);
        }
        // Its ping: "{0}ms" (ILLPlayerState GetPingDisplayText 0x140269480: none at 0), good to 150 ms, bad to 300,
        // very bad past (Get_PingText_ColorAndOpacity). PLACEHOLDER: the round trip for its Ping (the player state's
        // replicated ping, x4).
        if let Some(t) = w.find_mut("PingText") {
            let ping = m.ping.min(999);
            let c = color(&defaults[if ping <= 150 { "PingGoodColor" } else if ping <= 300 { "PingBadColor" } else { "PingVeryBadColor" }], [1.0; 4]);
            t.props["Text"] = json!({ "LocalizedString": if ping == 0 { String::new() } else { format!("{ping}ms") } });
            t.props["ColorAndOpacity"] = json!({ "SpecifiedColor": { "R": c[0], "G": c[1], "B": c[2], "A": c[3] } });
        }
        // Its speaker: on while they talk, its alpha 0.15, 0.35 once they've talked, 0.9 while they talk
        // (Get_SpeakingIcon_Brush, Get_SpeakingIcon_ColorAndOpacity); muted here, the mute overlay.
        if let Some(icon) = w.find_mut("SpeakingIcon") {
            let file = textures.get(if m.talking { "Img_Speaker_On_01" } else { "Img_Speaker_Off_01" });
            if let Some(file) = file {
                icon.props["Brush"]["ResourceObject"] = json!({ "File": file });
            }
            let a = if m.talking { 0.9 } else if m.talked { 0.35 } else { 0.15 };
            icon.props["ColorAndOpacity"] = json!({ "R": 1.0, "G": 1.0, "B": 1.0, "A": a });
        }
        if let Some(muted) = w.find_mut("MutedIcon") {
            let shown = Some(m.id) != me && mutes.is_some_and(|x| x.0.contains(&m.name));
            muted.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
        }
        if let Some(h) = w.find_mut("HostArrowImage") {
            h.set_visibility(if m.id == 0 { Vis::Visible } else { Vis::Hidden });
        }
        for (name, shown) in [
            ("ConnectingSpinner", false),
            ("KillerPickerButtonWidget_0", false),
            ("SelectionBar", Some(m.id) == me),
        ] {
            if let Some(x) = w.find_mut(name) {
                x.set_visibility(if shown { Vis::Visible } else { Vis::Hidden });
            }
        }
        list.children.push(w);
    }
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

/// A [`RadialFill`] brush's material colour (its Color vector), which its widget's played ColorAndOpacity tints with
/// its brush's (the hold ring: hidden until its widget's OnInteractable fades it in).
#[derive(Component)]
struct RadialTint([f32; 4]);

#[derive(Component)]
struct TintBase([f32; 4]);

/// Applies each changed [`WidgetColor`].
fn widget_colors(
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
struct BaseAlpha(f32);

/// Each Border's content opacity on what's in it: all of it when it changes, and anything new in it. Nested Borders
/// multiply (each tints its content: SBorder's ContentScale/ColorAndOpacity), so what's in several takes all of theirs.
fn content_alpha(
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
struct WidgetAnim {
    length: f32,
    tracks: Vec<(String, f13_core::curve::RichCurve)>,
    colors: Vec<(String, [f13_core::curve::RichCurve; 4])>,
    scales: Vec<(String, [f13_core::curve::RichCurve; 2])>,
    offsets: Vec<(String, [f13_core::curve::RichCurve; 4])>,
    /// A Border's BrushColor curves (red, green, blue, alpha).
    brush_colors: Vec<(String, [f13_core::curve::RichCurve; 4])>,
}

/// Loads a widget class's animations' ContentColorAndOpacity alpha tracks, ColorAndOpacity tracks and RenderTransform
/// scale tracks (ui/<class>.json "Animations").
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
pub struct PlayingAnims(Vec<(String, String, f32, bool)>);

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
fn entry_hover(
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
fn widget_anims(mut commands: Commands, time: Res<Time>, hud: Res<Hud>, playing: Res<PlayingAnims>, mut borders: Query<&mut ContentAlpha>, mut colors: Query<&mut WidgetColor>, (mut nodes, mut xforms): (Query<&mut Node>, Query<&mut UiTransform>)) {
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

/// CounselorHUDWidget's UpdateFearOpacity (its tick, once the match is in progress and MatchStart isn't playing):
/// CounselorFearWidgets (the inventory, the stamina ring) takes SCHUDWidget's FearOpacity. Its writer is
/// SCCounselorCharacter's tick (0x1403d8de0 -> 0x140441430: (1, 1, 1, a)): a FInterpTo at 1 toward 0 while she hides,
/// else HUDOpacityFearCurve(x), x her CosmeticFear (0 dead, escaped, or in a special move that ends in her death:
/// bDieOnFinish); her death (0x1403c0ac0) sets it to 0. MatchStart (which holds them at 0 for 5 s) isn't played on her
/// HUD: in the game her pockets show as she spawns (seen by the user, 2026-10-12): her HUD is built for her pawn, after
/// the match's start (OnMatchStarted) went out. (The minimap takes it
/// too unless NervesOfSteel; left as built.)
#[allow(clippy::too_many_arguments)]
fn fear_widgets(
    time: Res<Time>,
    hud: Res<Hud>,
    game: Option<Res<crate::match_flow::GameMatch>>,
    counselor: Option<Res<crate::character::CounselorCharacter>>,
    escaped: Option<Single<&crate::vehicle::Escaped, With<crate::controls::LocalPlayer>>>,
    players: Query<(&crate::player::Player, Option<&f13_bevy::CounselorFear>)>,
    mut colors: Query<&mut WidgetColor>,
    mut alpha: Local<f32>,
) {
    if let Some((p, fear)) = players.iter().next() {
        let alive = p.counselor.alive();
        let target = if p.hidden {
            0.0
        } else {
            let x = if !alive || escaped.is_some() { 0.0 } else { fear.map_or(0.0, |f| f.0.cosmetic) };
            counselor.as_ref().and_then(|c| c.0.info.curve("HUDOpacityFearCurve")).map_or(1.0, |c| c.eval(x))
        };
        *alpha = match () {
            _ if !alive => 0.0,
            _ if p.hidden => f13_core::fear::finterp_to(*alpha, target, time.delta_secs(), 1.0),
            _ => target,
        };
    }
    let Some(&e) = hud.names.get("CounselorHUDWidget.CounselorFearWidgets_33") else { return };
    let Ok(mut c) = colors.get_mut(e) else { return };
    let started = game.is_some_and(|g| g.state.state == f13_core::game_mode::MatchState::InProgress);
    c.set_if_neq(WidgetColor([1.0, 1.0, 1.0, if started { *alpha } else { 0.0 }]));
}


/// SpectatorHUDWidget (its graph): as she's out, the intro (OnShowSpectatorIntro: SpectatorIntroCanvas, YouDied or
/// YouSurvived shown and the other at alpha 0, YOUAREDEADCue / YouSurvivedCue); the first toggle ends it
/// (OnSpectatorIntroFinished, IntroFadeOut, then CheckHUDVisibility: SpectatorCanvas shown, the intro hidden);
/// OnSpectatorUpdateStatus: StatusText the status, SpectateModeDisplay's TitleText the next mode, NextView and
/// PreviousView shown while it says "Spectating:" or "Viewing:" (else collapsed).
fn spectator_hud(
    mut commands: Commands,
    hud: Res<Hud>,
    status: Option<Res<crate::spectate::SpectatorStatus>>,
    mut nodes: Query<&mut Visibility>,
    mut colors: Query<&mut WidgetColor>,
    mut texts: Query<&mut Text>,
    mut played: Local<Option<bool>>,
    (time, mut playing): (Res<Time>, ResMut<PlayingAnims>),
    (mut shown_colors, mut faded): (Local<bool>, Local<bool>),
    (keys, mut hidden): (Res<ButtonInput<KeyCode>>, Local<bool>),
) {
    let Some(st) = status.filter(|s| s.out) else {
        *played = None;
        (*shown_colors, *faded, *hidden) = (false, false, false);
        return;
    };
    let get = |n: &str| hud.names.get(&format!("SpectatorHUDWidget.{n}")).copied();
    let Some(intro) = get("SpectatorIntroCanvas") else { return };
    // The intro's cue and its widget's OnPlaySpectatorIntro (BackgroundIn, AnimationIn), once.
    if played.is_none() {
        *played = Some(st.died);
        let owner = if st.died { "YouDied" } else { "YouSurvived" };
        for anim in ["BackgroundIn", "AnimationIn"] {
            playing.play(owner, anim, time.elapsed_secs(), false);
        }
        let cue = hud.classes.prop("SpectatorHUDWidget_C", if st.died { "YOUAREDEADCue" } else { "YouSurvivedCue" });
        if let Some(c) = crate::audio::cue_name(&cue) {
            commands.trigger(crate::audio::PlaySound { cue: c, at: None });
        }
    }
    let mut show = |e: Option<Entity>, on: bool| {
        if let Some(mut v) = e.and_then(|e| nodes.get_mut(e).ok()) {
            v.set_if_neq(if on { Visibility::Inherited } else { Visibility::Hidden });
        }
    };
    // SPEC_ToggleHUD (H): OnToggleSpecatorHUD, AnimAllHUD forward to hide, reversed to show (once the intro's over).
    if st.intro_over && keys.just_pressed(KeyCode::KeyH) {
        *hidden = !*hidden;
        playing.play("SpectatorHUDWidget", "AnimAllHUD", time.elapsed_secs(), !*hidden);
    }
    // CheckHUDVisibility: the intro hidden once it has faded.
    show(Some(intro), !st.intro_over || !*faded);
    show(get("SpectatorCanvas"), st.intro_over);
    let arrows = st.status.contains("Spectating:") || st.status.contains("Viewing:");
    show(get("NextView"), arrows);
    show(get("PreviousView"), arrows);
    if !*shown_colors {
        *shown_colors = true;
        for (name, on) in [("YouDied", st.died), ("YouSurvived", !st.died)] {
            if let Some(mut c) = get(name).and_then(|e| colors.get_mut(e).ok()) {
                c.set_if_neq(WidgetColor([1.0, 1.0, 1.0, if on { 1.0 } else { 0.0 }]));
            }
        }
        // AnimAllHUD at its start (Border_1 hidden) until the intro's over.
        playing.play("SpectatorHUDWidget", "AnimAllHUD", time.elapsed_secs(), false);
    }
    // OnSpectatorIntroFinished: IntroFadeOut, then (its OnAnimationFinished) AnimAllHUD reversed.
    if st.intro_over && !*faded {
        *faded = true;
        let now = time.elapsed_secs();
        playing.play("SpectatorHUDWidget", "IntroFadeOut", now, false);
        playing.play("SpectatorHUDWidget", "AnimAllHUD", now + 0.25, true);
    }
    let mut set = |e: Option<Entity>, s: &str| {
        if let Some(mut t) = e.and_then(|e| texts.get_mut(e).ok()) {
            if t.0 != s {
                t.0 = s.to_string();
            }
        }
    };
    set(get("StatusText"), &st.status);
}

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
    /// The menus' preview captures (counselor_preview.rs: CustomizationPreviewMaterial's and WardrobePreviewMaterial's
    /// textures).
    preview: Option<(Handle<Image>, Handle<Image>, Handle<Image>)>,
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
            // Turned about its RenderTransformPivot (a fraction of its size; the centre by default): Bevy turns it
            // about its centre, so it moves by d - R d, d the pivot from the centre.
            let pivot = &p["RenderTransformPivot"];
            let d = Vec2::new((pivot["X"].as_f64().unwrap_or(0.5) as f32 - 0.5) * r.w, (pivot["Y"].as_f64().unwrap_or(0.5) as f32 - 0.5) * r.h) * self.scale;
            let t = d - Rot2::degrees(angle) * d;
            self.commands.entity(e).insert(UiTransform { translation: Val2::px(t.x, t.y), scale: Vec2::new(s("X"), s("Y")), rotation: Rot2::degrees(angle) });
        }
        match w.kind.as_str() {
            "Image" => {
                // Its ColorAndOpacity can be played (WidgetColor), over its brush's tint and the borders' around it.
                let cao = color(&p["ColorAndOpacity"], [1.0; 4]);
                self.commands.entity(e).insert((ColorBase(mul(color(&p["Brush"]["TintColor"], [1.0; 4]), self.content)), WidgetColor(cao)));
                self.brush(e, &p["Brush"], mul(cao, self.content));
            }
            "Border" => {
                // Its BrushColor tints its brush and can be played (WidgetColor over the brush's tint and the borders'
                // around it).
                let bc = color(&p["BrushColor"], [1.0; 4]);
                self.brush(e, &p["Background"], mul(bc, self.content));
                if p["BrushColor"].is_object() {
                    self.commands.entity(e).insert((ColorBase(mul(color(&p["Background"]["TintColor"], [1.0; 4]), self.content)), WidgetColor(bc)));
                }
            }
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
            "Slider" => {
                // SSlider::OnPaint: its bar (BarThickness) across, inset half the thumb each side, centred; the thumb at
                // Value along the width less its own.
                let s = &p["WidgetStyle"];
                let size = |b: &Value, d: f32| Vec2::new(b["ImageSize"]["X"].as_f64().unwrap_or(d as f64) as f32, b["ImageSize"]["Y"].as_f64().unwrap_or(d as f64) as f32);
                let thumb = size(&s["NormalThumbImage"], 8.0);
                let thick = s["BarThickness"].as_f64().unwrap_or(2.0) as f32;
                let value = p["Value"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0) as f32;
                let k = self.scale;
                let at = |x: f32, y: f32, w: f32, h: f32| Node { position_type: PositionType::Absolute, left: Val::Px(x * k), top: Val::Px(y * k), width: Val::Px(w * k), height: Val::Px(h * k), ..default() };
                let bar = self.commands.spawn((at(thumb.x / 2.0, (r.h - thick) / 2.0, (r.w - thumb.x).max(0.0), thick), Pickable::IGNORE, ChildOf(e))).id();
                self.brush(bar, &s["NormalBarImage"], self.content);
                let pip = self.commands.spawn((at(value * (r.w - thumb.x).max(0.0), (r.h - thumb.y) / 2.0, thumb.x, thumb.y), Pickable::IGNORE, ChildOf(e))).id();
                self.brush(pip, &s["NormalThumbImage"], self.content);
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
        if res["Parent"].as_str() == Some("M_UI_JasonHud_01") && !textures.is_empty() {
            self.commands.entity(e).insert(AdditiveBrush::new(self.assets.load(textures[0].to_string()), c, &res["Scalars"]));
            return;
        }
        // CustomizationPreviewMaterial, WardrobePreviewMaterial: the preview's capture (its colour, opacity 1 - its alpha:
        // the image's alpha here).
        let capture = match res["Name"].as_str() {
            Some("Material'CustomizationPreviewMaterial'") => Some(0),
            Some("Material'WardrobePreviewMaterial'") => Some(1),
            Some("Material'EmotePreviewMaterial'") => Some(2),
            _ => None,
        };
        if let Some(i) = capture {
            if let Some((counselor, wardrobe, emote)) = self.preview.clone() {
                self.commands.entity(e).insert(ImageNode::new([counselor, wardrobe, emote][i].clone()).with_color(linear(c)));
            }
            return;
        }
        if res["Name"].as_str() == Some("Material'HoldInteractMaterial'") && !textures.is_empty() {
            // The interaction widgets' hold ring (SCInteractWidget's tick sets its Alpha: ui/prompts.rs).
            let v = res["Vectors"]["Color"].as_array().map_or([1.0; 4], |a| [0, 1, 2, 3].map(|i| a.get(i).and_then(|x| x.as_f64()).unwrap_or(1.0) as f32));
            let ring: Handle<Image> = self.assets.load(textures[0].to_string());
            let m = self.materials.add(RadialFill {
                color: LinearRgba::from(linear(mul(c, v))),
                fill: Vec4::new(res["Scalars"]["Alpha"].as_f64().unwrap_or(0.0) as f32, 0.0, 2.0, 0.0),
                ring: ring.clone(),
                gradient: ring,
            });
            self.commands.entity(e).insert((MaterialNode(m), RadialTint(v)));
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
        // A material of texture parameters (LobbyReadyButtonMaterial: its UI Element and UI Glow Element by its Glow):
        // the glow's past half way. PLACEHOLDER: a lerp's halves for the lerp (its graph is cooked away).
        let element = if res["Scalars"]["Glow"].as_f64().unwrap_or(0.0) >= 0.5 { "UI Glow Element" } else { "UI Element" };
        let file = res["File"].as_str().or(textures.first().copied()).or_else(|| res["TextureParameters"][element].as_str());
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
                // A tiled brush repeats at its ImageSize (in the HUD's scale; inside a scale box, its too) along its
                // axes and stretches along the others (Img_Lobby_Frame_01, 1742 wide, shows its left half in 874).
                // PLACEHOLDER: the texture taken as ImageSize's size.
                let (tx, ty) = match b["Tiling"].as_str().unwrap_or("").rsplit(':').next().unwrap_or("") {
                    "Horizontal" => (true, false),
                    "Vertical" => (false, true),
                    "Both" => (true, true),
                    _ => (false, false),
                };
                if tx || ty {
                    image.image_mode = NodeImageMode::Tiled { tile_x: tx, tile_y: ty, stretch_value: self.scale * self.text_scale };
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
    player: Option<Single<&Transform, (With<Player>, With<crate::controls::LocalPlayer>)>>,
    camera: Option<Single<&GlobalTransform, With<crate::player::PlayerCamera>>>,
    nodes: Query<&MaterialNode<Minimap>>,
    mut maps: ResMut<Assets<Minimap>>,
) {
    let Some(map) = &hud.map else { return };
    // His map's image (MapImage): the level's KillerMapTexture in place of MiniMap_Mat_Inst's (SCMiniMapBase's
    // vt+0x4c8 from InitializeMinimap 0x14029c050), across SC_KillerMap_Widget (the click maps across it). Whether or
    // not a counselor is played here (a client playing Jason has none).
    if let (Some(file), Some(mut img)) = (&map.killer_file, hud.names.get("SC_KillerMap_Widget.Image_0").and_then(|&e| images.get_mut(e).ok())) {
        let handle = asset_server.load(file.clone());
        if img.image != handle {
            img.image = handle;
        }
    }
    let (Some(player), Some(camera)) = (player, camera) else { return };
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
    player: Query<&Inventory, (With<Player>, With<crate::controls::LocalPlayer>)>,
    items: Query<&Item>,
    (vehicles, boxes, phones, game, escaped): (
        Query<&crate::props::Vehicle>,
        Query<&crate::police::PhoneBox>,
        Query<&crate::police::Phone>,
        Option<Res<crate::match_flow::GameMatch>>,
        Option<Single<&crate::vehicle::Escaped, With<crate::controls::LocalPlayer>>>,
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
                    c.set_if_neq(WidgetColor(curves.each_ref().map(|k| if k.keys.is_empty() { 1.0 } else { k.eval(x) })));
                }
            }
        }
    }
    let size = window.size();
    let has_map = player.single().is_ok_and(|inv| inv.special.iter().any(|&e| items.get(e).is_ok_and(|i| i.kind == crate::props::ItemKind::Map)));
    let rows = objective_rows(&vehicles, &boxes, &phones, game.as_deref(), escaped.as_ref().map(|e| **e));
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
    let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, content: [1.0; 4], names: &mut names, preview: None };
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
                        // Its own colour under its map's (widget_colors multiplies a changing ancestor's colour over
                        // this: captured from the node instead, it was the closed map's transparent one, and the icons
                        // stayed unseen until the map's MapFade had played).
                        TintBase(brush),
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

/// The wiggle widget's node in the HUD tree.
const WIGGLE: &str = "WiggleWidgetInstance";

/// The local character's stun minigame (its completion, GetWiggleValue, and StunText), while it runs: hers in his grab
/// (CounselorGrabStun "Break Free") or a bear trap ("Open Trap"); his stuns' when he's played.
fn wiggle_state(
    play: Option<Res<crate::jason_player::PlayJason>>,
    struggle: Option<Res<crate::jason::Struggle>>,
    player: Option<Single<(Has<crate::jason::Held>, Option<&crate::items::Trapped>), With<Player>>>,
    jasons: Query<&crate::jason::Jason>,
) -> Option<(f32, &'static str)> {
    if play.is_some() {
        return jasons.iter().find_map(|j| j.stun_minigame());
    }
    let (held, trapped) = player.map_or((false, None), |p| *p);
    if let Some(c) = trapped.and_then(|t| t.struggle()) {
        return Some((c, "Open Trap"));
    }
    (held && jasons.iter().any(|j| j.holding())).then(|| (struggle.map_or(0.0, |s| s.wiggle.completion), "Break Free"))
}

/// SC_WiggleMinigame_Widget while a minigame runs (OnStartWiggleWidget / OnStopWiggleWidget): ProgressBar_Left and
/// ProgressBar_Right at GetWiggleValue (the completion), TextBlock_119 the character's StunText (the stun class's,
/// BeginStun 0x1402e3fc0), its ActionMapWidget GLB_Wiggle's key.
fn wiggle_widget(
    state: In<Option<(f32, &'static str)>>,
    mut commands: Commands,
    hud: Res<Hud>,
    mut vis: Query<&mut Visibility>,
    bars: Query<&Progress>,
    mut texts: Query<&mut Text>,
) {
    let owner = if hud.names.contains_key(&format!("KillerHUDWidget.{WIGGLE}")) { "KillerHUDWidget" } else { "CounselorHUDWidget" };
    let Some(&root) = hud.names.get(&format!("{owner}.{WIGGLE}")) else { return };
    if let Ok(mut v) = vis.get_mut(root) {
        v.set_if_neq(if state.0.is_some() { Visibility::Inherited } else { Visibility::Hidden });
    }
    let Some((completion, text)) = state.0 else { return };
    for name in ["ProgressBar_Left", "ProgressBar_Right"] {
        if let Some(p) = hud.names.get(&format!("SC_WiggleMinigame_Widget.{name}")).and_then(|&e| bars.get(e).ok()) {
            set_progress(&mut commands, p.fill, p, completion);
        }
    }
    if let Some(mut t) = hud.names.get("SC_WiggleMinigame_Widget.TextBlock_119").and_then(|&e| texts.get_mut(e).ok()) {
        if t.0 != text {
            t.0 = text.to_string();
        }
    }
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
    player: Option<Single<&Player, With<crate::controls::LocalPlayer>>>,
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
    (mut unlocked, mut ability_state): (Local<[Option<Entity>; 4]>, Local<[(bool, bool); 4]>),
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
        for (slot, &kill) in crate::jason::grab_kill_slots().iter().enumerate() {
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
        // OnAbilityUnlocked: the unlock bar hidden (its asset gives Morph's the Stalk material, which would otherwise
        // stay drawn over Morph), the bar and the bracket shown, the bracket opaque when the amount is full.
        let i = ability as usize;
        // Done once per built HUD (the unlock bar's node it was done to): the HUD is rebuilt as its contents change
        // (the match's timer after the intro), its widgets back as built, and Morph unlocks before that.
        let unlock_bar = hud.names.get(&format!("KillerAbilityWidget.{n}UnlockBar")).copied();
        if a.is_unlocked(ability) && unlock_bar.is_some() && unlocked[i] != unlock_bar {
            unlocked[i] = unlock_bar;
            for (part, v) in [("UnlockBar", Visibility::Hidden), ("Bar", Visibility::Inherited)] {
                if let Some(mut w) = hud.names.get(&format!("KillerAbilityWidget.{n}{part}")).and_then(|&e| vis.get_mut(e).ok()) {
                    w.set_if_neq(v);
                }
            }
            let bracket = format!("KillerAbilityWidget.Bracket{n}");
            if let Some(mut w) = hud.names.get(&bracket).and_then(|&e| vis.get_mut(e).ok()) {
                w.set_if_neq(Visibility::Inherited);
            }
            let alpha = if amount >= 1.0 { 1.0 } else { 0.0 };
            if let Some(mut w) = hud.names.get(&bracket).and_then(|&e| icon_colors.get_mut(e).ok()) {
                w.set_if_neq(WidgetColor([0.854993, 0.030713, 0.017642, alpha]));
            }
        }
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
    // The widget's native tick (FUN_140434aa0): per ability, IsAbilityActive and IsAbilityAvailable against the
    // cached bytes; the first change fires its event and ends the tick (one event a frame). Active flips:
    // OnAbilityActive plays <ability>ActiveAnim looping (NumLoopsToPlay 0), OnAbilityEnded stops it (KeepState: it
    // keeps its last frame). Available flips: OnAbilityAvailable plays <ability>ReadyToUse, OnAbilityUsed
    // <ability>FadeOut.
    for ability in Ability::ALL {
        let i = ability as usize;
        let n = format!("{ability:?}");
        // IsAbilityActive (0x1403f4e00): bIsMorphActive, bIsSenseActive, bIsShiftActive, bIsStalkActive.
        let active = match ability {
            Ability::Morph => morph_map.as_ref().is_some_and(|m| m.morph),
            Ability::Sense => k.sensing(now),
            Ability::Shift => matches!(j.state, crate::jason::JasonState::Shifting { .. }),
            Ability::Stalk => j.stalk.is_some(),
        };
        let available = a.is_available(ability, &Blockers::default());
        let (was_available, was_active) = ability_state[i];
        if active != was_active {
            let anim = format!("{n}ActiveAnim");
            if active {
                playing.play("KillerAbilityWidget", &anim, now, false);
            } else {
                playing.0.retain(|(o, x, _, _)| !(o == "KillerAbilityWidget" && *x == anim));
            }
            ability_state[i].1 = active;
            break;
        }
        if available != was_available {
            playing.play("KillerAbilityWidget", &format!("{n}{}", if available { "ReadyToUse" } else { "FadeOut" }), now, false);
            ability_state[i].0 = available;
            break;
        }
    }
    // The ActiveAnims loop (2 s each).
    for (o, x, start, _) in playing.0.iter_mut() {
        if o == "KillerAbilityWidget" && x.ends_with("ActiveAnim") && now - *start >= 2.0 {
            *start += 2.0 * ((now - *start) / 2.0).floor();
        }
    }
    let rage = if a.rage_active { 1.0 } else { (a.rage_timer / a.tuning.rage_unlock_time.max(1.0)).clamp(0.0, 1.0) };
    bar("RageWidget.ProgressBar_41", rage);
}

/// AbilityAvailable, his notice as an ability unlocks (KillerHUDWidget's OnJasonAbilityUnlocked; not in Paranoia or
/// SPChallenges, not ported) or rage starts (its OnRageActive): SetAbility(ability, rage) names it ("<Ability> is
/// available", "Rage is active") and shows its icon (Get_Icon<X>_Visibility by ActiveAbility, which a rage notice
/// leaves at its default, Morph, under IconRage by IsRage); PlayShow plays Show and after its end + 3 s PlayClose plays
/// Hide, then Cleanup removes it, as its Tick does once the match has ended (SCGameState HasMatchEnded 0x1403bc250:
/// PostMatchOutro, WaitingPostMatch, LeavingMap). The game adds a new widget each time, at ZOrder -1 (under his HUD);
/// here a new one replaces the last.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn ability_popup(
    mut commands: Commands,
    time: Res<Time>,
    mut hud: ResMut<Hud>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<&crate::senses::Knowledge, With<crate::jason::Jason>>,
    game: Option<Res<crate::match_flow::GameMatch>>,
    (assets, mut materials): (Res<AssetServer>, ResMut<Assets<RadialFill>>),
    mut playing: ResMut<PlayingAnims>,
    mut state: Local<([bool; 4], bool, Option<(f32, bool, Entity)>)>,
) {
    use f13_core::game_mode::MatchState;
    use f13_core::killer_abilities::Ability;
    let (Some(root), Some((size, ..)), Some(tree)) = (hud.root, hud.built.clone(), hud.ability_popup.clone()) else { return };
    if play.is_none() {
        return;
    }
    let Some(k) = jasons.iter().next() else { return };
    let a = &k.abilities;
    let now = time.elapsed_secs();
    let mut show: Option<(Option<Ability>, bool)> = None;
    for ability in Ability::ALL {
        let i = ability as usize;
        if a.is_unlocked(ability) && !state.0[i] {
            state.0[i] = true;
            show = Some((Some(ability), false));
        }
    }
    if a.rage_active && !state.1 {
        show = Some((None, true));
    }
    state.1 = a.rage_active;
    let len = |n: &str| hud.anims.get("AbilityAvailable").and_then(|m| m.get(n)).map_or(0.0, |x| x.length);
    let (show_len, hide_len) = (len("Show"), len("Hide"));
    if let Some((ability, rage)) = show {
        let mut tree = tree;
        let mut set_text = |name: &str, s: &str| {
            if let Some(w) = tree.find_mut(name) {
                w.props["Text"] = json!({ "SourceString": s, "LocalizedString": s });
            }
        };
        let active = ability.unwrap_or(Ability::Morph);
        if rage {
            set_text("AbilityName", "Rage");
            set_text("AbilityText", " is active");
        } else {
            set_text("AbilityName", &format!("{active:?}"));
            set_text("AbilityText", " is available");
        }
        for (icon, on) in Ability::ALL.map(|x| (format!("Icon{x:?}"), x == active)).into_iter().chain([("IconRage".to_string(), rage)]) {
            if let Some(w) = tree.find_mut(&icon) {
                w.set_visibility(if on { Vis::Visible } else { Vis::Hidden });
            }
        }
        if let Some((_, _, e)) = state.2.take() {
            commands.entity(e).try_despawn();
        }
        let scale = hud.scale;
        let virt = size / scale;
        let full = Rect { x: 0.0, y: 0.0, w: virt.x, h: virt.y };
        let mut names = HashMap::new();
        let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, content: [1.0; 4], names: &mut names, preview: None };
        let e = b.spawn(&tree.layout(full), root, full, "AbilityAvailable");
        b.commands.entity(e).insert(ZIndex(-1));
        hud.names.extend(names);
        playing.0.retain(|(o, _, _, _)| o != "AbilityAvailable");
        playing.play("AbilityAvailable", "Show", now, false);
        state.2 = Some((now, false, e));
        info!("ui: ability notice: {}", if rage { "Rage".to_string() } else { format!("{active:?}") });
    }
    let ended = game.is_some_and(|g| matches!(g.state.state, MatchState::PostMatchOutro | MatchState::WaitingPostMatch | MatchState::LeavingMap));
    let close_at = |t0: f32| t0 + show_len + 3.0;
    match state.2 {
        Some((t0, false, e)) if now >= close_at(t0) && !ended => {
            playing.play("AbilityAvailable", "Hide", now, false);
            state.2 = Some((t0, true, e));
        }
        Some((t0, hiding, e)) if ended || (hiding && now >= close_at(t0) + hide_len) => {
            playing.0.retain(|(o, _, _, _)| o != "AbilityAvailable");
            commands.entity(e).try_despawn();
            hud.names.retain(|k, _| !k.starts_with("AbilityAvailable."));
            state.2 = None;
        }
        _ => {}
    }
}

/// His Morph map's cursor: SetMapStatus shows the mouse cursor as Crosshairs (8) while the map's out for a Morph, and
/// UserInterfaceSettings' CrosshairsCursor makes that the Morph_Area_Cursor widget (Image_0: 50 x 50, centred on its
/// canvas, Morph_Cursor_02 on M_UI_MorphCursor_01, tinted EE3124) in place of the system's. PLACEHOLDER: the widget
/// drawn centred on the mouse (where Slate puts a software cursor widget isn't read). The Hand cursor
/// (Morph_Area_Bad_Cursor, AttemptMorph's refusal) isn't used: nothing is refused in this build.
#[allow(clippy::type_complexity)]
fn morph_cursor(
    mut commands: Commands,
    mut hud: ResMut<Hud>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    morph_map: Option<Res<crate::jason_player::MorphMap>>,
    (assets, mut materials): (Res<AssetServer>, ResMut<Assets<RadialFill>>),
    (window, mut cursor): (Single<&Window, With<PrimaryWindow>>, Single<&mut bevy::window::CursorOptions>),
    mut nodes: Query<(&mut Node, &mut Visibility)>,
    mut spawned: Local<Option<Entity>>,
) {
    let (Some(root), Some(tree)) = (hud.root, hud.morph_cursor.clone()) else { return };
    let out = play.is_some() && morph_map.is_some_and(|m| m.morph);
    let at = window.cursor_position().filter(|_| out);
    if out && cursor.visible {
        cursor.visible = false;
    }
    let e = match *spawned {
        Some(e) => e,
        None if at.is_some() => {
            let e = commands.spawn((Node { position_type: PositionType::Absolute, ..default() }, Visibility::Hidden, Pickable::IGNORE, GlobalZIndex(i32::MAX), ChildOf(root))).id();
            let none = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
            let scale = hud.scale;
            let mut names = HashMap::new();
            let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, content: [1.0; 4], names: &mut names, preview: None };
            b.spawn(&tree.layout(none), e, none, "Morph_Area_Cursor");
            hud.names.extend(names);
            *spawned = Some(e);
            return;
        }
        None => return,
    };
    // Gone with a rebuilt HUD: spawned again.
    let Ok((mut node, mut vis)) = nodes.get_mut(e) else {
        *spawned = None;
        return;
    };
    vis.set_if_neq(if at.is_some() { Visibility::Inherited } else { Visibility::Hidden });
    if let Some(at) = at {
        node.left = Val::Px(at.x);
        node.top = Val::Px(at.y);
    }
}

/// Rage on his HUD, once as it starts (it lasts the match): KillerHUDWidget's OnRageActive plays RageActive (RageBorder,
/// the screen's red edge: pulsing over 5 s, then clear), calls KillerAbilityWidget's OnRageActive (FrameRage
/// SelfHitTestInvisible, at its designed alpha 0; OnRageActiveAnim, the diamond's glow over 4 s; each ability bar's
/// M_UI_JasonHud_01: background Smoke 0, Brightness 0, fill Smoke 1, Brightness 10) and RageWidget's ActivateRage
/// (OnRageActive: its glow pulsing over 5 s, then lit). Its AbilityAvailable notice: [`ability_popup`].
#[allow(clippy::type_complexity)]
fn rage_hud(
    time: Res<Time>,
    hud: Res<Hud>,
    play: Option<Res<crate::jason_player::PlayJason>>,
    jasons: Query<&crate::senses::Knowledge, With<crate::jason::Jason>>,
    mut playing: ResMut<PlayingAnims>,
    mut vis: Query<&mut Visibility>,
    bars: Query<&Progress>,
    mut brushes: Query<&mut AdditiveBrush>,
    mut was: Local<bool>,
) {
    use f13_core::killer_abilities::Ability;
    if play.is_none() || hud.built.as_ref().is_none_or(|b| !b.2) {
        return;
    }
    let Some(k) = jasons.iter().next() else { return };
    let rage = k.abilities.rage_active;
    if rage == *was {
        return;
    }
    *was = rage;
    if !rage {
        return;
    }
    let now = time.elapsed_secs();
    playing.play("KillerHUDWidget", "RageActive", now, false);
    playing.play("KillerAbilityWidget", "OnRageActiveAnim", now, false);
    playing.play("RageWidget", "OnRageActive", now, false);
    if let Some(mut v) = hud.names.get("KillerAbilityWidget.FrameRage").and_then(|&e| vis.get_mut(e).ok()) {
        v.set_if_neq(Visibility::Inherited);
    }
    for ability in Ability::ALL {
        let Some(bar) = hud.names.get(&format!("KillerAbilityWidget.{ability:?}Bar")).and_then(|&e| bars.get(e).ok().map(|b| (e, b.image))) else { continue };
        for (e, smoke, brightness) in [(bar.0, 0.0, 0.0), (bar.1, 1.0, 10.0)] {
            if let Ok(mut b) = brushes.get_mut(e) {
                b.smoke = smoke;
                b.brightness = brightness;
            }
        }
    }
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
    player: Option<Single<&Player, With<crate::controls::LocalPlayer>>>,
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
            f13_core::perception::noise_level(&v.noise, &c.stats, c.stance, c.sprinting, c.running, true, c.noise_perk())
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

/// The interaction dial while a hold interaction with pips runs (repairs): progress, the needle, the pips by state, the
/// next pip's mouse button. A hold without pips (a door's lock, a barricade) fills its prompt's hold ring instead
/// (SCInteractWidget::ShowHoldRing: only when its interactable has no pips; ui/prompts.rs).
fn dial(
    mut commands: Commands,
    hud: Res<Hud>,
    interaction: Query<&crate::interact::Interaction, With<crate::controls::LocalPlayer>>,
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
    let hold = interaction.single().ok().and_then(|i| i.hold.as_ref()).filter(|h| h.started && !h.pips.is_empty());
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

