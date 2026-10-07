// f13-export: exports meshes and level layouts from YOUR OWN install of Friday the 13th: The Game to glTF + JSON,
// for f13-rs to load. Nothing is uploaded or written into the install; output goes to --out (keep it out of the repo).
//
//   f13-export --game <F13Game dir> --out <dir> assets <object path>...   e.g. /Game/Characters/Counselors/Athlete/Meshes/Body/Athlete_Body
//   f13-export --game <F13Game dir> --out <dir> level <map path>          e.g. /Game/Maps/CrystalLake_Small/CrystalLake_Small
//   f13-export --game <F13Game dir> --out <dir> mvp                       everything the MVP needs
using System.Text.Json;
using Serilog;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Actor;
using CUE4Parse.UE4.Assets.Exports.Component;
using CUE4Parse.UE4.Assets.Exports.Component.StaticMesh;
using CUE4Parse.UE4.Assets.Exports.SkeletalMesh;
using CUE4Parse.UE4.Assets.Exports.StaticMesh;
using CUE4Parse.UE4.Objects.Core.Math;
using CUE4Parse.UE4.Objects.Engine;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Versions;
using CUE4Parse.UE4.Assets;
using CUE4Parse.UE4.Assets.Readers;
using CUE4Parse.UE4.Assets.Objects;

using CUE4Parse.UE4.Assets.Exports.Material;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse_Conversion;
using CUE4Parse_Conversion.Exporters;
using CUE4Parse_Conversion.Options;

string? game = null, outDir = null;
var rest = new List<string>();
for (var i = 0; i < args.Length; i++)
{
    switch (args[i])
    {
        case "--game": game = args[++i]; break;
        case "--out": outDir = args[++i]; break;
        default: rest.Add(args[i]); break;
    }
}
if (game == null || outDir == null || rest.Count == 0)
{
    Console.Error.WriteLine("usage: f13-export --game <F13Game dir> --out <dir> (assets <path>... | level <map> | mvp)");
    return 2;
}

if (InstallProblem(game) is { } problem)
{
    Console.Error.WriteLine(problem);
    return 1;
}

Serilog.Log.Logger =new Serilog.LoggerConfiguration().MinimumLevel.Warning().WriteTo.Console().CreateLogger();
PropertyUtil.SearchPropertyInTemplate = true;
foreach (var c in new[] { "InstancedStaticMeshComponent", "HierarchicalInstancedStaticMeshComponent", "FoliageInstancedStaticMeshComponent" })
    ObjectTypeRegistry.RegisterClass(c, typeof(F13InstancedMeshComponent)); // Blueprint component instances only store deltas from their templates
var provider = new DefaultFileProvider(new DirectoryInfo(Path.Combine(game, "SummerCamp")), SearchOption.AllDirectories,
    new VersionContainer(EGame.GAME_UE4_18), StringComparer.OrdinalIgnoreCase);
provider.Initialize();
provider.Mount();
Console.WriteLine($"{provider.Files.Count} files");
Collision.LoadProfiles(game);
// The animations' PlaySound notifies (anim_sounds.json); returns their cues for the sound export.
List<string> WriteAnimSounds()
{
    var (json, cues) = AnimSounds.Collect(provider, Mvp.CounselorAnims.Concat(Mvp.KillerAnims));
    File.WriteAllText(Path.Combine(outDir, "anim_sounds.json"), json.ToString(Newtonsoft.Json.Formatting.Indented));
    Console.WriteLine($"anim sounds: {json.Count} animations, {cues.Distinct().Count()} cues");
    return cues;
}
// Simple collision of every exported static mesh, by glTF path (collision.json).
var meshCollision = new SortedDictionary<string, Collision.MeshShapes>(StringComparer.OrdinalIgnoreCase);

var session = new ExportSession();
ClassExporter classes = null!;
var queued = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
// Morph targets are off: CUE4Parse writes empty morph buffers for some F13 face meshes, which glTF loaders reject.
var options = new ExportOptions(meshFormat: EMeshFormat.Gltf2, textureFormat: ETextureFormat.Png, exportMorphTargets: false);
Directory.CreateDirectory(outDir);
classes = new ClassExporter(provider, Queue, o => o is UStaticMesh sm ? OutPath(sm) : new SkinnedAssetExporter((USkeletalMesh)o).SavePath + ".glb");

void Queue(UObject obj)
{
    if (obj is not (UStaticMesh or USkeletalMesh or UMaterialInterface or UTexture) || !queued.Add(obj.GetPathName())) return;
    if (obj is UStaticMesh sm)
    {
        MeshMaterials.RestoreStaticMaterials(sm);
        if (Collision.Shapes(sm) is { } shapes) meshCollision[OutPath(sm)] = shapes;
    }
    session.Add(obj);
}

void ExportAsset(string path)
{
    var pkg = provider.LoadPackage(ToFile(path));
    var n = 0;
    foreach (var e in pkg.GetExports())
    {
        if (e is UStaticMesh or USkeletalMesh) { Queue(e); n++; }
    }
    Console.WriteLine($"{path}: {n} meshes");
}

