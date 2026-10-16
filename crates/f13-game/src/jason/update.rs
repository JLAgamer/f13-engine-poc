//! His tick (update_jason): his state machine, the AI's choices, his moves, attacks, grabs and kills.

use super::*;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn update_jason(
    mut commands: Commands,
    (time, anims, water, drown): (Res<Time>, Res<crate::Anims>, Option<Res<crate::water::Water>>, Option<Res<DrownKillCamera>>),
    (tuning, target): (Option<Res<KillerTuning>>, Res<JasonTarget>),
    ptuning: Res<PlayerTuning>,
    nav: Option<ResMut<Nav>>,
    (killer, counselor_model): (Option<Res<KillerCharacter>>, Option<Res<crate::character::CounselorCharacter>>),
    collision: crate::collision::Collision,
    (mut rng, mut struggle, mut controls, config): (ResMut<crate::GameRng>, ResMut<Struggle>, Option<ResMut<crate::jason_player::JasonControls>>, Res<JasonConfig>),
    (mut jasons, morph_points, mut morph_map, grab_kills): (
        Query<(Entity, &mut Jason, &mut Transform, (Option<&HitJason>, Option<&KillJason>), Option<&mut crate::senses::Knowledge>), Without<Player>>,
        Option<Res<MorphPoints>>,
        Option<ResMut<crate::jason_player::MorphMap>>,
        Res<GrabKillsAvailable>,
    ),
    (inventories, items, mut fuse_boxes, (mut grids, mut phone_boxes, mut phones, mut radios, knife_pickups), vehicles): (
        Query<&crate::interact::Inventory>,
        Query<&crate::props::Item>,
        Query<(Entity, &mut crate::power::FuseBox)>,
        (
            Option<ResMut<crate::power::PowerGrids>>,
            Query<(Entity, &mut crate::police::PhoneBox)>,
            Query<&mut crate::police::Phone>,
            Query<(Entity, &mut crate::radio::Radio)>,
            Query<(Entity, &crate::props::KnifePickup, &GlobalTransform, Has<crate::props::PickedUp>)>,
        ),
        Query<(&crate::props::Vehicle, &GlobalTransform)>,
    ),
    mut players: Query<(Entity, &mut Player, &mut Transform, Option<&crate::vehicle::Driving>, Option<&mut crate::hiding::Hiding>, Has<crate::items::Blocking>), Without<Jason>>,
    mut doors: Query<(Entity, &mut crate::doors::Door, &GlobalTransform)>,
    (mut barricades, mut walls): (Query<&mut crate::windows::Barricade>, Query<(Entity, &mut crate::doors::Wall, &GlobalTransform)>),
    (mut spots, cabinets, mut placed_kills): (Query<(Entity, &mut crate::hiding::HidingSpot, &GlobalTransform)>, Query<&GlobalTransform, With<crate::props::Cabinet>>, Query<&mut crate::placed_kills::PlacedKill>),
    mut rigs: Query<(&mut Rig, &ChildOf, &Transform), (Without<Jason>, Without<Player>)>,
    (globals, navmesh, leaves, parents, interact_limits, special_moves, aligns): (
        Query<&GlobalTransform>,
        Option<Res<crate::navigation::Navmesh>>,
        Query<(Entity, &ChildOf), With<crate::navigation::NotOnNavmesh>>,
        Query<&ChildOf>,
        Query<&crate::interact::InteractLimits>,
        Option<Res<crate::interact::SpecialMoves>>,
        Query<&AlignTo>,
    ),
) {
    let navmesh = navmesh.as_deref().map(|n| &*n.0);
    let (Some(t), Some(mut nav), Some(killer)) = (tuning, nav, killer) else { return };
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    // Broken doors' leaves are gone, and a swinging leaf doesn't shove him; an open one stops him like a wall.
    let open_leaves: Vec<Entity> = leaves
        .iter()
        .filter(|(_, pivot)| parents.get(pivot.parent()).ok().and_then(|root| doors.get(root.parent()).ok()).is_some_and(|(_, d, _)| d.broken || d.angle != d.target))
        .map(|(e, _)| e)
        .collect();
    let (grid, lists) = collision.without(|e| open_leaves.contains(&e));
    let level = (grid, lists.as_slice());
    // The counselors' capsules (on foot, not hidden, alive) block him while he moves freely (their CapsuleComponents
    // block the Pawn channel); his grabs, kills and other moves are placed as their animations take him.
    let counselors: Vec<Vec3> = players.iter().filter(|(_, p, _, car, hiding, _)| p.counselor.alive() && car.is_none() && hiding.is_none()).map(|(_, _, xf, ..)| xf.translation).collect();
    let Some(Ok((pe, mut p, mut pxf, in_car, mut hiding, blocking))) = target.0.map(|t| players.get_mut(t)) else { return };
    let driving = in_car.is_some();
    // Her car's driver door (the game's DriverGrab context kill, where he stands to pull the driver out) and its speed.
    // (The driver's door: only she is pulled out.)
    let car_door = in_car.filter(|d| d.seat.is_none()).and_then(|d| vehicles.get(d.vehicle).ok()).map(|(v, g)| {
        let at = v.driver_grab.map_or_else(
            || {
                let seat = v.driver_seat;
                let out = Vec3::new(seat.x, 0.0, seat.z).normalize_or_zero();
                Transform::from_translation(g.transform_point(seat + out))
            },
            |grab| crate::hiding::HidingSpot::world(g, &grab),
        );
        (at, v.speed)
    });
    for (je, mut j, mut xf, (hit, kill_by), mut know) in &mut jasons {
        let free = matches!(j.state, JasonState::Waiting(_) | JasonState::Hunting | JasonState::Swinging { .. } | JasonState::Flinching { .. } | JasonState::Stunned { .. } | JasonState::Throwing { .. } | JasonState::PlacingTrap { .. });
        let others = f13_core::collision::TriList::new(if free {
            counselors.iter().filter(|at| at.distance(xf.translation) < 4.0).flat_map(|&at| crate::player::pawn_prism(at, crate::player::CAPSULE_RADIUS_M, crate::player::CAPSULE_HALF_HEIGHT_M)).collect()
        } else {
            vec![]
        });
        let world = (&level, &others);
        let kill_by = kill_by.map(|k| (k.take, k.by));
        let align = aligns.get(je).ok().copied();
        // Under water (his tick 0x14040d900): in the water with his capsule's top under its surface; there he walks
        // the bottom at UnderwaterSpeed scaled by his feet's depth (GetUnderwaterDepthModifier: up to
        // UnderwaterFullSpeedDepth) from MaxSlowRunSpeed, and he's unseen outside his special moves
        // (NativeOnUnderwaterUpdate: his mesh, mask and weapon hidden).
        let centre = xf.translation + Vec3::Y * t.capsule.half_height;
        let surface = water.as_ref().and_then(|w| w.surface_at(centre));
        let under = surface.is_some_and(|s| centre.y + t.capsule.half_height <= s);
        if under != j.underwater {
            j.underwater = under;
            info!("Jason: {} the water", if under { "under" } else { "out of" });
            // OnRep_UnderWater (0x1403fdb30): going under drops a counselor he holds, unless in a context kill
            // (MULTICAST_DropCounselor 0x1403f6450: his grab and its animation end). In the water she isn't moved
            // or staggered (SCCounselorCharacter::OnDropped 0x1403c1610 with bInWater): her grab and its animation
            // end and she swims (or stands, in shallow water) from where he held her.
            if under && matches!(j.state, JasonState::Holding { .. }) {
                info!("Jason: dropped her going under");
                commands.entity(pe).remove::<(Held, PlayAction)>();
                commands.entity(je).remove::<PlayAction>();
                j.state = JasonState::Hunting;
            }
            // And ends a Shift.
            if under && matches!(j.state, JasonState::Shifting { .. }) {
                info!("Jason: Shift over (under the water)");
                j.state = JasonState::Hunting;
            }
        }
        j.underwater_speed = surface.filter(|_| under).map(|s| {
            let depth = ((s - xf.translation.y) * CM).clamp(0.0, t.underwater_full_speed_depth);
            let m = (depth / t.underwater_full_speed_depth).clamp(0.0, 1.0);
            (t.underwater_speed - t.slow_run_speed) * m + t.slow_run_speed
        });
        let hide = under && !matches!(j.state, JasonState::Killing { .. });
        if hide != j.under_hidden {
            j.under_hidden = hide;
            commands.entity(je).insert(if hide || j.faded { Visibility::Hidden } else { Visibility::Inherited });
        }
        j.stun_immune = (j.stun_immune - dt).max(0.0);
        j.knife_cooldown = (j.knife_cooldown - dt).max(0.0);
        // Stalk runs down (slower standing still); the teleport effect fades him while he shifts or morphs.
        let moving = j.velocity.length_squared() > 0.0;
        if let Some(s) = j.stalk.as_mut() {
            if !s.tick(dt, moving, t.idle_stalk_modifier) {
                info!("Jason: Stalk over");
                j.stalk = None;
            }
        }
        // The Shift's rush (a loop) lasts as long as the Shift, however it ends.
        if j.shifting() != j.shift_sound {
            j.shift_sound = j.shifting();
            if j.shift_sound {
                crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorLoop(crate::audio::ActorLoop { actor: je, prop: SHIFT_CUE }));
            } else {
                crate::net_world::shout(&mut commands, crate::net_world::Shout::StopLoop(crate::audio::StopLoop { actor: je, prop: SHIFT_CUE }));
            }
        }
        let effect = matches!(j.state, JasonState::Shifting { .. } | JasonState::Morphing { .. });
        j.fade.tick(dt, effect);
        let faded = j.fade.0 < 0.5;
        if faded != j.faded {
            j.faded = faded;
            commands.entity(je).insert(if faded || j.under_hidden { Visibility::Hidden } else { Visibility::Inherited });
        }
        // A counselor's item hit him.
        if let Some(h) = hit {
            commands.entity(je).remove::<HitJason>();
            commands.trigger(crate::match_flow::JasonDamaged);
            // Her weapon's damage (the item stuns below score their own).
            if matches!(h.stun, None | Some(Stun::Weapon { .. })) {
                if let Some(who) = h.by {
                    commands.trigger(Score { who, event: "DamageJason" });
                }
            }
            let can_stun = j.stunnable();
            // Blocking (SCKillerCharacter's TakeDamage 0x14040b600): a hit whose damage type can be blocked
            // (bCanBeBlocked: only BaseWeaponStun among his stuns; a shotgun never) doesn't stun him, takes
            // BlockedHitDamageModifier of its damage (his health isn't modelled), and plays one of his
            // BlockingHitMontages (FUN_1402fdbb0 with BlockData.bIsBlocking).
            let blocked = j.blocking && matches!(h.stun, None | Some(Stun::Weapon { .. }));
            // His health: the hit's damage (blocked: x BlockedHitDamageModifier), down to MinimumHealth; the mask
            // pops off once the hit takes him to MaskPopOffThreshold: MaskKnockedOffStun, KnockMaskOff to her.
            let damage = h.damage * if blocked { t.blocked_damage } else { 1.0 };
            let mut stun_now = h.stun;
            if damage > 0.0 {
                let pops = j.mask_on && j.health - damage <= t.mask_threshold;
                j.health = (j.health - damage).clamp(t.min_health, t.max_health);
                info!("Jason: {damage:.1} damage (health {:.1})", j.health);
                if pops {
                    j.mask_on = false;
                    info!("Jason: his mask knocked off");
                    if let Some(who) = h.by {
                        commands.trigger(Score { who, event: "KnockMaskOff" });
                    }
                    let left = matches!(h.stun, Some(Stun::Weapon { left: true }));
                    stun_now = Some(Stun::MaskOff { left });
                }
            }
            // His mask off and the sweater's stun on him (0x1291), her melee hit (not a gun; the damage type's
            // bCanBeBlocked: not a stun type): his JasonKillObject, his kill stance (TakeDamage 0x14040b600).
            let melee = h.by.is_some() && h.damage > 0.0 && matches!(h.stun, None | Some(Stun::Weapon { .. }));
            if melee && !j.mask_on && j.sweater_stunned && !j.dead() {
                info!("Jason: in his kill stance");
                j.health = t.min_health;
                j.state = JasonState::KillStance { t: 0.0 };
                j.velocity = Vec2::ZERO;
                commands.entity(je).insert(PlayAction(KILL_STANCE.0.into()));
                stun_now = None;
            }
            // (The mask's and the sweater's stuns: ApplyStun with the class, whatever he's doing.)
            let can_stun = can_stun || matches!(stun_now, Some(Stun::MaskOff { .. } | Stun::Sweater));
            if stun_now == Some(Stun::Sweater) {
                j.sweater_stunned = true;
            }
            match stun_now {
                _ if j.kneeling() => {}
                _ if blocked => {
                    let anim = BLOCK_REACTIONS[rng.0.index(BLOCK_REACTIONS.len())];
                    info!("Jason: blocked the hit");
                    j.state = JasonState::Flinching { t: 0.0, anim };
                    commands.entity(je).insert(PlayAction(anim.into()));
                }
                Some(stun) if can_stun => {
                    info!("Jason: stunned ({stun:?}, {:.1}s)", stun.seconds() * h.stun_scale);
                    if let Some(event) = stun.score_event() {
                        if let Some(who) = h.by {
                            commands.trigger(Score { who, event });
                        }
                        crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: pe, prop: "VO:JasonStun" }));
                    }
                    if matches!(j.state, JasonState::Holding { .. }) {
                        let in_water = water.as_ref().is_some_and(|w| w.surface_at(xf.translation + Vec3::Y * t.capsule.half_height).is_some());
                        drop_counselor(&mut pxf, &xf, t.capsule.half_height, in_water, &world);
                        release(&mut commands, pe, GRAB_BREAK, counselor_model.as_deref());
                    }
                    j.state = JasonState::Stunned { t: 0.0, stun };
                    j.stun_scale = h.stun_scale;
                    j.velocity = Vec2::ZERO;
                    commands.entity(je).insert(PlayAction(stun.anims().0.into()));
                }
                _ if matches!(j.state, JasonState::Hunting) => {
                    let front = angle_to(xf.rotation, Vec2::new(h.from.x - xf.translation.x, h.from.z - xf.translation.z)) < 90.0;
                    info!("Jason: hit, not stunned");
                    let anim = if front { FLINCH_FRONT } else { FLINCH_BACK };
                    j.state = JasonState::Flinching { t: 0.0, anim };
                    commands.entity(je).insert(PlayAction(anim.into()));
                }
                _ => info!("Jason: hit, shrugged it off ({:?})", j.state),
            }
        }
        let to = pxf.translation;
        let flat = Vec2::new(to.x - xf.translation.x, to.z - xf.translation.z);
        let dist = flat.length();
        let angle = angle_to(xf.rotation, flat);
        let alive = p.counselor.alive();
        // The player's Jason: this step's input; he faces and aims where they look.
        let player_input = controls.as_mut().map(|c| c.take());
        let face_dir = player_input.map_or(flat, |i| i.forward);
        let state = j.state.clone();
        match state {
            JasonState::Waiting(left) => {
                j.state = if left > dt { JasonState::Waiting(left - dt) } else { JasonState::Hunting };
                if j.state == JasonState::Hunting {
                    info!("Jason: hunting ({:.0} m away)", dist);
                }
            }
            JasonState::Hunting => {
                if !alive {
                    j.state = JasonState::Done;
                    continue;
                }
                j.cooldown -= dt;
                j.hunted += dt;
                // Her in a car that's (nearly) stopped, him at its driver door: he pulls her out into his grab
                // (DriverGrab; the car door only works under MinUseCarDoorSpeed). The AI does it when he gets there,
                // the player when they grab.
                if let Some((door, speed)) = car_door {
                    let at_door = Vec2::new(door.translation.x - xf.translation.x, door.translation.z - xf.translation.z).length() < CAR_DOOR_REACH_M;
                    let wants = player_input.map_or(true, |i| i.presses.grab);
                    if at_door && speed.abs() < MIN_USE_CAR_DOOR_SPEED && wants && alive {
                        info!("Jason: pulling Vanessa out of the car");
                        struggle.wiggle = default();
                        j.velocity = Vec2::ZERO;
                        p.velocity = Vec2::ZERO;
                        commands.entity(pe).remove::<crate::vehicle::Driving>().insert((Held, Visibility::Inherited));
                        if killer.info.animations.contains_key(CAR_GRAB) {
                            // The paired take from DriverGrab: she rides his GrabSocket out of the seat.
                            *xf = killer.place(CAR_GRAB, door).0;
                            snap_rig(&mut rigs, je);
                            j.state = JasonState::PullingOut { t: 0.0 };
                            commands.entity(je).insert(PlayAction(CAR_GRAB.into()));
                            commands.entity(pe).insert(PlayAction(CAR_GRAB.into()));
                        } else {
                            pxf.translation = Vec3::new(door.translation.x, xf.translation.y, door.translation.z);
                            j.state = JasonState::Holding { t: 0.0 };
                            j.fast_walk = false;
                            commands.entity(je).insert(PlayAction(GRAB_ENTER.into()));
                            commands.entity(pe).insert(PlayAction(GRABBED_ENTER.into()));
                        }
                        continue;
                    }
                }
                // Under water, deep enough, a swimming counselor in reach (CanInteractUnderwater 0x1403e6f60, its
                // victims FUN_1403e87f0): his drowning kill (OnInteractUnderwater 0x1403f9c10). The AI does it at
                // once, the player with interact.
                if let Some(s) = surface.filter(|_| j.underwater && alive && !driving && hiding.is_none()) {
                    let feet_depth = (s - xf.translation.y) * CM;
                    let her_centre = pxf.translation + Vec3::Y * crate::player::CAPSULE_HALF_HEIGHT_M;
                    // The UnderwaterInteractComponent rides UnderwaterVisualRoot at the surface over him (Jason_Base's
                    // OnUnderWaterUpdate): its DistanceLimit is his distance to it plus 50 cm.
                    let reach = (s + UNDERWATER_INTERACT_HEIGHT_M - centre.y).abs() + UNDERWATER_REACH_PAD_M;
                    // DrowningDepthTrace: ground within 350 cm under her.
                    let ground = f13_core::collision::segment_hit(&world, her_centre, her_centre - Vec3::Y * DROWNING_DEPTH_TRACE_M, 0.5).is_some();
                    let victim = p.counselor.stance == f13_core::counselor::Stance::Swimming
                        && ground
                        && feet_depth >= t.underwater_kill_depth
                        && her_centre.distance(centre) <= reach;
                    let wants = player_input.map_or(true, |i| i.presses.interact);
                    debug!("drown check: feet {feet_depth:.0} cm, swimming {}, ground {ground}, dist {:.2} reach {reach:.2}", p.counselor.stance == f13_core::counselor::Stance::Swimming, her_centre.distance(centre));
                    if feet_depth >= t.underwater_full_speed_depth && victim && wants && killer.info.animations.contains_key(DROWN_KILL.0) {
                        // The kill rides his mesh (attached, snapped), moved to her: he takes its place, his feet
                        // there (the special move's desired transform adds his half height), turned as his mesh is;
                        // she stays.
                        let mesh_rot = xf.rotation * killer.rig_transform().rotation;
                        let spot = Transform { translation: her_centre + Vec3::Y * 0.01, rotation: crate::character::flat(Transform::from_rotation(mesh_rot * crate::character::FACING.inverse())).rotation, ..default() };
                        info!("Jason: drowning Vanessa ({} under)", feet_depth as i32);
                        struggle.wiggle = default();
                        j.velocity = Vec2::ZERO;
                        p.velocity = Vec2::ZERO;
                        j.kill_move = DROWN_KILL;
                        j.kill_score = Some((format!("{DROWN_KILL_KEY}_C"), "ContextKillComponent"));
                        j.kill_holds = false;
                        *xf = killer.place(DROWN_KILL.0, spot).0;
                        snap_rig(&mut rigs, je);
                        j.state = JasonState::Killing { t: 0.0 };
                        // Its camera spline, on the kill (at her, turned as his mesh).
                        let camera = Transform::from_translation(her_centre).with_rotation(mesh_rot) * drown.as_ref().map_or(Transform::IDENTITY, |d| d.0);
                        debug!("drowning kill: her {:.2}, him {:.2} facing {:.2}, camera {:.2} facing {:.2}", pxf.translation, xf.translation, xf.rotation * Vec3::Z, camera.translation, camera.rotation * Vec3::X);
                        commands.entity(je).insert((PlayAction(DROWN_KILL.0.into()), crate::kill_camera::KillCam::at(DROWN_KILL_KEY, camera).of(pe)));
                        commands.entity(pe).insert((Held, PlayAction(DROWN_KILL.1.into())));
                        continue;
                    }
                }
                // The player's Jason (jason_player.rs): what they pressed, else the AI below.
                if let Some(i) = player_input {
                    let p = i.presses;
                    let moving = (j.velocity.length_squared() > 0.0).then(|| Vec3::new(j.velocity.x, 0.0, j.velocity.y).normalize_or_zero());
                    let centre = xf.translation + Vec3::Y * t.capsule.half_height;
                    j.focus = killer_focus(&doors, &walls, &interact_limits, centre, i.look, xf.rotation * Vec3::Z, moving, j.focus);
                    let blockers = Blockers { underwater: j.underwater, ..Blockers::default() };
                    let ready = |know: &Option<Mut<crate::senses::Knowledge>>, a: Ability| know.as_ref().is_some_and(|k| k.abilities.is_available(a, &blockers));
                    if p.shift && ready(&know, Ability::Shift) {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Shift);
                        }
                        info!("Jason: Shift");
                        j.state = JasonState::Shifting { shift: Shift::start() };
                        j.velocity = Vec2::ZERO;
                        continue;
                    }
                    // KLR_Morph_PC (FUN_1403e4a70): his map out for a Morph (not over his plain map), or put away.
                    if let Some(map) = morph_map.as_deref_mut() {
                        if p.morph {
                            if map.morph {
                                map.morph = false;
                                map.open = false;
                            } else if !map.open && ready(&know, Ability::Morph) {
                                map.morph = true;
                                map.open = true;
                                map.target = None;
                            }
                        }
                        // AttemptMorph: the place picked on the map; Morph (SetMorphLocation) takes the morph point
                        // nearest it, the map put away, or nothing happens.
                        if let Some(at) = map.target.take().filter(|_| map.morph && ready(&know, Ability::Morph)) {
                            if let Some(to) = morph_points.as_ref().and_then(|m| m.pick(at)) {
                                if let Some(k) = know.as_mut() {
                                    k.abilities.consume(Ability::Morph);
                                }
                                info!("Jason: Morph");
                                map.morph = false;
                                map.open = false;
                                j.state = JasonState::Morphing { t: 0.0, to };
                                crate::net_world::shout(&mut commands, crate::net_world::Shout::Distortion(crate::jason::Distortion { at: xf.translation, dest: Some(to.translation) }));
                                j.velocity = Vec2::ZERO;
                                continue;
                            }
                            info!("Jason: no morph point near that place");
                        }
                    }
                    // KLR_Stalk_PC (FUN_1403e4df0): not with his map out; on, it's cancelled (SERVER_CancelStalk
                    // 0x1404046d0: bIsStalkActive off, its recharge timer back to 0: no refund); else used if available.
                    let map_out = morph_map.as_deref().is_some_and(|m| m.open);
                    if p.stalk && !map_out && j.stalk.is_some() {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Stalk);
                        }
                        info!("Jason: Stalk cancelled");
                        j.stalk = None;
                    } else if p.stalk && !map_out && ready(&know, Ability::Stalk) {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Stalk);
                        }
                        info!("Jason: Stalk");
                        j.stalk = Some(Stalk::start(t.stalk_active_time));
                        crate::net_world::shout(&mut commands, crate::net_world::Shout::Distortion(crate::jason::Distortion { at: xf.translation, dest: None }));
                    }
                    // KLR_FastWalk_PC: OnStartFastWalk leaves combat stance, then SetRunning; OnStopFastWalk clears it.
                    // SCKillerCharacter's SetRunning (0x140407400) won't turn it on while he holds her (GrabbedCounselor).
                    let holding = matches!(j.state, JasonState::Holding { .. } | JasonState::Killing { .. } | JasonState::PullingOut { .. });
                    match p.fast_walk {
                        Some(true) if holding => {}
                        Some(true) => {
                            if j.combat {
                                j.combat = false;
                                info!("Jason: combat stance false (fast walk)");
                            }
                            j.fast_walk = true;
                        }
                        Some(false) => j.fast_walk = false,
                        None => {}
                    }
                    // GLB_Combat_PC: in or out of combat stance.
                    if p.combat {
                        j.combat = !j.combat;
                        info!("Jason: combat stance {}", j.combat);
                    }
                    // KLR_KnifeThrow held (PressKnifeThrow): he takes a knife out and aims until it's let go.
                    if p.knife && j.knives > 0 && j.knife_cooldown <= 0.0 {
                        info!("Jason: aiming a knife ({} left)", j.knives);
                        j.state = JasonState::Throwing { t: 0.0, thrown: false, release: None };
                        j.velocity = Vec2::ZERO;
                        commands.entity(je).insert(PlayAction(KNIFE_SELECT.into()));
                        continue;
                    }
                    if p.trap && j.traps > 0 && killer.info.animations.contains_key(TRAP_SET) {
                        info!("Jason: setting a trap ({} left)", j.traps);
                        j.state = JasonState::PlacingTrap { t: 0.0 };
                        commands.entity(je).insert(PlayAction(TRAP_SET.into()));
                        continue;
                    }
                    // E on a closed door (SCDoor::OnInteract 0x140351150, the killer's press): raged, he bursts
                    // through it whatever its lock; locked or barricaded, the LockedDoorSound; else he opens it.
                    if let Some(Focus::Door(de)) = j.focus.filter(|_| p.interact) {
                        if let Ok((_, d, g)) = doors.get(de) {
                            let raged = know.as_ref().is_some_and(|k| k.abilities.rage_active);
                            let barred = barricades.iter().any(|b| b.door == Some(de) && b.enabled);
                            let (state, anim) = if raged {
                                info!("Jason: bursting through a door (raged)");
                                (JasonState::BurstingDoor { door: de, t: 0.0 }, DOOR_BURST)
                            } else if d.locked || barred {
                                crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: de, prop: "LockedDoorSound" }));
                                continue;
                            } else {
                                info!("Jason: opening a door");
                                let anim = if crate::doors::Door::is_inside(g, xf.translation) { DOOR_OPEN_INSIDE } else { DOOR_OPEN_OUTSIDE };
                                (JasonState::OpeningDoor { door: de, t: 0.0, anim }, anim)
                            };
                            j.velocity = Vec2::ZERO;
                            j.state = state;
                            commands.entity(je).insert(PlayAction(anim.into()));
                            continue;
                        }
                    }
                    // A fuse box at hand: his Jason_Generator_Destruction move cuts its power.
                    if p.interact && j.cooldown <= 0.0 {
                        let here = xf.translation;
                        if let Some((fe, fuse)) = fuse_boxes.iter().filter(|(_, f)| !f.destroyed).find(|(_, f)| Vec2::new(f.interact.x - here.x, f.interact.z - here.z).length() * 100.0 <= f.reach + 100.0) {
                            info!("Jason: breaking a fuse box");
                            // OnInteract (0x14038c950): its DestroyScoreEvent as the destroy move begins.
                            commands.trigger(crate::match_flow::ClassScore::killer(FUSE_BOX_CLASS, "DestroyScoreEvent"));
                            let to = fuse.interact - xf.translation;
                            xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
                            j.state = JasonState::BreakingFuse { t: 0.0, fuse: fe, done: false };
                            j.velocity = Vec2::ZERO;
                            commands.entity(je).insert(PlayAction(BREAK_FUSE.into()));
                            continue;
                        }
                        let near = |at: Vec3, reach: f32| Vec2::new(at.x - here.x, at.z - here.z).length() * 100.0 <= reach + 100.0;
                        // Throwing knives lying there ([`knife_pickup_at`]): their AlignmentSpecialMove
                        // (Jason_ThrowingKnifePickup) plays his take; PerformPickup at its KnifePickup driver window.
                        if let Some(ke) = knife_pickup_at(&knife_pickups, &interact_limits, centre, i.look) {
                            info!("Jason: picking up throwing knives");
                            // Jason_ThrowingKnifePickup (SCSpecialMove_SoloMontage, bInterpToInteraction): onto its
                            // spot over InterpolationTime first (the state's clock runs from minus that), then the take.
                            let params = special_moves.as_deref().map_or_else(Default::default, |m| m.of("ThrowingKnifePickup_C", "AlignmentSpecialMove"));
                            if let Ok((_, k, kg, _)) = knife_pickups.get(ke) {
                                let to = crate::interact::standing_on(&xf, kg.compute_transform() * k.spot);
                                commands.entity(je).insert(AlignTo { from: *xf, to, params });
                            }
                            j.state = JasonState::BreakingFuse { t: -params.interp_time, fuse: ke, done: false };
                            j.velocity = Vec2::ZERO;
                            continue;
                        }
                        // A radio (SCRadio::OnInteract: his Destroy_SpecialMoveHandler; CanInteractWith: not broken).
                        if let Some((re, r)) = radios.iter().find(|(_, r)| !r.broken && near(r.interact, r.reach)) {
                            info!("Jason: breaking a radio");
                            let to = r.interact - xf.translation;
                            xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
                            j.state = JasonState::BreakingFuse { t: 0.0, fuse: re, done: false };
                            j.velocity = Vec2::ZERO;
                            commands.entity(je).insert(PlayAction(crate::radio::BREAK_TAKE.into()));
                            continue;
                        }
                        if let Some((be, b)) = phone_boxes.iter().find(|(_, b)| b.breakable() && near(b.interact, b.reach)) {
                            info!("Jason: breaking the phone box");
                            let to = b.interact - xf.translation;
                            xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
                            j.state = JasonState::BreakingFuse { t: 0.0, fuse: be, done: false };
                            j.velocity = Vec2::ZERO;
                            commands.entity(je).insert((PlayAction(BREAK_FUSE.into()), ActionSounds(BREAK_PHONE_BOX)));
                            continue;
                        }
                    }
                    // CMBT_KLR_Block (right mouse in combat stance): OnStartBlock while CanBlock (FUN_1402ded00: not
                    // attacking, stunned, interacting or in a special move: here, hunting), OnEndBlock as it's let go.
                    // Leaving combat stance ends it (SetCombatStance 0x1402e94e0 -> vt+0xea8 0x1402e6e20: bIsBlocking
                    // cleared, OnRep_Block).
                    let block = j.combat && i.block_held;
                    if block != j.blocking {
                        j.blocking = block;
                        info!("Jason: {}", if block { "blocking" } else { "block ends" });
                    }
                    // KLR_Grab: not in combat stance (FUN_1403e6e80), where the button blocks.
                    if j.cooldown <= 0.0 && p.grab && !j.combat {
                        info!("Jason: grab");
                        j.state = JasonState::Reaching { t: 0.0 };
                        commands.entity(je).insert(PlayAction(GRAB_STARTUP.into()));
                        continue;
                    }
                    // CMBT_KLR_Attack (0x1403f89a0): his best interactable an SCInteractComponent with
                    // bCanAttackToInteract (the doors' and walls'), he breaks it down when he may (FUN_14040fd00 a door:
                    // closed and intact, the check at 90 degrees; FUN_14040fdd0 a wall: standing, the check at
                    // MaxAttackInteractAngle): bWantsToBreakDown, BreakDownFrom{Inside,Outside}. Held, the loop goes
                    // on; let go (0x1403fec50 clears bWantsToBreakDown), it aborts at the next loop.
                    if p.attack && !j.blocking {
                        let her = alive.then_some(pxf.translation);
                        let breaks = match j.focus {
                            Some(Focus::Door(de)) => doors.get(de).ok().filter(|(_, d, _)| !d.open && !d.broken).is_some_and(|(_, d, g)| {
                                let side = crate::doors::Door::is_inside(g, xf.translation);
                                attack_interact(i.forward, xf.translation, g.transform_point(d.interact), 90.0, t.attack_interact_counselor_m, her.map(|h| (h, crate::doors::Door::is_inside(g, h) == side)))
                            }),
                            Some(Focus::Wall(we)) => walls.get(we).ok().filter(|(_, w, _)| !w.broken).is_some_and(|(_, w, g)| {
                                let side = crate::doors::Wall::is_inside(g, xf.translation);
                                attack_interact(i.forward, xf.translation, g.transform_point(w.interact), t.attack_interact_angle, t.attack_interact_counselor_m, her.map(|h| (h, crate::doors::Wall::is_inside(g, h) == side)))
                            }),
                            None => false,
                        };
                        let state = match j.focus.filter(|_| breaks) {
                            Some(Focus::Door(de)) => Some(JasonState::BreakingDoor { door: de, t: 0.0, section: 0, hit: false }),
                            Some(Focus::Wall(we)) => Some(JasonState::BreakingWall { wall: we, t: 0.0, section: 0, hit: false }),
                            None => None,
                        };
                        if let Some(state) = state {
                            info!("Jason: breaking it down ({state:?})");
                            j.velocity = Vec2::ZERO;
                            j.state = state;
                            commands.entity(je).insert(PlayAction(BREAK_DOOR[0].into()));
                            continue;
                        }
                    }
                    // CMBT_KLR_Attack (0x1403f89a0 -> 0x1402e5aa0): the NeutralAttack; in combat stance the button's
                    // hold starts counting (the heavy attack after 0.1 s, below).
                    // Not while blocking (0x1402e5aa0).
                    if j.cooldown <= 0.0 && p.attack && !j.blocking {
                        info!("Jason: swing");
                        j.state = JasonState::Swinging { t: 0.0, hit: false, heavy: false };
                        j.attack_hold = j.combat.then_some(0.0);
                        commands.entity(je).insert(PlayAction(attack_take().into()));
                        continue;
                    }
                    // In combat stance he walks at MaxCombatSpeed, facing where the player looks.
                    let speed = if j.combat { t.combat_speed } else if j.fast_walk && t.can_run { t.run_speed } else { t.walk_speed };
                    j.running = speed == t.run_speed;
                    let face = j.combat.then_some(i.forward);
                    walk(&mut j, &mut xf, i.movement, speed, face, dt, &t, &world);
                    continue;
                }
                let sees = know.as_ref().is_none_or(|k| k.sees);
                // Near the goal but not getting any closer: as good as there.
                let stalled = j.stall > STALL_SECONDS;
                // Where to go: her (her car's driver door when she drives), if he sees her; else the hiding spot he's
                // after, the last clue, or somewhere.
                let mut goal = car_door.map_or(to, |(door, _)| door.translation);
                // Going for a hiding spot or a clue (not her, not wandering).
                let mut following = false;
                // PLACEHOLDER (the AI's choice; the game's Jason is a player): not seeing her, he goes to break a
                // working phone box (its fuse in) before anything else, as a player would to stop the call.
                let phone_box = if sees { None } else { phone_boxes.iter().find(|(_, b)| b.breakable()).map(|(e, b)| (e, b.interact, b.reach)) };
                // PLACEHOLDER (the AI's choice): out of knives and not seeing her, he goes for the nearest throwing
                // knives within KNIFE_SEEK_M and takes them as a player does (their AlignmentSpecialMove).
                // (Not those whose spot he couldn't get onto lately.)
                let unreachable = j.unreachable.filter(|u| now - u.1 < UNREACHABLE_SECONDS);
                let knife_post = if sees || phone_box.is_some() || j.knives > 0 {
                    None
                } else {
                    knife_pickups
                        .iter()
                        .filter(|(_, _, _, taken)| !taken)
                        .map(|(e, k, g, _)| (e, g.transform_point(k.interact), k.reach, g.compute_transform() * k.spot))
                        .filter(|(_, at, _, spot)| at.distance(xf.translation) <= KNIFE_SEEK_M && unreachable.is_none_or(|u| spot.translation.distance(u.0) > 0.5))
                        .min_by(|a, b| a.1.distance(xf.translation).total_cmp(&b.1.distance(xf.translation)))
                };
                if let (None, Some((ke, at, _, spot))) = (phone_box, knife_post) {
                    if j.target_spot.is_none() && !j.wander.is_some_and(|w| w.0 == spot.translation) {
                        info!("Jason: going for throwing knives ({:.0} m)", at.distance(xf.translation));
                        j.wander = Some((spot.translation, now));
                    }
                    goal = spot.translation;
                    following = true;
                    // Close to the special move's spot: it puts him on it (as a hiding spot's); stuck short of it, he
                    // leaves them for a while.
                    let near = Vec2::new(spot.translation.x - xf.translation.x, spot.translation.z - xf.translation.z).length() < SPECIAL_MOVE_REACH_M;
                    if stalled && !near {
                        info!("Jason: can't get to the throwing knives");
                        j.stall = 0.0;
                        j.unreachable = Some((spot.translation, now));
                    } else if near {
                        info!("Jason: picking up throwing knives (AI)");
                        j.stall = 0.0;
                        let params = special_moves.as_deref().map_or_else(Default::default, |m| m.of("ThrowingKnifePickup_C", "AlignmentSpecialMove"));
                        commands.entity(je).insert(AlignTo { from: *xf, to: crate::interact::standing_on(&xf, spot), params });
                        j.state = JasonState::BreakingFuse { t: -params.interp_time, fuse: ke, done: false };
                        j.velocity = Vec2::ZERO;
                        continue;
                    }
                } else if let Some((be, at, reach)) = phone_box {
                    if j.target_spot.is_none() && !j.wander.is_some_and(|w| w.0 == at) {
                        info!("Jason: going for the phone box ({:.0} m)", at.distance(xf.translation));
                        j.wander = Some((at, now));
                    }
                    goal = at;
                    following = true;
                    if stalled || Vec2::new(at.x - xf.translation.x, at.z - xf.translation.z).length() * 100.0 <= reach + 100.0 {
                        info!("Jason: breaking the phone box (AI)");
                        j.stall = 0.0;
                        let to = at - xf.translation;
                        xf.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
                        j.state = JasonState::BreakingFuse { t: 0.0, fuse: be, done: false };
                        j.velocity = Vec2::ZERO;
                        commands.entity(je).insert((PlayAction(BREAK_FUSE.into()), ActionSounds(BREAK_PHONE_BOX)));
                        continue;
                    }
                } else if !sees {
                    if let Some(s) = know.as_ref().and_then(|k| k.hiding_spot).filter(|s| !j.searched.contains(s)) {
                        if j.target_spot != Some(s) {
                            j.target_side = None;
                        }
                        j.target_spot = Some(s);
                    }
                    // Clues near one he couldn't get to don't count for a while (Sense keeps showing her there).
                    let unreachable = j.unreachable.filter(|u| now - u.1 < UNREACHABLE_SECONDS);
                    let clue = know.as_ref().and_then(|k| k.last_known).filter(|c| c.1 > j.clue_done && unreachable.is_none_or(|u| c.0.distance(u.0) > UNREACHABLE_RADIUS_M));
                    if let Some(s) = j.target_spot {
                        match spots.get_mut(s) {
                            Ok((_, spot, g)) if !spot.destroyed || spot.occupant.is_some() => {
                                let side = *j.target_side.get_or_insert_with(|| spot.nearest_side(g, xf.translation));
                                let at = crate::hiding::HidingSpot::world(g, &spot.sides[side].search);
                                goal = at.translation;
                                following = true;
                                // Close to the special move's spot: it puts him on it (the game moves him there too).
                                if stalled || Vec2::new(at.translation.x - xf.translation.x, at.translation.z - xf.translation.z).length() < SPECIAL_MOVE_REACH_M {
                                    j.stall = 0.0;
                                    j.velocity = Vec2::ZERO;
                                    let s_ = spot.sides[side].clone();
                                    let victim = spot.occupant == Some(pe) && hiding.as_ref().is_some_and(|h| h.phase == crate::hiding::HidePhase::Hidden);
                                    match (victim, s_.killer_kill, s_.victim_kill, s_.search_anim) {
                                        (true, Some(kill), Some(vkill), _) => {
                                            info!("Jason: found Vanessa in a {:?}: hiding spot kill", spot.kind);
                                            let (origin, end) = killer.place(kill, crate::hiding::HidingSpot::world(g, &s_.kill));
                                            *xf = origin;
                                            j.state = JasonState::HideKill { spot: s, t: 0.0, end, anim: kill, victim: vkill };
                                            j.kill_score = Some((spot.class.clone(), s_.kill_component));
                                            commands.entity(je).insert(PlayAction(kill.into()));
                                            if let (Some(h), Some(c)) = (hiding.as_mut(), counselor_model.as_ref()) {
                                                h.phase = crate::hiding::HidePhase::Killed;
                                                *pxf = c.place(vkill, *pxf).0;
                                            }
                                            commands.entity(pe).insert((PlayAction(vkill.into()), Visibility::Inherited));
                                        }
                                        (false, _, _, Some(search)) => {
                                            info!("Jason: searching a {:?}", spot.kind);
                                            let (origin, end) = killer.place(search, at);
                                            *xf = origin;
                                            j.state = JasonState::Searching { spot: s, t: 0.0, end, anim: search };
                                            commands.entity(je).insert(PlayAction(search.into()));
                                        }
                                        _ => {
                                            j.searched.push(s);
                                            j.target_spot = None;
                                        }
                                    }
                                    continue;
                                }
                            }
                            _ => {
                                j.searched.push(s);
                                j.target_spot = None;
                            }
                        }
                    } else if let Some((at, when, how)) = clue {
                        goal = at;
                        following = true;
                        if stalled || Vec2::new(at.x - xf.translation.x, at.z - xf.translation.z).length() < 1.5 {
                            if stalled && !j.path_reaches {
                                info!("Jason: can't get to the clue ({:.0} m away)", Vec2::new(at.x - xf.translation.x, at.z - xf.translation.z).length());
                                j.unreachable = Some((at, now));
                            }
                            j.stall = 0.0;
                            // Nothing here: the hiding spots around it, nearest first.
                            j.clue_done = when;
                            j.target_side = None;
                            j.target_spot = spots
                                .iter()
                                .filter(|(e, s, g)| !s.destroyed && !j.searched.contains(e) && g.translation().distance(at) < SEARCH_RADIUS_M)
                                .min_by(|a, b| a.2.translation().distance(at).total_cmp(&b.2.translation().distance(at)))
                                .map(|(e, _, _)| e);
                            info!("Jason: lost her ({how:?}); {}", if j.target_spot.is_some() { "searching a hiding spot nearby" } else { "nothing to search here" });
                            j.wander = None;
                        }
                    } else {
                        // Wander between cabins.
                        let here = xf.translation;
                        let stale = stalled || j.wander.is_none_or(|(w, at)| now - at > WANDER_SECONDS || Vec2::new(w.x - here.x, w.z - here.z).length() < 2.0);
                        if stale {
                            j.stall = 0.0;
                        }
                        if stale {
                            let places: Vec<Vec3> = spots
                                .iter()
                                .map(|(_, _, g)| g.translation())
                                .chain(cabinets.iter().map(|g| g.translation()))
                                .filter(|p| (10.0..WANDER_RANGE_M).contains(&p.distance(here)))
                                .collect();
                            if !places.is_empty() {
                                let w = places[rng.0.index(places.len())];
                                debug!("Jason: wandering to ({:.0}, {:.0})", w.x, w.z);
                                j.wander = Some((w, now));
                            }
                        }
                        if let Some((w, _)) = j.wander {
                            goal = w;
                        }
                    }
                }
                let gflat = Vec2::new(goal.x - xf.translation.x, goal.z - xf.translation.z);
                let gdist = gflat.length();
                if now >= j.next_log {
                    j.next_log = now + 5.0;
                    let what = if sees { "her" } else if j.target_spot.is_some() { "a hiding spot" } else if j.wander.is_some() && goal == j.wander.unwrap().0 { "wandering" } else { "a clue" };
                    let door = doors.iter().map(|(_, d, g)| (g.translation().distance(xf.translation), d.open, d.locked, g.translation())).min_by(|a, b| a.0.total_cmp(&b.0));
                    debug!("Jason: going for {what}, {gdist:.1} m to go (at {:.1},{:.1},{:.1} -> {:.1},{:.1},{:.1}; path {:?}); nearest door {door:?}", xf.translation.x, xf.translation.y, xf.translation.z, goal.x, goal.y, goal.z, j.path);
                }
                // Under water (the AI's choice): straight at his goal along the bottom, for his drowning kill (above);
                // no swings, grabs or abilities there.
                if j.underwater {
                    let wish = if gdist > 0.3 { gflat.normalize_or_zero() } else { Vec2::ZERO };
                    walk(&mut j, &mut xf, wish, t.run_speed, None, dt, &t, &world);
                    continue;
                }
                // His abilities (when is the AI's choice; see the module docs).
                let blockers = Blockers { underwater: j.underwater, ..Blockers::default() };
                let ready = |know: &Option<Mut<crate::senses::Knowledge>>, a: Ability| know.as_ref().is_some_and(|k| k.abilities.is_available(a, &blockers));
                let after_clue = !sees && following;
                if sees && !driving && (SHIFT_RANGE_M.0..SHIFT_RANGE_M.1).contains(&dist) && ready(&know, Ability::Shift) {
                    if let Some(k) = know.as_mut() {
                        k.abilities.consume(Ability::Shift);
                    }
                    info!("Jason: Shift ({dist:.0} m from her)");
                    j.state = JasonState::Shifting { shift: Shift::start() };
                    j.velocity = Vec2::ZERO;
                    continue;
                }
                if after_clue && gdist > MORPH_MIN_M && j.hunted > MORPH_GRACE_S && ready(&know, Ability::Morph) {
                    let short = goal + (xf.translation - goal).with_y(0.0).normalize_or_zero() * MORPH_SHORT_M;
                    let to = morph_points.as_ref().and_then(|m| m.pick(short)).filter(|to| to.translation.distance(pxf.translation) > MORPH_KEEP_OFF_M);
                    if let Some(to) = to {
                        if let Some(k) = know.as_mut() {
                            k.abilities.consume(Ability::Morph);
                        }
                        info!("Jason: Morph ({gdist:.0} m to his goal; to a point {:.0} m from it)", Vec2::new(to.translation.x - goal.x, to.translation.z - goal.z).length());
                        j.state = JasonState::Morphing { t: 0.0, to };
                        j.velocity = Vec2::ZERO;
                        crate::net_world::shout(&mut commands, crate::net_world::Shout::Distortion(crate::jason::Distortion { at: xf.translation, dest: Some(to.translation) }));
                        continue;
                    }
                }
                // Stalk closing on a clue unseen, or straight out of a Morph (closing the rest of the way quietly).
                let morphed = std::mem::take(&mut j.morphed);
                if (after_clue && gdist < STALK_RANGE_M || morphed) && j.stalk.is_none() && ready(&know, Ability::Stalk) {
                    if let Some(k) = know.as_mut() {
                        k.abilities.consume(Ability::Stalk);
                    }
                    info!("Jason: Stalk ({gdist:.0} m to his goal)");
                    j.stalk = Some(Stalk::start(t.stalk_active_time));
                    crate::net_world::shout(&mut commands, crate::net_world::Shout::Distortion(crate::jason::Distortion { at: xf.translation, dest: None }));
                }
                // A knife at her when she's in throwing range.
                if sees && !driving && alive && j.knives > 0 && j.knife_cooldown <= 0.0 && (KNIFE_RANGE_M.0..=KNIFE_RANGE_M.1).contains(&dist) {
                    info!("Jason: throwing a knife ({dist:.0} m, {} left)", j.knives);
                    let aim_at = killer.length(KNIFE_SELECT).unwrap_or(0.37);
                    j.state = JasonState::Throwing { t: 0.0, thrown: false, release: Some(aim_at + KNIFE_AIM_SECONDS) };
                    j.velocity = Vec2::ZERO;
                    commands.entity(je).insert(PlayAction(KNIFE_SELECT.into()));
                    continue;
                }
                if sees && !driving && j.cooldown <= 0.0 {
                    // Going for a grab, he swings instead when she's inside the grab's minimum distance (else he'd
                    // stand there); going for a swing, he closes to swing range.
                    let can_grab = dist >= t.grab_min_m && dist <= t.grab_max_m && angle <= t.grab_angle_deg;
                    let can_swing = dist <= t.attack_range_m && angle <= t.attack_angle_deg;
                    if j.wants_grab && can_grab {
                        info!("Jason: grab ({dist:.2} m, {angle:.0} deg)");
                        j.state = JasonState::Reaching { t: 0.0 };
                        commands.entity(je).insert(PlayAction(GRAB_STARTUP.into()));
                        continue;
                    }
                    if can_swing && (!j.wants_grab || dist < t.grab_min_m) {
                        // At a raised guard, the heavy swing (AI choice).
                        let heavy = blocking && killer.info.animations.contains_key(heavy_take());
                        info!("Jason: {}swing ({dist:.2} m, {angle:.0} deg)", if heavy { "heavy " } else { "" });
                        j.state = JasonState::Swinging { t: 0.0, hit: false, heavy };
                        commands.entity(je).insert(PlayAction(if heavy { heavy_take() } else { attack_take() }.into()));
                        continue;
                    }
                }
                // Close in: straight at the goal when near, else along a path. A goal in the water (the AI's choice;
                // the navmesh ends at the shore): straight in once his path runs out there, and along the bottom.
                let wet_goal = water.as_ref().is_some_and(|w| w.surface_at(goal + Vec3::Y * 0.5).is_some());
                // At the shore: the last point of a path that stops short of the water (its end reached, not cut).
                let shore_end = j.path.len() == 1 && j.path.first().is_some_and(|p| Vec2::new(p.x - xf.translation.x, p.z - xf.translation.z).length() < 1.5);
                j.at_shore = wet_goal && !j.path_reaches && (j.at_shore || shore_end);
                let direct = gdist < 1.5 || (wet_goal && (surface.is_some() || j.at_shore));
                let mut wish = if direct { gflat.normalize_or_zero() } else { steer(&mut j, &mut nav.0, navmesh, &world, xf.translation, goal, dt) };
                if sees && dist < t.attack_range_m * 0.7 {
                    wish = Vec2::ZERO; // close enough for anything; don't walk into her
                }
                // A closed door ahead: open it, or break it down when it's locked or barred.
                // A wall between him and her when he walks at her: through it.
                if sees {
                    if let Some(we) = wall_in_way(&walls, xf.translation, wish) {
                        info!("Jason: breaking through a wall");
                        j.velocity = Vec2::ZERO;
                        j.state = JasonState::BreakingWall { wall: we, t: 0.0, section: 0, hit: false };
                        commands.entity(je).insert(PlayAction(BREAK_DOOR[0].into()));
                        continue;
                    }
                }
                let raged = know.as_ref().is_some_and(|k| k.abilities.rage_active);
                if let Some((state, anim)) = door_in_way(&doors, &barricades, xf.translation, wish, wish == Vec2::ZERO, raged) {
                    j.velocity = Vec2::ZERO;
                    j.state = state;
                    commands.entity(je).insert(PlayAction(anim.into()));
                    continue;
                }
                let speed = if t.can_run && gdist > t.attack_range_m * 1.5 { t.run_speed } else { t.walk_speed };
                j.running = speed == t.run_speed;
                // Up close, square up to her (he only turns toward where he walks otherwise).
                let face = (sees && dist < t.grab_max_m * 1.5).then_some(flat);
                walk(&mut j, &mut xf, wish, speed, face, dt, &t, &world);
                trace!("Jason: at {:.2} wish {wish:.2} vel {:.0} grounded {} vy {:.2} path0 {:?}", xf.translation, j.velocity, j.grounded, j.vel_y, j.path.first());
                // Not getting anywhere: try another way for a moment.
                j.progress.1 += dt;
                if j.progress.1 >= 1.0 {
                    let moved = xf.translation.distance(j.progress.0);
                    // Not getting closer near the goal, or at the end of a path that doesn't get there.
                    if !sees && (gdist < STALL_RANGE_M || (!j.path_reaches && !wet_goal)) && gdist > j.last_gdist - 0.3 {
                        j.stall += 1.0;
                    } else {
                        j.stall = 0.0;
                    }
                    j.last_gdist = gdist;
                    if moved < 0.2 && wish != Vec2::ZERO && gdist > 1.5 {
                        // What's in the way (diagnostic).
                        let probe = xf.translation + Vec3::new(wish.x, 0.0, wish.y) * 0.5;
                        let mut hits = Vec::new();
                        f13_core::collision::TriSource::for_each_in(&world, probe - Vec3::new(0.3, 0.0, 0.3), probe + Vec3::new(0.3, 2.0, 0.3), &mut |t| {
                            hits.push((t.min(), t.max(), t.n));
                        });
                        hits.sort_by(|a, b| b.1.y.total_cmp(&a.1.y));
                        debug!("Jason: stuck at {:.2} heading {wish:.2}; {} triangles ahead, highest {:?}; navmesh here {:?}", xf.translation, hits.len(), hits.first(), navmesh.and_then(|m| m.nearest(xf.translation, 0.5)).map(|n| n.1));
                        let a = rng.0.frand() * std::f32::consts::TAU;
                        j.unstick = Some((Vec2::new(a.cos(), a.sin()), 0.6));
                        j.path.clear();
                    }
                    j.progress = (xf.translation, 0.0);
                }
            }
            JasonState::Shifting { mut shift } => {
                let phase = shift.tick(dt, t.shift_time);
                let in_reach = alive && !driving && dist >= t.grab_min_m && dist <= t.grab_max_m && angle <= t.grab_angle_deg;
                if phase == ShiftPhase::Over || !alive {
                    info!("Jason: Shift over");
                    j.state = JasonState::Hunting;
                } else if in_reach && phase == ShiftPhase::Moving {
                    // Grabbing ends the shift (CancelShift).
                    info!("Jason: grab out of a Shift ({dist:.2} m, {angle:.0} deg)");
                    j.state = JasonState::Reaching { t: 0.0 };
                    commands.entity(je).insert(PlayAction(GRAB_STARTUP.into()));
                } else {
                    j.state = JasonState::Shifting { shift };
                    let sees = know.as_ref().is_none_or(|k| k.sees);
                    let goal = if sees { to } else { know.as_ref().and_then(|k| k.last_known).map_or(to, |c| c.0) };
                    if phase == ShiftPhase::Moving {
                        // The game pushes him along his facing at ShiftSpeed; he turns as he steers.
                        let dir = if let Some(i) = player_input { i.forward } else if sees { flat.normalize_or_zero() } else { steer(&mut j, &mut nav.0, navmesh, &world, xf.translation, goal, dt) };
                        turn_toward(&mut xf, dir, t.turn_rate_deg, dt);
                        let yaw = yaw_of(xf.rotation);
                        let fwd = Vec2::new(yaw.sin(), yaw.cos());
                        walk(&mut j, &mut xf, fwd, t.shift_speed, Some(fwd), dt, &t, &world);
                    } else {
                        walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(face_dir), dt, &t, &world);
                    }
                }
            }
            JasonState::Throwing { t: at, thrown, release } => {
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(face_dir), dt, &t, &world);
                let len = |n: &str, d: f32| killer.length(n).unwrap_or(d);
                let aim_at = len(KNIFE_SELECT, 0.37);
                // The player's knife: thrown when the button's let go (after the select section), put away by
                // KLR_KnifeCancel (right mouse).
                let mut release = release;
                if let Some(i) = player_input.filter(|_| release.is_none()) {
                    if i.presses.grab {
                        info!("Jason: knife put away");
                        j.state = JasonState::Hunting;
                        commands.entity(je).remove::<PlayAction>();
                        continue;
                    }
                    if !i.knife_held {
                        release = Some(at.max(aim_at));
                    }
                }
                let throw_at = release.unwrap_or(f32::MAX);
                let let_go = throw_at + 0.18f32.min(len(KNIFE_THROW, 0.72));
                let end = throw_at + len(KNIFE_THROW, 0.72);
                let anim = if at < aim_at { KNIFE_SELECT } else if at < throw_at { KNIFE_AIM } else { KNIFE_THROW };
                commands.entity(je).insert(PlayAction(anim.into()));
                let mut thrown = thrown;
                if !thrown && at >= let_go {
                    thrown = true;
                    j.knives -= 1;
                    j.knife_cooldown = t.knife_cooldown;
                    // ThrowKnife (0x14040d100): from his camera, 120 cm out along its forward, at ThrowStrength along it;
                    // the computer Jason's from his eyes, aimed to arc onto her chest.
                    let eyes = xf.translation + Vec3::Y * crate::senses::EYE_M;
                    let (origin, vel) = match player_input {
                        // (No camera yet: his eyes and his look.)
                        Some(i) if i.view.1 == Vec3::ZERO => (eyes, i.look.normalize_or(Vec3::Z) * t.throw_strength),
                        Some(i) => (i.view.0, i.view.1.normalize_or(Vec3::Z) * t.throw_strength),
                        None => (eyes, knife_velocity(eyes, pxf.translation + Vec3::Y * 1.2, t.throw_strength)),
                    };
                    let from = origin + vel.normalize_or_zero() * KNIFE_START_M;
                    commands.trigger(ThrowKnife { from, vel, damage: t.knife_damage });
                }
                if at >= end {
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Throwing { t: at, thrown, release };
                }
            }
            JasonState::BreakingFuse { t: at, fuse, done } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                // On his way onto a special move's spot (negative time): swept there; arrived, its take. Blocked short
                // of its MoveThreshold, the move is cancelled (CancelSpecialMove).
                if let Some(a) = align {
                    let take = KNIFE_PICKUP;
                    let k = ((at + a.params.interp_time) / a.params.interp_time.max(1e-4)).clamp(0.0, 1.0);
                    let (from, to) = (a.from.rotation, a.to.rotation);
                    let to_rot = if from.dot(to) < 0.0 { -to } else { to };
                    let mut next = Transform { translation: a.from.translation.lerp(a.to.translation, k), rotation: (from * (1.0 - k) + to_rot * k).normalize(), ..a.to };
                    let sweep = |p: Vec3, q: Vec3| f13_core::collision::capsule_sweep(&world, p, q, t.capsule.radius, t.capsule.half_height);
                    if a.params.sweep {
                        if let Some(hit) = sweep(xf.translation, next.translation) {
                            next.translation = xf.translation.lerp(next.translation, hit);
                        }
                    }
                    *xf = next;
                    if at >= 0.0 {
                        commands.entity(je).remove::<AlignTo>();
                        let near = Vec2::new(xf.translation.x - a.to.translation.x, xf.translation.z - a.to.translation.z).length() <= a.params.threshold_m;
                        if a.params.sweep && sweep(xf.translation, a.to.translation).is_some() && !near {
                            info!("Jason: special move cancelled: blocked on the way to its spot");
                            // (The computer Jason leaves it for a while.)
                            if controls.is_none() {
                                j.unreachable = Some((a.to.translation, now));
                            }
                            j.state = JasonState::Hunting;
                            continue;
                        }
                        *xf = a.to;
                        commands.entity(je).insert(PlayAction(take.into()));
                        j.state = JasonState::BreakingFuse { t: 0.0, fuse, done };
                    } else {
                        j.state = JasonState::BreakingFuse { t: at, fuse, done };
                    }
                    continue;
                }
                // A radio's break take (its DestroyRadio driver window: OnBreakRadio), the knife pickup's (its
                // KnifePickup window: PerformPickup), else the fuse box's.
                let radio = radios.contains(fuse);
                let (take, driver) = if radio {
                    (crate::radio::BREAK_TAKE, "Driver: DestroyRadio")
                } else if knife_pickups.contains(fuse) {
                    (KNIFE_PICKUP, "Driver: KnifePickup")
                } else {
                    (BREAK_FUSE, "Driver: DestroyGenerator")
                };
                let window = killer.info.animations.get(take).and_then(|a| a.windows.iter().find(|w| w.name == driver)).map_or(1.0, |w| w.start);
                let mut done = done;
                if !done && at >= window {
                    done = true;
                    if let (Ok((_, mut f)), Some(g)) = (fuse_boxes.get_mut(fuse), grids.as_mut()) {
                        crate::power::set_destroyed(&mut f, true, g);
                        info!("Jason: destroyed a fuse box");
                    }
                    if let Ok((_, mut b)) = phone_boxes.get_mut(fuse) {
                        crate::police::break_phone_box(&mut b, &mut phones);
                        info!("Jason: broke the phone box");
                    }
                    if let Ok((_, mut r)) = radios.get_mut(fuse) {
                        crate::radio::break_radio(&mut r);
                        info!("Jason: broke a radio");
                    }
                    // PerformPickup (0x140390360): AddKnives(StackSize), MULTICAST_DisablePickup (hidden), its lifespan
                    // 5 s (DAT_14207a218).
                    if let Ok((_, k, _, _)) = knife_pickups.get(fuse) {
                        j.knives += k.count;
                        info!("Jason picked up {} throwing knife(s) ({} now)", k.count, j.knives);
                        crate::net_world::shout(&mut commands, crate::net_world::Shout::PickupTaken(PickupTaken(fuse)));
                    }
                }
                if at >= killer.length(take).unwrap_or(2.0) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<(PlayAction, ActionSounds)>();
                } else {
                    j.state = JasonState::BreakingFuse { t: at, fuse, done };
                }
            }
            JasonState::Slamming { t: at } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                if at >= killer.length(CAR_SLAM).unwrap_or(1.87) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Slamming { t: at };
                }
            }
            JasonState::PlacingTrap { t: at } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                if at >= killer.length(TRAP_SET).unwrap_or(2.47) {
                    let yaw = yaw_of(xf.rotation);
                    let at_pos = xf.translation + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * t.trap_placement_m;
                    let floor = f13_core::collision::surface_below(&world, at_pos.x, at_pos.z, at_pos.y + 0.5, at_pos.y - 2.0).map_or(at_pos.y, |s| s.0);
                    let spot = Vec3::new(at_pos.x, floor, at_pos.z);
                    j.traps -= 1;
                    j.trap_spots.push(spot);
                    commands.trigger(crate::items::SetKillerTrap(Transform::from_translation(spot).with_rotation(xf.rotation)));
                    info!("Jason: trap set at ({:.1}, {:.1})", spot.x, spot.z);
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::PlacingTrap { t: at };
                }
            }
            JasonState::Morphing { t: at, to: dest } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                if at >= t.teleport_screen_time {
                    info!("Jason: morphed to ({:.0}, {:.0})", dest.translation.x, dest.translation.z);
                    *xf = dest;
                    // OnRep_Teleport as the effect ends: the distortion where he arrived.
                    crate::net_world::shout(&mut commands, crate::net_world::Shout::Distortion(crate::jason::Distortion { at: dest.translation, dest: None }));
                    j.path.clear();
                    j.progress = (dest.translation, 0.0);
                    j.stall = 0.0;
                    j.last_gdist = f32::MAX;
                    j.morphed = true;
                    j.state = JasonState::Hunting;
                } else {
                    j.state = JasonState::Morphing { t: at, to: dest };
                }
            }
            JasonState::Swinging { t: at, hit, heavy } => {
                let mut at = at + dt;
                let mut heavy = heavy;
                // SCCharacter's tick: the attack held over 0.1 s in combat stance is the heavy attack (vt+0xd38 ->
                // MULTICAST_Attack(true)): the light swing playing turns into the weapon's ForwardAttack at the same
                // fraction of it.
                if let Some(h) = j.attack_hold {
                    let held = player_input.is_some_and(|i| i.attack_held) && j.combat;
                    j.attack_hold = held.then_some(h + dt);
                    if held && h + dt > HEAVY_HOLD_S && !heavy && killer.info.animations.contains_key(heavy_take()) {
                        j.attack_hold = None;
                        heavy = true;
                        at = t.heavy_length * (at / t.attack_length.max(1e-3)).min(1.0);
                        info!("Jason: heavy swing");
                        commands.entity(je).insert((PlayAction(heavy_take().into()), crate::character::SeekAction(at)));
                    }
                }
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(face_dir), dt, &t, &world);
                let mut hit = hit;
                let (window, length) = if heavy { (t.heavy_window, t.heavy_length) } else { (t.attack_window, t.attack_length) };
                // (Not through the world: the AttackTraceNotify's weapon sweep stops at what it meets first; PLACEHOLDER: a
                // chest-high line from him to her, the map and its doors and walls in the way.)
                let clear = || f13_core::collision::segment_hit(&level, xf.translation + Vec3::Y * 1.3, pxf.translation + Vec3::Y * 1.0, 1.0).is_none();
                if !hit && at >= window.0 && at <= window.1 && alive && dist <= t.attack_range_m && angle_to(xf.rotation, flat) <= t.attack_angle_deg && clear() {
                    hit = true;
                    let r = p.counselor.take_damage(Hit { amount: t.damage, blocked_with_weapon: blocking }, &ptuning.counselor);
                    // No impact effect: a melee hit's path to it, SCWeapon::SpawnHitEffect (0x1404558e0), is never called
                    // in this build (MODLOG "Melee hits spawn no impact effects").
                    info!("Jason: hit Vanessa for {:.0}{} (health {:.0})", r.dealt, if blocking { ", blocked" } else { "" }, p.counselor.health);
                    if blocking {
                        commands.trigger(crate::items::BlockedHit(pe));
                        // A heavy attack ends the block (after its blocked damage).
                        if heavy {
                            info!("Jason: his heavy swing broke her block");
                            commands.entity(pe).remove::<crate::items::Blocking>();
                        }
                    }
                    if r.died {
                        info!("Vanessa: killed by Jason's swing");
                        commands.trigger(crate::blood::Dismember { counselor: pe, at: None });
                        // Not a context kill: JasonWeaponKillScoreEvent (Butcher), then KillCounselor.
                        commands.trigger(crate::match_flow::KillerScore("Butcher"));
                        commands.trigger(crate::match_flow::KillerScore("KillCounselor"));
                        commands.entity(pe).insert(PlayAction(DEATH.into())).remove::<(Staggered, crate::items::ItemAction)>();
                    } else if r.dealt > 0.0 {
                        let reaction = if blocking { BLOCK_REACT } else { hit_reaction(&pxf, xf.translation) };
                        let len = counselor_model.as_ref().and_then(|c| c.length(reaction)).unwrap_or(0.67);
                        commands.entity(pe).insert((PlayAction(reaction.into()), Staggered(len))).remove::<crate::items::ItemAction>();
                        p.velocity = Vec2::ZERO;
                    }
                }
                if at >= length {
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    j.wants_grab = rng.0.frand() < GRAB_ODDS;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Swinging { t: at, hit, heavy };
                }
            }
            JasonState::OpeningDoor { door: de, t: at, anim } => {
                let at = at + dt;
                let Ok((_, mut d, g)) = doors.get_mut(de) else {
                    j.state = JasonState::Hunting;
                    continue;
                };
                let to_door = Vec2::new(g.translation().x - xf.translation.x, g.translation().z - xf.translation.z);
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(to_door), dt, &t, &world);
                // The leaf swings during the montage's OpenCloseDoor driver window.
                let (swing_at, swing_len) = killer.info.animations.get(anim).and_then(|a| a.window("Driver: OpenCloseDoor")).map_or((0.5, 0.9), |(s, e)| (s, e - s));
                if !d.open && at >= swing_at {
                    let barred = barricades.iter().any(|b| b.door == Some(de) && b.enabled);
                    if d.locked || barred {
                        // Locked while he reached for it.
                        info!("Jason: the door's locked now; breaking it down");
                        j.state = JasonState::BreakingDoor { door: de, t: 0.0, section: 0, hit: false };
                        commands.entity(je).insert(PlayAction(BREAK_DOOR[0].into()));
                        continue;
                    }
                    d.press(g, xf.translation);
                    d.drive(killer.info.animations.get(anim).and_then(|a| a.curve("DoorRotation")), at, swing_at + swing_len);
                }
                if at >= killer.length(anim).unwrap_or(1.6) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                    // The computer Jason's: a trap in the doorway when he isn't after her and has none of his near.
                    let sees = know.as_ref().is_some_and(|k| k.sees);
                    let here = xf.translation;
                    if player_input.is_none() && !sees && j.traps > 0 && j.trap_spots.iter().all(|s| s.distance(here) > TRAP_SPACING_M) && killer.info.animations.contains_key(TRAP_SET) {
                        info!("Jason: setting a trap in the doorway ({} left)", j.traps);
                        j.state = JasonState::PlacingTrap { t: 0.0 };
                        commands.entity(je).insert(PlayAction(TRAP_SET.into()));
                    }
                } else {
                    j.state = JasonState::OpeningDoor { door: de, t: at, anim };
                }
            }
            JasonState::BurstingDoor { door: de, t: at } => {
                let at = at + dt;
                let Ok((_, mut d, g)) = doors.get_mut(de) else {
                    j.state = JasonState::Hunting;
                    continue;
                };
                let burst_at = killer.info.animations.get(DOOR_BURST).and_then(|a| a.window("Driver: BurstDoor")).map_or(0.29, |w| w.0);
                if !d.broken && at >= burst_at {
                    // SCDoor::BurstDestroyDoor: health 0 at once.
                    let health = d.health;
                    d.damage(health, false, &mut commands);
                    info!("Jason: burst through the door");
                    if let Some(v) = &d.visuals {
                        let origin = crate::doors::BreakApart::origin(g.transform_point(d.out_arrow), g.transform_point(d.interact), xf.translation, 5.0);
                        crate::net_world::shout(&mut commands, crate::net_world::Shout::BreakApart(crate::doors::BreakApart { piece: v.leaf, origin, radius: 30.0, force: d.burst_force, lifespan: d.lifespan }));
                    }
                    commands.entity(d.pivot).insert(Visibility::Hidden);
                    for mut b in barricades.iter_mut().filter(|b| b.door == Some(de)) {
                        b.enabled = false;
                    }
                }
                // The montage's root motion: on through the doorway, facing it.
                let yaw = yaw_of(xf.rotation);
                let forward = Vec2::new(yaw.sin(), yaw.cos());
                let speed = anims.0.get(DOOR_BURST_MONTAGE).map_or(100.0, |v| v.y.abs());
                let to_door = Vec2::new(g.translation().x - xf.translation.x, g.translation().z - xf.translation.z);
                let face = (at < burst_at).then_some(to_door);
                walk(&mut j, &mut xf, if at < burst_at { Vec2::ZERO } else { forward }, speed, face, dt, &t, &world);
                if at >= killer.length(DOOR_BURST).unwrap_or(1.6) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::BurstingDoor { door: de, t: at };
                }
            }
            JasonState::BreakingDoor { door: de, t: at, section, hit } => {
                let Ok((_, mut d, g)) = doors.get_mut(de) else {
                    j.state = JasonState::Hunting;
                    continue;
                };
                let to_door = Vec2::new(g.translation().x - xf.translation.x, g.translation().z - xf.translation.z);
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(to_door), dt, &t, &world);
                if section == ABORT {
                    if abort(&mut j, &killer, at + dt) {
                        commands.entity(je).remove::<PlayAction>();
                    }
                    continue;
                }
                let barred = barricades.iter().any(|b| b.door == Some(de) && b.enabled);
                // The computer Jason's: unlocked (or unbarred) from inside before it gave way, he stops.
                if player_input.is_none() && section < 2 && !d.broken && !d.locked && !barred {
                    info!("Jason: the door's no longer locked; stopped breaking it");
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                    continue;
                }
                let timing = break_timing(&killer);
                let (mut at, mut section, mut hit) = (at + dt, section, hit);
                if !hit && at >= timing.hits[section] {
                    if section < 2 && d.health <= crate::doors::Door::hit_damage(t.world_damage, barred) {
                        // BreakDownSync: this hit finishes it.
                        (at, section) = (0.0, 2);
                    } else {
                        hit = true;
                        if d.damage(t.world_damage, barred, &mut commands) {
                            info!("Jason: broke the door down");
                            if let Some(v) = &d.visuals {
                                let origin = crate::doors::BreakApart::origin(g.transform_point(d.out_arrow), g.transform_point(d.interact), xf.translation, 5.0);
                                crate::net_world::shout(&mut commands, crate::net_world::Shout::BreakApart(crate::doors::BreakApart { piece: v.leaf, origin, radius: 30.0, force: d.break_force, lifespan: d.lifespan }));
                            }
                            commands.entity(d.pivot).insert(Visibility::Hidden);
                            for mut b in barricades.iter_mut().filter(|b| b.door == Some(de)) {
                                b.enabled = false;
                            }
                        } else {
                            info!("Jason: hit the door ({:.0} left)", d.health);
                        }
                    }
                }
                if at >= timing.lengths[section] {
                    if section == 2 {
                        j.state = JasonState::Hunting;
                        commands.entity(je).remove::<PlayAction>();
                        continue;
                    }
                    if player_input.is_some_and(|i| !i.attack_held) {
                        info!("Jason: let go; stopped breaking the door");
                        j.state = JasonState::BreakingDoor { door: de, t: 0.0, section: ABORT, hit: false };
                        commands.entity(je).insert(PlayAction(BREAK_ABORT.into()));
                        continue;
                    }
                    // On to (another) Loop.
                    (at, section, hit) = (at - timing.lengths[section], 1, false);
                }
                commands.entity(je).insert(PlayAction(BREAK_DOOR[section].into()));
                j.state = JasonState::BreakingDoor { door: de, t: at, section, hit };
            }
            JasonState::BreakingWall { wall: we, t: at, section, hit } => {
                let Ok((_, mut w, g)) = walls.get_mut(we) else {
                    j.state = JasonState::Hunting;
                    continue;
                };
                let to_wall = Vec2::new(g.translation().x - xf.translation.x, g.translation().z - xf.translation.z);
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, Some(to_wall), dt, &t, &world);
                if section == ABORT {
                    if abort(&mut j, &killer, at + dt) {
                        commands.entity(je).remove::<PlayAction>();
                    }
                    continue;
                }
                let timing = break_timing(&killer);
                let (mut at, mut section, mut hit) = (at + dt, section, hit);
                if !hit && at >= timing.hits[section] {
                    if section < 2 && w.health <= t.world_damage {
                        // This blow goes through: on to the Success section.
                        (at, section) = (0.0, 2);
                    } else {
                        hit = true;
                        let broke = w.damage(we, t.world_damage, crate::doors::Wall::is_inside(g, xf.translation), &mut commands);
                        // OnRep_Health: every blow SpawnEmitterAttached(HitParticle, InteractComponent); DestroyWall:
                        // DestroyParticle there (0x14033cfc0; scale 1, no offset, auto-destroyed). Its place: the
                        // Interactable's (its rotation the wall's here).
                        if let Some(fx) = if broke { w.destroy_particle.clone() } else { w.hit_particle.clone() } {
                            let at = Transform::from_translation(g.transform_point(w.interact)).with_rotation(g.rotation());
                            commands.write_message(crate::particles::SpawnParticles::at(&fx, at));
                        }
                        if broke {
                            info!("Jason: broke through the wall");
                            if let Some(s) = w.section {
                                let interact = g.transform_point(w.interact);
                                let origin = crate::doors::BreakApart::origin(interact, interact, xf.translation, 10.0);
                                crate::net_world::shout(&mut commands, crate::net_world::Shout::BreakApart(crate::doors::BreakApart { piece: s, origin, radius: 3.0, force: w.hit_force, lifespan: w.lifespan }));
                            }
                            crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: we, prop: "WallDestroySoundCue" }));
                        } else {
                            info!("Jason: hit the wall ({:.0} left)", w.health);
                            crate::net_world::shout(&mut commands, crate::net_world::Shout::ActorSound(crate::audio::ActorSound { actor: we, prop: "WallHitSoundCue" }));
                        }
                    }
                }
                if at >= timing.lengths[section] {
                    if section == 2 {
                        j.state = JasonState::Hunting;
                        commands.entity(je).remove::<PlayAction>();
                        continue;
                    }
                    if player_input.is_some_and(|i| !i.attack_held) {
                        info!("Jason: let go; stopped breaking the wall");
                        j.state = JasonState::BreakingWall { wall: we, t: 0.0, section: ABORT, hit: false };
                        commands.entity(je).insert(PlayAction(BREAK_ABORT.into()));
                        continue;
                    }
                    (at, section, hit) = (at - timing.lengths[section], 1, false);
                }
                commands.entity(je).insert(PlayAction(BREAK_DOOR[section].into()));
                j.state = JasonState::BreakingWall { wall: we, t: at, section, hit };
            }
            JasonState::Reaching { t: at } => {
                let at = at + dt;
                // The Default section's root motion (EnterGrabBegin: his move input ignored), turning to where he faces.
                root_move(&mut j, &mut xf, grab_root(&anims, GRAB_STARTUP), Some(face_dir), dt, &t, &world);
                if at < PERFORM_GRAB_TEST_S {
                    j.state = JasonState::Reaching { t: at };
                } else if alive && !driving && dist <= t.grab_max_m && angle_to(xf.rotation, flat) <= t.grab_angle_deg {
                    // A pocket knife in her pockets breaks the grab at once (and is used up).
                    let knife = inventories.get(pe).ok().and_then(|inv| {
                        inv.pockets.iter().copied().find(|&e| items.get(e).is_ok_and(|i| i.kind == crate::props::ItemKind::PocketKnife))
                    });
                    if let Some(k) = knife {
                        info!("Jason: grabbed Vanessa; she stabs him with her pocket knife");
                        commands.entity(pe).queue(move |mut e: EntityWorldMut| {
                            if let Some(mut inv) = e.get_mut::<crate::interact::Inventory>() {
                                inv.pockets.retain(|&x| x != k);
                            }
                        });
                        commands.entity(k).despawn();
                        let len = counselor_model.as_ref().and_then(|c| c.length(KNIFE_BREAK)).unwrap_or(2.0);
                        commands.entity(pe).insert((PlayAction(KNIFE_BREAK.into()), Staggered(len)));
                        j.state = JasonState::Stunned { t: 0.0, stun: Stun::PocketKnife };
                        j.velocity = Vec2::ZERO;
                        commands.entity(je).insert(PlayAction(Stun::PocketKnife.anims().0.into()));
                        continue;
                    }
                    info!("Jason: grabbed Vanessa");
                    struggle.wiggle = default();
                    // Out of a trap, into his hands.
                    commands.entity(pe).remove::<crate::items::Trapped>();
                    j.state = JasonState::Holding { t: 0.0 };
                    j.fast_walk = false;
                    j.velocity = Vec2::ZERO;
                    commands.entity(je).insert(PlayAction(GRAB_ENTER.into()));
                    commands.entity(pe).insert((Held, PlayAction(GRABBED_ENTER.into()))).remove::<(Staggered, crate::items::ItemAction)>();
                    p.velocity = Vec2::ZERO;
                } else {
                    info!("Jason: grab missed");
                    j.state = JasonState::Missed { t: 0.0 };
                    commands.entity(je).insert(PlayAction(GRAB_MISS.into()));
                }
            }
            JasonState::Missed { t: at } => {
                let at = at + dt;
                root_move(&mut j, &mut xf, grab_root(&anims, GRAB_MISS), None, dt, &t, &world);
                if at - dt < GRAB_MISS_SHAKE.2 && at >= GRAB_MISS_SHAKE.2 {
                    commands.trigger(JasonCameraShake { shake: GRAB_MISS_SHAKE.0, scale: GRAB_MISS_SHAKE.1 });
                }
                if at >= GRAB_MISS_END_S {
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    j.wants_grab = rng.0.frand() < GRAB_ODDS;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Missed { t: at };
                }
            }
            JasonState::Holding { t: at } => {
                let at = at + dt;
                let jrig = rigs.iter().find(|(_, c, _)| c.parent() == je);
                let crig = rigs.iter().find(|(_, c, _)| c.parent() == pe);
                let placed = match (jrig, crig, &counselor_model) {
                    (Some((jr, _, _)), Some((cr, _, cxf)), Some(c)) => place_held(pe, &mut pxf, false, jr, (cr, cxf), &killer.info, &c.info, &globals),
                    _ => false,
                };
                if !placed {
                    let yaw = yaw_of(xf.rotation);
                    pxf.translation = xf.translation + Vec3::new(yaw.sin(), 0.0, yaw.cos()) * GRAB_HOLD_M;
                    pxf.rotation = Quat::from_rotation_y(yaw + std::f32::consts::PI);
                }
                // Past the grab's GrabSuccess section: his locomotion with the grab layer over it (grab_layer); he moves
                // at his normal speeds (GetMaxSpeed has no grab case) from its EnterGrabFinish, the player walking him;
                // the AI stands.
                let success = killer.length(GRAB_ENTER).unwrap_or(2.2);
                if at >= success {
                    commands.entity(je).remove::<PlayAction>();
                }
                let speed = if j.combat { t.combat_speed } else if player_input.is_some() && j.fast_walk && t.can_run { t.run_speed } else { t.walk_speed };
                j.running = speed == t.run_speed;
                if at - dt < GRAB_SUCCESS_SHAKE.2 && at >= GRAB_SUCCESS_SHAKE.2 {
                    commands.trigger(JasonCameraShake { shake: GRAB_SUCCESS_SHAKE.0, scale: GRAB_SUCCESS_SHAKE.1 });
                }
                // PLACEHOLDER (the AI's: the game's Jason is a player): an unused placed kill within CARRY_TO_KILL_M
                // that isn't at hand yet, he carries her to.
                let centre = xf.translation + Vec3::Y * t.capsule.half_height;
                let facing = Vec2::new((xf.rotation * Vec3::Z).x, (xf.rotation * Vec3::Z).z);
                let carry_to = (player_input.is_none() && config.kill.is_none())
                    .then(|| placed_kills.iter().filter(|k| !k.used && k.interact.distance(centre) < CARRY_TO_KILL_M).min_by(|a, b| a.interact.distance(centre).total_cmp(&b.interact.distance(centre))))
                    .flatten()
                    .filter(|k| !k.in_reach(centre, facing))
                    .map(|k| Vec2::new(k.interact.x - xf.translation.x, k.interact.z - xf.translation.z));
                if at < success {
                    // The GrabSuccess section's root motion carries him (and her with him) until it's played out.
                    root_move(&mut j, &mut xf, grab_root(&anims, GRAB_ENTER), None, dt, &t, &world);
                } else {
                    let wish = player_input.map_or(carry_to.map_or(Vec2::ZERO, |d| d.normalize_or_zero()), |i| i.movement);
                    walk(&mut j, &mut xf, wish, speed, carry_to, dt, &t, &world);
                }
                if at >= counselor_model.as_ref().and_then(|c| c.length(GRABBED_ENTER)).unwrap_or(2.0) {
                    commands.entity(pe).insert(PlayAction(GRABBED_IDLE.into()));
                }
                let input = struggle.inputs.get(&pe).copied().unwrap_or(0.0);
                // FUN_1403bc910: her progress per wiggle slowed while she's wounded and against a raged Jason.
                let combat = &ptuning.counselor.combat;
                let raged = know.as_ref().is_some_and(|k| k.abilities.rage_active);
                let mut wiggle = grab_struggle(&p.counselor.stats);
                wiggle.percent_per_wiggle *= combat.wiggle_mod(p.counselor.wounded, raged);
                // x his GrabModifier (Part 3's 0.9: his grip holds harder; constructor 1).
                wiggle.percent_per_wiggle *= killer.info.values.get("GrabModifier").copied().unwrap_or(1.0);
                // The stun's time x its StunTimeModifier: 1 + her perks' BreakGrabDurationModifier (TakeDamage
                // FUN_1403d7410, the instigator Jason; the stun's timer FUN_1402ddfd0).
                wiggle.total_time *= 1.0 + p.counselor.perks.get("BreakGrabDurationModifier");
                if struggle.wiggle.tick(dt, input, &wiggle) {
                    // FUN_1403a6c10: BreakFreeStaminaBoost back.
                    // (FUN_1403a6c10: x (1 + her perks' BreakGrabStaminaModifier).)
                    let back = combat.break_free_stamina(&p.counselor.stats) * (1.0 + p.counselor.perks.get("BreakGrabStaminaModifier"));
                    p.counselor.stamina = (p.counselor.stamina + back).min(ptuning.counselor.stamina_max);
                    info!("Vanessa: struggled free after {at:.1}s (stamina +{back:.0})");
                    commands.trigger(Score { who: pe, event: Stun::GrabBreak.score_event().unwrap_or_default() });
                    let in_water = water.as_ref().is_some_and(|w| w.surface_at(xf.translation + Vec3::Y * t.capsule.half_height).is_some());
                    drop_counselor(&mut pxf, &xf, t.capsule.half_height, in_water, &world);
                    release(&mut commands, pe, GRAB_BREAK, counselor_model.as_deref());
                    j.state = JasonState::Stunned { t: 0.0, stun: Stun::GrabBreak };
                    commands.entity(je).insert(PlayAction(Stun::GrabBreak.anims().0.into()));
                } else if let Some(mut pk) = placed_kills
                    .iter_mut()
                    .find(|k| k.in_reach(xf.translation + Vec3::Y * t.capsule.half_height, player_input.map_or(Vec2::new((xf.rotation * Vec3::Z).x, (xf.rotation * Vec3::Z).z), |i| i.forward)))
                    // CanGrabKill (0x1403e6f10): bGrabKillsActive, his grab's enter played out.
                    .filter(|_| config.kill.is_none() && at >= success && player_input.map_or(at >= HOLD_BEFORE_KILL, |i| i.presses.interact))
                {
                    // A placed kill at hand (the player's E; the AI uses one when it can): he takes its place.
                    let (his, hers, name) = pk.takes();
                    info!("Jason: {name} (placed kill)");
                    pk.used = true;
                    // KillStarted: its prop plays its own animation.
                    if let Some((prop, anim, at_start)) = pk.prop {
                        commands.entity(prop).insert(crate::prop_anim::PropAnim::new(anim));
                        // Its relative transform under the actor as it now is (turned toward him, if it turns).
                        if let Some(xf) = at_start {
                            commands.entity(prop).insert(pk.prop_start(xf));
                        }
                    }
                    j.kill_move = (his, hers, name);
                    j.kill_score = Some((crate::placed_kills::KINDS[pk.kind].0.to_string(), "ContextKillComponent"));
                    *xf = killer.place(his, pk.spot).0;
                    j.state = JasonState::Killing { t: 0.0 };
                    commands.entity(je).insert((PlayAction(his.into()), crate::kill_camera::KillCam::at(&pk.camera_key, pk.camera).of(pe)));
                    commands.entity(pe).insert(PlayAction(hers.into()));
                    snap_rig(&mut rigs, je);
                } else if player_input.map_or(at >= HOLD_BEFORE_KILL && (carry_to.is_none() || at >= HOLD_BEFORE_KILL * 2.0), |i| {
                    // A kill key (1-4, its slot) or a click (slot 1), its kill available (FUN_1403ffa80).
                    (i.presses.attack || i.presses.kill.is_some()) && grab_kills.0[i.presses.kill.unwrap_or(1).min(3)]
                }) {
                    // The AI kills after a while; the player with a kill key or a click. One of the weapon's grab kills.
                    let picked = player_input.and_then(|i| grab_kill_slots()[i.presses.kill.unwrap_or(1).min(3)]);
                    // (The AI: PLACEHOLDER, one of his weapon's kills or the hand kills at random.)
                    // (Only the kills of his slots' weapon this port has, and the hand kills.)
                    let own: Vec<usize> = (0..kills().len()).filter(|&k| kills()[k].weapon.is_empty() || kills()[k].weapon == own_weapon()).collect();
                    j.kill = picked.or(config.kill.as_deref().and_then(kill_by_name)).unwrap_or_else(|| own[rng.0.index(own.len())]);
                    let Kill { his, hers, class, name, .. } = &kills()[j.kill];
                    let (his, hers, class, name) = (his.as_str(), hers.as_str(), class.as_str(), name.as_str());
                    info!("Jason: {name}");
                    j.kill_move = (his, hers, name);
                    j.kill_score = Some((format!("{class}_C"), "ContextKillComponent"));
                    j.state = JasonState::Killing { t: 0.0 };
                    commands.entity(je).insert((PlayAction(his.into()), crate::kill_camera::KillCam::new(class, *xf).of(pe)));
                    commands.entity(pe).insert(PlayAction(hers.into()));
                } else {
                    j.state = JasonState::Holding { t: at };
                }
            }
            JasonState::PullingOut { t: at } => {
                let at = at + dt;
                j.velocity = Vec2::ZERO;
                let jrig = rigs.iter().find(|(_, c, _)| c.parent() == je);
                let crig = rigs.iter().find(|(_, c, _)| c.parent() == pe);
                if let (Some((jr, _, _)), Some((cr, _, cxf)), Some(c)) = (jrig, crig, &counselor_model) {
                    place_held(pe, &mut pxf, true, jr, (cr, cxf), &killer.info, &c.info, &globals);
                }
                if at >= killer.length(CAR_GRAB).unwrap_or(6.0) {
                    // Out: he stays where the take left him, holding her (past the grab's enter).
                    if let Some((_, last)) = killer.root_motion(CAR_GRAB) {
                        *xf = crate::character::flat(Transform::from_matrix(xf.to_matrix() * last.to_matrix()));
                        snap_rig(&mut rigs, je);
                    }
                    j.state = JasonState::Holding { t: HOLD_AFTER_CAR_GRAB };
                    j.fast_walk = false;
                    commands.entity(je).remove::<PlayAction>();
                    commands.entity(pe).insert(PlayAction(GRABBED_IDLE.into()));
                } else {
                    j.state = JasonState::PullingOut { t: at };
                }
            }
            JasonState::Killing { t: at } => {
                let at = at + dt;
                let (kill, her_kill, kill_name) = j.kill_move;
                let jrig = rigs.iter().find(|(_, c, _)| c.parent() == je);
                let crig = rigs.iter().find(|(_, c, _)| c.parent() == pe);
                if alive {
                    if let (true, Some((jr, _, _)), Some((cr, _, cxf)), Some(c)) = (j.kill_holds, jrig, crig, &counselor_model) {
                        place_held(pe, &mut pxf, true, jr, (cr, cxf), &killer.info, &c.info, &globals);
                    }
                    // She dies, and goes limp, where her side of the kill says (he plays his out).
                    if at >= ragdoll_time(counselor_model.as_deref(), her_kill) {
                        let health = p.counselor.health;
                        p.counselor.take_damage(Hit { amount: health, blocked_with_weapon: false }, &ptuning.counselor);
                        commands.entity(pe).remove::<(Held, PlayAction)>();
                        info!("Vanessa: killed by Jason ({kill_name})");
                        if let Some((class, component)) = j.kill_score.take() {
                            commands.trigger(crate::match_flow::ContextKillScore { class, component });
                        }
                    }
                }
                if at >= killer.length(kill).unwrap_or(12.0) {
                    // Her side runs on past his (Head Stomp: his 8.47 s, hers 9.80 s, no shouldRagdoll): her special
                    // move's bDieOnFinish kills her as it ends.
                    let dies_at = ragdoll_time(counselor_model.as_deref(), her_kill);
                    if alive && p.counselor.alive() && dies_at > at {
                        commands.entity(pe).insert(FinishingKill { left: dies_at - at, kill_name, score: j.kill_score.take() });
                    }
                    // He stays where the kill took him (its root motion), not back at its start.
                    if let Some((_, last)) = killer.root_motion(kill) {
                        *xf = crate::character::flat(Transform::from_matrix(xf.to_matrix() * last.to_matrix()));
                        snap_rig(&mut rigs, je);
                    }
                    j.state = JasonState::Done;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Killing { t: at };
                }
            }
            JasonState::Stunned { t: at, stun } => {
                if at == 0.0 {
                    j.stun_wiggle = default();
                    j.stun_done = None;
                    j.next_wiggle = (-1.0, 0.0);
                }
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, None, dt, &t, &world);
                // Shaking it off: a player's GLB_Wiggle (FUN_1402dcfa0: -1 while E is down, here or from his client;
                // each press a wiggle); the AI wiggles at a steady rate (the axis flips every 1 / rate s).
                if let (Some(game), None) = (stun.minigame().map(|g| f13_core::stun::WiggleTuning { total_time: g.total_time * j.stun_scale, ..g }), j.stun_done) {
                    // The axis flips every AI wiggle interval.
                    if at >= j.next_wiggle.1 {
                        j.next_wiggle = (-j.next_wiggle.0, at + AI_WIGGLE_INTERVAL.0 + (AI_WIGGLE_INTERVAL.1 - AI_WIGGLE_INTERVAL.0) * rng.0.frand());
                    }
                    let input = match controls.as_deref() {
                        Some(c) => if c.keys.pressed(KeyCode::KeyE) { -1.0 } else { 0.0 },
                        None => j.next_wiggle.0,
                    };
                    if j.stun_wiggle.tick(dt, input, &game) {
                        info!("Jason: shook off the stun after {at:.1}s");
                        j.stun_done = Some(at);
                    }
                }
                let (enter, looped, exit) = stun.anims();
                let len = |n: &str| killer.length(n).unwrap_or(1.0);
                // The exit plays out the last part of the stun (from when he shook it off).
                let exit_at = match j.stun_done {
                    Some(d) => d,
                    None => exit.map_or(stun.seconds() * j.stun_scale, |e| (stun.seconds() * j.stun_scale - len(e)).max(len(enter))),
                };
                let end = exit.map_or(exit_at, |e| exit_at + len(e));
                let anim = if at < len(enter) {
                    enter
                } else if at < exit_at {
                    looped.unwrap_or(enter)
                } else {
                    exit.unwrap_or(enter)
                };
                commands.entity(je).insert(PlayAction(anim.into()));
                if at >= end {
                    info!("Jason: recovered");
                    j.sweater_stunned = false;
                    j.state = JasonState::Hunting;
                    j.cooldown = t.attack_delay;
                    j.stun_immune = STUN_IMMUNE_SECONDS;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Stunned { t: at, stun };
                }
            }
            JasonState::Searching { spot, t: at, end, anim } => {
                let at = at + dt;
                if at >= killer.length(anim).unwrap_or(6.0) {
                    *xf = end;
                    j.state = JasonState::Hunting;
                    j.searched.push(spot);
                    j.target_spot = None;
                    if let Ok((_, mut s, _)) = spots.get_mut(spot) {
                        s.searched(&mut commands);
                    }
                    commands.entity(je).remove::<PlayAction>();
                    snap_rig(&mut rigs, je);
                    info!("Jason: searched it: empty");
                } else {
                    j.state = JasonState::Searching { spot, t: at, end, anim };
                }
            }
            JasonState::HideKill { spot, t: at, end, anim, victim } => {
                let at = at + dt;
                if alive && at >= ragdoll_time(counselor_model.as_deref(), victim) {
                    let health = p.counselor.health;
                    p.counselor.take_damage(Hit { amount: health, blocked_with_weapon: false }, &ptuning.counselor);
                    info!("Vanessa: killed by Jason in her hiding spot");
                    if let Some((class, component)) = j.kill_score.take() {
                        commands.trigger(crate::match_flow::ContextKillScore { class, component });
                    }
                }
                if at >= killer.length(anim).unwrap_or(4.0) {
                    *xf = end;
                    j.state = JasonState::Done;
                    commands.entity(je).remove::<PlayAction>();
                    snap_rig(&mut rigs, je);
                    commands.entity(pe).remove::<crate::hiding::Hiding>();
                    if let Ok((_, mut s, _)) = spots.get_mut(spot) {
                        s.occupant = None;
                        s.searched(&mut commands);
                    }
                } else {
                    j.state = JasonState::HideKill { spot, t: at, end, anim, victim };
                }
            }
            JasonState::Flinching { t: at, anim } => {
                let at = at + dt;
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, None, dt, &t, &world);
                let end = killer.info.animations.get(anim).and_then(|a| a.windows.iter().find(|w| w.name == "OnEndBlockReaction")).map(|w| w.start);
                if at >= end.or(killer.length(anim)).unwrap_or(0.5) {
                    j.state = JasonState::Hunting;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    j.state = JasonState::Flinching { t: at, anim };
                }
            }
            JasonState::Done | JasonState::Dead => {
                walk(&mut j, &mut xf, Vec2::ZERO, t.walk_speed, None, dt, &t, &world);
            }
            JasonState::KillStance { t: at } => {
                let at = at + dt;
                let len = |n: &str| killer.length(n).unwrap_or(1.0);
                let (enter, idle, exit) = KILL_STANCE;
                // (Its montage has no exit take: the idle holds to the end.)
                let exit_len = killer.length(exit).unwrap_or(0.0);
                let exit_at = (KILL_STANCE_S - exit_len).max(len(enter));
                if let Some((take, by)) = kill_by {
                    // A counselor's kill (jason_death.rs): his take from where he kneels.
                    info!("Jason: being killed ({take})");
                    *xf = killer.place(take, *xf).0;
                    j.state = JasonState::Dying { t: 0.0, take, by };
                    commands.entity(je).insert(PlayAction(take.into())).remove::<KillJason>();
                    snap_rig(&mut rigs, je);
                } else if at >= exit_at + exit_len {
                    info!("Jason: rose from his kill stance");
                    j.sweater_stunned = false;
                    j.state = JasonState::Hunting;
                    j.stun_immune = STUN_IMMUNE_SECONDS;
                    commands.entity(je).remove::<PlayAction>();
                } else {
                    let anim = if at < len(enter) { enter } else if at < exit_at || exit_len == 0.0 { idle } else { exit };
                    commands.entity(je).insert(PlayAction(anim.into()));
                    j.state = JasonState::KillStance { t: at };
                }
            }
            JasonState::Dying { t: at, take, by } => {
                let at = at + dt;
                if at >= killer.length(take).unwrap_or(5.0) {
                    info!("Jason: dead");
                    j.state = JasonState::Dead;
                    // JasonDeath's KillInfo (KillJason_KillInfo_C): its ScoreEvent KillJason to her.
                    commands.trigger(Score { who: by, event: "KillJason" });
                } else {
                    j.state = JasonState::Dying { t: at, take, by };
                }
            }
        }
    }
}
