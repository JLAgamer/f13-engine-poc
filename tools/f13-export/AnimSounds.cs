// The sounds animations play (AnimNotify_PlaySound) for f13-game's anim_sounds.json.
using System.Text.RegularExpressions;
using Newtonsoft.Json.Linq;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Animation;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Assets.Objects;
using CUE4Parse.UE4.Assets.Exports.SkeletalMesh;

static class AnimSounds
{
    /// Each animation's PlaySound notifies, by the name f13-game plays it under (a sequence's, or each sequence a
    /// montage plays): [[time s, cue name]...] in that sequence's time. A sequence's own notifies, and those of the
    /// characters' montage segments playing it (by name without the `f_`/`m_` prefix, or its take number: the
    /// counselors' female takes mirror the shared montages, whose notifies hold the sounds), each in the segment it
    /// falls in. Jason's montages (under Characters/Killers) also go in as "killer/<sequence>": the paired kills' montages
    /// share their names with the counselor's, and each side plays its own. Also returns the cues' paths, for the sound
    /// export.
    public static (JObject Json, List<string> Cues) Collect(IFileProvider provider, IEnumerable<string> animations)
    {
        // Every montage's segments with sounds, by their sequences' bare names (the first montage wins).
        var segments = new Dictionary<string, List<(float Time, string Cue)>>(StringComparer.OrdinalIgnoreCase);
        foreach (var f in provider.Files.Keys.Where(k => k.EndsWith(".uasset") && k.Split('/')[^1].StartsWith("AM_", StringComparison.OrdinalIgnoreCase)).Order())
        {
            if (Load(provider, "/Game/" + f["SummerCamp/Content/".Length..^".uasset".Length]) is not UAnimMontage montage) continue;
            foreach (var (seq, found) in BySegment(montage))
                if (found.Count > 0)
                {
                    segments.TryAdd(Bare(seq), found);
                    segments.TryAdd(Unnumbered(seq), found);
                }
        }
        var json = new JObject();
        var cues = new List<string>();
        // A cue's kind prefix ("VO:", "MUSIC:" or none) and its path.
        static (string Prefix, string Path) Split(string cue) =>
            cue.StartsWith("VO:") ? ("VO:", cue[3..]) : cue.StartsWith("MUSIC:") ? ("MUSIC:", cue[6..]) : ("", cue);
        void Add(string name, List<(float Time, string Cue)> found)
        {
            if (found.Count == 0 || json.ContainsKey(name)) return;
            cues.AddRange(found.Select(n => Split(n.Cue).Path).Where(c => c.Contains('/')));
            json[name] = new JArray(found.Distinct().OrderBy(n => n.Time).Select(n => new JArray(n.Time, Split(n.Cue).Prefix + Split(n.Cue).Path.Split('.')[^1].Split('/')[^1])));
        }
        if (Environment.GetEnvironmentVariable("F13_ANIM_SOUNDS_DEBUG") is { } q)
            foreach (var k in segments.Keys.Where(k => k.Contains(q))) Console.WriteLine($"segment {k}: {string.Join(", ", segments[k])}");
        foreach (var path in animations)
        {
            var obj = Load(provider, path);
            if (obj is UAnimMontage m)
            {
                // By the sequence, and by the montage too (montages sharing a take can sound different: the fuse
                // box's and the phone box's breaks).
                foreach (var (seq, segment) in BySegment(m))
                {
                    Add(seq, segment);
                    Add($"{m.Name}/{seq}", segment);
                    if (path.Contains("/Characters/Killers/", StringComparison.OrdinalIgnoreCase)) Add($"killer/{seq}", segment);
                }
                continue;
            }
            if (obj == null) continue;
            var list = Sounds(obj).ToList();
            var bare = Bare(obj.Name);
            if (segments.TryGetValue(bare, out var found) || segments.TryGetValue(Unnumbered(obj.Name), out found))
                list.AddRange(found);
            Add(obj.Name, list);
        }
        return (json, cues);
    }

