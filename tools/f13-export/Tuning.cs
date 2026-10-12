// `tuning [out file]`: the gameplay values f13-rs reads (tuning.json; default ~/.f13-rs/tuning.json), from YOUR OWN
// install: the Blueprints' class defaults and component templates, curves and montages (through CUE4Parse, as the
// rest of the exporter reads them), the executable's C++ constructor defaults and DefaultGame.ini. A port of the
// former tools/extract_tuning.py (which needed Python and UAssetGUI's tojson): the same values come out (checked
// against its file, 2026-10-12: equal at float32, the precision f13-data reads).
using CUE4Parse.FileProvider;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

static class Tuning
{
    // C++-only defaults, read from the game executable. Each entry pins the exact instruction
    // (`mov dword ptr [reg+disp32], imm32`) so a different game build fails loudly instead of yielding garbage.
    static readonly (string Name, ulong Va, string Expect)[] ExeFloats =
    [
        ("rage_unlock_time", 0x1403E0CC0, "c78750130000"),
        ("stalk_active_time", 0x1403E0CB6, "c78700170000"),
        ("update_fear_delay", 0x14032CA2C, "c78740010000"),
        // character movement component constructor
        ("max_sprint_speed", 0x1403A2A45, "c78320070000"),
        ("max_run_speed", 0x1403A2A7C, "c78324070000"),
        ("max_slow_run_speed", 0x1403A2A86, "c78328070000"),
        ("max_combat_speed", 0x1403A2A90, "c7832c070000"),
        ("max_walk_speed", 0x1403A2A9A, "c783d4010000"),
        ("max_walk_speed_crouched", 0x1403A2AA4, "c783d8010000"),
        // counselor constructor (values a Blueprint may override; the Blueprint wins)
        ("stamina_max", 0x1403A095A, "c787dc120000"),
        ("stamina_idle_modifier", 0x1403A0984, "c78720130000"),
        ("stamina_hiding_modifier", 0x1403A098E, "48c78724130000"),
        ("stamina_sprinting_modifier", 0x1403A09A3, "c78730130000"),
        ("dodging_speed_mod", 0x1403A0A83, "c78788140000"),
        ("stumbling_speed_mod", 0x1403A0A8D, "c7878c140000"),
        ("water_speed_mod", 0x1403A0A97, "c78790140000"),
        ("grabbed_refill_base", 0x1403A13B0, "c78770130000"),
        ("grabbed_refill_strength", 0x1403A13BA, "c78774130000"),
        ("grabbed_refill_composure", 0x1403A13C4, "c78788130000"),
        ("stumble_wounded_chance", 0x1403A0D08, "c787c8190000"),
        ("stumble_cooldown", 0x1403A0D19, "c787cc190000"),
        ("stumble_stat_mod_base", 0x1403A145A, "c78798190000"),
        ("blocking_damage_mod_base", 0x1403A1446, "c78728180000"),
        ("dodge_stamina_cost_base", 0x1403A13EC, "c787d8160000"),
        // her combat stats' constructor BaseValues (StrengthAttackMod and StunChanceMod have none: the Blueprint sets them)
        ("hit_killer_stamina_boost_base", 0x1403A1414, "c78738170000"),
        ("break_free_stamina_boost_base", 0x1403A1428, "c78768170000"),
        ("heavy_attack_modifier", 0x1403A0C5C, "c78788180000"),
        ("wounded_stun_mod", 0x1403A0C66, "c7878c180000"),
        ("rage_grab_stun_mod", 0x1403A0C70, "c78790180000"),
        ("min_interact_hold_time", 0x1402DC507, "c786000e0000"),
        // interact components
        ("repair_hold_time", 0x14032E84B, "c787b8020000"),
        ("allowable_fails_before_killer_notify", 0x14032E357, "c78300040000"),
        // SCDriveableVehicle constructor
        ("min_restart_time", 0x140417688, "c78788040000"),
        ("max_restart_time", 0x140417692, "c7878c040000"),
        ("restart_time_mod_base", 0x140417F43, "c78790040000"),
        // minigame widget constructor
        ("max_pip_width", 0x14041A8B8, "c78340040000"),
        ("min_pip_width", 0x14041A8C2, "c78344040000"),
        // character constructor (`this` in rsi)
        ("max_health", 0x1402DC545, "c786540e0000"),
        // first aid constructor
        ("first_aid_health_restored", 0x14036944D, "c787b0050000"),
    ];

