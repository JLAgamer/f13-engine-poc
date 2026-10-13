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
                if (state.GetOrDefault<FPackageIndex>("SkeletalPropMesh")?.Load() is not USkeletalMesh mesh) continue;
                queue(mesh);
                var start = n.GetOrDefault<float>("LinkValue");
                windows.Add((start, start + n.GetOrDefault<float>("Duration"), meshPath(mesh), state.GetOrDefault<FName>("SocketName").Text, state.GetOrDefault<bool>("bHideEquippedItem")));
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

    static UObject? Load(IFileProvider provider, string path)
    {
        try { return provider.LoadPackageObject(path + "." + path.Split('/')[^1]); }
        catch (Exception e) { Console.WriteLine($"anim sounds {path}: {e.Message}"); return null; }
    }
}
