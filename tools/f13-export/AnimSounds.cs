// The sounds animations play (AnimNotify_PlaySound) for f13-game's anim_sounds.json.
using System.Text.RegularExpressions;
using Newtonsoft.Json.Linq;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Animation;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Assets.Objects;

static class AnimSounds
{
    /// Each animation's PlaySound notifies, by the name f13-game plays it under (a sequence's, or each sequence a
    /// montage plays): [[time s, cue name]...] in that sequence's time. A sequence's own notifies, and those of the
    /// characters' montage segments playing it (by name without the `f_`/`m_` prefix, or its take number: the
    /// counselors' female takes mirror the shared montages, whose notifies hold the sounds), each in the segment it
    /// falls in. Also returns the cues' paths, for the sound export.
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
        void Add(string name, List<(float Time, string Cue)> found)
        {
            if (found.Count == 0 || json.ContainsKey(name)) return;
            cues.AddRange(found.Select(n => n.Cue));
            json[name] = new JArray(found.Distinct().OrderBy(n => n.Time).Select(n => new JArray(n.Time, n.Cue.Split('.')[^1].Split('/')[^1])));
        }
        if (Environment.GetEnvironmentVariable("F13_ANIM_SOUNDS_DEBUG") is { } q)
            foreach (var k in segments.Keys.Where(k => k.Contains(q))) Console.WriteLine($"segment {k}: {string.Join(", ", segments[k])}");
        foreach (var path in animations)
        {
            var obj = Load(provider, path);
            if (obj is UAnimMontage m)
            {
                foreach (var (seq, segment) in BySegment(m)) Add(seq, segment);
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

    static string Bare(string name) => Regex.Replace(name, "^(AM_)?[fFmM]_", "");
    /// Without its take number (`_01`, `_02`: a female take and the montage's male one are numbered apart).
    static string Unnumbered(string name) => Regex.Replace(Bare(name), @"_\d\d$", "");

    /// An object's PlaySound notifies: (its time, the cue's path).
    static IEnumerable<(float Time, string Cue)> Sounds(UObject obj)
    {
        foreach (var n in obj.GetOrDefault<FStructFallback[]>("Notifies", []))
        {
            if (n.GetOrDefault<FPackageIndex>("Notify")?.Load() is not { ExportType: "AnimNotify_PlaySound" } notify) continue;
            if (notify.GetOrDefault<FPackageIndex>("Sound")?.ResolvedObject?.GetPathName() is { } sound)
                yield return (n.GetOrDefault<float>("LinkValue"), sound);
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