    static readonly (string Name, ulong Va, string Expect)[] ExeInts =
    [
        ("max_pip_count", 0x14041A8A4, "c78338040000"),
        ("min_pip_count", 0x14041A8AE, "c7833c040000"),
        // SCGameMode constructor
        ("post_match_outro_wait_time", 0x14036BD35, "c787e4040000"),
        ("post_match_wait_time", 0x14036BD3F, "c787e8040000"),
    ];

    static readonly (string Name, ulong Va, string Expect)[] ExeFlags =
    [
        ("force_stumble_on_stamina_depletion", 0x1403A0D23, "c687d0190000"),
    ];

    // The constants as the January 2025 build (the decomp's) has them: used when the install's exe doesn't have an
    // instruction where it's pinned. Builds since (October 2026's) ship the code encrypted on disk (Arxan: the
    // .arxinstmap beside the exe), so no instruction can be read there; constructor defaults rarely change between patches.
    static readonly Dictionary<string, object> KnownConstants = new()
    {
        ["rage_unlock_time"] = 900.0, ["stalk_active_time"] = 20.0, ["update_fear_delay"] = 0.05000000074505806,
        ["max_sprint_speed"] = 450.0, ["max_run_speed"] = 280.0, ["max_slow_run_speed"] = 200.0, ["max_combat_speed"] = 104.0,
        ["max_walk_speed"] = 104.0, ["max_walk_speed_crouched"] = 80.0, ["stamina_max"] = 100.0, ["stamina_idle_modifier"] = 1.0,
        ["stamina_hiding_modifier"] = 1.5, ["stamina_sprinting_modifier"] = 2.0, ["dodging_speed_mod"] = 2.0,
        ["stumbling_speed_mod"] = 0.75, ["water_speed_mod"] = 0.699999988079071, ["grabbed_refill_base"] = 2.5,
        ["grabbed_refill_strength"] = 15.0, ["grabbed_refill_composure"] = 15.0, ["stumble_wounded_chance"] = 3.0,
        ["stumble_cooldown"] = 15.0, ["stumble_stat_mod_base"] = 1.0, ["blocking_damage_mod_base"] = 1.0,
        ["dodge_stamina_cost_base"] = 20.0, ["hit_killer_stamina_boost_base"] = 50.0, ["break_free_stamina_boost_base"] = 50.0,
        ["heavy_attack_modifier"] = 2.0, ["wounded_stun_mod"] = 0.75, ["rage_grab_stun_mod"] = 0.5,
        ["min_interact_hold_time"] = 0.20000000298023224, ["repair_hold_time"] = 15.0, ["allowable_fails_before_killer_notify"] = 3.0,
        ["min_restart_time"] = 5.0, ["max_restart_time"] = 5.0, ["restart_time_mod_base"] = 1.0, ["max_pip_width"] = 30.0,
        ["min_pip_width"] = 15.0, ["max_health"] = 100.0, ["first_aid_health_restored"] = 100.0, ["max_pip_count"] = 10L,
        ["min_pip_count"] = 5L, ["post_match_outro_wait_time"] = 10L, ["post_match_wait_time"] = 20L,
        ["force_stumble_on_stamina_depletion"] = true,
    };

    // Fear event property -> json key; properties not set in an asset are 0 (no C++ constructor).
    static readonly (string Src, string Key)[] FearFields =
    [
        ("IncreaseFearValue", "increase"), ("DecreaseFearValue", "decrease"), ("IncreaseDelay", "increase_delay"),
        ("DecreaseDelay", "decrease_delay"), ("MaxFearCap", "max_fear_cap"), ("bDecreaseWhenActive", "decrease_when_active"),
        ("AutoDeactivateTimer", "auto_deactivate_time"),
    ];

