# Roadmap

## Milestone 1: MVP
One counselor (Vanessa) vs Jason Part 2 on Crystal Lake (small), with a single escape: the 2-seat car.
All gameplay values come from the player's own install (see README).

Content (asset paths in the player's install):
- Map: `Maps/CrystalLake_Small`
- Counselor: `Characters/Counselors/Athlete` (Vanessa)
- Jason: `Characters/Killers/Jason/J2` (Part 2)
- Car: `Blueprints/Vehicles/DriveableVehicles/Cars/Chevette/Chevette_2Seat_Bp`
  - Gas tank repair needs `CarGasCan`, hood repair needs `BP_CarBattery`
  - Starting the car and the driver seat need `CarKeys`; the second seat is a front passenger seat
  - Crystal Lake's world settings spawn both the 2-seat and the 4-seat Chevette; f13-rs follows the game

Mechanics: counselor side
- [x] Stats and stat-modified values
- [x] Fear (events, caps, delays, throttled updates)
- [x] Stamina, walk/run/sprint/crouch speeds, sprint rules, wounded state (f13-core::counselor)
- [x] Unreal walking movement: acceleration, friction, braking (f13-core::movement)
- [x] Stumbles: random (fear and stamina curves, wounds, stats, cooldown) and on running out of stamina (f13-core::stumble)
- [x] Combat stance and dodge: double-tap, stamina cost, root motion (f13-core::dodge, anims.json)
- [x] Swimming (water.rs, player.rs): the map's water volume (Swim_PhysicsVolume's brush box); she swims while her
      centre is under its surface (IsInWater), afloat at the surface, out where the bottom comes up; the swim
      blend (idle, slow, fast, sprint), swimming speed and stamina, the InWater fear event
- [ ] Swimming, remaining: diving, Jason in water
- [x] Fear events pushed in the client (fear_events.rs): darkness (f13-core::light, the map's point and spot lights),
  indoors (the cabins' indoor volumes, also used by Sense), wounded, Jason near and in view, grabbed, hidden, items
  held, in water, blackouts (inside a grid's volume while its power is out)
- [ ] Fear events, remaining: near friendlies and corpses (other counselors)
- [x] Hit reactions and death animations when Jason hits
- [x] Collision: the counselor's capsule (34 x 82 cm) against the game's simple collision (convex hulls, boxes,
  spheres, capsules of every mesh whose profile blocks pawns, BlockingVolumes, terrain), doors, windows and cars
  moving with their actors; steps (45 cm), walkable slopes (44.8°), falling; spawns put on the floor, and a
  character that ends up under the map is put back on top (f13-core::collision, f13-game collision.rs)
- [x] Inventory: large item (hands) and small items (pockets, 3); pickup, drop; use: first aid (other items with combat)
- [x] Repairs: car battery and gas, keys at the ignition (from the driver's seat)
- [x] Repair minigame (hold timer, pips, hit bonus, miss rollback, killer notify) and the real engine start time
  (f13-core::minigame)
- [x] Car: enter/exit, start, escape volume
- [x] Car handling from the game's vehicle setup (f13-core::vehicle): the movement component's torque curve,
      MaxRPM, automatic gearbox (ratios, shift points, latency), idle brake, the wheels' radius and steering lock,
      Unreal's defaults (mass, final ratio, brakes, drag, steering curve); a reduced model of the PhysX vehicle (no
      sliding, grip-capped drive, revs follow the wheels)
- [ ] Car handling, remaining: suspension, sliding, collisions' impulses
- [x] Cabinets (drawers): search, items inside
- [x] Doors: leaves exported and swinging on their pivot, open/close, lock/unlock by side (doors.rs; swing timing is a
  placeholder for the special-move montages)
- [x] Barricades (replace the lock, only on a closed door) and windows: open, close, vault, dive (breaks the glass,
  DiveDamage); windows.rs
- [x] Hiding: wardrobes, outhouses, beds, tents (hiding.rs: enter/idle/exit montages, hiding camera; Jason's search
  and hiding spot kills)
- [x] Hiding spots placed in the level (the outhouses and tents: 15 more) are usable, their parts spawned at runtime
      (they were drawn only, without their skeletal parts)
- [x] Hiding spot doors (the wardrobe's two, swinging out now; the outhouse's): Armoire_C's DoorRotation timelines
      (PLACEHOLDER: when they open and close within the move)
- [x] The tent's flaps: its own enter and exit takes (normal_stand_hidespot_tent_enter/exit) as she goes in and out
- [x] Health, wounded, death, blocking damage, first aid (f13-core::health)
- [x] Items against Jason (items.rs): weapon swings (stun chance, wear, breaking), flare gun, shotgun, firecrackers,
  bear traps, the map; stun times and animations from the game (swing reach/angle, throw speed and gun fire timing
  are placeholders)
- [x] Grab escape (wiggle, stat-scaled, 10/s cap) and the pocket knife
- [x] Blocking with a weapon (items.rs): right mouse in combat stance; CanBlock's conditions; BlockingDamageMod and the
  weapon's BlockingWear on a blocked hit; the block idle and reaction animations (the idle replaces the combat idle
  rather than playing in an upper-body slot)
- [x] Counselors caught in armed bear traps (items.rs; their own too, as bCanArmerTrigger allows): TrapDamage, the
  caught/struggle/exit animations by foot and gait, the game's stun minigame (f13-core::stun, also the grab struggle
  now: the larger of time and a wiggle meter that drains)
- [x] Jason shaking off stuns: the stuns with a minigame (weapon, flare, shotgun, bear trap; their DepletionRate) run
  f13-core::stun for him too, the computer Jason wiggling every SCCharacterAIProperties' 0.15..0.2 s
- [x] Jason's heavy attack (the pickaxe's ForwardAttack), used at a raised guard: it ends the block
- [x] The bear trap's own animations (prop_anim.rs): shut while lying about or carried, opening as it's armed
      (ArmedAnimation), snapping shut on a foot (TriggeredAnimation), opening as she steps out (EscapedAnimation)