// `extra`: levels the game loads from code rather than as streaming levels (the weather's).
void ExportLevel(string mapPath, params string[] extra)
{
    var layout = new LevelLayout { Map = mapPath };
    var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
    var pending = new Queue<string>();
    pending.Enqueue(mapPath);
    foreach (var e in extra) pending.Enqueue(e);
    while (pending.Count > 0)
    {
        var path = pending.Dequeue();
        if (!seen.Add(path)) continue;
        UWorld world;
        try { world = (UWorld)provider.LoadPackage(ToFile(path)).GetExports().First(e => e is UWorld); }
        catch (Exception ex) { Console.WriteLine($"  skip {path}: {ex.Message}"); continue; }
        layout.Levels.Add(path);
        foreach (var s in world.StreamingLevels ?? [])
        {
            var ls = s.Load<ULevelStreaming>();
            var asset = ls?.WorldAsset?.AssetPathName.Text;
            if (!string.IsNullOrEmpty(asset)) pending.Enqueue(asset.Split('.')[0]);
        }
        var level = world.PersistentLevel.Load<ULevel>();
        if (level == null) continue;
        var levelName = path.Split('/').Last();
        var components = world.Owner!.GetExports().Where(e => e.Outer != null && e.ExportType.EndsWith("Component"))
            .GroupBy(e => e.Outer!.Name.Text).ToDictionary(g => g.Key, g => g.ToList());
        foreach (var actorRef in level.Actors)
        {
            if (actorRef == null || actorRef.IsNull) continue;
            UObject? actor;
            try { actor = actorRef.Load(); } catch { continue; }
            if (actor == null) continue;
            var root = actor.GetOrDefault<FPackageIndex?>("RootComponent")?.Load<USceneComponent>();
            var info = new ActorInfo
            {
                Level = levelName, Name = actor.Name, Class = actor.ExportType,
                Transform = root != null ? Xf(root.GetAbsoluteTransform()) : null,
            };
            layout.Actors.Add(info);
            try { classes.Add(actor.Class?.Load()); } catch { }
            if (actor.ExportType is not ("StaticMeshActor" or "BlockingVolume"))
            {
                // Gameplay data: the actor's own properties and its non-mesh components (spawners, volumes, ...).
                info.Props = ClassExporter.Sanitize(ClassExporter.PropsOf(actor));
                info.Components = [];
                foreach (var c in components.GetValueOrDefault(actor.Name) ?? [])
                {
                    if (c is UStaticMeshComponent) continue;
                    var j = classes.Describe(c, c.Name, null);
                    if (j == null) continue;
                    if (c is USceneComponent sc) j["World"] = ClassExporter.Xf(sc.GetAbsoluteTransform());
                    info.Components.Add(j);
                }
                if (actor.ExportType == "SCWorldSettings") AddWorldSettingsClasses(info.Props);
                // The ambient cubemap's mean radiance (linear RGB): f13-game lights the scene with it.
                if (actor.ExportType == "PostProcessVolume" && actor.GetOrDefault<FStructFallback?>("Settings") is { } pp
                    && pp.GetOrDefault<FPackageIndex?>("AmbientCubemap")?.Load<UTextureCube>() is { } cube && CubeAverage(cube) is { } avg)
                    info.Props["AmbientCubemapAverage"] = new Newtonsoft.Json.Linq.JArray(avg);
            }
            if (actor.ExportType == "BlockingVolume")
            {
                // Its brush's convex hulls, in world space.
                var brush = root;
                var bs = brush?.GetOrDefault<FPackageIndex?>("BrushBodySetup")?.Load() as CUE4Parse.UE4.Objects.PhysicsEngine.UBodySetup;
                if (brush != null && bs?.AggGeom != null && Collision.BlocksPawn(brush, "BlockAll"))
                {
                    var w = brush.GetAbsoluteTransform();
                    info.Hulls = bs.AggGeom.ConvexElems.Where(c => c.VertexData is { Length: >= 4 })
                        .Select(c => c.VertexData.SelectMany(v => { var p = w.TransformPosition(c.Transform.TransformPosition(v)); return new[] { p.X, p.Y, p.Z }; }).ToArray())
                        .ToList();
                }
            }
            if (actor.ExportType == "NavMeshBoundsVolume")
            {
                var bs = root?.GetOrDefault<FPackageIndex?>("BrushBodySetup")?.Load() as CUE4Parse.UE4.Objects.PhysicsEngine.UBodySetup;
                if (root != null && bs?.AggGeom != null)
                {
                    var w = root.GetAbsoluteTransform();
                    var pts = bs.AggGeom.ConvexElems.SelectMany(c => c.VertexData.Select(v => w.TransformPosition(c.Transform.TransformPosition(v)))).ToList();
                    if (pts.Count > 0)
                        info.Bounds = [pts.Min(p => p.X), pts.Min(p => p.Y), pts.Min(p => p.Z), pts.Max(p => p.X), pts.Max(p => p.Y), pts.Max(p => p.Z)];
                }
            }
            if (actor is ALandscapeProxy proxy)
            {
                var lx = new LandscapeMeshExporter(proxy);
                session.Add(lx);
                layout.Landscapes.Add(new LandscapeInfo { Level = levelName, Actor = actor.Name, File = lx.SavePath + ".glb" });
                // Each paint layer's physical surface (its LayerInfo's PhysMaterial), next to the weight maps
                // (<layer info>_LayerInfo.png): footsteps take the surface of the layer that weighs most.
                var surfaces = new SortedDictionary<string, int>();
                foreach (var ci in proxy.LandscapeComponents)
                    if (ci.Load<CUE4Parse.UE4.Assets.Exports.Component.Landscape.ULandscapeComponent>() is { } lc)
                        foreach (var alloc in lc.GetWeightmapLayerAllocations())
                            if (alloc.LayerInfo.Load<UObject>() is { } li)
                                surfaces[li.Name] = Collision.SurfaceOf(li.GetOrDefault<FPackageIndex?>("PhysMaterial")?.Load<UObject>());
                var sdir = Path.Combine(outDir, lx.SavePath);
                Directory.CreateDirectory(sdir);
                File.WriteAllText(Path.Combine(sdir, "surfaces.json"), System.Text.Json.JsonSerializer.Serialize(surfaces, new JsonSerializerOptions { WriteIndented = true }));
            }
        }
        // Mesh components: every StaticMeshComponent export in the level package (includes Blueprint actor parts).
        foreach (var e in world.Owner!.GetExports())
        {
            if (e is not UStaticMeshComponent smc) continue;
            var mesh = smc.GetOrDefault<FPackageIndex?>("StaticMesh")?.Load<UStaticMesh>();
            // The engine's sky sphere isn't in the game's paks: keep its material's parameters (the sky's colour).
            if (mesh == null && smc.GetOrDefault<FPackageIndex?>("StaticMesh") is { IsNull: false } meshRef && meshRef.Name == "SM_SkySphere"
                && smc.GetOrDefault<FPackageIndex[]>("OverrideMaterials", []).FirstOrDefault(m => m is { IsNull: false })?.Load<UMaterialInstanceConstant>() is { } sky)
            {
                var j = new Newtonsoft.Json.Linq.JObject { ["Material"] = sky.Name };
                foreach (var s in sky.ScalarParameterValues) j[s.Name] = s.ParameterValue;
                foreach (var v in sky.VectorParameterValues)
                    if (v.ParameterValue is { } c) j[v.Name] = new Newtonsoft.Json.Linq.JArray(c.R, c.G, c.B);
                layout.Sky = j;
            }
            if (mesh == null) continue;
            Queue(mesh);
            var p = new Placement
            {
                Level = levelName,
                Component = e.GetPathName(),
                Mesh = mesh.GetPathName(),
                File = OutPath(mesh),
                Hidden = smc.GetOrDefault("bHiddenInGame", false) || !smc.GetOrDefault("bVisible", true),
                Materials = smc.GetOrDefault<FPackageIndex[]>("OverrideMaterials", [])
                    .Select(m => m?.ResolvedObject?.GetPathName()).ToArray(),
                MaterialMap = MeshMaterials.OverrideMap(mesh, smc.GetOrDefault<FPackageIndex[]>("OverrideMaterials", []), Queue),
                BlocksPawn = Collision.BlocksPawn(smc),
                CastShadow = smc.GetOrDefault("CastShadow", true),
                // Foliage instances fade out at InstanceEndCullDistance; other components at their max draw distance
                // (LDMaxDrawDistance, or the cull distance volume's CachedMaxDrawDistance), the nearer one when both are set.
                CullDistance = new[] { smc.GetOrDefault("InstanceEndCullDistance", 0), (int)smc.GetOrDefault("LDMaxDrawDistance", 0f), (int)smc.GetOrDefault("CachedMaxDrawDistance", 0f) }
                    .Where(d => d > 0).DefaultIfEmpty(0).Min(),
            };
            var world_ = AbsoluteTransform(smc);
            if (smc is F13InstancedMeshComponent ism)
            {
                p.Instances = ism.Instances.Select(x => Xf(x.TransformData * world_)).ToList();
            }
            else
            {
                p.Transform = Xf(world_);
            }
            layout.Placements.Add(p);
        }
        Console.WriteLine($"  {path}: {level.Actors.Length} actors");
    }
    var json = Path.Combine(outDir!, "levels", layout.Map.Split('/').Last() + ".json");
    Directory.CreateDirectory(Path.GetDirectoryName(json)!);
    File.WriteAllText(json, Newtonsoft.Json.JsonConvert.SerializeObject(layout));
    Console.WriteLine($"{json}: {layout.Placements.Count} placements, {layout.Actors.Count} actors, {layout.Landscapes.Count} landscapes");
}

// Classes the match-start spawning can create: vehicles and the large/small item lists.
void AddWorldSettingsClasses(Newtonsoft.Json.Linq.JObject ws)
{
    foreach (var name in ws.SelectTokens("VehiclesToSpawn[*].VehicleClasses[*]").Concat(ws.SelectTokens("LargeItemSpawnData[*].ActorClass"))
                 .Concat(ws.SelectTokens("SmallItemSpawnData[*].ItemClass")).Select(t => (string?)t))
        if (name != null) AddClassByName(name);
}

// Loads a Blueprint class by short name ("CarKeys_C") via the package of the same name.
void AddClassByName(string name)
{
    if (!name.EndsWith("_C")) { classes.AddNative(name); return; }
    var pkgName = name[..^2];
    var file = provider.Files.Keys.FirstOrDefault(k => k.EndsWith("/" + pkgName + ".uasset", StringComparison.OrdinalIgnoreCase));
    if (file == null) { Console.WriteLine($"  no package for class {name}"); return; }
    try { classes.Add(provider.LoadPackage(file).GetExports().FirstOrDefault(e => e.Name == name)); }
    catch (Exception ex) { Console.WriteLine($"  class {name}: {ex.Message}"); }
}

// collision.json (see Collision.cs).
// kill_cameras.json: each kill's camera spline (its SCCameraSplineComponent template's SplineCurves: Position, Rotation,
// Scale (the handicam strength), FOVCurve, HoldTimeCurve, TimeToNextCurve as Unreal InterpCurve points; curves left at
// their defaults are absent), by kill class.
void WriteKillCameras()
{
    var all = new Newtonsoft.Json.Linq.JObject();
    foreach (var path in Mvp.KillCameras)
    {
        var name = path.Split('/')[^1];
        try
        {
            var spline = provider.LoadPackage(ToFile(path)).GetExports().FirstOrDefault(e => e.ExportType == "SCCameraSplineComponent");
            var curves = spline == null ? null : Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(spline))["Properties"]?["SplineCurves"];
            if (curves == null)
            {
                Console.WriteLine($"kill camera {name}: no spline");
                continue;
            }
            all[name] = curves;
        }
        catch (Exception e)
        {
            Console.WriteLine($"kill camera {name}: {e.Message}");
        }
    }
    File.WriteAllText(Path.Combine(outDir!, "kill_cameras.json"), all.ToString(Newtonsoft.Json.Formatting.Indented));
    Console.WriteLine($"kill_cameras.json: {all.Count} kills");
}