    static readonly string[] StatNames = ["Strength", "Speed", "Intelligence", "Stamina", "Luck", "Composure", "Stealth"];

    static Dictionary<string, object> ReadExeConstants(string exe)
    {
        var data = File.ReadAllBytes(exe);
        var pe = BitConverter.ToInt32(data, 0x3C);
        var imageBase = BitConverter.ToUInt64(data, pe + 24 + 24);
        var nsec = BitConverter.ToUInt16(data, pe + 6);
        var sec0 = pe + 24 + BitConverter.ToUInt16(data, pe + 20);
        long FileOff(ulong va)
        {
            var rva = (long)(va - imageBase);
            for (var i = 0; i < nsec; i++)
            {
                var s = sec0 + 40 * i;
                long vsize = BitConverter.ToUInt32(data, s + 8), vaddr = BitConverter.ToUInt32(data, s + 12);
                long rsize = BitConverter.ToUInt32(data, s + 16), raddr = BitConverter.ToUInt32(data, s + 20);
                if (vaddr <= rva && rva < vaddr + Math.Max(vsize, rsize)) return raddr + rva - vaddr;
            }
            throw new InvalidDataException($"{va:x}");
        }
        var outv = new Dictionary<string, object>();
        var fallback = 0;
        // The instruction there, its immediate's offset; or null when it isn't (a fallback is used).
        long? At(string name, ulong va, string expect)
        {
            var o = FileOff(va);
            var e = Convert.FromHexString(expect);
            if (o + e.Length + 4 <= data.Length && data.AsSpan((int)o, e.Length).SequenceEqual(e)) return o + e.Length;
            if (!KnownConstants.ContainsKey(name))
                throw new InvalidDataException($"{name}: unexpected instruction at {va:x}; this game build is not supported yet");
            fallback++;
            return null;
        }
        foreach (var (n, va, e) in ExeFloats)
            outv[n] = At(n, va, e) is { } o ? (double)BitConverter.ToSingle(data, (int)o) : KnownConstants[n];
        foreach (var (n, va, e) in ExeInts)
            outv[n] = At(n, va, e) is { } o ? (long)BitConverter.ToInt32(data, (int)o) : KnownConstants[n];
        foreach (var (n, va, e) in ExeFlags)
            outv[n] = At(n, va, e) is { } o ? data[o] != 0 : KnownConstants[n];
        if (fallback > 0)
            Console.Error.WriteLine($"warning: {fallback} of the exe's constants aren't readable in this build (its code is encrypted on disk); using the January 2025 build's values for them");
        return outv;
    }

    class Reader(IFileProvider provider)
    {
        readonly Dictionary<string, JArray> cache = new();

        /// rel like 'Characters/Killers/Jason/Base/Jason_Base' -> its exports as JSON.
        public JArray Load(string rel)
        {
            if (cache.TryGetValue(rel, out var hit)) return hit;
            var exports = new JArray();
            foreach (var e in provider.LoadPackage("SummerCamp/Content/" + rel).GetExports())
            {
                try { exports.Add(JObject.Parse(JsonConvert.SerializeObject(e))); }
                catch (Exception) { } // an export CUE4Parse can't write (a mesh it misreads): not one the tuning reads
            }
            return cache[rel] = exports;
        }

        public static JObject Export(JArray exports, Func<string, bool> name) =>
            (JObject)exports.First(e => name((string?)e["Name"] ?? ""));

        /// A Blueprint's class defaults (its Default__ object's properties).
        public JObject Cdo(string rel) => Props(Export(Load(rel), n => n.StartsWith("Default__")));

        public static JObject Props(JObject export) => export["Properties"] as JObject ?? new JObject();
    }

    static JToken? P(JObject props, string name, int index = 0) => props[index == 0 ? name : $"{name}[{index}]"];

    static double Num(JToken v) => v.Type is JTokenType.Integer or JTokenType.Float ? v.Value<double>() : double.Parse(v.ToString().Replace("+", ""), System.Globalization.CultureInfo.InvariantCulture);

