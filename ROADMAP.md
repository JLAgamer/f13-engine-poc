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
- [ ] Swimming
- [ ] Fear events pushed in the client (darkness, indoors, wounded, water, Jason near...)
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
- [ ] Car handling from the game's vehicle setup (now a placeholder kinematic car)
- [x] Cabinets (drawers): search, items inside
- [x] Doors: leaves exported and swinging on their pivot, open/close, lock/unlock by side (doors.rs; swing timing is a
  placeholder for the special-move montages)
- [x] Barricades (replace the lock, only on a closed door) and windows: open, close, vault, dive (breaks the glass,
  DiveDamage); windows.rs. Vault/dive are instant moves for now; barricade mesh placement unverified
- [x] Hiding: wardrobes, outhouses, beds, tents (hiding.rs: enter/idle/exit montages, hiding camera; Jason's search
  and hiding spot kills). Door swing is a placeholder; outhouse door and tent flaps don't animate
- [x] Health, wounded, death, blocking damage, first aid (f13-core::health); limp animation later
- [x] Items against Jason (items.rs): weapon swings (stun chance, wear, breaking), flare gun, shotgun, firecrackers,
  bear traps, the map; stun times and animations from the game (swing reach/angle, throw speed and gun fire timing
  are placeholders)
- [x] Grab escape (wiggle, stat-scaled, 10/s cap) and the pocket knife
- [ ] Combat, remaining: blocking with a weapon, counselors caught in placed traps, Jason shaking off stuns
- [x] Ragdoll on death (f13-core::ragdoll, the physics asset's bodies against the world collision)

Mechanics: Jason side
- [x] Ability unlock order, recharge, rage
- [x] Jason Part 2 as a computer player (jason.rs): J2's movement values, paths over the collision (f13-core::nav),
  opening doors and breaking locked or barricaded ones (BaseDoorHealth vs weapon damage), pickaxe swings (damage,
  AttackDelay, reach/angle, hits in the AttackTraceNotify window), grabs (min/max distance, angle) and the pickaxe's
  grab kill (Throat Hack, both sides' paired animation)
- [x] Jason's perception (senses.rs, f13-core::perception): sight, the game's noise blips, Sense; he investigates
  clues and searches nearby hiding spots
- [x] Navmesh built at load time from the collision with the game's Recast settings (f13-core::recast/navmesh); the
  cooked navmesh is exported (navmesh-<agent>.json) as a reference
- [ ] Jason AI, remaining: heavy attacks, the grabbed counselor on his grab socket, kill cameras; playing as Jason;
  holding breath while hiding
- [ ] Shift, Morph, Stalk (Sense done)
- [ ] More kills, throwing knives
- [ ] Breaking walls, generators; traps
- [ ] Car interactions: stopping and grabbing out of the car

Match
- [ ] Match flow: lobby, spawn, match timer, end (all dead or escaped), results
- [x] Item and car spawning, as the game mode does it (f13-core::spawn, f13-data::world)
- [ ] How the game chooses one of the four cabin layouts per match

Host CLI (`f13-host`)
- [ ] Spawn items
- [ ] Teleport players
- [ ] Make a player Jason, live in game
- [ ] Force start and end matches
- [ ] Kick players

Presentation
- [x] Map, characters and car exported to glTF (`tools/f13-export`)
- [x] Map loaded in Bevy with its cabin layouts (`f13-game`)
- [x] Materials: cabins (Master_Blend_01 two-set vertex-alpha blend, tinted blends, curtains), props/items/car (RMA),
      foliage (RSS roughness, mask threshold) and terrain layers (height-blended weight
      maps), ported from the game's compiled shaders; per-placement material overrides; glass; invisible blockers
      hidden (f13-game materials.rs, shaders/); textures as the game's compressed DDS data with mips
- [ ] Materials, remaining: water (first pass only: colour, gloss, panning ripples; no depth fade, foam or
      reflections), POM parallax, emissive/light materials, foliage subsurface, the Chevette's paint mask
- [x] Car, furniture, doors (frames and leaves), windows, barricades and items placed in the scene
- [ ] Barricade plank: check its placement (not visible across the door in a screenshot)
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
- [ ] Animations, remaining: start/stop transitions, turn in place, foot-marker sync (now normalised time), limp
      movement, aim offsets, repairs, car enter/exit and sitting (she's hidden in the car), door/window montages
- [ ] Special moves: door open/close/lock and window open/close/vault/dive montages (now a fixed 0.5 s, 90° door swing
      and instant vault/dive)
- [ ] Repair minigame dial UI (now a text dial in the HUD)
- [ ] HUD: health, stamina, fear and inventory (now debug text)
- [ ] Night lighting, fog, flashlight
- [ ] Audio: footsteps, music and Jason's stingers, doors, windows, car

Setup
- [x] Tools refuse a packed (stock) install with a clear message; they need the content extracted to loose files
- [ ] Read the stock pak directly (it has a custom footer and a protected index; work out the format from the game's
      pak loader), so users don't need to extract it first. Do this with the host CLI's first-run setup

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