// water.json: the map's water volumes (volumes with bWaterVolume, e.g. Swim_PhysicsVolume): the actor's name (its
// transform is in the level layout) and its brush's boxes (BrushBodySetup AggGeom convex elements' ElemBox, actor
// space, cm).
void WriteWater()
{
    var all = new Newtonsoft.Json.Linq.JArray();
    foreach (var e in provider.LoadPackage(ToFile(Mvp.Map)).GetExports())
    {
        if (!e.GetOrDefault("bWaterVolume", false)) continue;
        var body = e.GetOrDefault<FPackageIndex?>("BrushComponent")?.Load<UObject>()?.GetOrDefault<FPackageIndex?>("BrushBodySetup")?.Load<UObject>();
        var props = body == null ? null : Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(body))["Properties"];
        var boxes = new Newtonsoft.Json.Linq.JArray();
        foreach (var c in props?["AggGeom"]?["ConvexElems"] ?? new Newtonsoft.Json.Linq.JArray())
            if (c["ElemBox"] is { } b) boxes.Add(new Newtonsoft.Json.Linq.JObject { ["Min"] = b["Min"], ["Max"] = b["Max"] });
        all.Add(new Newtonsoft.Json.Linq.JObject { ["Name"] = e.Name, ["Class"] = e.ExportType, ["FluidFriction"] = e.GetOrDefault("FluidFriction", 0.3f), ["Boxes"] = boxes });
    }
    File.WriteAllText(Path.Combine(outDir!, "water.json"), all.ToString(Newtonsoft.Json.Formatting.Indented));
    Console.WriteLine($"water.json: {all.Count} water volumes");
}

void WriteCollision()
{
    var path = Path.Combine(outDir!, "collision.json");
    File.WriteAllText(path, Newtonsoft.Json.JsonConvert.SerializeObject(meshCollision));
    Console.WriteLine($"{path}: {meshCollision.Count} meshes with collision; profiles used: "
        + string.Join(", ", Collision.Used.Select(kv => $"{kv.Key} {kv.Value}")));
}

void WriteClasses()
{
    foreach (var c in Mvp.Classes) AddClassByName(c);
    classes.Run();
    var path = Path.Combine(outDir!, "classes.json");
    File.WriteAllText(path, Newtonsoft.Json.JsonConvert.SerializeObject(classes.Classes));
    Console.WriteLine($"{path}: {classes.Classes.Count} classes");
}

// Mean colour of an RGBA16F cubemap's top mip (all six faces), or null for another format.
static float[]? CubeAverage(UTextureCube cube)
{
    if (cube.Format != EPixelFormat.PF_FloatRGBA || cube.PlatformData.Mips[0].BulkData?.Data is not { } data) return null;
    var sum = new double[3];
    var n = data.Length / 8;
    for (var i = 0; i < n; i++)
        for (var c = 0; c < 3; c++) sum[c] += (float)BitConverter.ToHalf(data, i * 8 + c * 2);
    return [(float)(sum[0] / n), (float)(sum[1] / n), (float)(sum[2] / n)];
}

string ToFile(string objectPath)
{
    var p = objectPath.Split('.')[0];
    if (p.StartsWith("/Game/")) p = "SummerCamp/Content/" + p["/Game/".Length..];
    return p;
}

// Why --game can't be read, or null. The tool needs the game's content extracted to loose files: the stock pak is
// not a standard Unreal pak (custom footer, protected index), so CUE4Parse can't mount it.
static string? InstallProblem(string game)
{
    var root = Path.Combine(game, "SummerCamp");
    if (!Directory.Exists(root))
        return $"--game: no SummerCamp folder in \"{game}\"; pass the F13Game install folder (the one containing SummerCamp).";
    if (File.Exists(Path.Combine(root, "Content", "Maps", "CrystalLake_Small", "CrystalLake_Small.umap")))
        return null;
    var paks = Path.Combine(root, "Content", "Paks");
    var packed = Directory.Exists(paks) && Directory.EnumerateFiles(paks, "*.pak").Any();
    return packed
        ? $"\"{game}\" is a stock install: its content is still packed in Content/Paks/*.pak, which this tool can't read.\n" +
          "Extract the main pak to loose files under SummerCamp/Content first (the usual modding setup), then run this again."
        : $"\"{game}\" has no extracted game content (SummerCamp/Content/Maps/CrystalLake_Small is missing).";
}

static string OutPath(UStaticMesh mesh) => new StaticMeshExporter(mesh).SavePath + ".glb";

// A component's world transform through its attach parents. CUE4Parse's GetAbsoluteTransform stops at a parent it
// doesn't load as a scene component (the game's PoweredSpotLightComponent: the cabins' light fixtures hang off them),
// so past one the chain is composed here, each relative transform read through the component's archetypes.
static FTransform AbsoluteTransform(USceneComponent c)
{
    var parent = c.GetOrDefault<FPackageIndex?>("AttachParent")?.Load();
    if (parent == null || parent is USceneComponent { } && Chain(parent).All(p => p is USceneComponent)) return c.GetAbsoluteTransform();
    var t = RelativeTransform(c);
    foreach (var p in Chain(parent)) t = t * RelativeTransform(p);
    return t;

    static IEnumerable<UObject> Chain(UObject? p)
    {
        for (var n = 0; p != null && n < 32; p = p.GetOrDefault<FPackageIndex?>("AttachParent")?.Load(), n++) yield return p;
    }
}

static FTransform RelativeTransform(UObject c)
{
    T Get<T>(string name, T fallback)
    {
        for (UObject? a = c; a != null; a = a.Template?.Load())
            if (a.TryGetValue(out T v, name)) return v;
        return fallback;
    }
    return new FTransform(Get("RelativeRotation", FRotator.ZeroRotator).Quaternion(), Get("RelativeLocation", FVector.ZeroVector), Get("RelativeScale3D", FVector.OneVector));
}

static Xform Xf(FTransform t) => new()
{
    T = [t.Translation.X, t.Translation.Y, t.Translation.Z],
    R = [t.Rotation.X, t.Rotation.Y, t.Rotation.Z, t.Rotation.W],
    S = [t.Scale3D.X, t.Scale3D.Y, t.Scale3D.Z],
};