    static double F(JObject props, string name, double fallback) => P(props, name) is { } v ? Num(v) : fallback;

    /// StatModifiedValue -> {"base", "modifiers"[7]} (unset fields are 0; `base` kept when BaseValue isn't set).
    static JObject StatValue(JToken? node, double @base = 0.0)
    {
        var mods = new double[7];
        if (node is JObject o)
            foreach (var (k, v) in o)
            {
                if (k == "BaseValue") @base = Num(v!);
                else if (k == "StatModifiers") mods[0] = Num(v!);
                else if (k.StartsWith("StatModifiers[")) mods[int.Parse(k["StatModifiers[".Length..^1])] = Num(v!);
            }
        return new JObject { ["base"] = @base, ["modifiers"] = new JArray(mods) };
    }

    /// An object reference's package, content-relative and without extension ('Blueprints/StatsCurves/X').
    static string? RefPath(JToken? r)
    {
        var p = (string?)r?["ObjectPath"];
        if (string.IsNullOrEmpty(p)) return null;
        p = p[..p.LastIndexOf('.')];
        return p.StartsWith("SummerCamp/Content/") ? p["SummerCamp/Content/".Length..] : p;
    }

    /// An object reference's object name ("BlueprintGeneratedClass'LittlePig_C'" -> "LittlePig_C").
    static string? RefName(JToken? r)
    {
        var n = (string?)r?["ObjectName"];
        if (string.IsNullOrEmpty(n)) return null;
        var q = n.IndexOf('\'');
        return q < 0 ? n : n[(q + 1)..].TrimEnd('\'');
    }

    static JArray CurveKeys(Reader rd, string rel)
    {
        var e = (JObject)rd.Load(rel)[0];
        var keys = new JArray();
        foreach (var k in Reader.Props(e)["FloatCurve"]?["Keys"] ?? new JArray())
        {
            var interp = (string?)k["InterpMode"] switch { "RCIM_Linear" => "linear", "RCIM_Constant" => "constant", _ => "cubic" };
            keys.Add(new JObject
            {
                ["time"] = Num(k["Time"]!), ["value"] = Num(k["Value"]!), ["interp"] = interp,
                ["arrive_tangent"] = Num(k["ArriveTangent"] ?? 0.0), ["leave_tangent"] = Num(k["LeaveTangent"] ?? 0.0),
            });
        }
        return keys;
    }

    /// Play length of an AnimMontage at rate 1 (what PlayAnimMontage returns).
    static double MontageLength(Reader rd, string rel)
    {
        var p = Reader.Props(Reader.Export(rd.Load(rel), n => !n.StartsWith("Default__")));
        return Num(p["SequenceLength"]!) / F(p, "RateScale", 1.0);
    }