- [x] Ragdoll on death (f13-core::ragdoll, the physics asset's bodies against the world collision); in kills at the
  counselor montage's shouldRagdoll notify

Mechanics: Jason side
- [x] Ability unlock order, recharge, rage
- [x] Jason Part 2 as a computer player (jason.rs): J2's movement values, paths over the collision (f13-core::nav),
  opening doors (Jason_OpenDoorInside/Outside) and breaking locked or barricaded ones (Jason_BreakDoor_Axe's
  Enter/Loop/Success sections; BaseDoorHealth vs weapon damage, halved by a barricade), pickaxe swings (damage,
  AttackDelay, reach/angle, hits in the AttackTraceNotify window), grabs (min/max distance, angle) and the pickaxe's
  grab kill (Throat Hack, both sides' paired animation)
- [x] Jason's perception (senses.rs, f13-core::perception): sight, the game's noise blips, Sense; he investigates
  clues and searches nearby hiding spots
- [x] Navmesh built at load time from the collision with the game's Recast settings (f13-core::recast/navmesh):
  ChunkyMonotone regions merged as the cooked navmesh shows (93% the same walkable area, a few percent apart in
  polygon count), tiles on the game's grid; paths pulled taut and shortened along the mesh; the HUD shows the build.
  The cooked navmesh is exported (navmesh-<agent>.json) as a reference
- [ ] Navmesh, remaining: detail meshes (heights come from the polygon corners), border vertex removal
- [x] Holding breath while hiding (f13-core::breath, hiding.rs): the breath meter's drain, refill, threshold and wait;
  her HoldBreath / HoldBreathEnd / KillerIsNearWhileHiding voice lines; Jason hears the last within its range
  (placeholder distance and repeat delay)
- [x] Kill cameras (f13-core::camera_spline, kill_camera.rs): SCSplineCamera riding the kill's camera spline
      (holds, times to the next point, Unreal InterpCurves, field of view, the handicam shake) in the kill's space
- [x] Kill cameras' CameraTrace (SCSplineCamera's tick): something between the camera and what it looks at pulls the
      camera to it and widens its field of view (indoor kills no longer film from behind walls)
- [ ] Kill cameras, remaining: blending back
- [x] Playing as Jason (`--play-jason`, jason_player.rs): walk, run, look; swing, heavy swing, grab and kill, knives
      where he looks, traps, Shift, Morph (the point nearest 30 m ahead), Stalk; doors as he walks into them; his
      camera; the escape bot plays the counselor
- [ ] Playing as Jason, remaining: Jason's HUD (KillerHUDWidget), Sense on demand, the map for Morph, choosing kills
- [x] Shift, Morph, Stalk (f13-core::killer_moves, jason.rs): Shift's 0.7 s wind-up, ShiftSpeed and ShiftTime (J2:
  1000 cm/s, 4 s), ending in a grab; Stalk's StalkActiveTimer (IdleStalkModifier standing still); Morph to the
  nearest SCKillerMorphPoint within 30 m of the spot after TeleportScreenTime; the teleport fade (he's hidden below
  half). When the computer Jason uses them is a placeholder
