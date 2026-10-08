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
- [ ] Car handling, remaining: suspension, sliding, collisions' impulses
      (reference: PhysX 3.4's published vehicle source, downloaded to F13-decomp/ref with the user's OK; the game's
      statically linked copy is stripped)

Mechanics: Jason side
- [ ] Played Jason, bugs to fix:
      - his HUD: the abilities' unlock progress looks wrong
      - he holds nothing while throwing a knife (no knife in his hand)
      - walking into a door opens it and places a trap: the computer Jason's door and trap logic still running for
        the player
      - his pickaxe has no material: M_JasonV2_PickAxe is its own master, with only WeaponBloodMask as a parameter, so
        its base textures are in the material graph, which the export doesn't read (may fold into Materials, remaining)
      - his footsteps: some steps play no sound

Presentation
- [ ] Audio, remaining: the other stingers, scary sounds, the other voice lines (chases, calls, dead bodies), the
      audio volumes' reverb (what's done: MODLOG "Milestone 1, done")

Decomp audit (nothing guessed: every rule read from the decomp or the game's data)
- [ ] Check each PLACEHOLDER (9 notes, `grep -rn PLACEHOLDER crates`, besides Bevy's `Entity::PLACEHOLDER`; 6 of
      them, in jason.rs, are the AI's own choices and stay) and each "guess"/"assumed"/"not read" note (about 10)
      against the decomp and the export (Ghidra, MODLOG, f13-export `debug-json` / `pseudo` / `cdo` / `grep`); port
      what the game does, or record in MODLOG why it can't be found. Most are in jason.rs, hiding.rs, audio.rs,
      police.rs, doors.rs, items.rs, interact.rs. Jason's decisions stay ours (his AI was cut), marked as such.
      (Found and fixed so far: MODLOG "Milestone 1, done".)

The finished Milestone 1 work, with its references to the decomp, is in F13-decomp's MODLOG ("Milestone 1, done").

## Milestone 2: more escapes
Police, 4-seat car, boat, then the others, one at a time.
- [ ] Police, remaining (the escape itself works in M1's match: see above): the escape's camera spline, the phone box's
      broken mesh and lights, the computer Jason breaking boxes, the free-spot traces when he's turned back
      (PoliceStunTrace)

Moved out of Milestone 1 (they need more than one counselor; M1 has Vanessa alone):
- [ ] Fear events near friendlies and corpses (other counselors)
- [ ] Car passengers, and the car running counselors over
- [ ] The lobby (M3), the Tommy Jarvis timer and Jason dying (killing him needs Tommy and other counselors;
      Tommy is in "Afterwards")

Moved from Milestone 1 (not tied to gameplay):
- [ ] Match flow, remaining: the level intro and outro sequences. (Badges: their award sites and classes are read,
      see MODLOG; the game shows them in the lobby's SCMatchEndLobbyWidget, so with the lobby in Milestone 3)
- [ ] Materials, remaining: water (first pass only: colour, gloss, panning ripples; no depth fade, foam or
      reflections), POM parallax, the other emissive masters (signs, the CB tower, the phone box), foliage
      subsurface, the Chevette's paint mask
- [ ] Particle effects (Cascade ParticleSystems): none ported yet. The walls' and doors' hit dust (HitParticle), the
      knife's trail (P_Jason_ThrowingKnife_01), blood, the campfires and fireplaces, the cars' exhaust, rain
- [ ] Counselor model, remaining: facial animation and hair dynamics (their own anim Blueprints), the eyes' iris and
      lid overlays, outfit choice
- [ ] Animations, remaining: directional starts (BS_F_Start_Normal_*Hemisphere 45-180: their root doesn't turn, the body turns about it; how the actor turns with them is unread), foot-marker
      sync (now normalised time), aim offsets (Base_Counselor_AnimBP's AimOffsets, the Hunter's poses on the shared
      skeleton, by AimPitch while bIsAiming), BS_Run_Lean, the rest of the anim Blueprints' graphs (f13-rs plays their
      locomotion and the grab and block layers; their look-at, IK and additive nodes aren't ported). Not in the game:
      turn-in-place takes, limp movement
- [ ] Lighting, remaining: Enlighten GI, local light shadows, light functions, the sky texture, rain (the colour
      grading and bloom: Decomp audit's post-processing)
- [ ] Read the stock pak directly, so users don't need to extract it first (with the host CLI's first-run setup).
      Its footer is known (an encrypted-index flag, magics 'Atae' and 'kcid', version 5, index offset and size,
      hash); the index is AES-encrypted with the game's key, so reading it means decrypting with that key: on hold
      for a decision on that
- [ ] Post-processing, remaining: SSAO (FPostProcessAmbientOcclusion*: it darkens indirect light only; settings and
      cvars read, see MODLOG "SSAO (started)"); then compare
      against the game's own screenshots. (Bloom's fast blur: ported, ComputeOutputDesc's half-width X pass)

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