    static JObject CounselorSection(Reader rd, Dictionary<string, object> c, out JObject characters, JObject curves)
    {
        const string baseRel = "Characters/Counselors/Base/Base_Counselor";
        var d = rd.Cdo(baseRel);
        var move = Reader.Props(Reader.Export(rd.Load(baseRel), n => n == "CharMoveComp"));
        double Fc(string name, string konst) => F(d, name, (double)c[konst]);
        JObject Smv(string name, double @base = 0.0) => StatValue(P(d, name), @base);
        JToken Curve(string name)
        {
            var path = RefPath(P(d, name));
            if (path == null) return JValue.CreateNull();
            var cname = path[(path.LastIndexOf('/') + 1)..];
            if (curves[cname] == null) curves[cname] = CurveKeys(rd, path);
            return cname;
        }
        var animBp = rd.Cdo("Characters/Counselors/Base/Base_Counselor_AnimBP");
        JObject Montage(string name, JObject? props = null)
        {
            var path = RefPath(P(props ?? d, name));
            if (path == null) return new JObject { ["name"] = null, ["length"] = 0.0 };
            return new JObject { ["name"] = path[(path.LastIndexOf('/') + 1)..], ["length"] = MontageLength(rd, path) };
        }
        var grabbedMods = new double[7];
        grabbedMods[0] = (double)c["grabbed_refill_strength"];
        grabbedMods[5] = (double)c["grabbed_refill_composure"];
        var t = new JObject
        {
            ["max_walk_speed"] = (double)c["max_walk_speed"],
            ["max_walk_speed_crouched"] = (double)c["max_walk_speed_crouched"],
            ["max_slow_run_speed"] = (double)c["max_slow_run_speed"],
            ["max_run_speed"] = (double)c["max_run_speed"],
            ["max_sprint_speed"] = (double)c["max_sprint_speed"],
            ["max_combat_speed"] = (double)c["max_combat_speed"],
            ["max_swim_speed"] = Num(move["MaxSwimSpeed"]!),
            ["max_acceleration"] = Num(move["MaxAcceleration"]!),
            ["dodging_speed_mod"] = Fc("DodgingSpeedMod", "dodging_speed_mod"),
            ["stumbling_speed_mod"] = Fc("StumblingSpeedMod", "stumbling_speed_mod"),
            ["water_speed_mod"] = Fc("WaterSpeedMod", "water_speed_mod"),
            ["speed_stat_mod"] = Smv("SpeedStatMod"),
            ["stamina_max"] = (double)c["stamina_max"],
            ["stamina_min_threshold"] = F(d, "StaminaMinThreshold", 0.0),
            ["stamina_deplete_rate"] = F(d, "StaminaDepleteRate", 0.0),
            ["stamina_refill_rate"] = Smv("StaminaRefillRate"),
            ["stamina_idle_modifier"] = Fc("StaminaIdleModifier", "stamina_idle_modifier"),
            ["stamina_hiding_modifier"] = Fc("StaminaHidingModifier", "stamina_hiding_modifier"),
            ["stamina_walking_modifier"] = F(d, "StaminaWalkingModifier", 0.0),
            ["stamina_running_modifier"] = F(d, "StaminaRunningModifier", 0.0),
            ["stamina_sprinting_modifier"] = Fc("StaminaSprintingModifier", "stamina_sprinting_modifier"),
            ["stamina_swimming_modifier"] = F(d, "StaminaSwimmingModifier", 0.0),
            ["stamina_crouching_modifier"] = F(d, "StaminaCrouchingModifier", 0.0),
            ["stamina_stat_mod"] = Smv("StaminaStatMod"),
            ["grabbed_stamina_refill"] = new JObject { ["base"] = (double)c["grabbed_refill_base"], ["modifiers"] = new JArray(grabbedMods) },
            ["stamina_fear_curve"] = Curve("StaminaFearRefillModifier"),
            ["force_stumble_on_stamina_depletion"] = (bool)c["force_stumble_on_stamina_depletion"],
            ["wounded_threshold"] = F(d, "WoundedThreshold", 0.0),
            ["jason_near_min_range"] = F(d, "JasonNearMinRange", 0.0),
            ["jason_near_max_range"] = F(d, "JasonNearMaxRange", 0.0),
            ["health"] = new JObject
            {
                ["max_health"] = (double)c["max_health"],
                ["blocking_damage_mod"] = Smv("BlockingDamageMod", (double)c["blocking_damage_mod_base"]),
            },
            // Her hit on Jason (FUN_1403cbef0): damage = StrengthAttackMod x the weapon's Damage (x HeavyAttackModifier when
            // heavy); stun when the weapon's StunChance beats RandHelper(101 - StunChanceMod); a hit or stun returns
            // HitKillerStaminaBoost. Breaking free returns BreakFreeStaminaBoost (FUN_1403a6c10). The grab's wiggle progress
            // x WoundedStunMod when wounded, x RageGrabStunMod against a raged Jason (FUN_1403bc910).
            ["combat"] = new JObject
            {
                ["strength_attack_mod"] = Smv("StrengthAttackMod"),
                ["stun_chance_mod"] = Smv("StunChanceMod"),
                ["hit_killer_stamina_boost"] = Smv("HitKillerStaminaBoost", (double)c["hit_killer_stamina_boost_base"]),
                ["break_free_stamina_boost"] = Smv("BreakFreeStaminaBoost", (double)c["break_free_stamina_boost_base"]),
                ["heavy_attack_modifier"] = Fc("HeavyAttackModifier", "heavy_attack_modifier"),
                ["wounded_stun_mod"] = Fc("WoundedStunMod", "wounded_stun_mod"),
                ["rage_grab_stun_mod"] = Fc("RageGrabStunMod", "rage_grab_stun_mod"),
            },
            ["stumble"] = new JObject
            {
                ["fear_frequency_curve"] = Curve("StumbleFearFrequecy"), // (sic) the property's name in the game
                ["stamina_frequency_curve"] = Curve("StumbleStaminaFrequency"),
                ["fear_chance_curve"] = Curve("StumbleFearChance"),
                ["stamina_chance_curve"] = Curve("StumbleStaminaChance"),
                ["stat_mod"] = Smv("StumbleStatMod", (double)c["stumble_stat_mod_base"]),
                ["wounded_chance"] = Fc("StumbleWoundedChance", "stumble_wounded_chance"),
                ["cooldown"] = Fc("StumbleCooldown", "stumble_cooldown"),
                ["sprint"] = Montage("SprintStumbleAnim"),
                ["run"] = Montage("RunStumbleAnim"),
                ["walk"] = Montage("WalkStumbleAnim"),
                ["out_of_stamina"] = Montage("OutOfStaminaStumbleAnim"),
            },
            ["minigame"] = MinigameSection(rd, c, Smv("MinigameSpeedMod", 1.0)),
            ["dodge"] = new JObject
            {
                ["stamina_cost"] = Smv("DodgeStaminaCost", (double)c["dodge_stamina_cost_base"]),
                ["left"] = Montage("DodgeLeftMontage", animBp),
                ["right"] = Montage("DodgeRightMontage", animBp),
                ["back"] = Montage("DodgeBackMontage", animBp),
            },
        };
        // Playable counselors in the MVP: stats from each character Blueprint.
        characters = new JObject();
        foreach (var (key, rel) in new[] { ("Vanessa", "Characters/Counselors/Athlete/Athlete_Counselor") })
        {
            var p = rd.Cdo(rel);
            characters[key] = new JObject { ["stats"] = new JArray(Enumerable.Range(0, 7).Select(i => (long)Num(P(p, "Stats", i)!))) };
        }
        return t;
    }