    /// Each montage's SCPropSpawnerNotifyState windows (a prop mesh on a socket while it lasts), by the names f13-game
    /// plays its sequences under (as [`Collect`]: "<seq>", "<montage>/<seq>", Jason's "killer/<seq>", a counselor's
    /// "counselor/<seq>"): [[start s, end s,
    /// mesh glTF, socket, bHideEquippedItem]...] in that sequence's time. `queue` gets each prop mesh for export;
    /// `meshPath` names its file.
    public static JObject Props(IFileProvider provider, IEnumerable<string> animations, Action<UObject> queue, Func<USkeletalMesh, string> meshPath)
    {
        var json = new JObject();
        foreach (var path in animations)
        {
            if (Load(provider, path) is not UAnimMontage m) continue;
            var windows = new List<(float Start, float End, string Mesh, string Socket, bool Hide)>();
            foreach (var n in m.GetOrDefault<FStructFallback[]>("Notifies", []))
            {
                if (n.GetOrDefault<FPackageIndex>("NotifyStateClass")?.Load() is not { ExportType: "SCPropSpawnerNotifyState" } state) continue;
                // (No mesh: the window only hides the equipped item, as the choke's does. bHideEquippedItem's default is
                // true: SCPropSpawnerNotifyState's constructor 0x1402f44e0.)
                var mesh = state.GetOrDefault<FPackageIndex>("SkeletalPropMesh")?.Load() as USkeletalMesh;
                if (mesh != null) queue(mesh);
                var start = n.GetOrDefault<float>("LinkValue");
                windows.Add((start, start + n.GetOrDefault<float>("Duration"), mesh == null ? "" : meshPath(mesh), state.GetOrDefault<FName>("SocketName").Text, state.GetOrDefault("bHideEquippedItem", true)));
            }
            if (windows.Count == 0) continue;
            foreach (var slot in m.SlotAnimTracks ?? [])
            {
                var segs = (slot.AnimTrack.AnimSegments ?? []).ToList();
                foreach (var seg in segs)
                {
                    if (seg.AnimReference.ResolvedObject?.Name.Text is not { } seq) continue;
                    var rate = seg.AnimPlayRate == 0 ? 1f : Math.Abs(seg.AnimPlayRate);
                    var length = (seg.AnimEndTime - seg.AnimStartTime) / rate * Math.Max(seg.LoopingCount, 1);
                    var end = seg.StartPos + length;
                    var mine = windows.Where(w => w.Start < end && w.End > seg.StartPos)
                        .Select(w => new JArray(seg.AnimStartTime + (Math.Max(w.Start, seg.StartPos) - seg.StartPos) * rate,
                            seg.AnimStartTime + (Math.Min(w.End, end) - seg.StartPos) * rate, w.Mesh, w.Socket, w.Hide)).ToList();
                    if (mine.Count == 0) continue;
                    var names = new List<string> { seq, $"{m.Name}/{seq}" };
                    if (path.Contains("/Characters/Killers/", StringComparison.OrdinalIgnoreCase)) names.Add($"killer/{seq}");
                    if (path.Contains("/Characters/Counselors/", StringComparison.OrdinalIgnoreCase)) names.Add($"counselor/{seq}");
                    foreach (var name in names)
                        if (!json.ContainsKey(name)) json[name] = new JArray(mine);
                }
            }
        }
        return json;
    }

