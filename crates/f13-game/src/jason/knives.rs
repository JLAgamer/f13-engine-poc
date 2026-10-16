//! His throwing knives: the pickups he takes, the throw, their flight and impacts, and the knife in his hand.

use super::*;

/// Jason_ThrowingKnifePickup's ContextMontage (AM_normal_stand_throwingknife_grab): its KnifePickup driver window
/// (0.89 s) calls PerformPickup.
pub const KNIFE_PICKUP: &str = "normal_stand_throwingknife_grab";
/// PerformPickup's SetLifeSpan (DAT_14207a218, s).
pub(super) const PICKUP_LIFESPAN_S: f32 = 5.0;

/// A knife pickup taken (MULTICAST_DisablePickup: on every machine): hidden, out of reach, gone after its lifespan.
#[derive(Event, Clone, Debug)]
pub struct PickupTaken(pub Entity);

pub(super) fn pickup_taken(ev: On<PickupTaken>, mut commands: Commands) {
    if let Ok(mut e) = commands.get_entity(ev.0) {
        e.insert((crate::props::PickedUp(PICKUP_LIFESPAN_S), Visibility::Hidden));
    }
}

/// The throwing knife pickup the played Jason's E takes: within its Interactable's DistanceLimit (+1 m, as the other
/// interactables here) and its yaw limit (MaxYawOffset 70); SCThrowingKnifePickup's CanInteractWith (0x140375000)
/// refuses him only while he holds her (he's hunting here). The nearest.
pub(crate) fn knife_pickup_at(
    pickups: &Query<(Entity, &crate::props::KnifePickup, &GlobalTransform, Has<crate::props::PickedUp>)>,
    limits: &Query<&crate::interact::InteractLimits>,
    centre: Vec3,
    look: Vec3,
) -> Option<Entity> {
    pickups
        .iter()
        .filter(|(e, k, g, taken)| {
            if *taken {
                return false;
            }
            let at = g.transform_point(k.interact);
            Vec2::new(at.x - centre.x, at.z - centre.z).length() * 100.0 <= k.reach + 100.0
                && limits.get(*e).ok().and_then(|l| l.0.get("Interactable")).is_none_or(|l| l.facing(g, centre - at, look))
        })
        .min_by(|a, b| a.2.transform_point(a.1.interact).distance(centre).total_cmp(&b.2.transform_point(b.1.interact).distance(centre)))
        .map(|(e, ..)| e)
}

