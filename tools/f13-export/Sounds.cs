// Sounds f13-game plays: each SoundCue as its node graph (sounds/<cue>.json: the cue's volume, pitch and attenuation,
// then the nodes: Random (weights), Modulator (volume/pitch ranges), Mixer, Looping, Delay, Concatenator, Attenuation
// (its settings), WavePlayer (the wave file)), and every wave decoded (sounds/waves/<wave>.<format>). A path that names
// a SoundWave instead of a cue gets a one-node cue.
//
// The counselor's footstep sounds aren't data: Base_Counselor_AnimBP's GetFootstepSound / GetLandingSound /
// GetFootstepInWater pick them in Blueprint script. Each of their "Select" nodes compiles to the soft-object constants
// stored in temporaries (EX_Let temp = EX_SoftObjectConst "path") and an EX_SwitchValue over the surface type
// (EX_ByteConst case -> EX_LocalVariable temp). sounds/footsteps.json has every switch, its cues by surface type
// (null: none), and which table it is (female/male, run/walk) by the cues it holds.
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Sound;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Assets.Objects;
using CUE4Parse_Conversion.Sounds;

class SoundExporter(IFileProvider provider, string outDir, Func<string, string> toFile)
{
    readonly HashSet<string> written = new(StringComparer.OrdinalIgnoreCase);
    readonly Dictionary<string, string> waves = new(StringComparer.OrdinalIgnoreCase);
    string Dir => Path.Combine(outDir, "sounds");

    public const string CounselorAnimBp = "/Game/Characters/Counselors/Base/Base_Counselor_AnimBP";
    public const string JasonAnimBp = "/Game/Characters/Killers/Jason/Base/Jason_AnimBP";
    static readonly string[] FootstepFunctions = ["GetFootstepSound", "GetLandingSound", "GetFootstepInWater"];

    /// Writes the cues (and their waves); returns how many cues were written.
    public int Export(IEnumerable<string> paths)
    {
        Directory.CreateDirectory(Path.Combine(Dir, "waves"));
        var n = 0;
        foreach (var p in paths)
        {
            try
            {
                if (ExportCue(p)) n++;
            }
            catch (Exception e)
            {
                Console.WriteLine($"sound {p}: {e.Message}");
            }
        }
        return n;
    }

    static string NameOf(string path) => path.Split('.')[^1].Split('/')[^1];

    bool ExportCue(string path)
    {
        var name = NameOf(path);
        if (!written.Add(name)) return false;
        var obj = provider.LoadPackage(toFile(path)).GetExports().FirstOrDefault(e => e is USoundCue or USoundWave);
        JObject cue;
        if (obj is USoundWave w)
            cue = new JObject { ["Volume"] = 1f, ["Pitch"] = 1f, ["Root"] = WaveNode(w, new JObject()) };
        else if (obj is USoundCue c)
        {
            cue = new JObject
            {
                ["Volume"] = c.GetOrDefault("VolumeMultiplier", 1f),
                ["Pitch"] = c.GetOrDefault("PitchMultiplier", 1f),
            };
            if (Attenuation(c.GetOrDefault<FPackageIndex?>("AttenuationSettings")) is { } att) cue["Attenuation"] = att;
            if (c.GetOrDefault<FPackageIndex?>("FirstNode")?.Load<UObject>() is { } first) cue["Root"] = Node(first);
        }
        else return false;
        // Its SoundClassObject (USoundBase's: the sound mixes adjust sounds by class).
        if (obj.GetOrDefault<FPackageIndex?>("SoundClassObject")?.Load<UObject>() is { } sc) cue["SoundClass"] = sc.Name;
        File.WriteAllText(Path.Combine(Dir, name + ".json"), cue.ToString(Formatting.Indented));
        return true;
    }