    /// Each montage's SCPostProcessBlendableNotify windows (a post-process material on the owner's camera while it lasts),
    /// keyed as [`Props`]: [{Start, End, Material, Textures {param: texture path}, Scalars {param: default}, BlendIn,
    /// BlendOut, Min, Max, Hooks [[param, UseBlendIn, UseBlendOut]]}...] in that sequence's time; the textures (overrides
    /// and the material's own) go to `queue`. Unset properties are the constructor's (0x1402f4460: BlendInTime and
    /// BlendOutTime 0.25, MinIntensity 0, MaxIntensity 1, PlayForAllClients off).
    public static JObject Blendables(IFileProvider provider, IEnumerable<string> animations, Action<UObject> queue, Func<UObject, string> texturePath)
    {
        var json = new JObject();
        foreach (var path in animations)
        {
            if (Load(provider, path) is not UAnimMontage m) continue;
            var windows = new List<(float Start, float End, JObject Info)>();
            foreach (var n in m.GetOrDefault<FStructFallback[]>("Notifies", []))
            {
                if (n.GetOrDefault<FPackageIndex>("NotifyStateClass")?.Load() is not { ExportType: "SCPostProcessBlendableNotify" } state) continue;
                if (state.GetOrDefault<FPackageIndex>("MaterialInstance")?.Load() is not { } material) continue;
                var start = n.GetOrDefault<float>("LinkValue");
                // The material's own parameters (its expressions' defaults), then the notify's overrides.
                var textures = new JObject();
                var scalars = new JObject();
                foreach (var e in material.Owner!.GetExports())
                {
                    var name = e.GetOrDefault<FName>("ParameterName").Text;
                    if (name is null or "None") continue;
                    if (e.ExportType == "MaterialExpressionScalarParameter") scalars[name] = e.GetOrDefault<float>("DefaultValue");
                    if (e.ExportType.StartsWith("MaterialExpressionTextureSampleParameter") && e.GetOrDefault<FPackageIndex>("Texture")?.Load() is { } t)
                    {
                        queue(t);
                        textures[name] = texturePath(t);
                    }
                }
                foreach (var o in state.GetOrDefault<FStructFallback[]>("OverrideTextures", []))
                {
                    if (o.GetOrDefault<FPackageIndex>("Texture")?.Load() is not { } t) continue;
                    queue(t);
                    textures[o.GetOrDefault<FName>("ParameterName").Text] = texturePath(t);
                }
                var hooks = new JArray(state.GetOrDefault<FStructFallback[]>("IntensityHooks", [])
                    .Select(h => new JArray(h.GetOrDefault<FName>("ParameterName").Text, h.GetOrDefault<bool>("UseBlendIn"), h.GetOrDefault<bool>("UseBlendOut"))));
                windows.Add((start, start + n.GetOrDefault<float>("Duration"), new JObject
                {
                    ["Material"] = material.Name,
                    ["Textures"] = textures,
                    ["Scalars"] = scalars,
                    ["BlendIn"] = state.GetOrDefault("BlendInTime", 0.25f),
                    ["BlendOut"] = state.GetOrDefault("BlendOutTime", 0.25f),
                    ["Min"] = state.GetOrDefault("MinIntensity", 0f),
                    ["Max"] = state.GetOrDefault("MaxIntensity", 1f),
                    ["ForAll"] = state.GetOrDefault("PlayForAllClients", false),
                    ["Hooks"] = hooks,
                }));
            }
            if (windows.Count == 0) continue;
            foreach (var slot in m.SlotAnimTracks ?? [])
            {
                foreach (var seg in slot.AnimTrack.AnimSegments ?? [])
                {
                    if (seg.AnimReference.ResolvedObject?.Name.Text is not { } seq) continue;
                    var rate = seg.AnimPlayRate == 0 ? 1f : Math.Abs(seg.AnimPlayRate);
                    var end = seg.StartPos + (seg.AnimEndTime - seg.AnimStartTime) / rate * Math.Max(seg.LoopingCount, 1);
                    var mine = windows.Where(w => w.Start < end && w.End > seg.StartPos).Select(w =>
                    {
                        var j = (JObject)w.Info.DeepClone();
                        j["Start"] = seg.AnimStartTime + (Math.Max(w.Start, seg.StartPos) - seg.StartPos) * rate;
                        j["End"] = seg.AnimStartTime + (Math.Min(w.End, end) - seg.StartPos) * rate;
                        return j;
                    }).ToList();
                    if (mine.Count == 0) continue;
                    var names = new List<string>();
                    if (path.Contains("/Characters/Killers/", StringComparison.OrdinalIgnoreCase)) names.Add($"killer/{seq}");
                    if (path.Contains("/Characters/Counselors/", StringComparison.OrdinalIgnoreCase)) names.Add($"counselor/{seq}");
                    foreach (var name in names)
                        if (!json.ContainsKey(name)) json[name] = new JArray(mine);
                }
            }
        }
        return json;
    }

