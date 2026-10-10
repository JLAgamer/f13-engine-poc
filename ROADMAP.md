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
- [ ] Particle effects (Cascade): the level's systems simulate (particles.rs) and the fire masters are ported (the
      campfire's and fireplace's flames, sparks and smoke, the candles, the lamps' flies, dust specks, the low fog, the
      intro's Savini embers and eye glows).
      Remaining: the translucency
      lighting volume the lit ones read (a stand-in now: MODLOG "Particle materials"), particle lights' high-quality (shadowed) variant,
      collision's view-distance cull and physics impulses, the thrown knife's ribbon trail
      (TracerTrail: its base pass is a glow at 0.025 opacity, the rest is refraction, which no material has yet),
      the impact effects' decals (the shotgun pellets' FX and sounds by surface are in), whether melee
      hits spawn impact effects at all (MODLOG "A gun's pellets and their impacts"), the GPU emitters' modules (CollisionGPU, the event generator and receiver)
- [ ] Counselor model, remaining: the eye look against a capture of the game (it rolls the eyes 90 degrees from their bind: MODLOG
      "The eyes' look")
      (MODLOG "The counselor's face"), hair dynamics (its anim
      Blueprint), the eyes' iris and lid overlays, outfit choice
- [ ] Animations, remaining: directional starts (BS_F_Start_Normal_*Hemisphere 45-180: their root doesn't turn, the body turns about it; how the actor turns with them is unread), 
      the rest of the anim Blueprints' graphs (f13-rs
      plays their locomotion, the grab and block layers and the gun branch; their look-at and IK nodes aren't
      ported). Not in the game:
      turn-in-place takes, limp movement
- [ ] Lighting, remaining: Enlighten GI, local light shadows (a real-time budget of the 4 nearest for now, unverified
      visually and for cost), the rain material's refraction, the falling rain's cost (49 CPU-simulated systems,
      about 3 ms a frame; the game's are GPU sprites: MODLOG "Rain")
- [ ] Read the stock pak directly, so users don't need to extract it first (with the host CLI's first-run setup).
      Its footer is known (an encrypted-index flag, magics 'Atae' and 'kcid', version 5, index offset and size,
      hash); the index is AES-encrypted with the game's key, so reading it means decrypting with that key: on hold
      for a decision on that
- [ ] Not from the game's code, so marked PLACEHOLDER (MODLOG "Decomp audit, where it ends"): which events say the
      counselor's other lines, X3DAudio's panning matrix,
      the doors' APEX debris, the death and spectating cameras, the counselor HUD's FearOpacity (her pockets' and stamina
      ring's tint during the match: its native writer wasn't found)
- [ ] Audio, remaining: the audio volumes' reverb (the game's XAudio2 device runs XAudio2's own reverb from each
      volume's ReverbEffect; what's done, and why the stingers, calls and dead bodies' lines aren't listed: MODLOG
      "Scary sounds", "Her voice lines: where and when")
- [ ] Post-processing, remaining: compare against the game's own screenshots. (Bloom's fast blur: ported, ComputeOutputDesc's half-width X pass)

## Milestone 3: Networking
Server-authoritative netcode, tested with bots, then an 8-player lobby.

Moved from Milestone 2 (they need more than one counselor):
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