    /// Hold-to-interact timing and the repair pips (SCInteractMinigame_Widget plus constructor defaults).
    static JObject MinigameSection(Reader rd, Dictionary<string, object> c, JObject speedMod)
    {
        var d = rd.Cdo("UI/Interact/SCInteractMinigame_Widget");
        return new JObject
        {
            ["min_hold_time"] = (double)c["min_interact_hold_time"],
            ["repair_hold_time"] = (double)c["repair_hold_time"],
            ["allowable_fails"] = (double)c["allowable_fails_before_killer_notify"],
            ["max_pip_count"] = P(d, "MaxPipCount") is { } a ? (long)Num(a) : (long)c["max_pip_count"],
            ["min_pip_count"] = P(d, "MinPipCount") is { } b ? (long)Num(b) : (long)c["min_pip_count"],
            ["max_pip_width"] = F(d, "MaxPipWidth", (double)c["max_pip_width"]),
            ["min_pip_width"] = F(d, "MinPipWidth", (double)c["min_pip_width"]),
            ["max_pip_count_mod"] = StatValue(P(d, "MaxPipCountMod")),
            ["min_pip_count_mod"] = StatValue(P(d, "MinPipCountMod")),
            ["max_pip_width_mod"] = StatValue(P(d, "MaxPipWidthMod")),
            ["min_pip_width_mod"] = StatValue(P(d, "MinPipWidthMod")),
            ["speed_mod"] = speedMod,
        };
    }