    static string Bare(string name) => Regex.Replace(name, "^(AM_)?[fFmM]_", "");
    /// Without its take number (`_01`, `_02`: a female take and the montage's male one are numbered apart).
    static string Unnumbered(string name) => Regex.Replace(Bare(name), @"_\d\d$", "");

    /// An object's PlaySound notifies: (its time, the cue's path); its SCAnimNotify_PlayCounselorVO notifies as
    /// "VO:<VoiceOverEvent>" (her voice object's line), and SCAnimNotify_PlayCounselorVOBySoundCue's cue as
    /// "VO:<cue path>", and SCAnimNotify_PlayCinematicMusic's MusicTrackToPlay as "MUSIC:<sound path>".
    static IEnumerable<(float Time, string Cue)> Sounds(UObject obj)
    {
        foreach (var n in obj.GetOrDefault<FStructFallback[]>("Notifies", []))
        {
            var notify = n.GetOrDefault<FPackageIndex>("Notify")?.Load();
            var time = n.GetOrDefault<float>("LinkValue");
            switch (notify?.ExportType)
            {
                case "AnimNotify_PlaySound":
                    if (notify.GetOrDefault<FPackageIndex>("Sound")?.ResolvedObject?.GetPathName() is { } sound) yield return (time, sound);
                    break;
                case "SCAnimNotify_PlayCounselorVO":
                    if (notify.GetOrDefault<FName>("VoiceOverEvent").Text is { } line && line != "None") yield return (time, "VO:" + line);
                    break;
                case "SCAnimNotify_PlayCounselorVOBySoundCue":
                    if (notify.GetOrDefault<FPackageIndex>("VoiceOverSoundCue")?.ResolvedObject?.GetPathName() is { } cue) yield return (time, "VO:" + cue);
                    break;
                case "SCAnimNotify_PlayCinematicMusic":
                    if (notify.GetOrDefault<FPackageIndex>("MusicTrackToPlay")?.ResolvedObject?.GetPathName() is { } track) yield return (time, "MUSIC:" + track);
                    break;
            }
        }
    }

    /// A montage's sounds in each segment's sequence, in that sequence's time (every slot's: some montages carry the
    /// female takes in a slot of their own).
    static List<(string Seq, List<(float Time, string Cue)> List)> BySegment(UAnimMontage m)
    {
        var sounds = Sounds(m).ToList();
        var result = new List<(string, List<(float, string)>)>();
        foreach (var slot in m.SlotAnimTracks ?? [])
        {
        var segs = (slot.AnimTrack.AnimSegments ?? []).ToList();
        foreach (var seg in segs)
        {
            if (seg.AnimReference.ResolvedObject?.Name.Text is not { } name) continue;
            var rate = seg.AnimPlayRate == 0 ? 1f : Math.Abs(seg.AnimPlayRate);
            var length = (seg.AnimEndTime - seg.AnimStartTime) / rate * Math.Max(seg.LoopingCount, 1);
            // A notify on a boundary belongs to the segment starting there (the last one keeps its end).
            var end = seg.StartPos + length + (seg == segs[^1] ? 1e-3f : -1e-4f);
            var mine = sounds.Where(s => s.Time >= seg.StartPos - 1e-4f && s.Time < end)
                .Select(s => (seg.AnimStartTime + (s.Time - seg.StartPos) * rate, s.Cue)).ToList();
            result.Add((name, mine));
        }
        }
        return result;
    }