- [ ] Shift, Morph, Stalk, remaining: the dissolve and screen effects, Stalk's and Shift's sounds and music, Morph's
      excluded areas
- [x] Throwing knives (J2's two, the knife's damage and cooldown, the throw montage and its ThrowKnife notify,
  SCJasonAIProperties' 5..25 m; placeholder knife speed) and Jason's traps (J2's seven KillerTraps, set in doorways
  he opens; they catch counselors, not him; his trap's own mesh isn't exported)
- [x] Knife pickups: the map's ThrowingKnifePickups (large items) add their StackSize to Jason's knives as he passes
      (AddKnives, no cap); the computer Jason doesn't seek them out, and there's no pick-up move
- [x] More kills: the pickaxe's five grab kills (Throat Hack, Can Opener, Curb Stomp, Sweep The Leg, Top Popper),
      paired montages and kill cameras; the AI picks one at random, `--kill <name>` forces one
- [ ] Kills, remaining: the other weapons' kills, environment (placed) kills, choosing kills as the player
- [x] Generators (power.rs): the fuse boxes and their power grids; a destroyed fuse box puts out the powered lights
      in its grid's box (and they stop lighting her: darkness fear) and lights its red power-out light; counselors
      repair it (hold E, 15 s, pips), Jason destroys it (the player's E: Jason_Generator_Destruction_Machete's
      montage, the power going in its DestroyGenerator window); its generator hums while it's intact. The game's
      computer Jason doesn't cut power (its behavior assets don't use fuse boxes)
- [x] Breaking walls (SCDestructibleWall in the cabin layouts): Jason breaks through a wall in his way (the player
      walking into it; the AI when it stands between him and her), the door break's montage and timing, his world
      damage against its BaseHealth 60; the section and its collision go
- [ ] Breaking walls, remaining: the wall break montages, dust (its section's chunks fly as the doors' do)
- [x] Car interactions: Jason in front of (or behind) a moving car stops it dead (KillersInFrontOfCar); with the car
      under MinUseCarDoorSpeed (50 cm/s), Jason at the driver door pulls her out into his grab (the AI goes for the
      door; the player grabs)
- [x] Pulled out of the car: Jason on the car's DriverGrab spot, both sides' paired take (AM_normal_stand_car_grab_left,
      she on his GrabSocket as in kills), into his grab
- [x] A car hitting Jason at speed: he slams it (AM_normal_stand_car_stop_slam, its crash sound) as it stops dead
- [ ] Car interactions, remaining: passengers, the car running counselors over

Match
- [x] Match flow (f13-core::game_mode, match_flow.rs): the game mode's states and timer, the end (dead or
  escaped, or time out), outro wait, results with the score events' XP (awarded where the game does), leaving
- [ ] Match flow, remaining: lobby, level intro and outro sequences, police and Tommy Jarvis timers, badges, Jason
      dying
- [x] Item and car spawning, as the game mode does it (f13-core::spawn, f13-data::world)
- [x] How the game chooses one of the four cabin layouts per match: SCWorldSettings' RandomLevels, one picked at
  random when the level loads (`--layout` overrides)
