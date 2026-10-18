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

mod jason_hud;
mod map;
mod menus;
mod prompts;
mod widget_anims;
use jason_hud::*;
pub use map::*;
pub use menus::*;
pub use prompts::{Prompt, PromptSource, Prompts};
pub use widget_anims::*;

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

/// The pips' colors (SCInteractMinigame_Widget's defaults, all half transparent: the progress and the needle show
/// through), as FUN_140441e10 sets each pip's material Color by its state: waiting PipDefaultColor, active
/// PipActiveColor, hit PipHitColor until the progress is past it (then PipActiveColor), missed PipMissColor.
const PIP_WAITING: [f32; 4] = [0.212596, 0.203007, 0.23, 0.5];
const PIP_ACTIVE: [f32; 4] = [0.995, 1.0, 1.0, 0.5];
const PIP_HIT: [f32; 4] = [0.106459, 0.865, 0.284412, 0.5];
const PIP_MISSED: [f32; 4] = [0.854993, 0.030713, 0.017642, 0.5];

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
    /// The Jason menu (Jason_Select_Widget: lobby.rs JasonMenu) and its strengths' and weaknesses' rows
    /// (Attribute_Display).
    jason_menu: Option<Widget>,
    attribute: Option<Widget>,
    /// The grab kill picker (Kill_Select_Widget: killer_profile.rs KillMenu) and its buttons (GrabKillButtonLarge).
    kill_menu: Option<Widget>,
    kill_button: Option<Widget>,
    /// The weapon picker (WeaponSelectionMenu: killer_profile.rs WeaponMenu) and its entries (KillerWeaponListItem).
    weapon_menu: Option<Widget>,
    weapon_item: Option<Widget>,
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
        jason_menu: Widget::load(dir, "Jason_Select_Widget").map(|mut w| {
            // Not ported: customization points, the locked banner (all unlocked) and its action bar (the keys:
            // lobby.rs, killer_profile.rs).
            if let Some(h) = w.find_mut("MenuHeaderWidget") {
                let title = h.props["Text"].clone();
                if let Some(t) = h.find_mut("TitleText").filter(|_| title.is_object()) {
                    t.props["Text"] = title;
                }
            }
            // (AbilityUnlockPanelButton and AbilityOrderText: hidden in the game too, its graph never shows them.)
            for name in ["QuickPlayOverlayWidget", "MenuActionBarWidget_455", "CPBox", "LockedBorder", "AbilityUnlockPanelButton", "AbilityOrderText", "ActionMapPrev", "ActionMapNext"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            w
        }),
        attribute: Widget::load(dir, "Attribute_Display"),
        kill_menu: Widget::load(dir, "Kill_Select_Widget").map(|mut w| {
            // Not ported: purchases (all owned: the cost, level and purchase lines, the lock), customization points,
            // its weapon image and its action bar (the keys: killer_profile.rs).
            if let Some(h) = w.find_mut("MenuHeaderWidget") {
                let title = h.props["Text"].clone();
                if let Some(t) = h.find_mut("TitleText").filter(|_| title.is_object()) {
                    t.props["Text"] = title;
                }
            }
            for name in ["MenuActionBarWidget_148", "MetaRequirements", "LockIcon", "CPBox", "CurrentWeaponImg"] {
                if let Some(x) = w.find_mut(name) {
                    x.set_visibility(Vis::Collapsed);
                }
            }
            w
        }),
        kill_button: Widget::load(dir, "GrabKillButtonLarge"),
        weapon_menu: Widget::load(dir, "WeaponSelectionMenu").map(|mut w| {
            if let Some(h) = w.find_mut("MenuHeaderWidget") {
                let title = h.props["Text"].clone();
                if let Some(t) = h.find_mut("TitleText").filter(|_| title.is_object()) {
                    t.props["Text"] = title;
                }
            }
            // (Its action bar: the keys, killer_profile.rs.)
            if let Some(x) = w.find_mut("MenuActionBarWidget") {
                x.set_visibility(Vis::Collapsed);
            }
            w
        }),
        weapon_item: Widget::load(dir, "KillerWeaponListItem"),
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
    (perk_menu, perks, profiles, jason_menu, jason_list, kill_menu, killer_profiles, weapon_menu): (
        Option<Res<crate::perks::PerkMenu>>,
        Option<Res<crate::perks::Perks>>,
        Option<Res<crate::wardrobe::CounselorProfiles>>,
        Option<Res<crate::lobby::JasonMenu>>,
        Option<Res<crate::lobby::JasonList>>,
        Option<Res<crate::killer_profile::KillMenu>>,
        Option<Res<crate::killer_profile::KillerProfiles>>,
        Option<Res<crate::killer_profile::WeaponMenu>>,
    ),
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
    // Or the Jason menu.
    let menu_jason = open_lobby.and(jason_menu.as_ref()).and_then(|m| jason_list.as_ref()?.0.get(m.index).cloned()).filter(|_| hud.jason_menu.is_some());
    if let Some(l) = open_lobby {
        pocket_key = l.members.iter().map(|m| format!("lobby:{}:{}:{}:{:?}:{}:{}:{}:{}:{}", m.id, m.name, m.ready, m.preference, m.ping, m.talked, m.talking, mutes.as_ref().is_some_and(|x| x.0.contains(&m.name)), m.class)).collect();
        pocket_key.push(format!("ready focus {}", ready_focus.as_ref().is_none_or(|f| f.0)));
        pocket_key.push(format!("countdown:{:?}", l.countdown.map(|t| t.ceil() as i32)));
        if let Some(c) = &menu_class {
            let slots = profiles.as_ref().map(|p| p.perks.get(c).cloned().unwrap_or_default());
            pocket_key = vec![format!("counselor_menu:{c}:{slots:?}")];
        }
        if let Some(k) = &menu_jason {
            let picks = killer_profiles.as_ref().and_then(|p| p.grab_kills.get(&k.0).cloned());
            let weapon = killer_profiles.as_ref().map(|p| p.weapon_of(&k.0, &k.1));
            pocket_key = vec![format!("jason_menu:{}:{picks:?}:{weapon:?}", k.0)];
            if let Some(m) = weapon_menu.as_ref().filter(|_| hud.weapon_menu.is_some()) {
                pocket_key = vec![format!("weapon_menu:{}:{}:{weapon:?}", m.killer, m.index)];
            }
            if let Some(m) = kill_menu.as_ref().filter(|_| hud.kill_menu.is_some()) {
                pocket_key = vec![format!("kill_menu:{}:{}:{}:{picks:?}", m.killer, m.slot, m.index)];
            }
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
    } else if let (Some((_, killer)), Some(m), Some(t), Some(_)) = (&menu_jason, &weapon_menu, &hud.weapon_menu, open_lobby) {
        let mut t = t.clone();
        fill_weapon_menu(&hud, &mut t, m, killer);
        t
    } else if let (Some((name, killer)), Some(m), Some(t), Some(_)) = (&menu_jason, &kill_menu, &hud.kill_menu, open_lobby) {
        let mut t = t.clone();
        let picks = killer_profiles.as_ref().and_then(|p| p.grab_kills.get(name).cloned()).unwrap_or_default();
        let weapon = killer_profiles.as_ref().map_or_else(|| crate::killer_profile::own_weapons(killer).0, |p| p.weapon_of(name, killer));
        fill_kill_menu(&hud, &mut t, m, killer, &picks, &weapon);
        t
    } else if let (Some((name, killer)), Some(t), Some(_)) = (&menu_jason, &hud.jason_menu, open_lobby) {
        let mut t = t.clone();
        let picks = killer_profiles.as_ref().and_then(|p| p.grab_kills.get(name).cloned()).unwrap_or_default();
        let weapon = killer_profiles.as_ref().map_or_else(|| crate::killer_profile::own_weapons(killer).0, |p| p.weapon_of(name, killer));
        fill_jason_menu(&hud, &mut t, killer, &picks, &weapon);
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
    let preview = preview_image.map(|p| (p.counselor.clone(), p.wardrobe.clone(), p.emote.clone(), p.killer.clone()));
    let mut b = Builder { commands: &mut commands, assets: &assets, materials: &mut materials, hud: &hud, scale, text_scale: 1.0, content: [1.0; 4], names: &mut names, preview };
    let full = Rect { x: 0.0, y: 0.0, w: virt.x, h: virt.y };
    let laid = tree.layout(full);
    b.spawn(&laid, root, full, if board.is_some() { "ScoreboardWidget" } else if badge.is_some() { "MatchEnd_Badges" } else if ended.is_some() { "MatchEnd" } else if menu_jason.is_some() && weapon_menu.is_some() && open_lobby.is_some() && hud.weapon_menu.is_some() { "WeaponSelectionMenu" } else if menu_jason.is_some() && kill_menu.is_some() && open_lobby.is_some() && hud.kill_menu.is_some() { "Kill_Select_Widget" } else if menu_jason.is_some() && open_lobby.is_some() { "Jason_Select_Widget" } else if menu_class.is_some() && open_lobby.is_some() { "Counselor_Menu" } else if open_lobby.is_some() { "LobbyWidget" } else if jason { "KillerHUDWidget" } else if spectator { "SpectatorHUDWidget" } else { "CounselorHUDWidget" });
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
    preview: Option<(Handle<Image>, Handle<Image>, Handle<Image>, Handle<Image>)>,
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
            // (Jason_Select_Widget's PreviewImage: taken as the counselor's, its graph not read: PLACEHOLDER.)
            Some("Material'KillerPreviewMaterial'") => Some(3),
            _ => None,
        };
        if let Some(i) = capture {
            if let Some((counselor, wardrobe, emote, killer)) = self.preview.clone() {
                self.commands.entity(e).insert(ImageNode::new([counselor, wardrobe, emote, killer][i].clone()).with_color(linear(c)));
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
            // (Its end + 1.5 degrees, as the game's: DAT_142179354 over the 1/360 scale.)
            PipState::Hit if progress <= (p.position + p.width + 1.5) / 360.0 => PIP_HIT,
            PipState::Hit | PipState::Active => PIP_ACTIVE,
            PipState::Missed => PIP_MISSED,
            PipState::Waiting => PIP_WAITING,
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