    /// Each take's gore notifies, keyed as [`Props`]: SCApplyBloodMaskAnimNotify ({Time, Kind "Blood", Limb (ELimb's
    /// name), Mask (texture path), Parameter, Elements}) and SCDetachLimbAnimNotify ({Time, Kind "Detach", Limb, Impulse
    /// [x, y, z], Mask (BloodMaskOverride, or none), ShowLimb}), in that sequence's time; the masks go to `queue`.
    public static JObject Gore(IFileProvider provider, IEnumerable<string> animations, Action<UObject> queue, Func<UObject, string> texturePath)
    {
        var json = new JObject();
        foreach (var path in animations)
        {
            if (Load(provider, path) is not UAnimSequenceBase anim) continue;
            var events = new List<(float Time, JObject Info)>();
            foreach (var n in anim.GetOrDefault<FStructFallback[]>("Notifies", []))
            {
                if (n.GetOrDefault<FPackageIndex>("Notify")?.Load() is not { } notify) continue;
                // Its mask: a texture reference (soft for the detach's override, read either way). Unset, the
                // notify uses the limb's own (its LimbUVTexture, TextureParameterName, ParentMaterialElementIndices).
                var maskName = notify.ExportType == "SCDetachLimbAnimNotify" ? "BloodMaskOverride" : "BloodMask";
                UObject? tex = null;
                try { tex = notify.GetOrDefault<FPackageIndex?>(maskName)?.Load(); } catch { }
                if (tex == null && notify.GetOrDefault<FSoftObjectPath?>(maskName) is { } sp && sp.AssetPathName.Text is { Length: > 0 } a && a != "None" && sp.TryLoad(out UObject soft)) tex = soft;
                string? maskPath = null;
                if (tex != null)
                {
                    queue(tex);
                    maskPath = texturePath(tex);
                }
                var info = notify.ExportType switch
                {
                    "SCApplyBloodMaskAnimNotify" => new JObject
                    {
                        ["Kind"] = "Blood",
                        ["Limb"] = notify.GetOrDefault<FName>("Limb").Text,
                        ["Mask"] = maskPath,
                        ["Parameter"] = notify.GetOrDefault<FName>("TextureParameter").Text,
                        ["Elements"] = new JArray(notify.GetOrDefault<int[]>("MaterialElementIndices", [])),
                    },
                    "SCDetachLimbAnimNotify" => new JObject
                    {
                        ["Kind"] = "Detach",
                        ["Limb"] = notify.GetOrDefault<FName>("LimbToDetach").Text,
                        ["Impulse"] = notify.GetOrDefault<CUE4Parse.UE4.Objects.Core.Math.FVector?>("Impulse") is { } i ? new JArray(i.X, i.Y, i.Z) : new JArray(0, 0, 0),
                        ["Mask"] = maskPath,
                        ["ShowLimb"] = notify.GetOrDefault("bShowLimbAfterDetach", false),
                    },
                    _ => null,
                };
                if (info != null) events.Add((n.GetOrDefault<float>("LinkValue"), info));
            }
            if (events.Count == 0) continue;
            void Add(string seq, IEnumerable<JObject> mine)
            {
                var names = new List<string> { seq };
                if (path.Contains("/Characters/Killers/", StringComparison.OrdinalIgnoreCase)) names.Add($"killer/{seq}");
                if (path.Contains("/Characters/Counselors/", StringComparison.OrdinalIgnoreCase)) names.Add($"counselor/{seq}");
                var list = mine.ToList();
                foreach (var name in names)
                {
                    if (json[name] is JArray existing) foreach (var e in list) existing.Add(e);
                    else json[name] = new JArray(list);
                }
            }
            if (anim is UAnimMontage m)
            {
                foreach (var slot in m.SlotAnimTracks ?? [])
                    foreach (var seg in slot.AnimTrack.AnimSegments ?? [])
                    {
                        if (seg.AnimReference.ResolvedObject?.Name.Text is not { } seq) continue;
                        var rate = seg.AnimPlayRate == 0 ? 1f : Math.Abs(seg.AnimPlayRate);
                        var end = seg.StartPos + (seg.AnimEndTime - seg.AnimStartTime) / rate * Math.Max(seg.LoopingCount, 1);
                        Add(seq, events.Where(e => e.Time >= seg.StartPos && e.Time < end).Select(e =>
                        {
                            var j = (JObject)e.Info.DeepClone();
                            j["Time"] = seg.AnimStartTime + (e.Time - seg.StartPos) * rate;
                            return j;
                        }));
                    }
            }
            else
            {
                Add(anim.Name, events.Select(e => { var j = (JObject)e.Info.DeepClone(); j["Time"] = e.Time; return j; }));
            }
        }
        return json;
    }

    static UObject? Load(IFileProvider provider, string path)
    {
        try { return provider.LoadPackageObject(path + "." + path.Split('/')[^1]); }
        catch (Exception e) { Console.WriteLine($"anim sounds {path}: {e.Message}"); return null; }
    }
}