switch (rest[0])
{
    case "find": // find <substring>...: package paths containing every substring
        foreach (var f in provider.Files.Keys.Where(k => rest.Skip(1).All(q => k.Contains(q, StringComparison.OrdinalIgnoreCase))).Order())
            Console.WriteLine(f);
        return 0;
    case "rawtexture": // rawtexture <package> <out file>: a texture's format, size and top mip's bytes (reverse engineering)
    {
        var tex = provider.LoadPackage(ToFile(rest[1])).GetExports().OfType<UTexture>().First();
        var pd = tex.PlatformData;
        var mip = pd.Mips[0];
        var bytes = mip.BulkData?.Data ?? [];
        File.WriteAllBytes(rest[2], bytes);
        Console.WriteLine($"{tex.GetType().Name} {tex.Format} {mip.SizeX}x{mip.SizeY}x{mip.SizeZ} mips {pd.Mips.Length}, {bytes.Length} bytes");
        return 0;
    }
    case "cat": // cat <file path>: a packed file's bytes to stdout (config .ini files)
    {
        using var stdout = Console.OpenStandardOutput();
        stdout.Write(provider.Files[rest[1]].Read());
        return 0;
    }
    case "navmesh": // the level's cooked navmeshes (navmesh-<agent>.json)
        NavMeshExporter.Write(provider, outDir, Mvp.Map);
        return 0;
    case "rawexport": // rawexport <package> <export name> <out file>: an export's serialized bytes (reverse engineering)
    {
        var pkg = (CUE4Parse.UE4.Assets.Package)provider.LoadPackage(ToFile(rest[1]));
        var files = provider.SavePackage(ToFile(rest[1]));
        var bytes = files.OrderBy(f => f.Key.EndsWith(".uexp")).SelectMany(f => f.Value).ToArray();
        var e = pkg.ExportMap.First(x => x.ObjectName.Text == rest[2]);
        File.WriteAllBytes(rest[3], bytes.AsSpan((int)e.SerialOffset, (int)e.SerialSize).ToArray());
        Console.WriteLine($"{rest[2]}: {e.SerialSize} bytes at {e.SerialOffset}; names {pkg.NameMap.Length}, None = {Array.FindIndex(pkg.NameMap, n => n.Name == "None")}");
        return 0;
    }
    case "imports": // imports <package>: the import and export tables (index: name), to read Blueprint bytecode
    {
        var pkg = (CUE4Parse.UE4.Assets.Package)provider.LoadPackage(ToFile(rest[1]));
        for (var i = 0; i < pkg.ImportMap.Length; i++)
            Console.WriteLine($"{-(i + 1)}: {pkg.ImportMap[i].ClassName.Text} {pkg.ImportMap[i].ObjectName.Text}");
        for (var i = 0; i < pkg.ExportMap.Length; i++)
            Console.WriteLine($"{i + 1}: {pkg.ExportMap[i].ClassName} {pkg.ExportMap[i].ObjectName.Text}");
        return 0;
    }
    case "assets":
        foreach (var a in rest.Skip(1)) ExportAsset(a);
        break;
    case "level":
        if (rest[1] == Mvp.Map) ExportLevel(Mvp.Map, Mvp.Weather);
        else ExportLevel(rest[1]);
        break;
    case "levels": // the MVP map and its cabin layouts, and the classes they use (classes.json)
        ExportLevel(Mvp.Map, Mvp.Weather);
        foreach (var l in Mvp.Layouts) ExportLevel(l);
        WriteClasses();
        break;
    case "materials":
        WriteMaterialIndex();
        return 0;
    case "debug-props":
        foreach (var e in provider.LoadPackage(ToFile(rest[1])).GetExports())
        {
            Console.WriteLine($"{e.ExportType} {e.Name}: {string.Join(", ", e.Properties.Select(p => p.Name.Text))}");
            if (e is UStaticMesh sm) Console.WriteLine($"  StaticMaterials(native)={sm.StaticMaterials?.Length.ToString() ?? "null"} Materials={sm.Materials?.Length.ToString() ?? "null"}");
        }
        return 0;
    case "debug-json": // debug-json <package> [substring of export type or name]
        foreach (var e in provider.LoadPackage(ToFile(rest[1])).GetExports())
        {
            if (rest.Count > 2 && !e.ExportType.Contains(rest[2], StringComparison.OrdinalIgnoreCase)
                && !e.Name.Contains(rest[2], StringComparison.OrdinalIgnoreCase)) continue;
            Console.WriteLine(Newtonsoft.Json.JsonConvert.SerializeObject(e, Newtonsoft.Json.Formatting.Indented));
        }
        return 0;
    case "debug-type": // debug-type <package> <export name>: CLR type and members of one export
        {
            var e = provider.LoadPackage(ToFile(rest[1])).GetExports().First(x => x.Name == rest[2]);
            for (var t = e.GetType(); t != null && t != typeof(object); t = t.BaseType)
            {
                Console.WriteLine($"{t.FullName}");
                foreach (var m in t.GetMembers(System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.DeclaredOnly))
                    if (m is System.Reflection.PropertyInfo or System.Reflection.FieldInfo || (m is System.Reflection.MethodInfo mi && !mi.IsSpecialName))
                        Console.WriteLine($"  {m.MemberType} {m}");
            }
        }
        return 0;
    case "debug-clr": // debug-clr <type name>: members of a CUE4Parse type
        foreach (var asm in AppDomain.CurrentDomain.GetAssemblies())
        foreach (var t in asm.GetTypes().Where(t => t.Name == rest[1] || t.FullName == rest[1]))
        {
            Console.WriteLine(t.FullName);
            foreach (var m in t.GetMembers(System.Reflection.BindingFlags.Public | System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.Static | System.Reflection.BindingFlags.DeclaredOnly))
                Console.WriteLine($"  {m.MemberType} {m}");
        }
        return 0;
    case "debug-ism":
        foreach (var e in provider.LoadPackage(ToFile(rest[1])).GetExports().OfType<F13InstancedMeshComponent>().Take(5))
            Console.WriteLine($"{e.ExportType} {e.Name} inst={e.Instances.Count} first={(e.Instances.Count > 0 ? e.Instances[0].TransformData.ToString() : "")}");
        return 0;
    case "mvp":
        foreach (var a in Mvp.Assets) ExportAsset(a);
        ExportLevel(Mvp.Map, Mvp.Weather);
        foreach (var l in Mvp.Layouts) ExportLevel(l);
        WriteClasses();
        NavMeshExporter.Write(provider, outDir, Mvp.Map);
        AnimExporter.Write(provider, outDir, Mvp.Montages);
        new CounselorExporter(provider, outDir, Queue).Export(Mvp.Counselor, Mvp.CounselorAnims, Mvp.KeepRoot);
        new CounselorExporter(provider, outDir, Queue).ExportKiller(Mvp.Killer, Mvp.KillerAnims, Mvp.KeepRoot);
        new CounselorExporter(provider, outDir, Queue).ExportPropAnims(Mvp.TrapMesh, Mvp.TrapAnims);
        WriteKillCameras();
        WriteWater();
        WriteCollision();
        {
            var sx = new SoundExporter(provider, outDir, ToFile);
            sx.Export(sx.FootstepTables().Concat(Mvp.Sounds).Concat(sx.ClassSounds()).Concat(WriteAnimSounds()));
        }
        {
            var wx = new WidgetExporter(provider, outDir, Queue, ToFile);
            foreach (var w in Mvp.Widgets) wx.Export(w);
            wx.ClassTextures();
            wx.Fonts(game!);
        }
        break;
    case "sounds": // the sounds f13-game plays (sounds/): the counselor's footsteps, the classes' cues, the animations' (anim_sounds.json), Mvp.Sounds
    {
        var sx = new SoundExporter(provider, outDir, ToFile);
        var n = sx.Export(sx.FootstepTables().Concat(Mvp.Sounds).Concat(sx.ClassSounds()).Concat(WriteAnimSounds()));
        Console.WriteLine($"sounds: {n} cues");
        return 0;
    }
    case "collision": // levels (for collision flags), classes and collision.json; no meshes or textures
        ExportLevel(Mvp.Map, Mvp.Weather);
        foreach (var l in Mvp.Layouts) ExportLevel(l);
        WriteClasses();
        WriteCollision();
        return 0;
    case "counselor": // the MVP counselor's parts and animations (and the meshes they need)
        new CounselorExporter(provider, outDir, Queue).Export(Mvp.Counselor, Mvp.CounselorAnims, Mvp.KeepRoot);
        break;
    case "ui": // ui [widget...]: widget trees and their textures (default: Mvp.Widgets)
    {
        var wx = new WidgetExporter(provider, outDir, Queue, ToFile);
        var n = 0;
        foreach (var w in rest.Count > 1 ? rest.Skip(1) : Mvp.Widgets) n += wx.Export(w);
        Console.WriteLine($"ui: {n} widgets, {wx.ClassTextures()} class textures, {wx.Fonts(game!)} fonts");
        break;
    }
    case "killer": // the MVP Jason's parts, weapon and animations (and the meshes they need)
        new CounselorExporter(provider, outDir, Queue).ExportKiller(Mvp.Killer, Mvp.KillerAnims, Mvp.KeepRoot);
        break;
    case "water": // the map's water volumes (water.json)
        WriteWater();
        break;
    case "killcams": // the kills' camera splines (kill_cameras.json)
        WriteKillCameras();
        break;
    case "props": // skeletal props' own animations (props/): the bear trap
        new CounselorExporter(provider, outDir, Queue).ExportPropAnims(Mvp.TrapMesh, Mvp.TrapAnims);
        break;
    case "anims":
        AnimExporter.Write(provider, outDir, Mvp.Montages);
        return 0;
    case "dds":
        DdsExporter.WriteAll(provider, outDir);
        return 0;
    default:
        Console.Error.WriteLine($"unknown command {rest[0]}");
        return 2;
}

Console.WriteLine($"exporting {session.TotalQueued} objects...");
var results = await session.RunAsync(outDir, options);
var failed = results.Where(r => !r.Success).ToList();
Console.WriteLine($"done: {results.Count - failed.Count} ok, {failed.Count} failed");
foreach (var f in failed.Take(20)) Console.WriteLine($"  failed {f.ObjectPath}");
WriteMaterialIndex();
DdsExporter.WriteAll(provider, outDir);
return 0;

