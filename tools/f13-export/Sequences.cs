// Level sequences (UE 4.18 MovieScene: times in seconds, curves as FRichCurve) as compact JSON for f13-game's sequence
// player (sequences/<name>.json): the playback range, the bindings (possessables, with the level object each resolves
// to through the LevelSequence's ObjectReferences), and every track with its sections' properties as the package has
// them (curves, keys, ranges), minus the easing objects.
using CUE4Parse.FileProvider;
using Newtonsoft.Json.Linq;

static class SequenceExporter
{
    public static void Write(IFileProvider provider, string outDir, IEnumerable<string> packages)
    {
        var dir = Path.Combine(outDir, "sequences");
        Directory.CreateDirectory(dir);
        var queue = new Queue<string>(packages);
        var done = new HashSet<string>();
        while (queue.TryDequeue(out var package))
        {
            if (!done.Add(package)) continue;
            var exports = provider.LoadPackage(package).GetExports().Select(e => JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(e))).ToList();
            // A reference's ObjectPath ends in the export's index in this package.
            JObject? Resolve(JToken? r)
            {
                var path = r?["ObjectPath"]?.ToString();
                if (path == null) return null;
                var dot = path.LastIndexOf('.');
                return dot >= 0 && int.TryParse(path[(dot + 1)..], out var i) && i >= 0 && i < exports.Count ? exports[i] : null;
            }
            var sequence = exports.FirstOrDefault(e => e["Type"]?.ToString() == "LevelSequence");
            var scene = Resolve(sequence?["Properties"]?["MovieScene"]);
            if (sequence == null || scene == null)
            {
                Console.WriteLine($"{package}: no level sequence");
                continue;
            }
            var props = (JObject)scene["Properties"]!;
            var refs = sequence["Properties"]?["ObjectReferences"] as JObject;
            JObject Section(JToken r)
            {
                var s = Resolve(r);
                var p = (JObject?)s?["Properties"]?.DeepClone() ?? [];
                p.Remove("Easing");
                p.Remove("Signature");
                // A shot's sub-sequence is exported too.
                if (p["SubSequence"]?["ObjectPath"]?.ToString() is { } sub) queue.Enqueue(sub[..sub.LastIndexOf('.')]);
                return new JObject { ["Type"] = s?["Type"], ["Props"] = p };
            }
            JObject Track(JToken r, string? binding)
            {
                var t = Resolve(r);
                var p = (JObject?)t?["Properties"]?.DeepClone() ?? [];
                var sections = new JArray();
                foreach (var key in new[] { "Sections", "AnimationSections", "ParticleSections", "AudioSections" })
                {
                    if (p[key] is JArray list) foreach (var s in list) sections.Add(Section(s));
                    p.Remove(key);
                }
                p.Remove("Signature");
                p.Remove("EvalOptions");
                return new JObject { ["Binding"] = binding, ["Type"] = t?["Type"], ["Props"] = p, ["Sections"] = sections };
            }
            var tracks = new JArray();
            foreach (var b in props["ObjectBindings"] as JArray ?? [])
                foreach (var t in b["Tracks"] as JArray ?? []) tracks.Add(Track(t, b["ObjectGuid"]?.ToString()));
            foreach (var t in props["MasterTracks"] as JArray ?? []) tracks.Add(Track(t, null));
            if (props["CameraCutTrack"] is JObject cut) tracks.Add(Track(cut, null));
            var bindings = new JArray();
            foreach (var p in props["Possessables"] as JArray ?? [])
            {
                var guid = p["Guid"]?.ToString() ?? "";
                var key = guid.Replace("-", "");
                bindings.Add(new JObject
                {
                    ["Guid"] = guid,
                    ["Name"] = p["Name"],
                    ["Class"] = p["PossessedObjectClass"]?["ObjectName"]?.ToString().Split('\'').ElementAtOrDefault(1),
                    ["Parent"] = p["ParentGuid"]?.ToString() is { } parent && parent != "00000000-00000000-00000000-00000000" ? parent : null,
                    ["Object"] = refs?[key]?["ObjectPath"],
                });
            }
            var range = props["PlaybackRange"];
            var o = new JObject
            {
                ["Name"] = sequence["Name"],
                ["Start"] = range?["LowerBound"]?["Value"] ?? 0.0,
                ["End"] = range?["UpperBound"]?["Value"] ?? 0.0,
                ["Bindings"] = bindings,
                ["Tracks"] = tracks,
            };
            var path = Path.Combine(dir, sequence["Name"] + ".json");
            File.WriteAllText(path, o.ToString(Newtonsoft.Json.Formatting.Indented));
            Console.WriteLine($"{path}: {bindings.Count} bindings, {tracks.Count} tracks");
        }
    }
}
