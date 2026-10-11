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

Milestone 1 is done: nothing left open.

Decomp audit: done; what's left marked and why is in MODLOG "Decomp audit, where it ends".

The finished Milestone 1 work, with its references to the decomp, is in F13-decomp's MODLOG ("Milestone 1, done").

## Milestone 2: Visual and Audio polish
Police, 4-seat car, boat, then the others, one at a time. The police and the 4-seat car are in (MODLOG); Crystal Lake
(small) has no boat or other escape, so those come with more maps (Afterwards).

Bugs:
- [ ] Grabbing a counselor as Jason still loops the struggle audio (the sound cues' looping and random nodes are
      ported, audio.rs Reloop; not the fix). Read so far (MODLOG "The grab's struggle loop"): the game repeats her
      Choke takes for as long as he holds her; the hold ends at her struggle's TotalStunTime (15 s, less with her
      stats), a kill or a drop, and traces show Choke stopping at each. Playing Jason, his E no longer counts as her
      wiggle. Needs a retest to say what still sounds wrong

Moved from Milestone 1 (not tied to gameplay):
- [ ] Materials, remaining: foliage subsurface (its transmission is the deferred light pass's, whose code isn't
      in the shipped global shader cache: MODLOG "Foliage subsurface: blocked")
- [ ] Counselor model, remaining: the eye look against a capture of the game (it rolls the eyes 90 degrees from their bind: MODLOG
      "The eyes' look")
      (MODLOG "The counselor's face")
- [ ] The debris chunks' remaining physics stand-ins: PhysX's contact reduction (ContactReductionAllIn,
      Dy::ContactReduction<6>) and friction anchors, the world meshes' cooked active-edge flags, and contacts between
      chunks of different broken pieces (MODLOG "PhysX's legacy contact generation for the debris")
- [ ] Lighting, remaining: Enlighten GI (the map's indirect light: its precomputed systems and probe sets are
      Geomerics' runtime data, undocumented; the solver is in the exe), the local lights' shadow caching and fade
      (which lights qualify is the game's; without the caching only the four highest resolutions render: MODLOG
      "Local lights' shadows"), the falling rain's cost (49 CPU-simulated systems, about 23k sprites, drawn as one sorted batch built
      in parallel: draw 5.6 ms and simulate 2.9 ms of the main world a frame, under the render world's ~27; the game's
      are GPU sprites: MODLOG "Rain")
- [ ] Read the stock pak directly, so users don't need to extract it first (with the host CLI's first-run setup).
      Its footer is known (an encrypted-index flag, magics 'Atae' and 'kcid', version 5, index offset and size,
      hash); the index is AES-encrypted with the game's key, so reading it means decrypting with that key: on hold
      for a decision on that
- [ ] Post-processing, remaining: compare against the game's own screenshots. (Bloom's fast blur: ported, ComputeOutputDesc's half-width X pass)

## Milestone 3: Networking
Server-authoritative netcode, tested with bots, then an 8-player lobby.

Moved from Milestone 2 (they need more than one counselor):
- [ ] The death and spectating cameras (a dead counselor becomes a spectator of the others)
- [ ] Fear events near friendlies and corpses (other counselors)
- [ ] Car passengers, and the car running counselors over
- [ ] The lobby (with the match's badges: their award sites and classes are read (MODLOG); the game shows them in its
      SCMatchEndLobbyWidget), the Tommy Jarvis timer and Jason dying (killing him needs Tommy and other counselors)

Moved from Afterwards:
- [ ] Tommy Jarvis
- [ ] Voice chat

## Afterwards
Gradually add content. Nice to have, not needed for the MVP: cutscenes,
more maps, Jasons and counselors, perks, skins, emotes, a main menu.

Future plans:
- All unlockables and perks available by default
- Server browser using Steamworks
- Mod support: a mod repository that clients pull from; modded lobbies are labelled in the browser and
  joining one downloads its mods
- Configurable community backends: progression, allowed mods, which servers are listed, player reports
  (for example vanilla-only, cheater-banning communities as well as the default "anything goes")

Mod ideas: custom game modes (infection), larger lobbies, custom counselors and Jasons, custom animations.