    /// Engine start time of the MVP car (the starter's hold time is set from these when a counselor drives).
    static JObject VehicleSection(Reader rd, Dictionary<string, object> c)
    {
        var d = rd.Cdo("Blueprints/Vehicles/DriveableVehicles/Cars/Chevette/Chevette_2Seat_Bp");
        return new JObject
        {
            ["min_restart_time"] = F(d, "MinRestartTime", (double)c["min_restart_time"]),
            ["max_restart_time"] = F(d, "MaxRestartTime", (double)c["max_restart_time"]),
            ["restart_time_mod"] = StatValue(P(d, "RestartTimeMod"), (double)c["restart_time_mod_base"]),
        };
    }

    /// Item values the counselor rules need (first aid for now).
    static JObject ItemsSection(Reader rd, Dictionary<string, object> c)
    {
        var d = rd.Cdo("Blueprints/Items/Small/BP_FirstAid");
        var use = RefPath(P(d, "UseMontages")?[0]) ?? throw new InvalidDataException("BP_FirstAid: no UseMontages");
        return new JObject
        {
            ["first_aid"] = new JObject
            {
                ["health_restored"] = F(d, "HealthRestoredPerUse", (double)c["first_aid_health_restored"]),
                ["use_length"] = MontageLength(rd, use),
            },
        };
    }

    /// A key's value in one section of an Unreal .ini file, or null.
    static string? IniValue(string text, string section, string key)
    {
        string? current = null;
        foreach (var raw in text.Split('\n'))
        {
            var line = raw.Trim();
            if (line.StartsWith('[') && line.EndsWith(']')) current = line[1..^1];
            else if (current == section && line.Split('=', 2)[0] == key) return line.Split('=', 2)[1];
        }
        return null;
    }

    /// Match timing (the offline game mode's RoundTime from DefaultGame.ini, the game mode constructor's waits) and
    /// every score event's XP.
    static JObject MatchSection(Reader rd, IFileProvider provider, Dictionary<string, object> c, string game)
    {
        var disk = Path.Combine(game, "SummerCamp", "Config", "DefaultGame.ini");
        var ini = File.Exists(disk) ? File.ReadAllText(disk)
            : System.Text.Encoding.UTF8.GetString(provider.Files["SummerCamp/Config/DefaultGame.ini"].Read());
        var roundTime = IniValue(ini, "/Script/SummerCamp.SCGameMode_OfflineBots", "RoundTime")
            ?? IniValue(ini, "/Script/SummerCamp.SCGameMode_Hunt", "RoundTime")
            ?? throw new InvalidDataException("DefaultGame.ini: no RoundTime for the offline game mode");
        var police = IniValue(ini, "/Script/SummerCamp.SCGameState", "PoliceResponseTime")
            ?? throw new InvalidDataException("DefaultGame.ini: no PoliceResponseTime");
        var events = new JObject();
        foreach (var name in Folder(provider, "Blueprints/ScoreEvents"))
        {
            var d = rd.Cdo($"Blueprints/ScoreEvents/{name}");
            string? Badge(string key) => RefName(P(d, key)) is { } n && n.Length > 0 ? (n.EndsWith("_C") ? n[..^2] : n) : null;
            events[name] = new JObject
            {
                ["experience"] = (long)F(d, "Experience", 0),
                ["category"] = ((string?)P(d, "EventCategory") ?? "EScoreEventCategory::None").Split("::")[^1],
                ["only_once"] = (bool?)P(d, "bOnlyScoreOnce") ?? false,
                ["all_counselors"] = (bool?)P(d, "bAllCounselors") ?? false,
                ["description"] = (string?)P(d, "EventDescription")?["SourceString"] ?? "",
                ["counselor_badge"] = Badge("CounselorBadge"),
                ["jason_badge"] = Badge("JasonBadge"),
            };
        }
        return new JObject
        {
            ["round_time"] = long.Parse(roundTime.Trim()),
            ["post_match_outro_wait_time"] = (long)c["post_match_outro_wait_time"],
            ["post_match_wait_time"] = (long)c["post_match_wait_time"],
            ["police_response_time"] = long.Parse(police.Trim()),
            ["score_events"] = events,
        };
    }

