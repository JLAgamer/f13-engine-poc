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
- [ ] Dodge, swimming
- [ ] Fear events pushed in the client (darkness, indoors, wounded, water, Jason near...)
- [ ] Real collision (capsule vs map); now ray casts only
- [x] Inventory: large item (hands) and small items (pockets, 3); pickup, drop (use: per item, later)
- [x] Repairs: car battery and gas, keys at the ignition (from the driver's seat)
- [ ] Repair minigame (pips) instead of a plain hold; real start time (`BaseRestartTime`)
- [x] Car: enter/exit, start, escape volume
- [ ] Car handling from the game's vehicle setup (now a placeholder kinematic car)
- [x] Cabinets (drawers): search, items inside
- [ ] Doors and windows: open, lock, barricade, window dive
- [ ] Hiding: closets, beds
- [ ] Health, wounds, limping, death
- [ ] Weapons and combat: hit, stun, block
- [ ] Grab escape (wiggle), pocket knife

Mechanics: Jason side
- [x] Ability unlock order, recharge, rage
- [ ] Movement, Shift, Morph, Sense, Stalk
- [ ] Attacks, grab, kills (one basic kill to start), throwing knives
- [ ] Breaking doors, walls, generators; traps
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
- [ ] Materials: cabins, terrain layers, water, SpeedTree pine leaves
- [x] Car, furniture, doors' frames and items placed in the scene (door leaves are destructible meshes: not exported yet)
- [ ] Characters placed in the scene
- [ ] Animations (placeholder until retargeting works)

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
