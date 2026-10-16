//! Jason's HUD: his knives and traps, his abilities' bars and glow, their unlock popups, the Morph cursor and his
//! rage.

use super::*;

/// Jason's HUD: his knives and traps left, each ability's unlock bar filling and its icon's glow while it's ready
/// (its charge bar as it recharges), and his rage building.
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
pub(super) fn jason_hud(
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
    let combat = j.combat || matches!(j.state, crate::jason::JasonState::Killing { .. }) || j.remote_state == "Killing";
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
    // (A client's Jason: the server's aim, replicated: net.rs NetJason knife_aim.)
    let aiming = j.remote.as_ref().map_or(matches!(j.state, crate::jason::JasonState::Throwing { release: None, .. }), |r| r.knife_aim);
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
                // (A client's Jason: the server's bar.)
                _ => j.remote.and_then(|r| r.shift_bar).unwrap_or_else(|| a.recharge_fraction(ability)),
            },
            Ability::Stalk => match &j.stalk {
                Some(s) => s.timer / tuning.as_ref().map_or(20.0, |t| t.stalk_active_time).max(1e-3),
                None => j.remote.and_then(|r| r.stalk_bar).unwrap_or_else(|| a.recharge_fraction(ability)),
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
            Ability::Shift => matches!(j.state, crate::jason::JasonState::Shifting { .. }) || j.remote.is_some_and(|r| r.shift_bar.is_some()),
            Ability::Stalk => j.stalk.is_some() || j.remote.is_some_and(|r| r.stalk_bar.is_some()),
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
pub(super) fn ability_popup(
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
pub(super) fn morph_cursor(
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
pub(super) fn rage_hud(
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
