# Roadmap

Open work only; what's done, with its references to the decomp, is in F13-decomp's MODLOG.

Base content (asset paths in the player's install; all gameplay values come from it, see README): Crystal Lake
(small) `Maps/CrystalLake_Small`, Vanessa `Characters/Counselors/Athlete`, Jason Part 2
`Characters/Killers/Jason/J2`, the Chevettes (`Blueprints/Vehicles/DriveableVehicles/Cars/Chevette`), the police.

## Todo
- [ ] Bot clients for the scenarios: the bots that set up their scene (teleports, locks) need a server-side hook
      (the net bot plays through its input only)
- [ ] The lobby's rest: the match-end lobby as its own screen (SCMatchEndLobbyWidget, its badges' icons and names:
      SCStatBadge; award sites and classes are read, MODLOG), back to the lobby after a match, the counselors'
      character picks, a full 8-player run (`--lobby` gathers, picks Jason and starts the match now)
- [ ] The Tommy Jarvis timer, Tommy Jarvis, and Jason dying
- [ ] Voice chat's rules: the walkie-talkies (SCWalkieTalkie MinVoIPDistance 400: carriers further apart hear each
      other by radio), who hears the dead and Jason, VoIPAudioComponent's real attenuation (UE's defaults stand in),
      the Psychic perk's VoIPDistanceModifier, push-to-talk and mute options
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
- [ ] Read the stock pak directly, so users don't need to extract it first (with the host CLI's first-run setup).
      Its footer is known (an encrypted-index flag, magics 'Atae' and 'kcid', version 5, index offset and size,
      hash); the index is AES-encrypted with the game's key, so reading it means decrypting with that key: on hold
      for a decision on that
- [ ] Optimize performance
- [ ] Blood: the kills' screen blood (their "Blendable: PP_Player_BloodEffect_01..06" windows)
- [ ] Blood: Jason's weapon bloodied by a kill (ShowWeaponBlood)
- [ ] Blood: a hit's blood on the hit bone's clothing ("Trying to apply blood")
- [ ] Blood: dismemberment (the death's dismember roll, the dismemberment meshes)
- [ ] Blood: the counselor's gore skin layer (SkinGoreMaster)
- [ ] Blood: check whether thrown knives and shotgun hits on a character spawn blood in the game
- [ ] Jason's knife pick-up: the knife in his hand during the take (its Prop: throwing_Dart window, 0.89-2.35 s), and
      the special move's 0.15 s interp and walk to its place (he only turns to it now)
- [ ] The computer Jason and knife pickups: he takes them in passing (no pick-up move) and doesn't go looking for them
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