    /// sounds/sound_classes.json: every SoundClass (Content/Sounds/SoundClasses), its parent and its own Volume and
    /// Pitch (FSoundClassProperties; 1 unless set).
    public void WriteSoundClasses()
    {
        var classes = new JObject();
        foreach (var f in provider.Files.Keys.Where(k => k.Contains("/Sounds/SoundClasses/", StringComparison.OrdinalIgnoreCase) && k.EndsWith(".uasset")))
            foreach (var sc in provider.LoadPackage(f).GetExports().Where(e => e.ExportType == "SoundClass"))
            {
                var p = sc.GetOrDefault<FStructFallback?>("Properties");
                classes[sc.Name] = new JObject
                {
                    ["Parent"] = sc.GetOrDefault<FPackageIndex?>("ParentClass")?.Load<UObject>()?.Name,
                    ["Volume"] = p?.GetOrDefault("Volume", 1f) ?? 1f,
                    ["Pitch"] = p?.GetOrDefault("Pitch", 1f) ?? 1f,
                };
            }
        File.WriteAllText(Path.Combine(Dir, "sound_classes.json"), classes.ToString(Formatting.Indented));
    }

    /// sounds/sound_mixes.json: the sound mixes f13-game pushes, by name: their fades and class adjusters (the EQ isn't
    /// played).
    public void WriteSoundMixes(IEnumerable<string> paths)
    {
        var mixes = new JObject();
        foreach (var path in paths)
            foreach (var m in provider.LoadPackage(toFile(path)).GetExports().Where(e => e.ExportType == "SoundMix"))
            {
                var effects = new JArray();
                foreach (var e in m.GetOrDefault<FStructFallback[]>("SoundClassEffects", []))
                    effects.Add(new JObject
                    {
                        ["Class"] = e.GetOrDefault<FPackageIndex?>("SoundClassObject")?.Load<UObject>()?.Name,
                        ["Volume"] = e.GetOrDefault("VolumeAdjuster", 1f),
                        ["Pitch"] = e.GetOrDefault("PitchAdjuster", 1f),
                        ["ApplyToChildren"] = e.GetOrDefault("bApplyToChildren", false),
                    });
                mixes[m.Name] = new JObject
                {
                    ["InitialDelay"] = m.GetOrDefault("InitialDelay", 0f),
                    ["FadeInTime"] = m.GetOrDefault("FadeInTime", 0.2f),
                    ["Duration"] = m.GetOrDefault("Duration", -1f),
                    ["FadeOutTime"] = m.GetOrDefault("FadeOutTime", 0.2f),
                    ["Effects"] = effects,
                };
            }
        File.WriteAllText(Path.Combine(Dir, "sound_mixes.json"), mixes.ToString(Formatting.Indented));
    }

    static JObject Props(UObject o) => JObject.Parse(JsonConvert.SerializeObject(o))["Properties"] as JObject ?? new JObject();

    // A SoundAttenuation asset's settings.
    static JObject? Attenuation(FPackageIndex? asset) =>
        asset?.Load<UObject>() is { } a ? Props(a)["Attenuation"] as JObject : null;

    JObject Node(UObject node)
    {
        var type = node.ExportType.StartsWith("SoundNode") ? node.ExportType["SoundNode".Length..] : node.ExportType;
        var props = Props(node);
        props.Remove("ChildNodes");
        var j = new JObject { ["Type"] = type, ["Props"] = props };
        if (type == "WavePlayer")
        {
            var wave = node.GetOrDefault<FPackageIndex?>("SoundWave")?.Load<USoundWave>();
            if (wave == null && node.GetOrDefault<FSoftObjectPath?>("SoundWaveAssetPtr") is { } sp && !sp.AssetPathName.IsNone)
                wave = provider.LoadPackage(toFile(sp.AssetPathName.Text)).GetExports().OfType<USoundWave>().FirstOrDefault();
            if (wave != null) WaveNode(wave, j);
        }
        if (type == "Attenuation" && Attenuation(node.GetOrDefault<FPackageIndex?>("AttenuationSettings")) is { } att) j["Settings"] = att;
        var children = new JArray();
        foreach (var c in node.GetOrDefault<FPackageIndex[]>("ChildNodes", []))
            children.Add(c.Load<UObject>() is { } child ? Node(child) : JValue.CreateNull());
        j["Children"] = children;
        return j;
    }