/// Taken throwing knives go once their lifespan runs out (PerformPickup's SetLifeSpan). Jason takes them with their
/// pick-up move (update_jason: a played Jason's E; the computer Jason's when he has none).
pub(super) fn pick_up_knives(mut commands: Commands, time: Res<Time>, mut taken: Query<(Entity, &mut crate::props::PickedUp)>) {
    for (e, mut t) in &mut taken {
        t.0 -= time.delta_secs();
        if t.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
}

/// A knife leaves Jason's hand (`fly_knives` takes it from there).
#[derive(Event, Clone, Copy, Debug)]
pub struct ThrowKnife {
    pub from: Vec3,
    pub vel: Vec3,
    pub damage: f32,
}

/// A thrown knife: its velocity, age, damage; stuck once it hits something.
#[derive(Component)]
pub(super) struct Knife {
    vel: Vec3,
    age: f32,
    damage: f32,
    stuck: bool,
    /// Its dart (shown once it stops).
    dart: Option<Entity>,
}

/// The velocity that arcs a knife from `from` onto `to` at `speed` (his ThrowStrength, m/s) under its gravity (the low
/// arc); straight at it when it's out of reach.
pub(super) fn knife_velocity(from: Vec3, to: Vec3, speed: f32) -> Vec3 {
    let d = to - from;
    let flat = Vec2::new(d.x, d.z);
    let x = flat.length().max(1e-3);
    let (v, g, y) = (speed, KNIFE_GRAVITY, d.y);
    let disc = v.powi(4) - g * (g * x * x + 2.0 * y * v * v);
    if disc < 0.0 {
        return d.normalize_or_zero() * v;
    }
    let angle = ((v * v - disc.sqrt()) / (g * x)).atan();
    let h = flat / x * v * angle.cos();
    Vec3::new(h.x, v * angle.sin(), h.y)
}

/// ThrowingKnife_C's dart (its SkeletalMesh component): its scene and its place on the knife, thrown and in his hand.
/// The knife's Blueprint turns it to pitch 270 and hides it as the ProjectileMovement activates (the throw: the
/// particle trail shows instead) and shows it when the projectile stops; in his hand it's at its own place.
#[derive(Resource)]
pub(crate) struct KnifeLook(Handle<WorldAsset>, Transform, Transform);

/// The thrown knife's ImpactTemplate's sounds (Killer_ImpactEffect_ThrowingKnife_C: [`crate::props::impact_sounds`]),
/// its CounselorFX and its CounselorDecal.
#[derive(Resource, Default)]
pub(super) struct KnifeImpact(Vec<Option<String>>, Option<String>, Option<crate::decals::ImpactDecal>);

impl KnifeImpact {
    /// GetImpactSound for a hit on `surface`: its own (none if unset: "No Sound Cue to Play!"), else DefaultSound (a
    /// character's hit too: ThrowingKnifeBodyImpact; the knife isn't heavy).
    fn sound(&self, surface: u8) -> Option<String> {
        if (1..=22).contains(&surface) {
            self.0.get(surface as usize).cloned().flatten()
        } else {
            self.0.first().cloned().flatten()
        }
    }
}

/// His weapons' melee impacts (each weapon's ImpactEffectClass, a Killer_ImpactEffect_*: SCWeapon's SpawnHitEffect
/// 0x140456f20, from his OnHit 0x140401820 at every melee hit, spawns it ("<weapon>_Impact", 0x140452930) and plays it
/// (FUN_14035e130, as a gun's pellets: GetImpactFX, GetImpactSound, the decals), its heavy flag his anim instance's
/// bIsHeavyAttack), by weapon class.
#[derive(Resource, Default)]
pub(crate) struct SwingImpacts(pub HashMap<String, crate::props::ImpactClass>);

/// The knife in his hand while he aims (SERVER_SpawnKnife 0x1404051d0: a ThrowingKnifeClass actor, CurrentThrowable,
/// attached to his Mesh at KnifeAttachSocket, SnapToTargetNotIncludingScale; the constructor's KnifeAttachSocket is
/// handGrabSocket), until it leaves his hand at the throw or he puts it away.
#[derive(Component)]
pub(super) struct HeldKnife;

pub(super) fn load_knife_look(mut commands: Commands, export: Res<crate::ExportRoot>, assets: Res<AssetServer>) {
    use f13_core::camera_spline::quaternion;
    let Ok(classes) = f13_data::classes::ClassDb::load(&export.0.join("classes.json")) else { return };
    let Some((file, local)) = classes.get("ThrowingKnife_C").and_then(|c| c.components.iter().find(|c| c.name == "SkeletalMesh")).and_then(|c| Some((c.mesh.clone()?, c.local?))) else {
        warn!("ThrowingKnife_C's dart isn't in classes.json (re-export: f13-export levels): stuck knives aren't shown");
        return;
    };
    // K2_SetRelativeRotation(FRotator(270, 0, 0)) as it's thrown.
    let held = crate::to_transform(&local);
    let local = f13_data::level::Xform { r: quaternion(Vec3::new(270.0, 0.0, 0.0)).to_array(), ..local };
    commands.insert_resource(KnifeLook(assets.load(GltfAssetLabel::Scene(0).from_asset(file)), crate::to_transform(&local), held));
    if let Some(t) = classes.prop("ThrowingKnife_C", "ImpactTemplate").as_str() {
        let fx = classes.prop(t, "CounselorFX")["AssetPathName"].as_str().and_then(|a| a.rsplit('.').next()).map(String::from);
        let decal = crate::decals::ImpactDecal::from_json(classes.prop(t, "CounselorDecal"));
        commands.insert_resource(KnifeImpact(crate::props::impact_sounds(&classes, t), fx, decal));
    }
    // His weapons' impact classes.
    let mut swings = SwingImpacts::default();
    for w in weapons() {
        let Some(t) = classes.prop(&w.class, "ImpactEffectClass").as_str().map(String::from) else { continue };
        swings.0.insert(w.class.clone(), crate::props::ImpactClass::load(&classes, &t));
    }
    if swings.0.is_empty() {
        warn!("no Jason weapon's ImpactEffectClass in classes.json (re-export: f13-export levels): his swings' hits are silent");
    }
    commands.insert_resource(swings);
    if let Some(c) = classes.get("Underwater_GrabKill_C").and_then(|c| c.components.iter().find(|c| c.kind == "SCCameraSplineComponent")) {
        commands.insert_resource(DrownKillCamera(crate::to_transform(&c.local.unwrap_or(f13_data::level::Xform::IDENTITY))));
    }
    // His grab kill slots (his killer JSON's names).
    let killer = std::fs::read_to_string(export.0.join("killers").join(format!("{}.json", crate::character::killer_class())));
    let killer_json = killer.ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).unwrap_or_default();
    if let Some(names) = Some(killer_json["Names"].clone()).filter(|n| !n.is_null()) {
        if names.get("DefaultGrabKill").is_some() {
            let slots = slots_from(&names);
            *DEFAULT_SLOTS.write().unwrap_or_else(|e| e.into_inner()) = Some(slots);
            *GRAB_KILL_SLOTS.write().unwrap_or_else(|e| e.into_inner()) = Some(slots);
            // His own weapon (Menu.WeaponClass, else his DefaultGrabKill's) and the ones he may carry.
            let menu = &killer_json["Menu"];
            let own = menu["WeaponClass"].as_str().map(str::to_string).or_else(|| {
                let d = names["DefaultGrabKill"].as_str().unwrap_or("").trim_end_matches("_C");
                kill_index(d).map(|k| kills()[k].weapon.clone())
            });
            let allowed: Vec<String> = menu["AllowedWeapons"].as_array().into_iter().flatten().filter_map(|w| w.as_str().map(str::to_string)).collect();
            if let Some(own) = own {
                *OWN.write().unwrap_or_else(|e| e.into_inner()) = Some((own, allowed, names.clone()));
                set_weapon("");
            }
        } else {
            warn!("{}'s grab kills aren't exported (re-export: f13-export killer): Part 2's", crate::character::killer_class());
        }
    }
    // The grab kills' KillBounds and HUD icons.
    let textures: std::collections::HashMap<String, String> =
        std::fs::read_to_string(export.0.join("ui").join("textures.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let mut info = GrabKillInfo { bounds: vec![None; kills().len()], icons: vec![None; kills().len()] };
    for (i, Kill { class, .. }) in kills().iter().enumerate() {
        let Some(c) = classes.get(&format!("{class}_C")) else {
            warn!("{class}_C isn't in classes.json (re-export: f13-export levels): its grab kill always fits and has no icon");
            continue;
        };
        if let Some(b) = c.components.iter().find(|c| c.name == "KillBounds") {
            let e = &b.props["BoxExtent"];
            let v = |k: &str| e[k].as_f64().unwrap_or(32.0) as f32;
            info.bounds[i] = Some((crate::to_transform(&b.local.unwrap_or(f13_data::level::Xform::IDENTITY)), Vec3::new(v("X"), v("Z"), v("Y")) / 100.0));
        }
        let icon = c.props["ContextKillHUDIcon"].as_str().and_then(|s| s.split('\'').nth(1));
        info.icons[i] = icon.and_then(|n| textures.get(n).cloned());
    }
    commands.insert_resource(info);
}

/// The knife's rotation: its forward (Unreal +X) along its velocity (its ProjectileMovement's bRotationFollowsVelocity).
pub(super) fn knife_rotation(vel: Vec3) -> Quat {
    Transform::IDENTITY.looking_to(vel.normalize_or(Vec3::X), Vec3::Y).rotation * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
}

/// His knife in hand: shown on KnifeAttachSocket while he aims, until it's thrown (or put away).
#[allow(clippy::type_complexity)]
pub(super) fn held_knife(
    mut commands: Commands,
    look: Option<Res<KnifeLook>>,
    killer: Option<Res<KillerCharacter>>,
    jasons: Query<&Jason>,
    rigs: Query<&Rig, Without<HeldKnife>>,
    globals: Query<&GlobalTransform>,
    mut held: Query<(Entity, &mut Transform), With<HeldKnife>>,
) {
    let (Some(look), Some(killer)) = (look, killer) else { return };
    let aiming = jasons.iter().any(|j| j.aiming_knife());
    let hand = rigs.iter().find(|r| matches!(r.model, crate::character::Model::Killer)).and_then(|r| r.socket(&killer.info, HAND_GRAB_SOCKET, &globals));
    match (aiming.then_some(hand).flatten(), held.single_mut()) {
        (Some(hand), Ok((_, mut xf))) => *xf = hand,
        (Some(hand), Err(_)) => {
            let knife = commands.spawn((HeldKnife, hand, Visibility::default())).id();
            commands.spawn((WorldAssetRoot(look.0.clone()), look.2, ChildOf(knife)));
        }
        (None, Ok((e, _))) => commands.entity(e).despawn(),
        (None, Err(_)) => {}
    }
}

pub(super) fn spawn_knife(ev: On<ThrowKnife>, mut commands: Commands, look: Option<Res<KnifeLook>>) {
    let knife = commands.spawn((Transform::from_translation(ev.from).with_rotation(knife_rotation(ev.vel)), Visibility::default())).id();
    let dart = knife_look(&mut commands, look.as_deref(), knife);
    commands.entity(knife).insert((Knife { vel: ev.vel, age: 0.0, damage: ev.damage, stuck: false, dart }, crate::net_world::NetVisual::Knife { stuck: false }));
}

/// A thrown knife's particle system's actor name ([`stop_knife_flight`]).
pub(super) fn knife_actor(knife: Entity) -> String {
    format!("thrown knife {}", knife.to_bits())
}

/// A thrown knife stuck: its flight's system stops and its spinning dart goes (PLACEHOLDER: the game's OnStop
/// (0x1403fe870) attaches it to what it hit; what ends P_Jason_ThrowingKnife_01's 1 s dart there isn't read: the user's
/// LAN test, 2026-10-14, saw it float and spin), the trail left to fade.
pub(crate) fn stop_knife_flight(commands: &mut Commands, knife: Entity) {
    commands.write_message(crate::particles::StopMeshParticles { actor: knife_actor(knife) });
}

/// A thrown knife's looks on `knife`: its dart (hidden while it flies) and its particles. Returns the dart.
pub(crate) fn knife_look(commands: &mut Commands, look: Option<&KnifeLook>, knife: Entity) -> Option<Entity> {
    let dart = look.map(|l| commands.spawn((WorldAssetRoot(l.0.clone()), l.1, Visibility::Hidden, ChildOf(knife))).id());
    // MULTICAST_Use (0x1403f7170) activates its ParticleSystem (P_Jason_ThrowingKnife_01 on its root): the spinning
    // dart (a mesh emitter) while it flies, and its ribbon trail (TracerTrail).
    commands.write_message(crate::particles::SpawnParticles {
        system: "P_Jason_ThrowingKnife_01".into(),
        transform: Transform::IDENTITY,
        follow: Some(knife),
        actor: knife_actor(knife),
        active: true,
        time_dilation: 1.0,
    });
    dart
}

/// Knives fly (ProjectileGravityScale 8), hit her (its damage, a hit reaction) or stick in what they hit, and go after
/// TimeToDestroy.
#[allow(clippy::type_complexity)]
pub(super) fn fly_knives(
    mut commands: Commands,
    time: Res<Time>,
    ptuning: Res<PlayerTuning>,
    counselor_model: Option<Res<crate::character::CounselorCharacter>>,
    collision: crate::collision::Collision,
    mut knives: Query<(Entity, &mut Knife, &mut Transform), Without<Player>>,
    mut players: Query<(Entity, &mut Player, &Transform)>,
    (impact, sounds): (Option<Res<KnifeImpact>>, Option<Res<crate::audio::Sounds>>),
) {
    // The ImpactTemplate's sound where it hit (heard by everyone: net_world shout).
    let impact_at = |commands: &mut Commands, at: Vec3, surface: u8| {
        if let Some(cue) = impact.as_ref().and_then(|i| i.sound(surface)) {
            crate::net_world::shout(commands, crate::net_world::Shout::Sound(crate::audio::PlaySound { cue, at: Some(at) }));
        }
    };
    let dt = time.delta_secs();
    let world = collision.all();
    for (ke, mut k, mut xf) in &mut knives {
        k.age += dt;
        // (Its life from when it stuck; a flight never ends on its own in the game, here after as long.)
        if k.age >= KNIFE_LIFE {
            stop_knife_flight(&mut commands, ke);
            commands.entity(ke).despawn();
            continue;
        }
        if k.stuck {
            continue;
        }
        k.vel.y -= KNIFE_GRAVITY * dt;
        let (k_vel, k_age) = (k.vel, k.age);
        let from = xf.translation;
        let to = from + k.vel * dt;
        // Her: the segment passing within her capsule (a 0.3 m radius around her from her feet to 1.8 m).
        let mut struck = false;
        for (pe, mut p, pxf) in &mut players {
            let (a, b) = (pxf.translation + Vec3::Y * 0.3, pxf.translation + Vec3::Y * 1.5);
            if p.counselor.alive() && segment_distance(from, to, a, b) < 0.35 {
                let r = p.counselor.take_damage(Hit { amount: k.damage, blocked_with_weapon: false }, &ptuning.counselor);
                info!("Jason's knife hit Vanessa for {:.0} (health {:.0})", r.dealt, p.counselor.health);
                impact_at(&mut commands, from, 0);
                // SCThrowable::OnStop (0x1403fe870) in a character: its JasonImpactNoise
                // (ThrowingKnifeImpactConfirmation) in 2D for its thrower's player.
                crate::net_world::shout(&mut commands, crate::net_world::Shout::JasonSound(crate::net_world::JasonSound("ThrowingKnifeImpactConfirmation".into())));
                // Its CounselorFX (P_Hit_Blood_Counselor_Default_01) where it struck, and its CounselorDecal: its gash on
                // her (following her), its splashes on what's around her (along the knife's way and down).
                // PLACEHOLDER: her surface there, 0.15 m from her capsule's axis toward the knife (no mesh trace).
                if let Some(k) = impact.as_deref() {
                    let dir = k_vel.normalize_or(Vec3::Z);
                    if let Some(fx) = &k.1 {
                        commands.write_message(crate::particles::SpawnParticles::impact(fx, from, -dir));
                    }
                    if let Some(d) = &k.2 {
                        let mut seed = (k_age.to_bits() as u64 ^ 0x9e37_79b9) | 1;
                        let mut r = || {
                            seed ^= seed << 13;
                            seed ^= seed >> 7;
                            seed ^= seed << 17;
                            (seed % 32768) as f32 / 32767.0
                        };
                        let axis = from.clamp(a.with_y(a.y.min(b.y)), b.with_y(a.y.max(b.y))).with_x(a.x).with_z(a.z);
                        let out = (from - axis).normalize_or(-dir);
                        // And the blood on her clothes there (blood.rs).
                        commands.trigger(crate::blood::ApplyBlood { counselor: pe, at: axis + out * 0.15 });
                        crate::decals::spawn_impact(&mut commands, d, axis + out * 0.15, out, dir.with_y(0.0).normalize_or(Vec3::Z), Some(pe), &mut r, |a, b| {
                            f13_core::collision::segment_hit_tri(&world, a, b, 2.0).map(|(t, n, _)| (a + (b - a) * t, n))
                        });
                    }
                }
                if r.died {
                    commands.trigger(crate::blood::Dismember { counselor: pe, at: Some(from) });
                    commands.trigger(crate::match_flow::KillerScore("Butcher"));
                    commands.trigger(crate::match_flow::KillerScore("KillCounselor"));
                    commands.entity(pe).insert(PlayAction(DEATH.into())).remove::<(Staggered, crate::items::ItemAction)>();
                } else {
                    let reaction = hit_reaction(pxf, from);
                    let len = counselor_model.as_ref().and_then(|c| c.length(reaction)).unwrap_or(0.67);
                    commands.entity(pe).insert((PlayAction(reaction.into()), Staggered(len))).remove::<crate::items::ItemAction>();
                }
                // OnStop in a character: destroyed at once (its flight's particles stopped first: the spinning dart's
                // mesh particles would stay where it was).
                stop_knife_flight(&mut commands, ke);
                commands.entity(ke).despawn();
                struck = true;
                break;
            }
        }
        if struck {
            continue;
        }
        if let Some((f, _, surface)) = f13_core::collision::segment_hit_tri(&world, from, to, 1.0) {
            xf.translation = from + (to - from) * f;
            let at = xf.translation;
            let surface = if surface == f13_core::collision::Tri::TERRAIN { sounds.as_ref().map_or(0, |s| s.terrain_surface(at.x, at.z)) } else { surface };
            impact_at(&mut commands, at, surface);
            k.stuck = true;
            let n = f13_core::collision::segment_hit_normal(&world, from, to, 1.0).map_or(Vec3::ZERO, |h| h.1);
            let fixed = f13_core::collision::segment_hit(world.0, from, to, 1.0).is_some();
            info!("Jason's knife stuck at {} after {:.2} s (from {from}, normal {n}, {})", xf.translation, k.age, if fixed { "the level" } else { "a prop" });
            k.age = 0.0;
            // OnProjectileStop: the dart shows.
            if let Some(d) = k.dart {
                commands.entity(d).insert(Visibility::Inherited);
            }
            stop_knife_flight(&mut commands, ke);
            commands.entity(ke).insert(crate::net_world::NetVisual::Knife { stuck: true });
            continue;
        }
        xf.translation = to;
        xf.rotation = knife_rotation(k.vel);
    }
}

/// The closest distance between segments p0..p1 and q0..q1.
pub(super) fn segment_distance(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> f32 {
    let (d1, d2, r) = (p1 - p0, q1 - q0, p0 - q0);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (s, t) = if a <= 1e-9 && e <= 1e-9 {
        (0.0, 0.0)
    } else if a <= 1e-9 {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= 1e-9 {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let mut s = if denom > 1e-9 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let mut t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
            (s, t)
        }
    };
    (p0 + d1 * s).distance(q0 + d2 * t)
}