- [ ] Level choices, remaining: the rain roll (RainChance, the `rain=` option; the rain level isn't used), the
      randomized pods

Host CLI (`f13-host`; f13-game `--host`, host.rs: line commands on 127.0.0.1)
- [x] Spawn items (`give <Class>`: one of the match's items of the class into her hands or pockets)
- [x] Teleport players (`teleport counselor|jason x y z`)
- [x] Make a player Jason, live in game (`jason`: his controls and camera take over; the counselor is left
      without a player)
- [x] Force start and end matches (`start`, `end`), `status`
- [x] Kick players (`kick`: single-player, so the game closes)

Presentation
- [x] Map, characters and car exported to glTF (`tools/f13-export`)
- [x] Map loaded in Bevy with its cabin layouts (`f13-game`)
- [x] Materials: cabins (Master_Blend_01 two-set vertex-alpha blend, tinted blends, curtains), props/items/car (RMA),
      foliage (RSS roughness, mask threshold) and terrain layers (height-blended weight
      maps), ported from the game's compiled shaders; per-placement material overrides; glass; invisible blockers
      hidden (f13-game materials.rs, shaders/); textures as the game's compressed DDS data with mips
- [x] Light fixtures' glow: M_Prop_Master_Lights' emissive (Diffuse.rgb * Diffuse.a * Glow, from its compiled base
      pass); a powered light's fixture glows at 10 while its grid has power and goes dark in a cut (the light
      component's OnVisibilityChanged), seen in screenshots. The fixtures hanging off powered light components were
      exported at their offset from the light (piled near the map origin): now placed in the cabins
- [x] Power curves: none on Crystal Lake (only Higgins Haven's layouts give lights a PowerOff/PowerOn flicker curve; without
      one the light component switches at once, as here)
- [ ] Materials, remaining: water (first pass only: colour, gloss, panning ripples; no depth fade, foam or
      reflections), POM parallax, the other emissive masters (signs, the CB tower, the phone box), foliage
      subsurface, the Chevette's paint mask
- [x] Car, furniture, doors (frames and leaves), windows, barricades and items placed in the scene
- [x] Each placement's CastShadow and cull distance (foliage InstanceEndCullDistance, max draw distance); sun shadows
      to 100 m in 3 cascades
- [x] Door damage states (the Light/Medium/Heavy destructible meshes at 75/50/25% health); Jason's rage burst
- [x] Door break effects: the leaf's destructible chunks fly from the blow (Break/BurstExplosionForce; PLACEHOLDER
      flight, not the APEX simulation), fall to the floor and clear after a few seconds; the break's sounds (the door's
      DoorDestroySoundCue, the montage's notifies)
- [x] Barricade plank: checked; across the door 0.8 m up, just inside it (the component named `..._01_b` holds the
  `_01_a` mesh; the leaning `_01_b` stands against the frame when it's down)
- [x] Counselor model: Vanessa's base body, default outfit (master pose), face on the head bone, hair on the face, skin
      clipped under the clothes by the outfit's skin mask (f13-game character.rs, shaders/skin.wgsl)
- [ ] Counselor model, remaining: facial animation and hair dynamics (their own anim Blueprints), the eyes' iris and
      lid overlays, outfit choice
- [x] Jason model (Part 2): body, sack mask (master pose), pickaxe on weapon_attach_r; locomotion
  (BS_Jason_Normal_Move), combat idle, swing, grab, grab kill
- [x] Characters placed in the scene (Vanessa at a counselor start, Jason at the killer start)
- [x] Animations: female sequences retargeted as Unreal does (rotations from the animation, translations only on the
      skeleton's "Animation" bones), blended like Athlete_AnimBP's blendspaces: idle/walk/run/sprint, wounded idle,
      crouch, combat strafing (8 directions), dodges, stumbles, first aid
- [x] Repair animations: her takes of the car repairs' montages (gas can, battery): start, cycle while she works,
      stop when done, cancel when she lets go
- [x] Sitting in the car: her capsule at the driver's seat's AttachSocket (SeatSocketFL on the car mesh; the exporter
      writes skeletal meshes' sockets in component space), playing f_normal_sit_car_drive_forward; getting in and out,
      her enter and exit takes with their root motion (enter ends in the seat, exit starts there)
- [x] Starting off and stopping: her forward start (walk, run, sprint) as she sets off ahead, her stop (by her
      fastest gait, on the foot her last step left forward) as she halts; their footstep notifies (PLACEHOLDER rules)
- [ ] Animations, remaining: directional starts (BS_F_Start_Normal_*Hemisphere 45-180: root rotation), foot-marker
      sync (now normalised time), aim offsets (Base_Counselor_AnimBP's AimOffsets, the Hunter's poses on the shared
      skeleton, by AimPitch while bIsAiming), BS_Run_Lean. Not in the game: turn-in-place takes, limp movement
      (bShouldPlayWounded only swaps in F_hurt_stand_idle_01, done)
- [x] Door special moves: her open and close takes (inside or outside), the leaf swinging in the montage's
      OpenCloseDoor driver window; she stands while they play
- [x] Window special moves: her open and close takes (the pane moving in the montage's WindowMoveDriver window),
      climbing through and diving (her take carries her across; a dive breaks the glass at its WindowBreakDriver
      and ends in the dive landing)
- [x] Door lock and unlock: her takes while she holds E
- [x] Barricade special moves: her pull-down and push-up takes (Counselor_Barricade_Pulldown / PushUp), the
      barricade moving as their UseBarricade window ends
- [x] Through windows on the takes' own root motion: the climb, the dive and its landing play from the window's
      CounselorVault / CounselorDive special move spot on her side
- [x] Doors on root motion: her open and close takes play from the door's special move spots (opening from outside
      carries her into the doorway, closing from outside walks her back out); locks and unlocks on theirs
- [ ] Special moves, remaining: the repairs' own root motion (they play on the repair spot)
- [x] Repair minigame dial UI: SCInteractMinigame_Widget (its progress circle filled clockwise, the needle turned, the
      pips as red arcs of its ring, the next pip's mouse button in the middle), for every hold interaction
- [x] HUD (ui.rs): CounselorHUDWidget from its UMG tree (f13-data::umg lays it out as Slate does), the game's font
      and textures: timer, stamina ring (StaminaBarMaterial as a radial-fill UI material), inventory (item widgets
      with their ImageOn icons, key numbers), breath bars while hiding; the game has no health or fear meter (F3:
      debug text)
- [x] Minimap map: the level's SCMiniMapVolume (its MapTexture over its brush's area) in SC_MiniMap_Widget's circle,
      on her, turned with the camera, the compass's N kept on north (DefaultMinimapZoom as the share of the map shown:
      PLACEHOLDER); widgets' RenderTransforms (the view cone's flip)
- [ ] HUD, remaining: the minimap's icons, the noise meter, help texts, XP popups, ability widgets
- [x] Night lighting, fog, flashlight (lighting.rs): the map's point/spot lights (inverse-square lumens, or Unreal's
  falloff-exponent lights matched at a third of their radius; colour temperature; draw-distance fades), the moon and
  height fog of the clear-weather level (CLsm_RainOff), the post-process volume's fixed exposure and ambient cubemap,
  the sky's colour; the counselor's flashlight (F; its Blueprint values, attachment, turn rate, wall check; lit for
  fear); power cuts (power.rs)
- [ ] Lighting, remaining: Enlighten GI, local light shadows, light functions, colour grading LUT/contrast, bloom,
      the sky texture, rain
- [ ] Audio (audio.rs, f13-data::sound): done: SoundCue playback (the cue graph's random, modulator, mixer, looping and
  attenuation nodes; Unreal's attenuation falloff; left/right panning around the camera) and the counselor's
  footsteps (her locomotion's Footstep notifies; the surface under her foot: a mesh's physical material or the
  terrain's heaviest paint layer; Base_Counselor_AnimBP's tables); doors, windows and barricades (their classes' cues);
  Jason's near music (NearCounselorMusic by distance, silenced by Stalk); the counselor's music (MusicCounselor:
  MatchStart, CounselorIdle after each track, the two-minute warning, death and survival music; fades); Jason's
  footsteps (Jason_AnimBP's tables, a step each half of his locomotion cycle); her landing (GetLandingSound, down
  from a fall) and wading steps (GetFootstepInWater: shallow or deep); items put down (DroppedSoundCue); the car's
  key turning and engine running (EngineStartCue, EngineCue, on the car); walls' hit and break cues; the
  ability-unlock warning (KiKiKiMaMaMa) and JasonTeleportedNearStinger; the animations' own sounds (their
  AnimNotify_PlaySound notifies, from each take's sequence and the montage segments playing it: anim_sounds.json):
  item use (first aid, flare, firecracker, trap), swings, dodges, doors, locks, windows, hiding spots, repairs,
  Jason's swings, door breaking, knives, traps and kills; the cycles' (the swim strokes) by their phase; the map's
  ambience (CrystalLake_Sm_Audio's AmbientSounds: the campground and mood tone loops, the lake's within its box
  attenuation); the car's horn (H) and brakes (their audio components' cues); her voice lines (Impact, Wounded,
  Choke, Stumble, Died, swings, windows, each kill's, stunning Jason, car parts fitted; PLACEHOLDER triggers). Remaining:
  the other stingers, scary sounds, the other voice lines (chases, calls, dead bodies), the audio volumes' reverb

Setup
- [x] Tools refuse a packed (stock) install with a clear message; they need the content extracted to loose files
- [ ] Read the stock pak directly, so users don't need to extract it first (with the host CLI's first-run setup).
      Its footer is known (an encrypted-index flag, magics 'Atae' and 'kcid', version 5, index offset and size,
      hash); the index is AES-encrypted with the game's key, so reading it means decrypting with that key: on hold
      for a decision on that

## Milestone 2: more escapes
Police, 4-seat car, boat, then the others, one at a time.

## Milestone 3: networking
Server-authoritative netcode, tested with bots, then an 8-player lobby.

## Afterwards
Gradually add content. Nice to have, not needed for the MVP: voice chat, Tommy Jarvis, cutscenes,
more maps, Jasons and counselors, perks, skins, emotes, a main menu.

Future plans:
- All unlockables and perks available by default
- Server browser using Steamworks
- Mod support: a mod repository that clients pull from; modded lobbies are labelled in the browser and
  joining one downloads its mods
- Configurable community backends: progression, allowed mods, which servers are listed, player reports
  (for example vanilla-only, cheater-banning communities as well as the default "anything goes")

Mod ideas: custom game modes (infection), larger lobbies, custom counselors and Jasons, custom animations.
