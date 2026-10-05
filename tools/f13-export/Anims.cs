// anims.json: play length and root motion of the montages the gameplay rules need (dodges, stumbles...).
// Root motion is the root bone's displacement from the first to the last key of each segment's animation, in the
// character's local Unreal space (cm; +X forward, +Y right).
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports.Animation;
using CUE4Parse_Conversion.Animations;

static class AnimExporter
{
    public static void Write(DefaultFileProvider provider, string outDir, IEnumerable<string> montages)
    {
        var result = new SortedDictionary<string, object>(StringComparer.OrdinalIgnoreCase);
        foreach (var path in montages)
        {
            var montage = provider.LoadPackageObject<UAnimMontage>(path);
            var rate = montage.RateScale == 0 ? 1f : montage.RateScale;
            float[] root = [0, 0, 0];
            foreach (var slot in montage.SlotAnimTracks ?? [])
            foreach (var seg in slot.AnimTrack.AnimSegments ?? [])
            {
                if (!seg.AnimReference.TryLoad(out UAnimSequence? seq) || seq == null || !seq.GetOrDefault<bool>("bEnableRootMotion")) continue;
                var set = seq.ConvertAnims();
                var track = set.Sequences[0].Tracks[0]; // bone 0 = root
                if (track.KeyPos.Length < 2) continue;
                var d = track.KeyPos[^1] - track.KeyPos[0];
                root[0] += d.X; root[1] += d.Y; root[2] += d.Z;
            }
            result[path.Split('/')[^1]] = new { Length = montage.SequenceLength / rate, RootMotion = root };
            Console.WriteLine($"{path.Split('/')[^1]}: {montage.SequenceLength / rate:F3} s, root motion ({root[0]:F1}, {root[1]:F1}, {root[2]:F1})");
        }
        File.WriteAllText(Path.Combine(outDir, "anims.json"), Newtonsoft.Json.JsonConvert.SerializeObject(result, Newtonsoft.Json.Formatting.Indented));
    }
}