    JObject WaveNode(USoundWave wave, JObject j)
    {
        j["Type"] = "WavePlayer";
        j["Wave"] = WaveFile(wave);
        var p = Props(wave);
        j["WaveLooping"] = p["bLooping"]?.Value<bool>() ?? false;
        j["WaveVolume"] = p["Volume"]?.Value<float>() ?? 1f;
        j["WavePitch"] = p["Pitch"]?.Value<float>() ?? 1f;
        if (!j.ContainsKey("Props")) j["Props"] = new JObject();
        return j;
    }

    // The wave decoded to a file (once); its path under sounds/.
    string? WaveFile(USoundWave wave)
    {
        if (waves.TryGetValue(wave.Name, out var done)) return done;
        SoundDecoder.Decode(wave, true, out var format, out var data);
        string? file = null;
        if (data != null && !string.IsNullOrEmpty(format))
        {
            file = $"waves/{wave.Name}.{format.ToLowerInvariant()}";
            File.WriteAllBytes(Path.Combine(Dir, file), data);
        }
        else Console.WriteLine($"wave {wave.Name}: no data");
        waves[wave.Name] = file ?? "";
        return file;
    }

    /// The cues classes.json's classes name (doors, windows, barricades, cars...), as package paths: classes.json keeps
    /// only asset names, so each is looked up among the game's packages (preferring /Sounds/).
    public List<string> ClassSounds()
    {
        // classes.json, and the characters' exports (their Sounds).
        var files = new[] { Path.Combine(outDir, "classes.json") }
            .Concat(new[] { "counselors", "killers" }.Select(d => Path.Combine(outDir, d)).Where(Directory.Exists).SelectMany(d => Directory.GetFiles(d, "*.json")))
            .Where(File.Exists);
        var text = string.Join('\n', files.Select(File.ReadAllText));
        var names = System.Text.RegularExpressions.Regex.Matches(text, @"Sound(?:Cue|Wave)'([^']+)'")
            .Select(m => m.Groups[1].Value).Distinct(StringComparer.OrdinalIgnoreCase).ToHashSet(StringComparer.OrdinalIgnoreCase);
        // Soft references to sounds (an item's DroppedSoundCue: {"AssetPathName": "/Game/Sounds/.../Name.Name"}).
        foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(text, @"/Game/Sounds/[\w/]+\.(\w+)"))
            names.Add(m.Groups[1].Value);
        var found = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var key in provider.Files.Keys.Where(k => k.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase)))
        {
            var name = Path.GetFileNameWithoutExtension(key);
            if (!names.Contains(name)) continue;
            if (!found.TryGetValue(name, out var have) || (!have.Contains("/Sounds/", StringComparison.OrdinalIgnoreCase) && key.Contains("/Sounds/", StringComparison.OrdinalIgnoreCase)))
                found[name] = key;
        }
        foreach (var n in names.Where(n => !found.ContainsKey(n))) Console.WriteLine($"sound {n}: not found");
        return found.Values.Select(k => k[..^".uasset".Length]).ToList();
    }

    /// Decodes the footstep functions' switches into sounds/footsteps.json; returns every cue path they name.
    /// The counselor's footstep tables (footsteps.json) and Jason's (footsteps_jason.json).
    public List<string> FootstepTables() => [.. FootstepTables(CounselorAnimBp, "footsteps.json"), .. FootstepTables(JasonAnimBp, "footsteps_jason.json")];

    public List<string> FootstepTables(string animBp, string outName)
    {
        Directory.CreateDirectory(Dir);
        var file = toFile(animBp);
        var pkg = (Package)provider.LoadPackage(file);
        var bytes = provider.SavePackage(file).OrderBy(f => f.Key.EndsWith(".uexp")).SelectMany(f => f.Value).ToArray();
        var cues = new List<string>();
        var outJ = new JObject();
        foreach (var fn in FootstepFunctions)
        {
            var e = pkg.ExportMap.FirstOrDefault(x => x.ObjectName.Text == fn);
            if (e == null) continue;
            var script = bytes.AsSpan((int)e.SerialOffset, (int)e.SerialSize).ToArray();
            var tables = new JArray();
            foreach (var (cases, dflt) in Switches(script))
            {
                var names = cases.Select(c => c == null ? null : NameOf(c)).ToList();
                cues.AddRange(cases.Where(c => c != null)!);
                var female = names.Count(n => n != null && n.Contains("Female", StringComparison.OrdinalIgnoreCase));
                var male = names.Count(n => n != null && n.Contains("Male", StringComparison.OrdinalIgnoreCase) && !n.Contains("Female", StringComparison.OrdinalIgnoreCase));
                var run = names.Count(n => n != null && n.Contains("Run", StringComparison.OrdinalIgnoreCase));
                var walk = names.Count(n => n != null && n.Contains("Walk", StringComparison.OrdinalIgnoreCase));
                tables.Add(new JObject
                {
                    ["Gender"] = female >= male ? "female" : "male",
                    ["Gait"] = fn == "GetLandingSound" ? "land" : run > walk ? "run" : walk > run ? "walk" : "",
                    ["Surfaces"] = new JArray(names),
                    ["Default"] = dflt == null ? null : NameOf(dflt),
                });
                if (dflt != null) cues.Add(dflt);
            }
            // Functions without a switch (GetFootstepInWater): every cue they name, in order.
            if (tables.Count == 0)
            {
                var all = System.Text.RegularExpressions.Regex.Matches(System.Text.Encoding.ASCII.GetString(script), @"/Game/[\w/]+\.\w+")
                    .Select(m => m.Value).Distinct().ToList();
                cues.AddRange(all);
                outJ[fn] = new JArray(all.Select(NameOf));
            }
            else outJ[fn] = tables;
        }
        File.WriteAllText(Path.Combine(Dir, outName), outJ.ToString(Formatting.Indented));
        return cues.Distinct().ToList();
    }

    // EX_Let (0x0F) temp; EX_LocalVariable (0x00) temp; EX_SoftObjectConst (0x67) EX_StringConst (0x1F) "path".
    static Dictionary<int, string?> Temps(byte[] b)
    {
        var temps = new Dictionary<int, string?>();
        for (var i = 0; i + 12 < b.Length; i++)
        {
            if (b[i] != 0x0F || b[i + 5] != 0x00 || b[i + 10] != 0x67 || b[i + 11] != 0x1F) continue;
            var temp = BitConverter.ToInt32(b, i + 1);
            if (temp != BitConverter.ToInt32(b, i + 6)) continue;
            var end = Array.IndexOf(b, (byte)0, i + 12);
            if (end < 0) continue;
            var s = System.Text.Encoding.ASCII.GetString(b, i + 12, end - i - 12);
            temps[temp] = s.Length == 0 ? null : s;
        }
        return temps;
    }

    // EX_SwitchValue (0x69): uint16 cases, uint32 end offset, index expr (a variable), then per case EX_ByteConst
    // (0x24) value, uint32 next offset, EX_LocalVariable temp; then the default (EX_LocalVariable temp).
    static IEnumerable<(List<string?> Cases, string? Default)> Switches(byte[] b)
    {
        var temps = Temps(b);
        for (var i = 0; i + 8 < b.Length; i++)
        {
            if (b[i] != 0x69) continue;
            int n = BitConverter.ToUInt16(b, i + 1);
            if (n < 2 || n > 64) continue;
            var j = i + 7;
            if (j >= b.Length || (b[j] != 0x00 && b[j] != 0x01)) continue;
            j += 5;
            var cases = new List<string?>();
            var ok = true;
            for (var k = 0; k < n && ok; k++)
            {
                if (j + 11 > b.Length || b[j] != 0x24 || b[j + 1] != k || b[j + 6] != 0x00) { ok = false; break; }
                var temp = BitConverter.ToInt32(b, j + 7);
                if (!temps.TryGetValue(temp, out var s)) { ok = false; break; }
                cases.Add(s);
                j += 11;
            }
            if (!ok) continue;
            string? dflt = null;
            if (j + 5 <= b.Length && b[j] == 0x00) temps.TryGetValue(BitConverter.ToInt32(b, j + 1), out dflt);
            yield return (cases, dflt);
        }
    }
}
