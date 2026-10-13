# Roadmap

Open work only; what's done, with its references to the decomp, is in F13-decomp's MODLOG.

Base content (asset paths in the player's install; all gameplay values come from it, see README): Crystal Lake
(small) `Maps/CrystalLake_Small`, Vanessa `Characters/Counselors/Athlete`, Jason Part 2
`Characters/Killers/Jason/J2`, the Chevettes (`Blueprints/Vehicles/DriveableVehicles/Cars/Chevette`), the police.

## Todo
- [ ] Bot clients for the scenarios: the bots that set up their scene (teleports, locks) need a server-side hook
      (the net bot plays through its input only)
- [ ] The lobby's rest: the match-end lobby as its own screen (SCMatchEndLobbyWidget, its badges' icons and names:
      SCStatBadge; award sites and classes are read, MODLOG), the counselors' character picks, a full 8-player run
      (`--lobby` gathers, picks Jason, starts the match and, the match over, starts again for a rematch)
- [ ] The Tommy Jarvis timer, Tommy Jarvis, and Jason dying
- [ ] Voice chat: VoIPAudioComponent's real attenuation (UE's defaults stand in), muting players (the scoreboard's,
      ILLGameVoiceBlueprintLibrary), the push-to-talk setting in a menu (`--open-mic` for now)
- [ ] Materials: foliage subsurface (its transmission is the deferred light pass's, whose code isn't in the shipped
      global shader cache: MODLOG "Foliage subsurface: blocked")
- [ ] Counselor model: the eye look against a capture of the game (it rolls the eyes 90 degrees from their bind:
      MODLOG "The eyes' look", "The counselor's face")
- [ ] Lighting: Enlighten GI (the map's indirect light: its precomputed systems and probe sets are Geomerics'
      runtime data, undocumented; the solver is in the exe)
- [ ] Lighting: the local lights' shadow caching and fade (which lights qualify is the game's; without the caching
      only the four nearest whose range reaches the camera render: MODLOG "Local lights' shadows")
- [ ] Lighting: the inverse-squared lights' unit (16/100^2 candela) is from engine memory, PLACEHOLDER until read
- [ ] The falling rain's simulation on the GPU as the game's (49 systems, about 23k sprites, simulated on the CPU:
      2.5 ms of the main world a frame; their draw is already on the GPU: MODLOG "The rain's draw")
- [ ] Post-processing and lighting: compare against the game's own screenshots (the user's cabin porch reference,
      2026-10-12: its wall lamp's spots light sharp bands up and down the logs)
- [ ] Optimize performance (2026-10-13, a counselor at her start, --perf --features profile: 11.4 ms a frame, the render
      world's CPU the critical path: encoding 4.4 ms, submit 1.6, preprocess bind groups 1.3, the shadow phases'
      instance buffers 1.3; main world 3.9, PostUpdate 2.5; particles 0.9)
- [ ] Blood: a hit's blood on the hit bone's clothing ("Trying to apply blood")
- [ ] Blood: dismemberment (the death's dismember roll, the dismemberment meshes)
- [ ] Blood: the counselor's gore skin layer (SkinGoreMaster)
- [ ] Blood: impact decals on characters (a thrown knife's gash on her, the shotgun's on Jason: decals that follow a
      character)
- [ ] Sound: the thrown knife's ThrowingKnifeImpactConfirmation (SCThrowable JasonImpactNoise: where it plays isn't
      found yet)
- [ ] Music: Jason's own music (his MusicComponent's JasonsMusic, after a kill's music too), the two-minute warning
      turned down under a kill's music
- [ ] More content, one at a time: maps (with their boat and other escapes), Jasons, counselors, perks, skins,
      emotes, a main menu

## Later
- All unlockables and perks available by default
- Server browser using Steamworks
- Mod support: a mod repository that clients pull from; modded lobbies are labelled in the browser and
  joining one downloads its mods
- Configurable community backends: progression, allowed mods, which servers are listed, player reports
  (for example vanilla-only, cheater-banning communities as well as the default "anything goes")

Mod ideas: custom game modes (infection), larger lobbies, custom counselors and Jasons, custom animations.