    /// The asset names directly in a content folder (the script's os.listdir of *.uasset), sorted as Python sorts.
    static IEnumerable<string> Folder(IFileProvider provider, string rel)
    {
        var prefix = $"SummerCamp/Content/{rel}/";
        return provider.Files.Keys
            .Where(k => k.StartsWith(prefix, StringComparison.OrdinalIgnoreCase) && k.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase) && !k[prefix.Length..].Contains('/'))
            .Select(k => Path.GetFileNameWithoutExtension(k[prefix.Length..]))
            .OrderBy(n => n, StringComparer.Ordinal);
    }

    public static int Write(IFileProvider provider, string game, string? outPath)
    {
        outPath ??= Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".f13-rs", "tuning.json");
        var exe = Path.Combine(game, "SummerCamp", "Binaries", "Win64", "SummerCamp.exe");
        var c = ReadExeConstants(exe);
        var rd = new Reader(provider);

        // Jason (base Blueprint).
        var j = rd.Cdo("Characters/Killers/Jason/Base/Jason_Base");
        var killer = new JObject
        {
            ["unlock_time"] = new JArray(Enumerable.Range(0, 4).Select(i => Num(P(j, "AbilityUnlockTime", i)!))),
            ["recharge_time"] = new JArray(Enumerable.Range(0, 4).Select(i => Num(P(j, "AbilityRechargeTime", i)!))),
            ["raged_recharge_modifier"] = Num(P(j, "RagedRechargeTimeModifier")!),
            ["rage_unlock_time"] = (double)c["rage_unlock_time"],
        };

        // Fear events.
        var fearEvents = new JObject();
        var curves = new JObject();
        foreach (var name in Folder(provider, "Blueprints/FearEvents"))
        {
            var p = rd.Cdo($"Blueprints/FearEvents/{name}");
            var ev = new JObject();
            foreach (var (src, key) in FearFields)
                ev[key] = key == "decrease_when_active" ? (JToken)((bool?)P(p, src) ?? false) : (P(p, src) is { } v ? Num(v) : 0.0);
            ev["distance_curve"] = null;
            if (RefPath(P(p, "FearDistanceCurve")) is { } path)
            {
                var cname = path[(path.LastIndexOf('/') + 1)..];
                ev["distance_curve"] = cname;
                if (curves[cname] == null) curves[cname] = CurveKeys(rd, path);
            }
            fearEvents[name] = ev;
        }

        // The counselor's fear modifier (the FearComponent template on the base counselor).
        var fc = Reader.Props(Reader.Export(rd.Load("Characters/Counselors/Base/Base_Counselor"), n => n == "FearComponent"));
        var fearModifier = StatValue(fc["FearModifier"], 1.0);
        fearModifier["stat_order"] = new JArray(StatNames);

        var counselor = CounselorSection(rd, c, out var characters, curves);
        var items = ItemsSection(rd, c);
        var vehicle = VehicleSection(rd, c);
        var match = MatchSection(rd, provider, c, game);
        var root = new JObject
        {
            ["format"] = 9,
            ["match"] = match,
            ["counselor"] = new JObject { ["Base_Counselor"] = counselor, ["characters"] = characters },
            ["items"] = items,
            ["vehicle"] = vehicle,
            ["killer"] = new JObject { ["Jason_Base"] = killer, ["stalk_active_time"] = (double)c["stalk_active_time"] },
            ["fear"] = new JObject
            {
                ["update_fear_delay"] = (double)c["update_fear_delay"],
                ["fear_modifier"] = fearModifier,
                ["events"] = fearEvents,
                ["curves"] = curves,
            },
        };
        Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(outPath))!);
        File.WriteAllText(outPath, root.ToString(Formatting.Indented));
        Console.WriteLine($"wrote {outPath}: {fearEvents.Count} fear events, {curves.Count} curves, {((JObject)match["score_events"]!).Count} score events");
        return 0;
    }
}