// materials.json: name -> exported JSON file, effective blend mode and two-sidedness. Material instances usually
// inherit these from a parent material, which the per-material JSON does not reflect.
void WriteMaterialIndex()
{
    var root = Path.GetFullPath(outDir!);
    var index = new SortedDictionary<string, MaterialInfo>(StringComparer.OrdinalIgnoreCase);
    var exported = Path.Combine(root, "SummerCamp");
    if (!Directory.Exists(exported))
        return; // nothing was exported
    foreach (var file in Directory.EnumerateFiles(exported, "*.json", SearchOption.AllDirectories))
    {
        var rel = Path.GetRelativePath(root, file).Replace(Path.DirectorySeparatorChar, '/');
        var pkg = rel[..^".json".Length];
        UObject? mat;
        try { mat = provider.LoadPackage(pkg).GetExports().FirstOrDefault(e => e.Name.Equals(Path.GetFileNameWithoutExtension(file), StringComparison.OrdinalIgnoreCase)); }
        catch { continue; }
        if (mat == null || !mat.ExportType.Contains("Material")) continue;
        string? blend = null;
        bool? twoSided = null;
        for (var m = mat; m != null && (blend == null || twoSided == null); m = m.GetOrDefault<FPackageIndex?>("Parent")?.Load())
        {
            var ov = m.GetOrDefault<CUE4Parse.UE4.Assets.Objects.FStructFallback?>("BasePropertyOverrides");
            if (blend == null && ov != null && ov.GetOrDefault("bOverride_BlendMode", false)) blend = ov.GetOrDefault<CUE4Parse.UE4.Objects.UObject.FName>("BlendMode").Text;
            if (twoSided == null && ov != null && ov.GetOrDefault("bOverride_TwoSided", false)) twoSided = ov.GetOrDefault("TwoSided", false);
            if (m.ExportType == "Material")
            {
                blend ??= m.GetOrDefault<CUE4Parse.UE4.Objects.UObject.FName>("BlendMode").Text;
                twoSided ??= m.GetOrDefault("TwoSided", false);
            }
        }
        blend = string.IsNullOrEmpty(blend) || blend == "None" ? "BLEND_Opaque" : blend.Split(':').Last();
        var info = new MaterialInfo { File = rel, Blend = blend, TwoSided = twoSided ?? false };
        ResolveParameters(mat, info);
        index[mat.Name] = info;
    }
    var path = Path.Combine(root, "materials.json");
    File.WriteAllText(path, JsonSerializer.Serialize(index, new JsonSerializerOptions { IncludeFields = true }));
    Console.WriteLine($"{path}: {index.Count} materials, {index.Values.Count(m => m.Blend != "BLEND_Opaque")} not opaque");
    foreach (var g in index.Values.GroupBy(m => m.Master).OrderByDescending(g => g.Count()).Take(15))
        Console.WriteLine($"  {g.Count(),4} {g.Key}");
}

// The material's master (root UMaterial) and every parameter it ends up with: the nearest instance override wins,
// then the master's parameter defaults. Cooked masters keep their parameter expressions (not the graph wiring), so
// the defaults are still there. Textures are export-relative paths without an extension.
static void ResolveParameters(UObject mat, MaterialInfo info)
{
    static string? TexPath(FPackageIndex? i) => i?.ResolvedObject?.GetPathName() is { } p ? p.Split('.')[0].TrimStart('/') : null;
    static string Key(FStructFallback s) =>
        s.GetOrDefault<FName>("ParameterName").Text is { Length: > 0 } n && n != "None" ? n
            : s.GetOrDefault<FStructFallback?>("ParameterInfo")?.GetOrDefault<FName>("Name").Text ?? "";
    for (var m = mat; m != null; m = m.GetOrDefault<FPackageIndex?>("Parent")?.Load())
    {
        if (m.ExportType == "Material")
        {
            info.Master = m.Name;
            foreach (var e in m.GetOrDefault<FPackageIndex[]>("Expressions", []).Select(x => x.Load()).OfType<UObject>())
            {
                var name = e.GetOrDefault<FName>("ParameterName").Text;
                if (string.IsNullOrEmpty(name) || name == "None") continue;
                switch (e.ExportType)
                {
                    case "MaterialExpressionScalarParameter":
                        info.Scalars.TryAdd(name, e.GetOrDefault("DefaultValue", 0f));
                        break;
                    case "MaterialExpressionVectorParameter":
                        var c = e.GetOrDefault("DefaultValue", new FLinearColor(0, 0, 0, 0));
                        info.Vectors.TryAdd(name, [c.R, c.G, c.B, c.A]);
                        break;
                    case "MaterialExpressionStaticSwitchParameter":
                        info.Switches.TryAdd(name, e.GetOrDefault("DefaultValue", false));
                        break;
                    case var t when t.StartsWith("MaterialExpressionTextureSampleParameter") || t == "MaterialExpressionTextureObjectParameter":
                        if (TexPath(e.GetOrDefault<FPackageIndex?>("Texture")) is { } tp) info.Textures.TryAdd(name, tp);
                        break;
                }
            }
            info.Landscape = LandscapeLayers(m);
            break;
        }
        foreach (var s in m.GetOrDefault<FStructFallback[]>("ScalarParameterValues", []))
            info.Scalars.TryAdd(Key(s), s.GetOrDefault("ParameterValue", 0f));
        foreach (var s in m.GetOrDefault<FStructFallback[]>("VectorParameterValues", []))
        {
            var c = s.GetOrDefault("ParameterValue", new FLinearColor(0, 0, 0, 0));
            info.Vectors.TryAdd(Key(s), [c.R, c.G, c.B, c.A]);
        }
        foreach (var s in m.GetOrDefault<FStructFallback[]>("TextureParameterValues", []))
            if (TexPath(s.GetOrDefault<FPackageIndex?>("ParameterValue")) is { } tp) info.Textures.TryAdd(Key(s), tp);
        // Static switches live in the instance's StaticParameters (native data CUE4Parse does not expose by name).
    }
}

// A landscape material's layers. Cooking strips most of the graph, but the LandscapeLayerBlend and TextureSample
// expressions survive with their inputs named: the blend feeding BaseColor gives each layer's diffuse (alpha =
// height), the one feeding Normal its normal map, and any other texture a layer samples is its RMA map.
static LandscapeMaterial? LandscapeLayers(UObject master)
{
    var exprs = master.GetOrDefault<FPackageIndex[]>("Expressions", []).Select(x => x.Load()).OfType<UObject>()
        .ToDictionary(e => e.Name, StringComparer.OrdinalIgnoreCase);
    var blends = exprs.Values.Where(e => e.ExportType == "MaterialExpressionLandscapeLayerBlend").ToList();
    if (blends.Count == 0) return null;
    static string? Input(FExpressionInput? i) => i?.ExpressionName.Text is { Length: > 0 } n && n != "None" ? n : null;
    string? Texture(string? expr) => expr != null && exprs.TryGetValue(expr, out var e) && e.ExportType == "MaterialExpressionTextureSample"
        ? e.GetOrDefault<FPackageIndex?>("Texture")?.ResolvedObject?.GetPathName().Split('.')[0].TrimStart('/') : null;
    var baseColor = Input(master.GetOrDefault<FExpressionInput?>("BaseColor"));
    var normal = Input(master.GetOrDefault<FExpressionInput?>("Normal"));
    var result = new LandscapeMaterial
    {
        MappingScale = exprs.Values.FirstOrDefault(e => e.ExportType == "MaterialExpressionLandscapeLayerCoords")?.GetOrDefault("MappingScale", 1f) ?? 1f,
    };
    var byName = new Dictionary<string, LandscapeLayer>();
    foreach (var b in blends)
    {
        foreach (var l in b.GetOrDefault<FStructFallback[]>("Layers", []))
        {
            var name = l.GetOrDefault<FName>("LayerName").Text;
            if (!byName.TryGetValue(name, out var layer))
            {
                layer = new LandscapeLayer { Name = name };
                byName[name] = layer;
                result.Layers.Add(layer);
            }
            var tex = Texture(Input(l.GetOrDefault<FExpressionInput?>("LayerInput")));
            if (tex == null) continue;
            if (b.Name == baseColor) layer.Diffuse = tex;
            else if (b.Name == normal) layer.Normal = tex;
            else layer.Rma ??= tex;
        }
    }
    return result;
}

