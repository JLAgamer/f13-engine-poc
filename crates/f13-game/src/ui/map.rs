//! The minimap, the map screen (its objectives) and their icons.

use super::*;

/// Keeps the minimap on her: her place on the map, turned with the camera, the compass's N kept on north.
#[allow(clippy::too_many_arguments)]
pub(super) fn minimap(
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
pub(super) fn map_dir(area: [f32; 4], d: Vec3) -> Vec2 {
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
pub(super) struct MapIconNode;

/// An icon widget's image and size (design units): its first Image, laid out about the icon's point.
pub(super) fn icon_image(dir: &std::path::Path, widget: &str) -> Option<(String, Vec2, [f32; 4])> {
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
pub(super) const PLAYER_ARROW: &str = "PlayerArrow_Widget";

/// Jason's map's icons (ILLMinimap::UpdateObjectLocations 0x1402ae080 on SC_KillerMap_Widget, whose map isn't centred
/// on him or turned): each MapIcon's KillerIcon, and his own (PlayerArrow_Widget), in its IconPanel where its place
/// falls across the map, at the angle its widget's UpdateMapWidget leaves (SetRenderAngle). They take the map's
/// opacity (MapFade).
#[allow(clippy::too_many_arguments)]
pub(super) fn killer_map_icons(
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
pub(super) fn counselor_map_icons(
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
pub(super) fn objective_rows(
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
pub(super) fn fill_objective_row(w: &mut Widget, done: &[bool], four: Option<bool>) {
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
pub(super) fn map_screen(
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
pub(super) fn place_map_icons(
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
pub(super) fn map_icons(
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
