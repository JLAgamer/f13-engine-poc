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

Serilog.Log.Logger = new Serilog.LoggerConfiguration().MinimumLevel.Warning().WriteTo.Console().CreateLogger();
PropertyUtil.SearchPropertyInTemplate = true;
foreach (var c in new[] { "InstancedStaticMeshComponent", "HierarchicalInstancedStaticMeshComponent", "FoliageInstancedStaticMeshComponent" })
    ObjectTypeRegistry.RegisterClass(c, typeof(F13InstancedMeshComponent)); // Blueprint component instances only store deltas from their templates
var provider = new DefaultFileProvider(new DirectoryInfo(Path.Combine(game, "SummerCamp")), SearchOption.AllDirectories,
    new VersionContainer(EGame.GAME_UE4_18), StringComparer.OrdinalIgnoreCase);
provider.Initialize();
provider.Mount();
Console.WriteLine($"{provider.Files.Count} files");

var session = new ExportSession();
ClassExporter classes = null!;
var queued = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
// Morph targets are off: CUE4Parse writes empty morph buffers for some F13 face meshes, which glTF loaders reject.
var options = new ExportOptions(meshFormat: EMeshFormat.Gltf2, textureFormat: ETextureFormat.Png, exportMorphTargets: false);
Directory.CreateDirectory(outDir);
classes = new ClassExporter(provider, Queue, o => o is UStaticMesh sm ? OutPath(sm) : new SkinnedAssetExporter((USkeletalMesh)o).SavePath + ".glb");

void Queue(UObject obj)
{
    if (obj is not (UStaticMesh or USkeletalMesh) || !queued.Add(obj.GetPathName())) return;
    if (obj is UStaticMesh sm) RestoreStaticMaterials(sm);
    session.Add(obj);
}

// Some F13 meshes (mostly foliage) carry extra native data before StaticMaterials, so CUE4Parse loses the material
// list after the render data has already loaded fine. The same list is also a tagged property; use that.
static void RestoreStaticMaterials(UStaticMesh sm)
{
    if (sm.StaticMaterials.Length > 0) return;
    var fromTags = sm.GetOrDefault("StaticMaterials", Array.Empty<FStaticMaterial>());
    if (fromTags.Length == 0) return;
    typeof(UStaticMesh).GetProperty(nameof(UStaticMesh.StaticMaterials))!.SetValue(sm, fromTags);
    typeof(UStaticMesh).GetProperty(nameof(UStaticMesh.Materials))!.SetValue(sm, fromTags.Select(m => m.MaterialInterface).ToArray());
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

void ExportLevel(string mapPath)
{
    var layout = new LevelLayout { Map = mapPath };
    var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
    var pending = new Queue<string>();
    pending.Enqueue(mapPath);
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
            }
            if (actor is ALandscapeProxy proxy)
            {
                var lx = new LandscapeMeshExporter(proxy);
                session.Add(lx);
                layout.Landscapes.Add(new LandscapeInfo { Level = levelName, Actor = actor.Name, File = lx.SavePath + ".glb" });
            }
        }
        // Mesh components: every StaticMeshComponent export in the level package (includes Blueprint actor parts).
        foreach (var e in world.Owner!.GetExports())
        {
            if (e is not UStaticMeshComponent smc) continue;
            var mesh = smc.GetOrDefault<FPackageIndex?>("StaticMesh")?.Load<UStaticMesh>();
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
            };
            var world_ = smc.GetAbsoluteTransform();
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

void WriteClasses()
{
    foreach (var c in Mvp.Classes) AddClassByName(c);
    classes.Run();
    var path = Path.Combine(outDir!, "classes.json");
    File.WriteAllText(path, Newtonsoft.Json.JsonConvert.SerializeObject(classes.Classes));
    Console.WriteLine($"{path}: {classes.Classes.Count} classes");
}

string ToFile(string objectPath)
{
    var p = objectPath.Split('.')[0];
    if (p.StartsWith("/Game/")) p = "SummerCamp/Content/" + p["/Game/".Length..];
    return p;
}

static string OutPath(UStaticMesh mesh) => new StaticMeshExporter(mesh).SavePath + ".glb";

static Xform Xf(FTransform t) => new()
{
    T = [t.Translation.X, t.Translation.Y, t.Translation.Z],
    R = [t.Rotation.X, t.Rotation.Y, t.Rotation.Z, t.Rotation.W],
    S = [t.Scale3D.X, t.Scale3D.Y, t.Scale3D.Z],
};

switch (rest[0])
{
    case "assets":
        foreach (var a in rest.Skip(1)) ExportAsset(a);
        break;
    case "level":
        ExportLevel(rest[1]);
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
    case "debug-ism":
        foreach (var e in provider.LoadPackage(ToFile(rest[1])).GetExports().OfType<F13InstancedMeshComponent>().Take(5))
            Console.WriteLine($"{e.ExportType} {e.Name} inst={e.Instances.Count} first={(e.Instances.Count > 0 ? e.Instances[0].TransformData.ToString() : "")}");
        return 0;
    case "mvp":
        foreach (var a in Mvp.Assets) ExportAsset(a);
        ExportLevel(Mvp.Map);
        foreach (var l in Mvp.Layouts) ExportLevel(l);
        WriteClasses();
        break;
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
return 0;

// materials.json: name -> exported JSON file, effective blend mode and two-sidedness. Material instances usually
// inherit these from a parent material, which the per-material JSON does not reflect.
void WriteMaterialIndex()
{
    var root = Path.GetFullPath(outDir!);
    var index = new SortedDictionary<string, MaterialInfo>(StringComparer.OrdinalIgnoreCase);
    foreach (var file in Directory.EnumerateFiles(Path.Combine(root, "SummerCamp"), "*.json", SearchOption.AllDirectories))
    {
        var rel = Path.GetRelativePath(root, file).Replace(Path.DirectorySeparatorChar, '/');
        var pkg = rel[..^".json".Length];
        UObject? mat;
        try { mat = provider.LoadPackage(pkg).GetExports().FirstOrDefault(e => e.Name == Path.GetFileNameWithoutExtension(file)); }
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
        index[mat.Name] = new MaterialInfo { File = rel, Blend = blend, TwoSided = twoSided ?? false };
    }
    var path = Path.Combine(root, "materials.json");
    File.WriteAllText(path, JsonSerializer.Serialize(index, new JsonSerializerOptions { IncludeFields = true }));
    Console.WriteLine($"{path}: {index.Count} materials, {index.Values.Count(m => m.Blend != "BLEND_Opaque")} not opaque");
}

static class Mvp
{
    public const string Map = "/Game/Maps/CrystalLake_Small/CrystalLake_Small";
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
    public static readonly string[] Classes = ["CarKeys_C"];
}

class MaterialInfo { public string File = "", Blend = ""; public bool TwoSided; }
class Xform { public float[] T = []; public float[] R = []; public float[] S = []; }
class Placement
{
    public string Level = "", Component = "", Mesh = "", File = "";
    public bool Hidden;
    public string?[] Materials = [];
    public Xform? Transform;
    public List<Xform>? Instances;
}
class ActorInfo
{
    public string Level = "", Name = "", Class = "";
    public Xform? Transform;
    public Newtonsoft.Json.Linq.JObject? Props;
    public List<Newtonsoft.Json.Linq.JObject>? Components;
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
