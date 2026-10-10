// Cascade particle systems (UE 4.18 UParticleSystem) as JSON for f13-game's particle player (particles/<name>.json):
// every export of the package (the system, its emitters, their LOD levels, modules and the modules' distributions)
// with its properties as the package has them. References inside the package keep their ObjectPath, which ends in the
// export's index in "Exports". Cooked distributions carry their baked lookup tables (FRawDistribution's Table).
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Objects.UObject;
using Newtonsoft.Json.Linq;

static class ParticleExporter
{
    /// Writes each system by name (resolved among the game's ParticleSystem packages) and queues the materials its
    /// emitters draw with. Returns the systems written.
    public static int Write(IFileProvider provider, string outDir, IEnumerable<string> names, Action<UObject> queue)
    {
        var dir = Path.Combine(outDir, "particles");
        Directory.CreateDirectory(dir);
        var n = 0;
        foreach (var name in names.Distinct())
        {
            var file = provider.Files.Keys.FirstOrDefault(k => k.EndsWith("/" + name + ".uasset", StringComparison.OrdinalIgnoreCase));
            if (file == null)
            {
                Console.WriteLine($"particles: {name}: no package");
                continue;
            }
            var pkg = file[..^".uasset".Length];
            var exports = provider.LoadPackage(pkg).GetExports().ToList();
            var list = new JArray();
            foreach (var e in exports)
            {
                var o = JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(e));
                list.Add(new JObject { ["Type"] = e.ExportType, ["Name"] = e.Name, ["Props"] = o["Properties"] ?? new JObject() });
                // Each emitter LOD's required module names its material.
                if (e.ExportType == "ParticleModuleRequired" && e.GetOrDefault<FPackageIndex?>("Material")?.Load() is { } m) queue(m);
                if (e.ExportType == "ParticleModuleTypeDataMesh" && e.GetOrDefault<FPackageIndex?>("Mesh")?.Load() is { } mesh) queue(mesh);
            }
            var json = new JObject { ["Name"] = name, ["Package"] = pkg, ["Exports"] = list };
            File.WriteAllText(Path.Combine(dir, name + ".json"), json.ToString(Newtonsoft.Json.Formatting.Indented));
            n++;
        }
        return n;
    }
}