static class Mvp
{
    public const string Map = "/Game/Maps/CrystalLake_Small/CrystalLake_Small";
    // The weather level (moon, height fog, sky); the game loads it, or CLsm_RainOn, from code. Clear for now.
    public const string Weather = "/Game/Maps/CrystalLake_Small/CLsm_RainOff";
    // Alternative cabin/spawn layouts; the game streams in one of these per match.
    public static readonly string[] Layouts = [.. Enumerable.Range(1, 4).Select(i => $"/Game/Maps/CrystalLake_Small/CLsm_Design_SL{i}")];
    public static readonly string[] Assets =
    [
        "/Game/Characters/Counselors/Athlete/Meshes/Body/Athlete_Body",
        "/Game/Characters/Counselors/Athlete/Meshes/Body/Athlete_Face",
        "/Game/Characters/Counselors/Athlete/Meshes/Body/Athlete_Hair_SM",
        "/Game/Characters/Killers/Jason/J2/Mesh/Skeletal/Jason_SackHead",
        "/Game/Characters/Killers/Jason/J2/Mesh/Skeletal/J2_Mask",
        "/Game/Characters/Killers/Jason/J2/Mesh/Skeletal/j2_Hair",
        "/Game/Animations/Props/Manmade/Vehicles/A_Vehicle_Chevette_LOD0",
        "/Game/Props/Manmade/Vehicles/Vehicle_battery_01",
        "/Game/Props/Manmade/LivingRoom/S_LivingRoom_GasCan_Red_01",
        "/Game/Props/Pickups/S_Pickup_Vehicle_Keys_01",
    ];
    // Classes the game spawns by reference from native code rather than from level data.
    // Classes no level places but the game reads (the cars' wheels: SCWheeledVehicleMovementComponent WheelSetups).
    public static readonly string[] Classes = ["CarKeys_C", "Chevette_FrontWheel_C", "Chevette_BackWheel_C"];
    // Montages whose length or root motion the rules use (anims.json).
    const string Shared = "/Game/Characters/Counselors/Base/Shared";
    public static readonly string[] Montages =
    [
        $"{Shared}/Combat/Dodge/AM_combat_stand_Dodge_Left",
        $"{Shared}/Combat/Dodge/AM_combat_stand_Dodge_Right",
        $"{Shared}/Combat/Dodge/AM_combat_stand_dodge_back",
        $"{Shared}/Stumble/AM_normal_stand_run_stumble_short",
        $"{Shared}/Stumble/AM_normal_stand_sprint_stumble_short",
        // Jason's rage door burst carries him through the doorway.
        "/Game/Characters/Killers/Jason/Base/Animation/Abilities/AM_Jason_uber_stand_wall_smash_02",
    ];
    public const string Counselor = "/Game/Characters/Counselors/Athlete/Athlete_Counselor";
    // Widgets f13-game draws: the counselor's HUD (with the widgets it nests) and the repair and struggle minigames.
    public static readonly string[] Widgets =
    [
        "/Game/UI/HUD/Counselor/CounselorHUDWidget",
        // Created at runtime: an inventory slot, the breath bar while hiding.
        "/Game/UI/HUD/Counselor/SC_InventoryItem_Widget",
        "/Game/UI/HUD/Counselor/SCBreathWidget",
        "/Game/UI/Interact/SCInteractMinigame_Widget",
        "/Game/UI/HUD/SC_WiggleMinigame_Widget",
    ];
    // Sounds f13-game plays besides the footsteps (SoundCues or SoundWaves).
    public static readonly string[] Sounds = [];
    // Animations f13-game plays on the counselor: Athlete_AnimBP's female overrides of Base_Counselor_AnimBP's
    // locomotion (BS_F_Normal_Move: idle, walk, run, sprint; BS_F_Combat strafing), crouch, wounded idle, and the
    // montages the rules use.
    const string Female = "/Game/Characters/Counselors/Base/Female/Animation";
    public static readonly string[] CounselorAnims =
    [
        $"{Female}/F_normal_stand_idle_01",
        $"{Female}/F_hurt_stand_idle_01",
        $"{Female}/Locomotion/F_normal_stand_walk_forward",
        $"{Female}/Locomotion/F_normal_stand_run_forward",
        $"{Female}/Locomotion/F_normal_stand_sprint_forward",
        // Swimming (idle, slow, fast, sprint).
        $"{Female}/Locomotion/Swimming/f_normal_swim_idle",
        $"{Female}/Locomotion/Swimming/f_normal_swim_slow",
        $"{Female}/Locomotion/Swimming/f_normal_swim_fast",
        $"{Female}/Locomotion/Swimming/f_normal_swim_sprint",
        $"{Female}/Locomotion/F_normal_stand_walk_back",
        $"{Female}/Locomotion/F_normal_stand_walk_left",
        $"{Female}/Locomotion/F_normal_stand_walk_right",
        $"{Female}/Locomotion/F_normal_stand_walk_forward_45_left",
        $"{Female}/Locomotion/F_normal_stand_walk_forward_45_right",
        $"{Female}/Locomotion/F_normal_stand_walk_back_45_left",
        $"{Female}/Locomotion/F_normal_stand_walk_back_45_right",
        $"{Female}/Locomotion/Crouch/f_normal_crouch_idle_01",
        $"{Female}/Locomotion/Crouch/f_normal_crouch_walk_forward",
        $"{Female}/Combat/f_combat_stand_idle_1h",
        $"{Female}/Combat/Dodge/f_combat_stand_dodge_left_01",
        $"{Female}/Combat/Dodge/f_combat_stand_dodge_right_01",
        $"{Female}/Combat/Dodge/f_combat_stand_dodge_back_01",
        $"{Female}/Locomotion/Stumbles/f_normal_stand_run_stumble_short_01",
        $"{Female}/Locomotion/Stumbles/f_normal_stand_sprint_stumble_short_01",
        $"{Female}/Interaction/Items/f_normal_stand_use_firstaid_self",
        // Jason's hits, grab and kill.
        $"{Female}/Death-Hit-Stun/Hits/f_normal_stand_hitreaction_back_01",
        $"{Female}/Death-Hit-Stun/Hits/f_normal_stand_hitreaction_left_01",
        $"{Female}/Death-Hit-Stun/Hits/f_normal_stand_hitreaction_right_01",
        $"{Female}/Death-Hit-Stun/Deaths/f_normal_stand_death_01",
        $"{Female}/Grab/f_normal_grab_enter",
        $"{Female}/Grab/f_normal_grab_idle",
        $"{Female}/Grab/f_normal_grab_struggle",
        // Blocking: the block idle and the reaction when a blocked hit lands.
        $"{Female}/Combat/Block/AM_F_Combat_Stand_Block_Idle",
        $"{Female}/Combat/Block/AM_F_Combat_Stand_Block_React",
        // Caught in a bear trap (the trap's CounselorLeft/RightFootCaught(Sprinting) montages, female): stepping in walking
        // or running, the struggle's cycle, getting free.
        $"{Female}/Interaction/Traps/f_normal_stand_trap_walk_enter_lf",
        $"{Female}/Interaction/Traps/f_normal_stand_trap_walk_enter_rf",
        $"{Female}/Interaction/Traps/f_normal_stand_trap_run_enter_lf",
        $"{Female}/Interaction/Traps/f_normal_stand_trap_run_enter_rf",
        $"{Female}/Interaction/Traps/f_normal_stand_trap_cycle_lf",
        $"{Female}/Interaction/Traps/f_normal_stand_trap_cycle_rf",
        $"{Female}/Interaction/Traps/f_normal_stand_trap_exit_lf",
        $"{Female}/Interaction/Traps/f_normal_stand_trap_exit_rf",
        // Items: weapon swings (each weapon's female NeutralAttack and ForwardAttack), throwing a firecracker, firing
        // the flare gun and the shotgun, placing a bear trap, breaking a grab (struggle, pocket knife).
        $"{Female}/Combat/Swings/AM_F_combat_stand_1h_attack_light",
        $"{Female}/Combat/Swings/AM_F_combat_stand_1h_attack_heavy",
        $"{Female}/Combat/Swings/AM_F_combat_stand_2h_axe_attack_light",
        $"{Female}/Combat/Swings/AM_F_combat_stand_2h_axe_attack_heavy",
        $"{Shared}/Interaction/Items/AM_normal_stand_use_firecracker",
        $"{Shared}/Interaction/Items/AM_normal_stand_use_flare",
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/AM_combat_stand_rifle_fire_01",
        $"{Shared}/Interaction/Trap/AM_normal_stand_trap_place",
        // Doors: her open and close takes, and the shared montages (their OpenCloseDoor driver windows time the leaf).
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Doors/f_normal_stand_door_open_inside_forward",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Doors/f_normal_stand_door_open_outside_forward",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Doors/f_normal_stand_door_close_inside_forward",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Doors/f_normal_stand_door_close_outside_forward",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Doors/f_normal_stand_door_lock",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Doors/f_normal_stand_door_unlock",
        // Starting off and stopping, forward (BS_F_Start_Normal_*Hemisphere at Direction 0, BS_F_Normal_Stop_*).
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Start/Walk/f_normal_stand_walk_forward_start_lf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Start/Walk/f_normal_stand_walk_forward_start_rf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Start/Run/f_normal_stand_run_forward_start_lf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Start/Run/f_normal_stand_run_forward_start_rf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Start/Sprint/f_normal_stand_sprint_forward_start_lf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Start/Sprint/f_normal_stand_sprint_forward_start_rf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Stop/Walk/f_normal_stand_walk_forward_stop_lf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Stop/Walk/f_normal_stand_walk_forward_stop_rf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Stop/Run/f_normal_stand_run_forward_stop_lf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Stop/Run/f_normal_stand_run_forward_stop_rf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Stop/Sprint/f_normal_stand_sprint_forward_stop_lf",
        "/Game/Characters/Counselors/Base/Female/Animation/Locomotion/Stop/Sprint/f_normal_stand_sprint_forward_stop_rf",
        // In the car: sitting (driving, steering, idle), getting in and out on the driver's side.
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_drive_forward",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_drive_forward_turnleft",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_drive_forward_turnright",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_ride_idle",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_enter_left",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_exit_left",
        // Car repairs (the repair components' CounselorInteractAnim): her takes of each montage's sections.
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Car/f_normal_stand_car_repair_gascan_start",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Car/f_normal_stand_car_repair_gascan_cycle",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Car/f_normal_stand_car_repair_gascan_stop",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Car/f_normal_stand_car_repair_gascan_cancel",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_repair_battery_start",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_repair_battery_cycle",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_repair_battery_stop",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_repair_battery_cancel",
        $"{Shared}/Interaction/Doors/AM_OpenDoorInside",
        $"{Shared}/Interaction/Doors/AM_OpenDoorOutside",
        $"{Shared}/Interaction/Doors/AM_CloseDoorInsideForward",
        $"{Shared}/Interaction/Doors/AM_CloseDoorOutsideForward",
        // Windows: her takes (open, close, climb through, dive) and the shared montages (their driver windows).
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Windows/f_normal_stand_window_up",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Windows/f_normal_stand_window_down",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Windows/f_normal_stand_window_climb_01",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Windows/f_normal_stand_window_dive",
        $"{Shared}/Interaction/Window/AM_normal_stand_window_up",
        $"{Shared}/Interaction/Window/AM_normal_stand_window_down",
        $"{Shared}/Interaction/Window/AM_normal_stand_window_climb",
        $"{Shared}/Interaction/Window/AM_normal_stand_window_jump",
        $"{Shared}/Grab/AM_Grab_Break",
        $"{Shared}/Grab/AM_normal_grab_pocketknife_break",
        .. CounselorKills,
        .. CounselorHiding,
        .. Montages,
    ];
    // Hiding spots: the sections of each enter special move's montage (AM_normal_stand_hidespot_closet_enter: enter,
    // HideIdle, HideExit; the outhouse reuses the closet's idle and exit) and the counselor's side of the pickaxe's
    // hiding spot kills (Counselor_Axe_Closet_CK, Counselor_Axe_Bed_CK) and the tent's Counselor_SleepingBagKill.
    // They keep their root motion: the game plays them from the special move's spot.
    const string Hide = $"{Female}/Interaction/Hidespots";
    public static string[] CounselorHiding =>
    [
        $"{Hide}/f_normal_stand_hidespot_closet_enter",
        $"{Hide}/f_normal_stand_hidespot_closet_idle",
        $"{Hide}/f_normal_stand_hidespot_closet_exit",
        $"{Hide}/f_normal_stand_hidespot_outhouse_enter",
        $"{Hide}/f_normal_stand_hidespot_bed_left_enter",
        $"{Hide}/f_normal_stand_hidespot_bed_left_exit",
        $"{Hide}/f_normal_stand_hidespot_bed_right_enter",
        $"{Hide}/f_normal_stand_hidespot_bed_right_exit",
        $"{Hide}/f_normal_stand_hidespot_bed_idle",
        $"{Hide}/f_normal_stand_hidespot_tent_enter",
        $"{Hide}/f_normal_stand_hidespot_tent_loop",
        $"{Hide}/f_normal_stand_hidespot_tent_exit",
        $"{Shared}/Cinematic/AM_cinematic_hidespot_closet_kill_Axe",
        $"{Shared}/Cinematic/AM_cinematic_hidespot_bed_kill_Axe",
        $"{Shared}/Cinematic/AM_cinematic_hidespot_hand_Sleepingbag_kill",
    ];
    // Jason's side: the pickaxe's search moves (Jason_SearchCloset_Axe, Jason_SearchBed_Axe; the tent's search maps the
    // pickaxe to the closet search) and context kills (Jason_ClosetAxe_CK, Jason_BedAxe_CK, Jason_SleepingbagKill).
    const string JasonHide = $"{JasonAnim}/Interaction/Hidespot";
    public static string[] JasonHiding =>
    [
        $"{JasonHide}/AM_cinematic_hidespot_axe_closet_search",
        $"{JasonHide}/AM_cinematic_hidespot_axe_bed_search",
        $"{JasonHide}/AM_cinematic_hidespot_axe_closet_kill",
        $"{JasonHide}/AM_cinematic_hidespot_axe_bed_kill",
        $"{JasonAnim}/Cinematic/AM_cinematic_hidespot_hand_sleepingbag_kill",
    ];
    // The pickaxe's grab kills (its DefaultGrabKill Pickaxe_ThroatHack_GrabKill, and the others): each plays a paired
    // Jason and counselor montage around one origin.
    static string[] PickaxeKills => ["pickaxe_throathack", "pickaxe_head_canopener", "pickaxe_curbstomp", "pickaxe_sweeptheleg", "pickaxe_toppopper"];
    static string[] CounselorKills => [.. PickaxeKills.Select(k => $"{Shared}/Cinematic/AM_cinematic_{k}")];
    const string JasonAnim = "/Game/Characters/Killers/Jason/Base/Animation";
    static string[] JasonKills => [.. PickaxeKills.Select(k => $"{JasonAnim}/Cinematic/AM_cinematic_{k}")];
    const string JasonStun = $"{JasonAnim}/Deaths_HitReacts_Stuns";
    public static string[] PairedKills => [.. CounselorKills, .. JasonKills];
    /// Getting in and out of the driver's seat (played from the seat's socket).
    static string[] CarSeat => ["/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_enter_left", "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_exit_left"];
    /// Animations that keep their root translation (played from a fixed spot: paired kills, hiding spots, car seats).
    public static string[] KeepRoot => [.. PairedKills, .. CounselorHiding, .. JasonHiding, .. CarSeat];
    public const string Killer = "/Game/Characters/Killers/Jason/J2/Jason_SackHead";
    /// The kills whose cameras f13-game plays (kill_cameras.json).
    public static readonly string[] KillCameras =
    [
        "/Game/Blueprints/ContextKills/Pickaxe_ThroatHack_GrabKill",
        "/Game/Blueprints/ContextKills/Pickaxe_Canopener_GrabKill",
        "/Game/Blueprints/ContextKills/PickAxe_CurbStomp_GrabKill",
        "/Game/Blueprints/ContextKills/PickAxe_SweepTheLeg_GrabKill",
        "/Game/Blueprints/ContextKills/PickAxe_TopPopper_GrabKill",
    ];
    // Jason_AnimBP's BS_Jason_Normal_Move (SpeedState 0 idle, 1 walk, 2 fast walk, 3 run), the pickaxe's combat idle,
    // attacks (PickAxe's NeutralAttack and ForwardAttack), the grab, and the grab kill.
    /// The bear trap's own animations (BearTrap_C ArmedAnimation, TriggeredAnimation, EscapedAnimation; its idles).
    public const string TrapMesh = "/Game/Props/SkeletalMeshes/Traps/A_Trap_Bear_01";
    public static readonly string[] TrapAnims =
    [
        "/Game/Props/SkeletalMeshes/Traps/Animation/A_Trap_Bear_01_Open_M",
        "/Game/Props/SkeletalMeshes/Traps/Animation/A_Trap_Bear_01_Enter_01_Anim",
        "/Game/Props/SkeletalMeshes/Traps/Animation/A_Trap_Bear_01_Exit_01_Anim",
        "/Game/Props/SkeletalMeshes/Traps/Animation/A_Trap_Bear_01_Idle_01_Anim",
        "/Game/Props/SkeletalMeshes/Traps/Animation/A_Trap_Bear_01_Close_Idle_01",
    ];

    public static readonly string[] KillerAnims =
    [
        $"{JasonAnim}/normal_stand_idle_01",
        $"{JasonAnim}/Locomotion/normal_stand_walk_forward",
        $"{JasonAnim}/Locomotion/normal_stand_walk_forward_fast",
        $"{JasonAnim}/Locomotion/normal_stand_run_forward_02",
        $"{JasonAnim}/Combat/Axe/combat_stand_idle_2h",
        $"{JasonAnim}/Combat/Axe/AM_combat_stand_2h_attack_light",
        $"{JasonAnim}/Combat/Axe/AM_combat_stand_2h_attack_light_impact",
        $"{JasonAnim}/Combat/Axe/AM_combat_stand_2h_attack_heavy",
        $"{JasonAnim}/Grab/normal_grab_startup",
        $"{JasonAnim}/Grab/normal_grab_enter_1",
        $"{JasonAnim}/Grab/normal_grab_idle",
        $"{JasonAnim}/Grab/normal_grab_miss",
        $"{JasonAnim}/Grab/AM_Jason_normal_grab_break",
        // Stuns (BaseWeaponStun_C / FlareWeaponStun_C, FirecrackerKillerStun_C, the pocket knife, bear traps) and
        // hit reactions.
        $"{JasonStun}/Stun/AM_combat_Falling_stun",
        $"{JasonStun}/Stun/AM_combat_stand_stun_left_01",
        $"{JasonStun}/Stun/AM_combat_stand_stun_right_01",
        $"{JasonStun}/Stun/AM_combat_stand_stun_back_01",
        $"{JasonStun}/Stun/AM_combat_stand_stunned_firecracker",
        $"{JasonStun}/Stun/AM_normal_stand_pocketKnife_Stun",
        $"{JasonStun}/HitReaction/AM_combat_stand_hit_reaction_front",
        $"{JasonStun}/HitReaction/AM_combat_stand_hit_reaction_back",
        $"{JasonAnim}/Interaction/Beartrap/AM_Jason_BearTrap_LF",
        // His kit: throwing a knife (ThrowKnife) and setting a trap (PlaceTrapMontage).
        $"{JasonAnim}/Interaction/AM_normal_stand_knife_throw",
        $"{JasonAnim}/Interaction/AM_normal_stand_trap_set",
        // Doors: Jason_OpenDoorInside / Jason_OpenDoorOutside, and the pickaxe's break down (Jason_BreakDoor_Axe:
        // Enter, Loop, Success, Abort sections).
        $"{JasonAnim}/Interaction/Doors/AM_normal_stand_door_open_inside",
        $"{JasonAnim}/Interaction/Doors/AM_normal_stand_door_open_outside",
        $"{JasonAnim}/Interaction/Doors/AM_normal_stand_door_break_axe",
        // Destroying a fuse box (Jason_Generator_Destruction_Machete: his weapon isn't the axe).
        $"{JasonAnim}/Interaction/AM_Break_Electronic_Box_Machete",
        // Raged, he bursts through a closed door instead (BurstDoorFrom*: Jason_WallSmash).
        $"{JasonAnim}/Abilities/AM_Jason_uber_stand_wall_smash_02",
        .. JasonKills,
        .. JasonHiding,
    ];
}

static class MeshMaterials
{
    // Some F13 meshes (mostly foliage) carry extra native data before StaticMaterials, so CUE4Parse loses the material
    // list after the render data has already loaded fine. The same list is also a tagged property; use that.
    public static void RestoreStaticMaterials(UStaticMesh sm)
    {
        if (sm.StaticMaterials.Length > 0) return;
        var fromTags = sm.GetOrDefault("StaticMaterials", Array.Empty<FStaticMaterial>());
        if (fromTags.Length == 0) return;
        typeof(UStaticMesh).GetProperty(nameof(UStaticMesh.StaticMaterials))!.SetValue(sm, fromTags);
        typeof(UStaticMesh).GetProperty(nameof(UStaticMesh.Materials))!.SetValue(sm, fromTags.Select(m => m.MaterialInterface).ToArray());
    }

    // Material overrides by the name of the slot material they replace (the glTF names its materials after the mesh's
    // own), queueing each override for export. Null when nothing is overridden.
    public static Dictionary<string, string>? OverrideMap(UStaticMesh mesh, FPackageIndex[] overrides, Action<UObject> queue)
    {
        RestoreStaticMaterials(mesh);
        Dictionary<string, string>? map = null;
        for (var i = 0; i < overrides.Length && i < mesh.StaticMaterials.Length; i++)
        {
            if (overrides[i]?.Load() is not UMaterialInterface m) continue;
            var slot = mesh.StaticMaterials[i].MaterialInterface?.Name;
            if (slot == null || slot == m.Name) continue;
            queue(m);
            (map ??= new())[slot] = m.Name;
        }
        return map;
    }
}

class MaterialInfo
{
    public string File = "", Blend = "", Master = "";
    public bool TwoSided;
    public SortedDictionary<string, float> Scalars = new();
    public SortedDictionary<string, float[]> Vectors = new();
    public SortedDictionary<string, bool> Switches = new();
    public SortedDictionary<string, string> Textures = new();
    [System.Text.Json.Serialization.JsonIgnore(Condition = System.Text.Json.Serialization.JsonIgnoreCondition.WhenWritingNull)]
    public LandscapeMaterial? Landscape;
}
class LandscapeMaterial { public float MappingScale = 1; public List<LandscapeLayer> Layers = []; }
class LandscapeLayer { public string Name = ""; public string? Diffuse, Normal, Rma; }
class Xform { public float[] T = []; public float[] R = []; public float[] S = []; }
class Placement
{
    public string Level = "", Component = "", Mesh = "", File = "";
    public bool Hidden;
    public string?[] Materials = [];
    public Dictionary<string, string>? MaterialMap;
    public Xform? Transform;
    public List<Xform>? Instances;
    /// Blocks counselors (its collision profile or custom responses); its shapes are in collision.json.
    public bool BlocksPawn = true;
    /// Casts shadows (CastShadow; baked ones included, which f13-game's dynamic sun stands in for).
    public bool CastShadow = true;
    /// Not drawn past this distance from the camera (cm; 0 = always drawn).
    public int CullDistance;
}
class ActorInfo
{
    public string Level = "", Name = "", Class = "";
    public Xform? Transform;
    public Newtonsoft.Json.Linq.JObject? Props;
    public List<Newtonsoft.Json.Linq.JObject>? Components;
    /// BlockingVolumes: their convex hulls as world-space point sets (cm).
    public List<float[]>? Hulls;
    /// NavMeshBoundsVolumes: the world-space box their brush covers (cm: min x, y, z, max x, y, z).
    public float[]? Bounds;
}
class LandscapeInfo { public string Level = "", Actor = "", File = ""; }
class LevelLayout
{
    public int Format = 1;
    public string Map = "";
    public string Units = "unreal: centimetres, X forward, Y right, Z up; rotations are quaternions (x,y,z,w)";
    public List<string> Levels = [];
    public List<Placement> Placements = [];
    public List<ActorInfo> Actors = [];
    public List<LandscapeInfo> Landscapes = [];
    /// The sky sphere's material parameters (null when the level has none).
    public Newtonsoft.Json.Linq.JObject? Sky;
}

/// UE 4.18 as shipped by F13 serializes 96-byte instance records (transform matrix + 32 bytes of lightmap/shadowmap
/// data); stock CUE4Parse expects 64 for 4.18 and drops the whole component. Only the transform is kept here.
class F13InstancedMeshComponent : UStaticMeshComponent
{
    public List<FInstancedStaticMeshInstanceData> Instances = [];

    public override void Deserialize(FAssetArchive Ar, long validPos)
    {
        base.Deserialize(Ar, validPos);
        var elementSize = Ar.Read<int>();
        var count = Ar.Read<int>();
        if (elementSize < 64 || count < 0 || Ar.Position + (long)elementSize * count > validPos)
            throw new InvalidDataException($"unexpected instance array: size {elementSize}, count {count}");
        for (var i = 0; i < count; i++)
        {
            var start = Ar.Position;
            Instances.Add(new FInstancedStaticMeshInstanceData(new FMatrix(Ar)));
            Ar.Position = start + elementSize;
        }
        Ar.Position = validPos; // the rest (HISM cluster tree etc.) is not needed
    }
}
