// f13-export: exports meshes and level layouts from YOUR OWN install of Friday the 13th: The Game to glTF + JSON,
// for f13-rs to load. Nothing is uploaded or written into the install; output goes to --out (keep it out of the repo).
//
//   f13-export --game <F13Game dir> --out <dir> assets <object path>...   e.g. /Game/Characters/Counselors/Athlete/Meshes/Body/Athlete_Body
//   f13-export --game <F13Game dir> --out <dir> level <map path>          e.g. /Game/Maps/CrystalLake_Small/CrystalLake_Small
//   f13-export --game <F13Game dir> --out <dir> mvp                       everything the MVP needs
using CUE4Parse.UE4.Objects.Engine.Curves;
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
// The engine content the game ships loose (Engine/Content): the sky sphere's mesh. Mounted on first use.
DefaultFileProvider? engineProvider = null;
UStaticMesh? EngineMesh(string path)
{
    if (engineProvider == null)
    {
        engineProvider = new DefaultFileProvider(new DirectoryInfo(Path.Combine(game, "Engine")), SearchOption.AllDirectories,
            new VersionContainer(EGame.GAME_UE4_18), StringComparer.OrdinalIgnoreCase);
        engineProvider.Initialize();
        engineProvider.Mount();
    }
    try { return engineProvider.LoadPackage(path).GetExports().OfType<UStaticMesh>().FirstOrDefault(); }
    catch (Exception e) { Console.WriteLine($"engine mesh {path}: {e.Message}"); return null; }
}
Collision.LoadProfiles(game);
// The animations' PlaySound notifies (anim_sounds.json); returns their cues for the sound export.
List<string> WriteAnimSounds()
{
    var (json, cues) = AnimSounds.Collect(provider, Mvp.CounselorAnims.Concat(Mvp.KillerAnims));
    File.WriteAllText(Path.Combine(outDir, "anim_sounds.json"), json.ToString(Newtonsoft.Json.Formatting.Indented));
    Console.WriteLine($"anim sounds: {json.Count} animations, {cues.Distinct().Count()} cues");
    // And their props (anim_props.json: SCPropSpawnerNotifyState), the meshes queued.
    var props = AnimSounds.Props(provider, Mvp.CounselorAnims.Concat(Mvp.KillerAnims), Queue, m => new SkinnedAssetExporter(m).SavePath + ".glb");
    File.WriteAllText(Path.Combine(outDir, "anim_props.json"), props.ToString(Newtonsoft.Json.Formatting.Indented));
    Console.WriteLine($"anim props: {props.Count} animations");
    // And their post-process blendables (anim_blendables.json: SCPostProcessBlendableNotify: the kills' screen blood).
    var blendables = AnimSounds.Blendables(provider, Mvp.CounselorAnims.Concat(Mvp.KillerAnims), Queue, t => t.GetPathName().Split('.')[0].TrimStart('/').Replace("Game/", "SummerCamp/Content/") + ".dds");
    File.WriteAllText(Path.Combine(outDir, "anim_blendables.json"), blendables.ToString(Newtonsoft.Json.Formatting.Indented));
    Console.WriteLine($"anim blendables: {blendables.Count} animations");
    return cues;
}
// Simple collision of every exported static mesh, by glTF path (collision.json).
var meshCollision = new SortedDictionary<string, Collision.MeshShapes>(StringComparer.OrdinalIgnoreCase);

var session = new ExportSession();
ClassExporter classes = null!;
var queued = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
// Morph targets only on the counselors' faces (their expressions and visemes: Blink_L, JawOpen, Fear...): CUE4Parse
// writes a target's empty deltas as an accessor without a bufferView (zeros, which glTF allows) that Bevy's loader
// rejects, so FillEmptyAccessors points those at a zero-filled view.
var options = new ExportOptions(meshFormat: EMeshFormat.Gltf2, textureFormat: ETextureFormat.Png, exportMorphTargets: false);
var faceOptions = new ExportOptions(meshFormat: EMeshFormat.Gltf2, textureFormat: ETextureFormat.Png, exportMorphTargets: true);
var faceSession = new ExportSession();
var faces = new List<string>();
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
    if (obj is USkeletalMesh face && face.Name.EndsWith("_Face", StringComparison.OrdinalIgnoreCase))
    {
        faceSession.Add(obj);
        faces.Add(new SkinnedAssetExporter(face).SavePath + ".glb");
    }
    else session.Add(obj);
}

void ExportAsset(string path)
{
    var pkg = provider.LoadPackage(ToFile(path));
    var n = 0;
    foreach (var e in pkg.GetExports())
    {
        if (e is UStaticMesh or USkeletalMesh or UTexture2D) { Queue(e); n++; }
    }
    Console.WriteLine($"{path}: {n} meshes or textures");
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
            // Attached to another actor's component (the cinematics' faces and hair on their body's headSocket).
            if (root?.GetOrDefault<FPackageIndex?>("AttachParent")?.Load<USceneComponent>() is { } parent && parent.Outer?.Name.Text is { } parentActor && parentActor != actor.Name)
            {
                info.AttachParent = parentActor;
                info.AttachSocket = root.GetOrDefault<FName>("AttachSocketName").Text is { } socket && socket != "None" ? socket : null;
            }
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
                    // Its attach parent's name (on the instance or its archetypes): the cabins' roof drips hang under Rain.
                    string? attach = null;
                    for (UObject? o = c; o != null && attach == null; o = o.Template?.Load())
                        attach = o.GetOrDefault<FPackageIndex?>("AttachParent")?.Name;
                    var j = classes.Describe(c, c.Name, attach);
                    if (j == null) continue;
                    if (c is USceneComponent sc) j["World"] = ClassExporter.Xf(sc.GetAbsoluteTransform());
                    info.Components.Add(j);
                }
                if (actor.ExportType == "SCWorldSettings") AddWorldSettingsClasses(info.Props);
                // The ambient cubemap's mean radiance (linear RGB): f13-game lights the scene with it.
                if (actor.ExportType == "PostProcessVolume" && actor.GetOrDefault<FStructFallback?>("Settings") is { } pp
                    && pp.GetOrDefault<FPackageIndex?>("AmbientCubemap")?.Load<UTextureCube>() is { } cube && CubeAverage(cube) is { } avg)
                    info.Props["AmbientCubemapAverage"] = new Newtonsoft.Json.Linq.JArray(avg);
                // Its colour grading LUT (a 256 x 16 strip of 16 slices; f13-game reads its PNG to build the volume's
                // combined LUT as the engine's CombineLUTs pass does).
                if (actor.ExportType == "PostProcessVolume" && actor.GetOrDefault<FStructFallback?>("Settings") is { } pps
                    && pps.GetOrDefault<FPackageIndex?>("ColorGradingLUT")?.Load<UTexture>() is { } lut)
                {
                    Queue(lut);
                    info.Props["ColorGradingLUTFile"] = lut.GetPathName().Split('.')[0].TrimStart('/');
                }
            }
            if (actor.ExportType == "BlockingVolume")
            {
                // Its brush's convex hulls, in world space.
                var brush = root;
                var bs = brush?.GetOrDefault<FPackageIndex?>("BrushBodySetup")?.Load() as CUE4Parse.UE4.Objects.PhysicsEngine.UBodySetup;
                var (pawn, camera, worldDynamic) = brush == null ? (false, false, false) : (Collision.BlocksPawn(brush, "BlockAll"), Collision.BlocksCamera(brush, "BlockAll"), Collision.BlocksWorldDynamic(brush, "BlockAll"));
                info.HullsBlockVehicle = brush != null && Collision.BlocksVehicle(brush, "BlockAll");
                if (brush != null && bs?.AggGeom != null && (pawn || camera || worldDynamic))
                {
                    info.HullsBlockPawn = pawn;
                    info.HullsBlockCamera = camera;
                    info.HullsBlockWorldDynamic = worldDynamic;
                    var w = brush.GetAbsoluteTransform();
                    info.Hulls = bs.AggGeom.ConvexElems.Where(c => c.VertexData is { Length: >= 4 })
                        .Select(c => c.VertexData.SelectMany(v => { var p = w.TransformPosition(c.Transform.TransformPosition(v)); return new[] { p.X, p.Y, p.Z }; }).ToArray())
                        .ToList();
                }
            }
            if (actor.ExportType == "AudioVolume")
            {
                // Its brush's convex hulls in world space (FAudioDevice::GetAudioVolumeSettings: the highest-priority
                // volume that encompasses the point).
                var bs = root?.GetOrDefault<FPackageIndex?>("BrushBodySetup")?.Load() as CUE4Parse.UE4.Objects.PhysicsEngine.UBodySetup;
                if (root != null && bs?.AggGeom != null)
                {
                    var w = root.GetAbsoluteTransform();
                    info.Hulls = bs.AggGeom.ConvexElems.Where(c => c.VertexData is { Length: >= 4 })
                        .Select(c => c.VertexData.SelectMany(v => { var p = w.TransformPosition(c.Transform.TransformPosition(v)); return new[] { p.X, p.Y, p.Z }; }).ToArray())
                        .ToList();
                }
            }
            if (actor.ExportType is "NavMeshBoundsVolume" or "SCMiniMapVolume")
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
            // The engine's sky sphere: from the loose engine content.
            if (mesh == null && smc.GetOrDefault<FPackageIndex?>("StaticMesh") is { IsNull: false } engineRef && engineRef.Name == "SM_SkySphere")
                mesh = EngineMesh("Engine/Content/EngineSky/SM_SkySphere");
            // The engine's sky sphere (Engine/Content, loose): its material's parameters too (the sky's colour without it).
            if (smc.GetOrDefault<FPackageIndex?>("StaticMesh") is { IsNull: false } meshRef && meshRef.Name == "SM_SkySphere"
                && smc.GetOrDefault<FPackageIndex[]>("OverrideMaterials", []).FirstOrDefault(m => m is { IsNull: false })?.Load<UMaterialInstanceConstant>() is { } sky)
            {
                var j = new Newtonsoft.Json.Linq.JObject { ["Material"] = sky.Name };
                foreach (var s in sky.ScalarParameterValues) j[s.Name] = s.ParameterValue;
                foreach (var v in sky.VectorParameterValues)
                    if (v.ParameterValue is { } c) j[v.Name] = new Newtonsoft.Json.Linq.JArray(c.R, c.G, c.B);
                layout.Sky = j;
                // The master's fixed texture samples too (M_Sky's stars, T_Stars, aren't a parameter).
                if (sky.GetOrDefault<FPackageIndex?>("Parent")?.Load<UObject>() is { } master)
                    foreach (var x in master.Owner!.GetExports().Where(x => x.ExportType.StartsWith("MaterialExpressionTextureSample")))
                        if (x.GetOrDefault<FPackageIndex?>("Texture")?.Load<UObject>() is UTexture t) Queue(t);
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
                BlocksCamera = Collision.BlocksCamera(smc),
                BlocksWorldDynamic = Collision.BlocksWorldDynamic(smc),
                BlocksVehicle = Collision.BlocksVehicle(smc),
                BlocksVisibility = Collision.BlocksVisibility(smc),
                CastShadow = smc.GetOrDefault("CastShadow", true),
                // Foliage instances fade out at InstanceEndCullDistance; other components at their max draw distance
                // (LDMaxDrawDistance, or the cull distance volume's CachedMaxDrawDistance), the nearer one when both are set.
                CullDistance = new[] { smc.GetOrDefault("InstanceEndCullDistance", 0), (int)smc.GetOrDefault("LDMaxDrawDistance", 0f), (int)smc.GetOrDefault("CachedMaxDrawDistance", 0f) }
                    .Where(d => d > 0).DefaultIfEmpty(0).Min(),
            };
            if (smc.ExportType == "SplineMeshComponent") p.Spline = SplineMeshSpline(smc);
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
// their defaults are absent), by kill class. Each also keeps the spline component's RelativeLocation and RelativeRotation
// (its place under the kill's root: a grab kill's spline is beside the kill, turned).
void WriteKillCameras()
{
    var all = new Newtonsoft.Json.Linq.JObject();
    foreach (var path in Mvp.KillCameras)
    {
        var name = path.Split('/')[^1];
        try
        {
            var spline = provider.LoadPackage(ToFile(path)).GetExports().FirstOrDefault(e => e.ExportType == "SCCameraSplineComponent");
            var props = spline == null ? null : Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(spline))["Properties"];
            var curves = props?["SplineCurves"] as Newtonsoft.Json.Linq.JObject;
            if (curves != null)
            {
                if (props["RelativeLocation"] != null) curves["RelativeLocation"] = props["RelativeLocation"];
                if (props["RelativeRotation"] != null) curves["RelativeRotation"] = props["RelativeRotation"];
            }
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

// The storm's values (SCZeus_BP's class defaults over SCZeus's constructor: InitialDelay 2, MinTimeBetweenLightning 8,
// MaxTimeBetweenLightning 25, LightningSoundThreshold 3) and its LightningCurves' keys (weather.json).
/// special_moves.json: each special move Blueprint's way onto its spot (SCSpecialMove_SoloMontage's fields; its
/// constructor 0x14041b450: bInterpToInteraction true, InterpolationTime 0.15, bMoveToward true, MoveThreshold 15,
/// bSweepTestOnLerp true).
void WriteSpecialMoves()
{
    var o = new Newtonsoft.Json.Linq.JObject();
    foreach (var f in provider.Files.Keys.Where(k => k.StartsWith("SummerCamp/Content/Blueprints/SpecialMoves/") && k.EndsWith(".uasset")).Order())
    {
        var name = f.Split('/')[^1][..^".uasset".Length];
        try
        {
            if (provider.LoadPackageObject<UBlueprintGeneratedClass>(f[..^".uasset".Length] + "." + name + "_C").ClassDefaultObject.Load() is not { } cdo) continue;
            o[name + "_C"] = new Newtonsoft.Json.Linq.JObject
            {
                ["InterpToInteraction"] = cdo.GetOrDefault("bInterpToInteraction", true),
                ["InterpolationTime"] = cdo.GetOrDefault("InterpolationTime", 0.15f),
                ["MoveToward"] = cdo.GetOrDefault("bMoveToward", true),
                ["MoveThreshold"] = cdo.GetOrDefault("MoveThreshold", 15f),
                ["SweepTestOnLerp"] = cdo.GetOrDefault("bSweepTestOnLerp", true),
            };
        }
        catch (Exception e)
        {
            Console.WriteLine($"special move {name}: {e.Message}");
        }
    }
    File.WriteAllText(Path.Combine(outDir, "special_moves.json"), o.ToString(Newtonsoft.Json.Formatting.Indented));
    Console.WriteLine($"special_moves.json: {o.Count} moves");
}

void WriteWeather()
{
    var cls = provider.LoadPackageObject<UBlueprintGeneratedClass>("SummerCamp/Content/Blueprints/SCZeus_BP.SCZeus_BP_C");
    var cdo = cls.ClassDefaultObject.Load()!;
    var curves = new Newtonsoft.Json.Linq.JArray();
    foreach (var c in cdo.GetOrDefault<FPackageIndex[]>("LightningCurves", []))
    {
        var keys = c.Load()?.GetOrDefault<FStructFallback>("FloatCurve")?.GetOrDefault<FRichCurveKey[]>("Keys", []) ?? [];
        curves.Add(new Newtonsoft.Json.Linq.JArray(keys.Select(k => new Newtonsoft.Json.Linq.JObject
        {
            ["Time"] = k.Time, ["Value"] = k.Value, ["ArriveTangent"] = k.ArriveTangent, ["LeaveTangent"] = k.LeaveTangent,
            ["Interp"] = k.InterpMode switch { ERichCurveInterpMode.RCIM_Linear => "linear", ERichCurveInterpMode.RCIM_Constant => "constant", _ => "cubic" },
        })));
    }
    var json = new Newtonsoft.Json.Linq.JObject
    {
        ["InitialDelay"] = cdo.GetOrDefault("InitialDelay", 2),
        ["MinTimeBetweenLightning"] = cdo.GetOrDefault("MinTimeBetweenLightning", 8),
        ["MaxTimeBetweenLightning"] = cdo.GetOrDefault("MaxTimeBetweenLightning", 25),
        ["LightningSoundThreshold"] = cdo.GetOrDefault("LightningSoundThreshold", 3f),
        ["LightningCurves"] = curves,
    };
    File.WriteAllText(Path.Combine(outDir!, "weather.json"), json.ToString());
    Console.WriteLine($"weather.json: {curves.Count} lightning curves");
}

void WriteClasses()
{
    foreach (var c in Mvp.Classes) AddClassByName(c);
    classes.Run();
    // The kills' KillInfo damage types (their ScoreEvent: the killer's Resourceful, Versatile or Seeker).
    var killInfos = classes.Classes.Values.SelectMany(c => c["Components"] as Newtonsoft.Json.Linq.JArray ?? [])
        .Select(c => c["Props"]?["KillInfo"]?.ToString()).Where(n => n != null && n.EndsWith("_C") && !classes.Classes.ContainsKey(n)).Distinct().ToList();
    foreach (var k in killInfos) AddClassByName(k!);
    classes.Run();
    // The weapons' impact effects (ImpactEffectClass, a gun's PelletImpactTemplate, a thrown knife's ImpactTemplate: SCImpactEffect Blueprints, their FX,
    // sounds and decals by surface type).
    var impacts = classes.Classes.Values.SelectMany(c => new[] { c["Props"]?["ImpactEffectClass"], c["Props"]?["PelletImpactTemplate"], c["Props"]?["ImpactTemplate"] })
        .Select(t => t?.ToString()).Where(n => n != null && n.EndsWith("_C") && !classes.Classes.ContainsKey(n)).Distinct().ToList();
    foreach (var k in impacts) AddClassByName(k!);
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
    case "grep": // grep <text> [path substring]: packages whose bytes contain <text> (ASCII: the name map)
    {
        var needle = System.Text.Encoding.ASCII.GetBytes(rest[1]);
        foreach (var f in provider.Files.Keys.Where(k => (k.EndsWith(".uasset") || k.EndsWith(".umap")) && (rest.Count < 3 || k.Contains(rest[2], StringComparison.OrdinalIgnoreCase))).Order())
        {
            try { if (provider.SaveAsset(f).AsSpan().IndexOf(needle) >= 0) Console.WriteLine(f); } catch { }
        }
        return 0;
    }
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
        if (rest[1] == Mvp.Map) ExportLevel(Mvp.Map);
        else ExportLevel(rest[1]);
        break;
    case "specialmoves": // special_moves.json only
        WriteSpecialMoves();
        return 0;
    case "levels": // the MVP map and its cabin layouts, and the classes they use (classes.json)
        ExportLevel(Mvp.Map);
        foreach (var w in Mvp.Weathers) ExportLevel(w);
        WriteWeather();
        foreach (var l in Mvp.Layouts) ExportLevel(l);
        WriteClasses();
        WriteSpecialMoves();
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
    case "bones": // bones <skeletal mesh package>: its reference skeleton (name, parent, local rotator and translation, Unreal space)
        foreach (var m in provider.LoadPackage(ToFile(rest[1])).GetExports().OfType<USkeletalMesh>())
            for (var i = 0; i < m.ReferenceSkeleton.FinalRefBoneInfo.Length; i++)
            {
                var b = m.ReferenceSkeleton.FinalRefBoneInfo[i];
                var p = m.ReferenceSkeleton.FinalRefBonePose[i];
                Console.WriteLine($"{i}\t{b.Name.Text}\t{b.ParentIndex}\t{p.Rotation.Rotator()}\t{p.Translation}");
            }
        // And its skeleton asset's own reference pose.
        foreach (var m in provider.LoadPackage(ToFile(rest[1])).GetExports().OfType<USkeletalMesh>())
            if (m.Skeleton.Load<CUE4Parse.UE4.Assets.Exports.Animation.USkeleton>() is { } sk)
                for (var i = 0; i < sk.ReferenceSkeleton.FinalRefBoneInfo.Length; i++)
                {
                    var p = sk.ReferenceSkeleton.FinalRefBonePose[i];
                    Console.WriteLine($"skeleton {i}\t{sk.ReferenceSkeleton.FinalRefBoneInfo[i].Name.Text}\t{p.Rotation.Rotator()}\t{p.Translation}");
                }
        return 0;
    case "cdo": // cdo <class name>...: each Blueprint class's defaults (its CDO's properties), one JSON line each
        foreach (var name in rest.Skip(1))
        {
            var file = provider.Files.Keys.FirstOrDefault(k => k.EndsWith("/" + name.TrimEnd('C').TrimEnd('_') + ".uasset", StringComparison.OrdinalIgnoreCase));
            var cdo = file == null ? null : provider.LoadPackage(file).GetExports().FirstOrDefault(e => e.Name.StartsWith("Default__"));
            var props = cdo == null ? null : Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(cdo))["Properties"];
            Console.WriteLine($"{name}\t{file}\t{props?.ToString(Newtonsoft.Json.Formatting.None)}");
        }
        return 0;
    case "pseudo": // pseudo <package> [--lines]: its Blueprint classes' bytecode as pseudo code (reverse engineering); --lines: one statement per line after its code offset
        provider.ReadScriptData = true;
        foreach (var e in provider.LoadPackage(ToFile(rest[1])).GetExports().OfType<UClass>())
        {
            Console.WriteLine($"// ===== {e.Name}");
            Console.WriteLine(e.DecompileBlueprintToPseudo(null!));
        }
        // And each function's bytecode (its Kismet expressions).
        foreach (var f in provider.LoadPackage(ToFile(rest[1])).GetExports().OfType<UFunction>())
        {
            Console.WriteLine($"// ===== function {f.Name}");
            if (rest.Count > 2 && rest[2] == "--lines") // one statement per line, after its code offset
                foreach (var x in f.ScriptBytecode)
                    Console.WriteLine($"@{x.StatementIndex} {Newtonsoft.Json.JsonConvert.SerializeObject(x)}");
            else
                Console.WriteLine(Newtonsoft.Json.JsonConvert.SerializeObject(f.ScriptBytecode, Newtonsoft.Json.Formatting.Indented));
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
        ExportLevel(Mvp.Map);
        foreach (var w in Mvp.Weathers) ExportLevel(w);
        WriteWeather();
        foreach (var l in Mvp.Layouts) ExportLevel(l);
        WriteClasses();
        WriteSpecialMoves();
        NavMeshExporter.Write(provider, outDir, Mvp.Map);
        AnimExporter.Write(provider, outDir, Mvp.Montages);
        new CounselorExporter(provider, outDir, Queue).Export(Mvp.Counselor, Mvp.CounselorAnims, Mvp.KeepRoot);
        new CounselorExporter(provider, outDir, Queue).ExportKiller(Mvp.Killer, Mvp.KillerAnims, Mvp.KeepRoot);
        new CounselorExporter(provider, outDir, Queue).ExportPropAnims(Mvp.TrapMesh, Mvp.TrapAnims); new CounselorExporter(provider, outDir, Queue).ExportPropAnims(Mvp.TentMesh, Mvp.TentAnims); new CounselorExporter(provider, outDir, Queue).ExportPropAnims(Mvp.CarMesh, Mvp.CarAnims);
        foreach (var (mesh, anim) in Mvp.PlacedKillProps) new CounselorExporter(provider, outDir, Queue).ExportPropAnims(mesh, [anim]);
        WriteKillCameras();
        WriteWater();
        WriteCollision();
        {
            var sx = new SoundExporter(provider, outDir, ToFile);
            sx.Export(sx.FootstepTables().Concat(Mvp.Sounds).Concat(sx.ClassSounds()).Concat(WriteAnimSounds()));
            sx.WriteSoundClasses();
            sx.WriteSoundMixes(Mvp.SoundMixes);
        }
        {
            var wx = new WidgetExporter(provider, outDir, Queue, ToFile);
            foreach (var w in Mvp.Widgets) wx.Export(w);
            // M_UI_JasonHud_01's own texture samples (its Smoke and Grime layers).
            ExportAsset("/Game/UI/Materials/UI_Smoke");
            ExportAsset("/Game/UI/Materials/Grime");
            wx.ClassTextures();
            wx.Fonts(game!);
        }
        break;
    case "tuning": // tuning [out file]: the gameplay values (tuning.json; default ~/.f13-rs/tuning.json), (a port of the former tools/extract_tuning.py)
        return Tuning.Write(provider, game!, rest.Count > 1 ? rest[1] : null);
    case "sounds": // the sounds f13-game plays (sounds/): the counselor's footsteps, the classes' cues, the animations' (anim_sounds.json), Mvp.Sounds
    {
        var sx = new SoundExporter(provider, outDir, ToFile);
        var n = sx.Export(sx.FootstepTables().Concat(Mvp.Sounds).Concat(sx.ClassSounds()).Concat(WriteAnimSounds()));
        sx.WriteSoundClasses();
        sx.WriteSoundMixes(Mvp.SoundMixes);
        Console.WriteLine($"sounds: {n} cues");
        // (On to the meshes: the animations' props.)
        break;
    }
    case "collision": // levels (for collision flags), classes and collision.json; no meshes or textures
        ExportLevel(Mvp.Map);
        foreach (var w in Mvp.Weathers) ExportLevel(w);
        WriteWeather();
        foreach (var l in Mvp.Layouts) ExportLevel(l);
        WriteClasses();
        WriteSpecialMoves();
        WriteCollision();
        foreach (var (k, n) in Collision.VisibilityDiffers) Console.WriteLine($"visibility differs from WorldDynamic: {n} x {k}");
        return 0;
    case "counselor": // the MVP counselor's parts and animations (and the meshes they need)
        new CounselorExporter(provider, outDir, Queue).Export(Mvp.Counselor, Mvp.CounselorAnims, Mvp.KeepRoot);
        break;
    case "ui": // ui [widget...]: widget trees and their textures (default: Mvp.Widgets)
    {
        var wx = new WidgetExporter(provider, outDir, Queue, ToFile);
        var n = 0;
        foreach (var w in rest.Count > 1 ? rest.Skip(1) : Mvp.Widgets) n += wx.Export(w);
        // M_UI_JasonHud_01's own texture samples (its Smoke and Grime layers).
        ExportAsset("/Game/UI/Materials/UI_Smoke");
        ExportAsset("/Game/UI/Materials/Grime");
        // The map's definition (SCMapDefinition): the lobby's MapNameText (DisplayName) and CurrentMapImage
        // (LobbyThumbnailImages' first) for the map played (ui/map.json).
        {
            var def = provider.LoadPackage(ToFile("/Game/Blueprints/MapRegistry/MapDef_CrystalLake_Small")).GetExports().FirstOrDefault(e => e.Name.StartsWith("Default__"));
            var p = def == null ? null : Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(def))["Properties"];
            var image = p?["LobbyThumbnailImages"]?[0]?["AssetPathName"]?.ToString().Split('.')[0];
            if (image != null) ExportAsset(image);
            File.WriteAllText(Path.Combine(outDir, "ui", "map.json"), new Newtonsoft.Json.Linq.JObject
            {
                ["Name"] = p?["DisplayName"]?["LocalizedString"]?.ToString(),
                ["Image"] = image == null ? null : ToFile(image) + ".png",
            }.ToString());
        }
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
        new CounselorExporter(provider, outDir, Queue).ExportPropAnims(Mvp.TrapMesh, Mvp.TrapAnims); new CounselorExporter(provider, outDir, Queue).ExportPropAnims(Mvp.TentMesh, Mvp.TentAnims); new CounselorExporter(provider, outDir, Queue).ExportPropAnims(Mvp.CarMesh, Mvp.CarAnims);
        foreach (var (mesh, anim) in Mvp.PlacedKillProps) new CounselorExporter(provider, outDir, Queue).ExportPropAnims(mesh, [anim]);
        break;
    case "anims":
        AnimExporter.Write(provider, outDir, Mvp.Montages);
        return 0;
    case "sequences": // the level intro and outro sequences (sequences/), and the levels they stage
        ExportLevel(Mvp.IntroLevel);
        ExportLevel(Mvp.OutroLevel);
        SequenceExporter.Write(provider, outDir, Mvp.Sequences);
        // The takes each staged skeletal actor plays (cinematic/<actor>.json and .anim files), on its own mesh's skeleton.
        {
            var actors = new Dictionary<string, string>();
            foreach (var lv in new[] { Mvp.IntroLevel, Mvp.OutroLevel })
            {
                var json = Newtonsoft.Json.Linq.JObject.Parse(File.ReadAllText(Path.Combine(outDir, "levels", lv.Split('/')[^1] + ".json")));
                foreach (var a in json["Actors"]!)
                    if (a["Components"]?.FirstOrDefault(c => c["Mesh"] != null)?["Mesh"]?.ToString() is { } m)
                        actors[a["Name"]!.ToString()] = m[..^".glb".Length];
            }
            var takes = new Dictionary<string, HashSet<string>>();
            foreach (var f in Directory.EnumerateFiles(Path.Combine(outDir, "sequences"), "*.json"))
            {
                var s = Newtonsoft.Json.Linq.JObject.Parse(File.ReadAllText(f));
                var objects = s["Bindings"]!.ToDictionary(b => b["Guid"]!.ToString(), b => b["Object"]?.ToString()?.Split('.')[^1]);
                foreach (var t in s["Tracks"]!.Where(t => t["Type"]?.ToString() == "MovieSceneSkeletalAnimationTrack"))
                    foreach (var sec in t["Sections"]!)
                        if (sec["Props"]?["Params"]?["Animation"]?["ObjectPath"]?.ToString() is { } anim
                            && objects.GetValueOrDefault(t["Binding"]?.ToString() ?? "") is { } actor && actors.ContainsKey(actor))
                            (takes.TryGetValue(actor, out var set) ? set : takes[actor] = []).Add(anim[..anim.LastIndexOf('.')]);
            }
            foreach (var (actor, set) in takes) new CounselorExporter(provider, outDir, Queue).ExportPropAnims(actors[actor], set, "cinematic", actor);
            // Their audio sections' sounds (sounds/).
            var sounds = new List<string>();
            foreach (var f in Directory.EnumerateFiles(Path.Combine(outDir, "sequences"), "*.json"))
                foreach (var t in Newtonsoft.Json.Linq.JObject.Parse(File.ReadAllText(f))["Tracks"]!.Where(t => t["Type"]?.ToString() == "MovieSceneAudioTrack"))
                    foreach (var sec in t["Sections"]!)
                        if (sec["Props"]?["Sound"]?["ObjectPath"]?.ToString() is { } sound) sounds.Add(sound[..sound.LastIndexOf('.')]);
            Console.WriteLine($"sequence sounds: {new SoundExporter(provider, outDir, ToFile).Export(sounds)} written");
        }
        // The staged TextRenderComponents' fonts (fonts/<name>.json: UFont's Characters (StartU, StartV, USize, VSize,
        // TextureIndex, VerticalOffset), Kerning and its atlas pages, queued as textures).
        foreach (var lv in new[] { Mvp.IntroLevel, Mvp.OutroLevel })
            foreach (var a in Newtonsoft.Json.Linq.JObject.Parse(File.ReadAllText(Path.Combine(outDir, "levels", lv.Split('/')[^1] + ".json")))["Actors"]!)
                foreach (var c in a["Components"] ?? new Newtonsoft.Json.Linq.JArray())
                {
                    if (c["Type"]?.ToString() != "TextRenderComponent" || c["Props"]?["Font"]?.ToString() is not { } fontRef) continue;
                    var name = fontRef.Split('\'')[1];
                    var file = provider.Files.Keys.FirstOrDefault(k => k.EndsWith("/" + name + ".uasset", StringComparison.OrdinalIgnoreCase));
                    var font = file == null ? null : provider.LoadPackage(file).GetExports().FirstOrDefault(e => e.ExportType == "Font");
                    if (font == null) { Console.WriteLine($"font {name}: not found"); continue; }
                    var props = Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(font))["Properties"]!;
                    var pages = new Newtonsoft.Json.Linq.JArray();
                    foreach (var t in font.GetOrDefault<FPackageIndex[]>("Textures", []))
                        if (t.Load() is UTexture2D tex) { Queue(tex); pages.Add(tex.Owner!.Name + "/" + tex.Name); }
                    var chars = new Newtonsoft.Json.Linq.JArray(props["Characters"]!.Select(ch => new Newtonsoft.Json.Linq.JArray(
                        (int)ch["StartU"]!, (int)ch["StartV"]!, (int)ch["USize"]!, (int)ch["VSize"]!, (int)ch["TextureIndex"]!, (int)ch["VerticalOffset"]!)));
                    var outJson = new Newtonsoft.Json.Linq.JObject { ["Pages"] = pages, ["Kerning"] = props["Kerning"] ?? 0, ["IsRemapped"] = props["IsRemapped"] ?? 0, ["Characters"] = chars };
                    Directory.CreateDirectory(Path.Combine(outDir, "fonts"));
                    File.WriteAllText(Path.Combine(outDir, "fonts", name + ".json"), outJson.ToString());
                    Console.WriteLine($"font {name}: {chars.Count} characters, {pages.Count} pages");
                }
        break;
    case "particles": // particles [system...]: Cascade systems (particles/) and their materials; default: every template the exported levels place and the classes name
        {
            var names = rest.Skip(1).ToList();
            if (names.Count == 0)
                foreach (var f in Directory.EnumerateFiles(Path.Combine(outDir, "levels"), "*.json"))
                    foreach (var a in Newtonsoft.Json.Linq.JObject.Parse(File.ReadAllText(f))["Actors"] ?? new Newtonsoft.Json.Linq.JArray())
                        foreach (var c in a["Components"] ?? new Newtonsoft.Json.Linq.JArray())
                            if (c["Props"]?["Template"]?.ToString() is { } t && t.StartsWith("ParticleSystem'")) names.Add(t.Split('\'')[1]);
            // And the classes' (their components' templates and properties: HitParticle, MuzzleFlash, ...).
            var classesJson = Path.Combine(outDir, "classes.json");
            if (rest.Count == 1 && File.Exists(classesJson))
                foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(File.ReadAllText(classesJson), @"ParticleSystem'([A-Za-z0-9_]+)'"))
                    names.Add(m.Groups[1].Value);
            // And their soft references to particle systems (the impact effects' FX by surface).
            if (rest.Count == 1 && File.Exists(classesJson))
                foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(File.ReadAllText(classesJson), @"/Game/Effects/ParticleSystems/[^""]*\.([A-Za-z0-9_]+)"""))
                    names.Add(m.Groups[1].Value);
            // PlayerCameraManager_BP's RainParticleSystem (a soft reference: the camera's rain grid, f13-game particles.rs).
            if (rest.Count == 1) names.Add("P_Rain_Environmental_01A_GPU");
            // SoundBlip_C's ParticleSystem (Jason_Base's SoundBlipClass: his sound blips, f13-game sound_blips.rs).
            if (rest.Count == 1) names.Add("P_Sound_Wave_Indicator_01");
            Console.WriteLine($"particles: {ParticleExporter.Write(provider, outDir, names, Queue)} systems written");
            // The impact effects' decal materials (their <surface>Decal's DecalMaterial and SplashDecals' Decals) and textures.
            if (rest.Count == 1 && File.Exists(classesJson))
            {
                var decals = System.Text.RegularExpressions.Regex.Matches(File.ReadAllText(classesJson), @"""(?:DecalMaterial""\s*:\s*|Decals""\s*:\s*\[)[^\]}]*")
                    .SelectMany(m => System.Text.RegularExpressions.Regex.Matches(m.Value, @"Material(?:InstanceConstant)?'([A-Za-z0-9_]+)'").Select(x => x.Groups[1].Value))
                    .Distinct().ToList();
                foreach (var d in decals)
                {
                    var file = provider.Files.Keys.FirstOrDefault(k => k.EndsWith("/" + d + ".uasset", StringComparison.OrdinalIgnoreCase));
                    if (file == null || provider.LoadPackage(file[..^".uasset".Length]).GetExports().FirstOrDefault(e => e.Name == d) is not { } m) continue;
                    Queue(m);
                    ParticleExporter.QueueTextures(m, Queue);
                }
                Console.WriteLine($"particles: {decals.Count} decal materials");
            }
        }
        break;
    case "dds":
        DdsExporter.WriteAll(provider, outDir);
        return 0;
    default:
        Console.Error.WriteLine($"unknown command {rest[0]}");
        return 2;
}

Console.WriteLine($"exporting {session.TotalQueued} objects...");
var results = (await session.RunAsync(outDir, options)).Concat(await faceSession.RunAsync(outDir, faceOptions)).ToList();
foreach (var f in faces)
{
    HealMorphSeams(Path.Combine(outDir, f));
    FillEmptyAccessors(Path.Combine(outDir, f));
}
results.AddRange(await ExportDefaultTextures());
var failed = results.Where(r => !r.Success).ToList();
Console.WriteLine($"done: {results.Count - failed.Count} ok, {failed.Count} failed");
foreach (var f in failed.Take(20)) Console.WriteLine($"  failed {f.ObjectPath}");
WriteMaterialIndex();
DdsExporter.WriteAll(provider, outDir);
return 0;

// A face glb's morph targets moved at its seams too: UE keeps a delta for every render vertex (FMorphTargetDelta's
// SourceIdx), so the copies of a vertex split at a UV or section seam move together, but the export leaves all but one
// copy at zero (every counselor face: Prep's Fear 155 of its 155 seam groups), tearing the face along its seams as an
// expression plays. Each target's zero-delta copies take the delta of a coincident vertex that has one.
static void HealMorphSeams(string path)
{
    if (!File.Exists(path)) return;
    var b = File.ReadAllBytes(path);
    var jsonLen = BitConverter.ToInt32(b, 12);
    var json = Newtonsoft.Json.Linq.JObject.Parse(System.Text.Encoding.UTF8.GetString(b, 20, jsonLen));
    var binAt = 20 + jsonLen;
    var bin = new List<byte>(binAt + 8 <= b.Length ? b.AsSpan(binAt + 8, BitConverter.ToInt32(b, binAt)).ToArray() : []);
    var accessors = (Newtonsoft.Json.Linq.JArray)json["accessors"]!;
    var views = (Newtonsoft.Json.Linq.JArray)(json["bufferViews"] ??= new Newtonsoft.Json.Linq.JArray());
    static int Components(string t) => t switch { "SCALAR" => 1, "VEC2" => 2, "VEC3" => 3, "VEC4" => 4, _ => 4 };
    int? Offset(Newtonsoft.Json.Linq.JToken a) => a["bufferView"] is { } v ? ((int?)views[(int)v]!["byteOffset"] ?? 0) + ((int?)a["byteOffset"] ?? 0) : null;
    float[] Read(int i)
    {
        var a = accessors[i]!;
        var data = new float[(int)a["count"]! * Components(a["type"]!.ToString())];
        if (Offset(a) is int at && (int)a["componentType"]! == 5126)
        {
            var raw = bin.ToArray();
            for (var k = 0; k < data.Length; k++) data[k] = BitConverter.ToSingle(raw, at + 4 * k);
        }
        return data;
    }
    var healed = 0;
    foreach (var mesh in json["meshes"] ?? new Newtonsoft.Json.Linq.JArray())
        foreach (var prim in mesh["primitives"] ?? new Newtonsoft.Json.Linq.JArray())
        {
            if (prim["targets"] is not Newtonsoft.Json.Linq.JArray targets || prim["attributes"]?["POSITION"] is not { } posIndex) continue;
            var pos = Read((int)posIndex);
            var groups = Enumerable.Range(0, pos.Length / 3)
                .GroupBy(v => (MathF.Round(pos[3 * v] * 20000), MathF.Round(pos[3 * v + 1] * 20000), MathF.Round(pos[3 * v + 2] * 20000)))
                .Where(g => g.Count() > 1).Select(g => g.ToArray()).ToList();
            if (groups.Count == 0) continue;
            foreach (var target in targets)
                foreach (var attr in ((Newtonsoft.Json.Linq.JObject)target).Properties().ToList())
                {
                    var index = (int)attr.Value;
                    var a = accessors[index]!;
                    var n = Components(a["type"]!.ToString());
                    var d = Read(index);
                    bool Moves(int v) { for (var c = 0; c < n; c++) if (MathF.Abs(d[n * v + c]) > 1e-6f) return true; return false; }
                    var changed = false;
                    foreach (var g in groups)
                    {
                        var from = g.FirstOrDefault(v => Moves(v), -1);
                        if (from < 0) continue;
                        foreach (var v in g.Where(v => !Moves(v)))
                        {
                            for (var c = 0; c < n; c++) d[n * v + c] = d[n * from + c];
                            changed = true;
                            healed++;
                        }
                    }
                    if (!changed) continue;
                    // Its own new view (others may share the old one: FillEmptyAccessors' zero view).
                    var bytes = d.SelectMany(BitConverter.GetBytes).ToArray();
                    while (bin.Count % 4 != 0) bin.Add(0);
                    views.Add(new Newtonsoft.Json.Linq.JObject { ["buffer"] = 0, ["byteOffset"] = bin.Count, ["byteLength"] = bytes.Length });
                    bin.AddRange(bytes);
                    a["bufferView"] = views.Count - 1;
                    a["byteOffset"] = 0;
                    if (a["min"] != null)
                    {
                        a["min"] = new Newtonsoft.Json.Linq.JArray(Enumerable.Range(0, n).Select(c => (double)Enumerable.Range(0, d.Length / n).Min(v => d[n * v + c])));
                        a["max"] = new Newtonsoft.Json.Linq.JArray(Enumerable.Range(0, n).Select(c => (double)Enumerable.Range(0, d.Length / n).Max(v => d[n * v + c])));
                    }
                }
        }
    if (healed == 0) return;
    while (bin.Count % 4 != 0) bin.Add(0);
    json["buffers"]![0]!["byteLength"] = bin.Count;
    var jsonBytes = System.Text.Encoding.UTF8.GetBytes(json.ToString(Newtonsoft.Json.Formatting.None));
    var jsonPadded = new byte[(jsonBytes.Length + 3) & ~3];
    Array.Fill(jsonPadded, (byte)' ');
    Array.Copy(jsonBytes, jsonPadded, jsonBytes.Length);
    using var o = new BinaryWriter(File.Create(path));
    o.Write(0x46546C67u);
    o.Write(2u);
    o.Write((uint)(12 + 8 + jsonPadded.Length + 8 + bin.Count));
    o.Write((uint)jsonPadded.Length);
    o.Write(0x4E4F534Au);
    o.Write(jsonPadded);
    o.Write((uint)bin.Count);
    o.Write(0x004E4942u);
    o.Write(bin.ToArray());
    Console.WriteLine($"{Path.GetFileName(path)}: {healed} morph deltas copied onto their seam twins");
}

// A glb's accessors without a bufferView (and not sparse: all zeros) pointed at one zero-filled view appended to its
// binary chunk.
static void FillEmptyAccessors(string path)
{
    if (!File.Exists(path)) return;
    var b = File.ReadAllBytes(path);
    var jsonLen = BitConverter.ToInt32(b, 12);
    var json = Newtonsoft.Json.Linq.JObject.Parse(System.Text.Encoding.UTF8.GetString(b, 20, jsonLen));
    var binAt = 20 + jsonLen;
    var bin = binAt + 8 <= b.Length ? b.AsSpan(binAt + 8, BitConverter.ToInt32(b, binAt)).ToArray() : [];
    var empty = json["accessors"]!.Where(a => a["bufferView"] == null && a["sparse"] == null).ToList();
    if (empty.Count == 0) return;
    static int Components(string t) => t switch { "SCALAR" => 1, "VEC2" => 2, "VEC3" => 3, "VEC4" => 4, "MAT4" => 16, _ => 4 };
    var size = empty.Max(a => (int)a["count"]! * Components(a["type"]!.ToString()) * 4);
    var offset = (bin.Length + 3) & ~3;
    var views = (Newtonsoft.Json.Linq.JArray)json["bufferViews"]!;
    views.Add(new Newtonsoft.Json.Linq.JObject { ["buffer"] = 0, ["byteOffset"] = offset, ["byteLength"] = size });
    foreach (var a in empty) a["bufferView"] = views.Count - 1;
    var newBin = new byte[(offset + size + 3) & ~3];
    Array.Copy(bin, newBin, bin.Length);
    json["buffers"]![0]!["byteLength"] = offset + size;
    var jsonBytes = System.Text.Encoding.UTF8.GetBytes(json.ToString(Newtonsoft.Json.Formatting.None));
    var jsonPadded = new byte[(jsonBytes.Length + 3) & ~3];
    Array.Fill(jsonPadded, (byte)' ');
    Array.Copy(jsonBytes, jsonPadded, jsonBytes.Length);
    using var o = new BinaryWriter(File.Create(path));
    o.Write(0x46546C67u);
    o.Write(2u);
    o.Write((uint)(12 + 8 + jsonPadded.Length + 8 + newBin.Length));
    o.Write((uint)jsonPadded.Length);
    o.Write(0x4E4F534Au);
    o.Write(jsonPadded);
    o.Write((uint)newBin.Length);
    o.Write(0x004E4942u);
    o.Write(newBin);
}

// The textures every exported material draws with that its own export left out: its master's texture parameter
// defaults up the Parent chain (M_Prep_Eye_Inst sets only IrisScale; its EyeDiffuse is EyeMaster_02_m's Eye_Diffuse,
// which materials.json names but nothing wrote).
async Task<List<ExportResult>> ExportDefaultTextures()
{
    var extra = new ExportSession();
    var exported = Path.Combine(Path.GetFullPath(outDir!), "SummerCamp");
    if (!Directory.Exists(exported)) return [];
    foreach (var file in Directory.EnumerateFiles(exported, "*.json", SearchOption.AllDirectories))
    {
        var pkg = Path.GetRelativePath(Path.GetFullPath(outDir!), file).Replace(Path.DirectorySeparatorChar, '/')[..^".json".Length];
        UObject? mat;
        try { mat = provider.LoadPackage(pkg).GetExports().FirstOrDefault(e => e.Name.Equals(Path.GetFileNameWithoutExtension(file), StringComparison.OrdinalIgnoreCase)); }
        catch { continue; }
        if (mat is not UMaterialInterface) continue;
        ParticleExporter.QueueTextures(mat, t =>
        {
            var tp = t.GetPathName().Split('.')[0].TrimStart('/');
            if (tp.StartsWith("Game/")) tp = "SummerCamp/Content/" + tp["Game/".Length..];
            if (queued.Add(t.GetPathName()) && !File.Exists(Path.Combine(outDir!, tp + ".png"))) extra.Add(t);
        });
    }
    Console.WriteLine($"materials' default textures: {extra.TotalQueued} to export");
    return (await extra.RunAsync(outDir, options)).ToList();
}

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
                // Paper2D's sprite sampler keeps its constructor's name ("SpriteTexture") out of the package.
                if ((string.IsNullOrEmpty(name) || name == "None") && e.ExportType == "MaterialExpressionSpriteTextureSampler") name = "SpriteTexture";
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
                    case var t when t.StartsWith("MaterialExpressionTextureSampleParameter") || t == "MaterialExpressionTextureObjectParameter" || t == "MaterialExpressionSpriteTextureSampler":
                        if (TexPath(e.GetOrDefault<FPackageIndex?>("Texture")) is { } tp) info.Textures.TryAdd(name, tp);
                        break;
                }
            }
            info.Landscape = LandscapeLayers(m);
            BaseLayerTextures(m, info);
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

// A SplineMeshComponent's SplineParams (Start/End Pos, Tangent, Scale, Roll, Offset), SplineUpDir, ForwardAxis,
// SplineBoundaryMin/Max and bSmoothInterpRollScale; unset ones are USplineMeshComponent's defaults (scale 1, up +Z,
// forward X).
static Newtonsoft.Json.Linq.JObject SplineMeshSpline(UObject c)
{
    static Newtonsoft.Json.Linq.JArray V3(FVector v) => [v.X, v.Y, v.Z];
    static Newtonsoft.Json.Linq.JArray V2(FVector2D v) => [v.X, v.Y];
    var s = c.GetOrDefault<FStructFallback?>("SplineParams");
    var one = new FVector2D(1, 1);
    var j = new Newtonsoft.Json.Linq.JObject();
    foreach (var end in new[] { "Start", "End" })
    {
        j[end + "Pos"] = V3(s?.GetOrDefault($"{end}Pos", FVector.ZeroVector) ?? FVector.ZeroVector);
        j[end + "Tangent"] = V3(s?.GetOrDefault($"{end}Tangent", FVector.ZeroVector) ?? FVector.ZeroVector);
        j[end + "Scale"] = V2(s?.GetOrDefault($"{end}Scale", one) ?? one);
        j[end + "Roll"] = s?.GetOrDefault($"{end}Roll", 0f) ?? 0f;
        j[end + "Offset"] = V2(s?.GetOrDefault($"{end}Offset", FVector2D.ZeroVector) ?? FVector2D.ZeroVector);
    }
    j["SplineUpDir"] = V3(c.GetOrDefault("SplineUpDir", new FVector(0, 0, 1)));
    var axis = c.GetOrDefault<FName>("ForwardAxis").Text;
    j["ForwardAxis"] = axis.EndsWith("Y") ? 1 : axis.EndsWith("Z") ? 2 : 0;
    j["SplineBoundaryMin"] = c.GetOrDefault("SplineBoundaryMin", 0f);
    j["SplineBoundaryMax"] = c.GetOrDefault("SplineBoundaryMax", 0f);
    j["bSmoothInterpRollScale"] = c.GetOrDefault("bSmoothInterpRollScale", false);
    return j;
}

// A layered master's base layer textures (M_JasonV2_PickAxe: MatLayerBlend_Simple's "Base Material" is a material
// function call, M_JasonV2_PickAxe_Layer, whose TextureSamples hold the pickaxe's own maps; the blood layer goes on
// top). Cooking strips the function's wiring, so each sample's role is its sampler type (Normal) or its texture's
// name suffix (_Diffuse, _RMA: R roughness, G metallic, B AO, as the master's own input masks still read). Parameters
// of the same names win.
static void BaseLayerTextures(UObject master, MaterialInfo info)
{
    var calls = master.GetOrDefault<FPackageIndex[]>("Expressions", []).Select(x => x.Load()).OfType<UObject>()
        .Where(e => e.ExportType == "MaterialExpressionMaterialFunctionCall").ToDictionary(e => e.Name, StringComparer.OrdinalIgnoreCase);
    foreach (var blend in calls.Values)
        foreach (var input in blend.GetOrDefault<FStructFallback[]>("FunctionInputs", []))
        {
            var i = input.GetOrDefault<FExpressionInput?>("Input");
            if (i?.InputName.Text != "Base Material" || !calls.TryGetValue(i.ExpressionName.Text, out var call)) continue;
            var function = call.GetOrDefault<FPackageIndex?>("MaterialFunction")?.Load();
            foreach (var e in function?.GetOrDefault<FPackageIndex[]>("FunctionExpressions", []).Select(x => x?.Load()).OfType<UObject>() ?? [])
            {
                if (e.ExportType != "MaterialExpressionTextureSample") continue;
                var tex = e.GetOrDefault<FPackageIndex?>("Texture")?.ResolvedObject?.GetPathName().Split('.')[0].TrimStart('/');
                if (tex == null) continue;
                var sampler = e.GetOrDefault<FName>("SamplerType").Text;
                var role = sampler.EndsWith("Normal") ? "Normal"
                    : tex.EndsWith("_Diffuse", StringComparison.OrdinalIgnoreCase) ? "Diffuse"
                    : tex.EndsWith("_RMA", StringComparison.OrdinalIgnoreCase) ? "RMA" : null;
                if (role != null) info.Textures.TryAdd(role, tex);
            }
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
    // The weather levels (moon, height fog, sky; the storm): the game loads one from code, CLsm_RainOn when it rolls
    // rain (SCWorldSettings::RainChance).
    public static readonly string[] Weathers = ["/Game/Maps/CrystalLake_Small/CLsm_RainOff", "/Game/Maps/CrystalLake_Small/CLsm_RainOn"];
    // Alternative cabin/spawn layouts; the game streams in one of these per match.
    public static readonly string[] Layouts = [.. Enumerable.Range(1, 4).Select(i => $"/Game/Maps/CrystalLake_Small/CLsm_Design_SL{i}")];
    // The match intro and outro: their staging levels and the J2 sequences (SCWorldSettings IntroSequences /
    // OutroSequences for Jason_SackHead: CLSM_Intro_Master_JTS, CLSM_Outro_JTS); a master's shots bring their
    // sub-sequences.
    public const string IntroLevel = "/Game/Maps/CrystalLake_Small/CLsm_IntroCinematic";
    public const string OutroLevel = "/Game/Maps/CrystalLake_Small/CLsm_OutroCinematic";
    public static readonly string[] Sequences =
    [
        "SummerCamp/Content/Cinematics/NewCinematics/Intro_New_Masters/Intro/CrystalLakeSmall/CLSM_Intro_Master_JTS",
        "SummerCamp/Content/Cinematics/NewCinematics/Outro_New/CrystalLakeSmall/CLSM_Outro_JTS",
    ];
    public static readonly string[] Assets =
    [
        // The light functions' textures (lighting.rs: the flashlight's, the car headlights').
        "/Game/Art/Textures/LightFunctionTest",
        "/Game/Lighting/LightFunctions/T_LightFunction_CarHeadlights_01",
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
        // Jason_Base's screen effects: Shift's BaseTexture1A (PP_Player_Jason_Shift_01), his Stalk and Sense ColorGrades, and his
        // StunScreenEffectMaterial's (PP_Player_StunEffect_01) Stun4 and BaseTexture1A/1B (Stun2).
        "/Game/Effects/Textures/PostProcess/T_Vignette_Simple_01",
        "/Game/Effects/ColorGrades/CG_Stalk_B",
        "/Game/Effects/ColorGrades/CG_Sense",
        "/Game/Effects/Textures/PostProcess/Stun4",
        "/Game/Effects/Textures/PostProcess/Stun2",
        // SenseCounselorMaterial's (PP_Jason_Sense_Counselor_01_MASKED) WispySmoke1A/1B/1C.
        "/Game/Effects/Textures/smoke/T_Wispy_Smoke_Tiled_01",
        // M_JasonV2_PickAxe's base layer (M_JasonV2_PickAxe_Layer's TextureSamples: not parameters, so the mesh export
        // doesn't write them).
        "/Game/Characters/Killers/Jason/J2/Textures/Jason_V2_Pick_Axe_Diffuse",
        "/Game/Characters/Killers/Jason/J2/Textures/Jason_V2_Pick_Axe_Normal",
        "/Game/Characters/Killers/Jason/J2/Textures/Jason_V2_Pick_Axe_RMA",
        // PhoneJunctionBox_BP's OnPhoneBoxDestroyed mesh, and its light material when repaired (RepairedLightMaterial).
        "/Game/Props/Interactive/S_Interactive_TelephoneBox_01_Broken",
        "/Game/Props/Interactive/M_Interactive_TelephoneBox_01_Emissive_Blue",
    ];
    // Classes the game spawns by reference from native code rather than from level data.
    // Classes no level places but the game reads (the cars' wheels: SCWheeledVehicleMovementComponent WheelSetups).
    public static readonly string[] Classes = ["CarKeys_C", "Chevette_FrontWheel_C", "Chevette_BackWheel_C",
        // The police car SCPoliceSpawner spawns (its PoliceSpawnClass).
        "Police_C",
        // Jason's throwing knife (his ThrowingKnifeClass) and his trap (his TrapClass: its TrapScoreEvent).
        "ThrowingKnife_C", "KillerTrap_C",
        // The storm (CLsm_RainOn's).
        "SCZeus_BP_C",
        // Jason under water: his drowning kill and the shape he shows on the surface.
        "Underwater_GrabKill_C", "KillerWaterImage_C",
        // The pickaxe's grab kills (SCKillerCharacter's GrabKills: KillerGrabWidget's icons, DoesKillVolumeFit's KillBounds).
        "Pickaxe_ThroatHack_GrabKill_C", "Pickaxe_Canopener_GrabKill_C", "PickAxe_CurbStomp_GrabKill_C",
        "PickAxe_SweepTheLeg_GrabKill_C", "PickAxe_TopPopper_GrabKill_C", "Choke_GrabKill_C"];
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
        // Jason_AnimBP's GrabMontage: its sections' root motion carries him through the grab.
        "/Game/Characters/Killers/Jason/Base/Animation/Grab/AM_Jason_Enter_Grab_Montage",
        // Her swings (the weapons' NeutralAttack / ForwardAttack, her female takes): their root motion.
        "/Game/Characters/Counselors/Base/Female/Animation/Combat/Swings/AM_F_combat_stand_2h_axe_attack_light",
        "/Game/Characters/Counselors/Base/Female/Animation/Combat/Swings/AM_F_combat_stand_2h_axe_attack_heavy",
        "/Game/Characters/Counselors/Base/Female/Animation/Combat/Swings/AM_F_combat_stand_1h_attack_light",
        "/Game/Characters/Counselors/Base/Female/Animation/Combat/Swings/AM_F_combat_stand_1h_attack_heavy",
    ];
    public const string Counselor = "/Game/Characters/Counselors/Athlete/Athlete_Counselor";
    // Widgets f13-game draws: the counselor's HUD (with the widgets it nests) and the repair and struggle minigames.
    public static readonly string[] Widgets =
    [
        "/Game/UI/HUD/Counselor/CounselorHUDWidget",
        // Created at runtime: an inventory slot, the breath bar while hiding.
        "/Game/UI/HUD/Counselor/SC_InventoryItem_Widget",
        "/Game/UI/HUD/Counselor/SCBreathWidget",
        // XPDisplayWidget's lines (AddNewXPLine), with the category icons its SetIconImage sets.
        "/Game/UI/HUD/XPLineWidget",
        "/Game/UI/Interact/SCInteractMinigame_Widget",
        "/Game/UI/HUD/SC_WiggleMinigame_Widget",
        // Jason's, when he's played.
        "/Game/UI/HUD/Killer/KillerHUDWidget",
        // His notice as an ability unlocks or rage starts.
        "/Game/UI/HUD/Killer/AbilityAvailable",
        // The software cursors (DefaultEngine.ini UserInterfaceSettings): Crosshairs, his Morph map's; Hand, its refusal.
        "/Game/UI/HUD/Minimap/Morph_Area_Cursor",
        "/Game/UI/HUD/Minimap/Morph_Area_Bad_Cursor",
        // Hers once she's out (dead or escaped).
        "/Game/UI/HUD/Spectator/SpectatorHUDWidget",
        // The lobby (its player list's rows: LobbyPlayerListWidget adds one per player).
        "/Game/UI/Lobby/LobbyWidget",
        "/Game/UI/Lobby/LobbyPlayerListItem",
        // Its key hints (an ActionMapWidget and a title).
        "/Game/UI/Controls/ActionDisplay",
        // The minimap's icons (the SCMinimapIconComponents' DefaultIcon, and Jason's KillerIcon).
        "/Game/UI/HUD/Minimap/SC_Cabin_Widget",
        "/Game/UI/HUD/Minimap/SC_Bathroom_Widget",
        "/Game/UI/HUD/Minimap/SC_BoatHouse_Widget",
        "/Game/UI/HUD/Minimap/SC_Garage_Widget",
        "/Game/UI/HUD/Minimap/SC_Camp_Widget",
        "/Game/UI/HUD/Minimap/SC_CarLoc_Widget",
        "/Game/UI/HUD/Minimap/SC_PoliceRescue_Widget",
        "/Game/UI/HUD/Minimap/SC_Killer_Cabin_Widget",
        "/Game/UI/HUD/Minimap/SC_Killer_BathroomBlock_Widget",
        "/Game/UI/HUD/Minimap/SC_Killer_BoatHouse_Widget",
        "/Game/UI/HUD/Minimap/SC_Killer_Garage_Widget",
        "/Game/UI/HUD/Minimap/SC_JasonShack_Killer_Widget",
        "/Game/UI/HUD/Minimap/SC_Killer_Camp_Widget",
        "/Game/UI/HUD/Minimap/SC_Killer_CarLoc_Widget",
        "/Game/UI/HUD/Minimap/SC_PoliceRescue_killer_Widget",
        "/Game/UI/HUD/Minimap/PhoneJunctionBox_Widget",
        "/Game/UI/HUD/Minimap/PowerGridFuseBox_Widget",
        // Jason's own icon on his map (Jason_Base's MinimapIconComponent KillerIcon), and his traps' (KillerTrap's).
        "/Game/UI/HUD/Minimap/PlayerArrow_Widget",
        "/Game/UI/HUD/Minimap/SC_Killer_Trap_Widget",
        // Her objectives and map screen (CounselorHUDWidget's OnShowCounselorMap) and the rows its FindObjectives adds.
        "/Game/UI/HUD/Objectives/ObjectivesWidget",
        "/Game/UI/HUD/Objectives/Objective_Car_Widget",
        "/Game/UI/HUD/Objectives/Objective_Police_Widget",
        // The interaction prompts (SCInteractComponent's InteractionWidgetClass, nesting its InteractionIconWidgetClass).
        "/Game/UI/Interact/ArmoireInteractionIcons",
        "/Game/UI/Interact/BarricadeInteractionIcons",
        "/Game/UI/Interact/BatteryRepairInteractionIcons",
        "/Game/UI/Interact/BearTrapInteractionIcons",
        "/Game/UI/Interact/BedCounselorInteractionIcons",
        "/Game/UI/Interact/BedKillerInteractionIcons",
        "/Game/UI/Interact/BoatDriverInteractionIcons",
        "/Game/UI/Interact/BoatSeatInteractionIcons",
        "/Game/UI/Interact/BoatStartInteractionIcons",
        "/Game/UI/Interact/CarDestroyInteractionIcons",
        "/Game/UI/Interact/CarDoorIntearctionIcons",
        "/Game/UI/Interact/CarDriverInteractionIcons",
        "/Game/UI/Interact/CarStartInteractionIcons",
        "/Game/UI/Interact/CBRadioInteractionIcons",
        "/Game/UI/Interact/DoorInteractionIcons",
        "/Game/UI/Interact/DrawerInteractionIcons",
        "/Game/UI/Interact/DrawerRightInteractionIcons",
        "/Game/UI/Interact/DriverDoorIntearctionIcons",
        "/Game/UI/Interact/EnvKills/ADKChairInteractionIcon",
        "/Game/UI/Interact/EnvKills/BirdBathInteractionIcon",
        "/Game/UI/Interact/EnvKills/BoomBoxInteractionIcon",
        "/Game/UI/Interact/EnvKills/CabinetTableInteractionIcon",
        "/Game/UI/Interact/EnvKills/CampfireInteractionIcon",
        "/Game/UI/Interact/EnvKills/DoorSlamInteractionIcon",
        "/Game/UI/Interact/EnvKills/FirePlaceInteractionIcon",
        "/Game/UI/Interact/EnvKills/GlassWindowThrowInteractionIcon",
        "/Game/UI/Interact/EnvKills/GrabThroughWallInteractionIcon",
        "/Game/UI/Interact/EnvKills/HeadSmashInteractionIcon",
        "/Game/UI/Interact/EnvKills/IronFenceInteractionIcon",
        "/Game/UI/Interact/EnvKills/KitchenKnifeInteractionIcon",
        "/Game/UI/Interact/EnvKills/ScrewdriverInteractionIcon",
        "/Game/UI/Interact/EnvKills/SPInteractionIcon",
        "/Game/UI/Interact/EnvKills/StabInteractionIcon",
        "/Game/UI/Interact/EnvKills/StoveTopInteractionIcon",
        "/Game/UI/Interact/EnvKills/ToiletInteractionIcon",
        "/Game/UI/Interact/EnvKills/TreeArmRipoffInteractionIcon",
        "/Game/UI/Interact/EnvKills/TreeBranchThrowInteractionIcon",
        "/Game/UI/Interact/EnvKills/TreeStumpInteractionIcon",
        "/Game/UI/Interact/EnvKills/UnderWaterInteractionIcon",
        "/Game/UI/Interact/EnvKills/VerticalBackBreakInteractionIcon",
        "/Game/UI/Interact/EnvKills/VoyeurInteractionIcon",
        "/Game/UI/Interact/EnvKills/WallFaceSmashInteractionIcon",
        "/Game/UI/Interact/EnvKills/WallHeadPunchInteractionIcon",
        "/Game/UI/Interact/EnvKills/WallHookInteractionIcon",
        "/Game/UI/Interact/EnvKills/WaterPumpInteractionIcon",
        "/Game/UI/Interact/GasCanRepairInteractionIcons",
        "/Game/UI/Interact/GeneratorRepairInteractionIcons",
        "/Game/UI/Interact/HidingSpotOccupied",
        "/Game/UI/Interact/ItemPickupInteractionIcon",
        "/Game/UI/Interact/JasonTrapInteractionIcons",
        "/Game/UI/Interact/KnifePickupInteractionIcon",
        "/Game/UI/Interact/LockedIcon",
        "/Game/UI/Interact/MapKioskIcon",
        "/Game/UI/Interact/MinigameActionMapWidget",
        "/Game/UI/Interact/OuthouseInteractionIcons",
        "/Game/UI/Interact/ParanoiaOutfitIcon",
        "/Game/UI/Interact/PhoneDisabledIcon",
        "/Game/UI/Interact/PhoneRepairInteractionIcons",
        "/Game/UI/Interact/PhoneUseInteractionIcons",
        "/Game/UI/Interact/PropellerRepairInteractionIcons",
        "/Game/UI/Interact/RadioInteractionIcons",
        "/Game/UI/Interact/SCAttackInteractionWidget",
        "/Game/UI/Interact/SCInteractionDoorWidget",
        "/Game/UI/Interact/SCInteractionWidget",
        "/Game/UI/Interact/TentInteractionIcons",
        "/Game/UI/Interact/WallInteractionIcons",
        "/Game/UI/Interact/WindowInteractionIcons",
    ];
    // Sounds f13-game plays besides the footsteps (SoundCues or SoundWaves).
    /// The map's ambient loops (CrystalLake_Sm_Audio's AmbientSounds).
    public static readonly string[] Sounds =
    [
        "/Game/Sounds/Environment/Lake_AmbienceCue.Lake_AmbienceCue",
        "/Game/Sounds/Environment/Mood_Tone_Ambience.Mood_Tone_Ambience",
        "/Game/Sounds/Environment/SoundWaves/main_campground_ambience_loop.main_campground_ambience_loop",
        // Jason_Base's ShiftAudioCue and TeleportCueCounselorHears.
        "/Game/Sounds/GamePlay/JasonShift/ShiftTunnelCue.ShiftTunnelCue",
        // Cabin_Window_BP's OnWindowDestroyed (PlaySoundAtLocation as the glass breaks).
        "/Game/Sounds/GamePlay/WindowBreak.WindowBreak",
        // Jason_Base's UnderWaterCue (CreateSound2D as he goes under).
        "/Game/Sounds/Swimming/JasonUnderwater_Cue.JasonUnderwater_Cue",
        "/Game/Sounds/GamePlay/VHS_SFXCue.VHS_SFXCue",
        // The world settings' PoliceCalledSFX (the call goes through).
        "/Game/Sounds/GamePlay/PoliceCalled.PoliceCalled",
        // AM_Break_Phone_Box_Machete's (its take is the fuse box break's, whose sound wins by take).
        "/Game/Sounds/GamePlay/PhoneBoxImpact.PhoneBoxImpact",
    ];
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
        // Turning a radio on or off, changing its song (Counselor_UseRadio).
        $"{Shared}/Interaction/Items/AM_normal_stand_use_radio",
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/AM_combat_stand_rifle_fire_01",
        // Aiming a gun (Base_Counselor_AnimBP's RangedAiming machine: the Hunter's enter / exit, the aim idles by
        // GunType) and the AimOffsets poses its RotationOffsetBlendSpace reads at yaw 0 (Aim_C_C its additive base).
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/hunter_Aiming_idle_To_AIm",
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/hunter_Aiming_To_Idle",
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/combat_stand_idle_hunter_01",
        "/Game/Characters/Counselors/Base/Male/Animation/Interaction/Items/m_normal_stand_flare_aim_idle",
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/AimOffset/Aim_B_C",
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/AimOffset/Aim_C_C",
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/AimOffset/Aim_U_L",
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/AimOffset/Aim_U_R",
        // Held items' idle poses (their IdleAsset; a loaded gun's LoadedIdle): the AnimBP's ItemIdleAddtive layers.
        "/Game/Characters/Counselors/Base/Shared/Grip/2x4Grip",
        "/Game/Characters/Counselors/Base/Shared/Grip/BatGrip",
        "/Game/Characters/Counselors/Base/Shared/Grip/FlareGunGrip",
        "/Game/Characters/Counselors/Base/Shared/Grip/PropGrip",
        "/Game/Characters/Counselors/Base/Shared/Grip/UniversalGrip",
        "/Game/Characters/Counselors/Base/Shared/Grip/macheteGrip",
        "/Game/Characters/Counselors/Base/Shared/Grip/pokerGrip",
        "/Game/Characters/Counselors/Base/Shared/Grip/potGrip",
        "/Game/Characters/Counselors/Base/Shared/Grip/stickGrip",
        "/Game/Characters/Counselors/Base/Shared/Additives/normal_stand_carry_heavy_01",
        "/Game/Characters/Counselors/Base/Shared/Additives/normal_stand_idle_carry_beartrap_add",
        "/Game/Characters/Counselors/Base/Male/Animation/Hunter/normal_stand_idle_hunter_01",
        // BS_Run_Lean's samples (Shared/Additives/Leans): the running lean's local-space additives.
        "/Game/Characters/Counselors/Base/Shared/Additives/Leans/normal_stand_run_lean_neutral",
        "/Game/Characters/Counselors/Base/Shared/Additives/Leans/normal_stand_run_lean_left",
        "/Game/Characters/Counselors/Base/Shared/Additives/Leans/normal_stand_run_lean_right",
        "/Game/Characters/Counselors/Base/Shared/Additives/Leans/normal_stand_sprint_lean_left",
        "/Game/Characters/Counselors/Base/Shared/Additives/Leans/normal_stand_sprint_lean_right",
        // Their additive base (RefPoseType ABPT_AnimFrame, frame 0).
        "/Game/Characters/Counselors/Base/Male/Animation/Locomotion/M_normal_stand_run_forward",
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
        // In the car: sitting (driving, steering, idle), getting in and out on the driver's side and the passenger's.
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_drive_forward",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_drive_forward_turnleft",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_drive_forward_turnright",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_ride_idle",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_enter_left",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_exit_left",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_enter_right",
        "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_exit_right",
        // The barricade: pulling it down and pushing it up (Counselor_Barricade_Pulldown / PushUp; their UseBarricade
        // driver windows).
        $"{Shared}/Interaction/Barricade/AM_normal_stand_barricade_pulldown",
        $"{Shared}/Interaction/Barricade/AM_normal_stand_barricade_pushup",
        // Searching a cabinet (Counselor_OpenDrawer / OpenDrawerLow): open, idle, close, take.
        $"{Shared}/Interaction/Search/Drawer/AM_normal_stand_drawer_open_mid",
        $"{Shared}/Interaction/Search/Drawer/AM_normal_stand_drawer_open_low",
        // Dropping items (Base_Counselor_AnimBP's DropLargeItemMontage / DropSmallItemMontage; their DropLargeItem /
        // DropSmallItem notifies drop them).
        $"{Shared}/Interaction/Items/AM_normal_stand_drop_item_right",
        $"{Shared}/Interaction/Items/AM_normal_stand_drop_item_left",
        // Pulled out of the car by Jason (the driver seat's CounselorGrabbedMontage).
        CarGrab[0],
        // The phone box's fuse (AM_normal_stand_repair_phonebox) and calling the police (AM_normal_stand_use_phone).
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Misc/f_normal_stand_repair_phonebox_enter",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Misc/f_normal_stand_repair_phonebox_cycle",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Misc/f_normal_stand_repair_phonebox_exit",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Misc/f_normal_stand_repair_phonebox_fail",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Electronics/f_normal_stand_use_phone_start",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Electronics/f_normal_stand_use_phone_cycle",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Electronics/f_normal_stand_use_phone_stop",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Electronics/f_normal_stand_use_phone_cancel",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Electronics/f_normal_stand_use_phone_fail",
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
    // Jason_SackHead's own grab kill (GrabKillClasses[0], inherited from Jason_Base): Choke_GrabKill's Jason_Choke_CK /
    // Counselor_Choke_CK.
    static string[] CounselorKills => [.. PickaxeKills.Select(k => $"{Shared}/Cinematic/AM_cinematic_{k}"), .. PlacedKillsHers.Select(m => $"{Shared}/Cinematic/{m}"), $"{Shared}/Cinematic/AM_cinematic_hand_choke"];
    /// The map's placed (environment) kills' montages: their ContextKillComponents' special moves' ContextMontage, the
    /// victim's (Counselor_*) and Jason's (Jason_*).
    static string[] PlacedKillsHers => ["AM_Counselor_cinematic_swim_grabkill_01", "AM_cinematic_adirondackchair_kill", "AM_cinematic_fireplace", "AM_cinematic_gravestone_headsmash", "AM_cinematic_hand_birdbath", "AM_cinematic_hand_ironfence", "AM_cinematic_hand_table", "AM_cinematic_hand_tree_arm_ripoff", "AM_cinematic_hand_treestump_faceslam", "AM_cinematic_hand_waterpump", "AM_cinematic_stick_campfire", "AM_cinematic_surface_vertical_backbreak", "AM_cinematic_treebranch_chesthole_01", "AM_cinematic_wall_facesmash", "AM_cinematic_wall_headpunch_01"];
    static string[] PlacedKillsHis => ["AM_Jason_cinematic_swim_grabkill_01", "AM_Jason_cinematic_gravestone_headsmash_01", "AM_Jason_cinematic_surface_vertical_backbreak", "AM_cinematic_f_adirondackchair_kill_01", "AM_cinematic_fireplace", "AM_cinematic_hand_birdbath", "AM_cinematic_hand_ironfence", "AM_cinematic_hand_table", "AM_cinematic_hand_tree_arm_ripoff", "AM_cinematic_hand_treestump_faceslam", "AM_cinematic_hand_waterpump", "AM_cinematic_m_wall_facesmash_01", "AM_cinematic_stick_campfire", "AM_cinematic_treebranch_chesthole_01", "AM_cinematic_wall_headpunch_01"];
    const string JasonAnim = "/Game/Characters/Killers/Jason/Base/Animation";
    static string[] JasonKills => [.. PickaxeKills.Select(k => $"{JasonAnim}/Cinematic/AM_cinematic_{k}"), .. PlacedKillsHis.Select(m => $"{JasonAnim}/Cinematic/{m}"), $"{JasonAnim}/Cinematic/AM_Jason_cinematic_hand_choke"];
    const string JasonStun = $"{JasonAnim}/Deaths_HitReacts_Stuns";
    public static string[] PairedKills => [.. CounselorKills, .. JasonKills];
    /// Getting in and out of the driver's seat and the passenger's (played from the seat's socket).
    static string[] CarSeat => ["/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_enter_left", "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_exit_left", "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_stand_car_enter_right", "/Game/Characters/Counselors/Base/Female/Animation/Car/f_normal_sit_car_exit_right"];
    /// The driver pulled out of the car: her side and his (paired, played from DriverGrab).
    static string[] CarGrab => ["/Game/Characters/Counselors/Base/Shared/Car/AM_normal_stand_car_grab_left", $"{JasonAnim}/Car/AM_normal_stand_car_grab_left"];
    /// Through a window: her climb, and the dive and its landing (played from the window's special move spots).
    static string[] WindowMoves =>
    [
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Windows/f_normal_stand_window_climb_01",
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Windows/f_normal_stand_window_dive",
        $"{Shared}/Interaction/Window/AM_normal_stand_window_jump",
    ];
    /// Animations that keep their root translation (played from a fixed spot: paired kills, hiding spots, car seats,
    /// windows).
    public static string[] KeepRoot => [.. PairedKills, .. CounselorHiding, .. JasonHiding, .. CarSeat, .. CarGrab, .. WindowMoves, .. DoorMoves, .. RepairMoves];
    /// The repair takes that carry root motion (bEnableRootMotion: the phone box's, the phone's, the gas can's cancel).
    static string[] RepairMoves =>
    [
        .. new[] { "enter", "cycle", "exit" }.Select(m => $"/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Misc/f_normal_stand_repair_phonebox_{m}"),
        .. new[] { "start", "cycle", "stop", "cancel", "fail" }.Select(m => $"/Game/Characters/Counselors/Base/Female/Animation/Interaction/Electronics/f_normal_stand_use_phone_{m}"),
        "/Game/Characters/Counselors/Base/Female/Animation/Interaction/Repair/Car/f_normal_stand_car_repair_gascan_cancel",
    ];
    /// Her door moves (open, close, lock, unlock; played from the door's special move spots).
    static string[] DoorMoves => [.. new[] { "open_inside_forward", "open_outside_forward", "close_inside_forward", "close_outside_forward", "lock", "unlock" }
        .Select(m => $"/Game/Characters/Counselors/Base/Female/Animation/Interaction/Doors/f_normal_stand_door_{m}")];
    public const string Killer = "/Game/Characters/Killers/Jason/J2/Jason_SackHead";
    /// The kills whose cameras f13-game plays (kill_cameras.json).
    /// The sound mixes f13-game pushes (Jason_Base's: Shift, Stalk, under water, what a counselor hears of a teleport).
    public static readonly string[] SoundMixes =
    [
        "/Game/Sounds/SoundMixes/JasonShiftSoundMix",
        "/Game/Sounds/SoundMixes/JasonStalkSoundMix",
        "/Game/Sounds/SoundMixes/JasonUnderwaterSoundMix",
        "/Game/Sounds/SoundMixes/TeleportCounselorHearsSoundMix",
    ];

    public static readonly string[] KillCameras =
    [
        "/Game/Blueprints/ContextKills/Pickaxe_ThroatHack_GrabKill",
        "/Game/Blueprints/ContextKills/Pickaxe_Canopener_GrabKill",
        "/Game/Blueprints/ContextKills/PickAxe_CurbStomp_GrabKill",
        "/Game/Blueprints/ContextKills/PickAxe_SweepTheLeg_GrabKill",
        "/Game/Blueprints/ContextKills/PickAxe_TopPopper_GrabKill",
        "/Game/Blueprints/ContextKills/Choke_GrabKill",
        // The placed kills' (their SCCameraSpline, under the ContextKillComponent).
        "/Game/Blueprints/ContextKills/Birdbath_PlacedKill",
        "/Game/Blueprints/ContextKills/HeadSmash_PlacedKill",
        "/Game/Blueprints/ContextKills/IronFence_PlacedKill",
        "/Game/Blueprints/ContextKills/TreeArmRipoff_PlacedKill",
        "/Game/Blueprints/ContextKills/Vertical_BackBreak_PlacedKil",
        "/Game/Blueprints/ContextKills/Wall_FaceSmash_PlacedKill",
        "/Game/Blueprints/ContextKills/WaterPump_PlacedKill",
        "/Game/Blueprints/ContextKills/AdirondackChair_PlacedKill",
        "/Game/Blueprints/ContextKills/LightPostArmRipoff_PlacedKill",
        "/Game/Blueprints/ContextKills/Stick_Campfire_PlacedKill",
        "/Game/Blueprints/ContextKills/TreeStumpSlam_PlacedKill",
        "/Game/Blueprints/ContextKills/Fireplace_PlacedKill",
        "/Game/Blueprints/ContextKills/Table_Placedkill",
        "/Game/Blueprints/ContextKills/TreeBranch_PlacedKill",
        "/Game/Blueprints/ContextKills/WallHeadPunch_PlacedKill",
        // Jason's drowning kill (his UnderwaterContextKillClass).
        "/Game/Blueprints/ContextKills/Underwater_GrabKill",
    ];
    // Jason_AnimBP's BS_Jason_Normal_Move (SpeedState 0 idle, 1 walk, 2 fast walk, 3 run), the pickaxe's combat idle,
    // attacks (PickAxe's NeutralAttack and ForwardAttack), the grab, and the grab kill.
    /// The placed kills' props that play their own animation as the kill starts (SetAnimatedKillMesh in their
    /// Blueprints' BeginPlay; SCContextKillActor::KillStarted plays it).
    public static readonly (string Mesh, string Anim)[] PlacedKillProps =
    [
        ("/Game/Props/SkeletalMeshes/General/birdbath_rig", "/Game/Props/SkeletalMeshes/General/Animation/cinematic_hand_birdbath"),
        ("/Game/Props/SkeletalMeshes/Interaction/A_Interactive_WaterPump_01", "/Game/Props/SkeletalMeshes/Interaction/Animation/A_Interactive_WaterPump_01_Kill"),
        ("/Game/Props/SkeletalMeshes/General/adirondack_chair", "/Game/Props/SkeletalMeshes/General/Animation/cinematic_f_adirondackchair_kill_01"),
    ];
    /// The tent's own animations (its flaps as she goes in and out, and while she hides).
    public const string TentMesh = "/Game/Props/SkeletalMeshes/Camp/A_Camp_Tent_Yellow_01_LOD0";
    public static readonly string[] TentAnims =
    [
        "/Game/Props/SkeletalMeshes/Camp/Animation/normal_stand_hidespot_tent_enter",
        "/Game/Props/SkeletalMeshes/Camp/Animation/normal_stand_hidespot_tent_exit",
        "/Game/Props/SkeletalMeshes/Camp/Animation/cinematic_hidespot_tent_idle",
    ];
    /// The bear trap's own animations (BearTrap_C ArmedAnimation, TriggeredAnimation, EscapedAnimation; its idles).
    // The Chevette's hood (Chevette_AnimBP: bIsHoodOpen's idle_open loop on the Hood branch; CloseHoodAnim's take).
    public const string CarMesh = "/Game/Animations/Props/Manmade/Vehicles/A_Vehicle_Chevette_LOD0";
    public static readonly string[] CarAnims =
    [
        "/Game/Props/SkeletalMeshes/Vehicles/Cars/Chevette/normal_stand_car_repair_hood_idle_open",
        "/Game/Props/SkeletalMeshes/Vehicles/Cars/Chevette/normal_stand_car_repair_hood_close",
    ];
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
        // AM_Jason_Enter_Grab_Montage's GrabMissed and GrabSuccess sections (its Default is normal_grab_enter_1).
        $"{JasonAnim}/Grab/normal_grab_fail",
        $"{JasonAnim}/Grab/normal_grab_Success_2",
        $"{JasonAnim}/Grab/AM_Jason_normal_grab_break",
        // His block (Jason_AnimBP's BlockMontages) and the reactions to a blocked hit (BlockingHitMontages).
        $"{JasonAnim}/Combat/Block/AM_Jason_Combat_Block",
        $"{JasonAnim}/Combat/Block/AM_combat_stand_1h_block_reaction_01",
        $"{JasonAnim}/Combat/Block/AM_combat_stand_1h_block_reaction_02",
        // Stuns (BaseWeaponStun_C / FlareWeaponStun_C, FirecrackerKillerStun_C, the pocket knife, bear traps) and
        // hit reactions.
        $"{JasonStun}/Stun/AM_combat_Falling_stun",
        $"{JasonStun}/Stun/AM_combat_stand_stun_left_01",
        $"{JasonStun}/Stun/AM_combat_stand_stun_right_01",
        $"{JasonStun}/Stun/AM_combat_stand_stun_back_01",
        $"{JasonStun}/Stun/AM_combat_stand_stunned_firecracker",
        $"{JasonStun}/Stun/AM_normal_stand_pocketKnife_Stun",
        // The police shooting him back out of their escape (PoliceShootJasonStun).
        $"{JasonStun}/Stun/AM_combat_police_stun",
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
        // Breaking the phone box (Jason_PhoneBox_Destruction_Machete).
        $"{JasonAnim}/Interaction/AM_Break_Phone_Box_Machete",
        // Breaking a radio (Jason_Radio_Destruction_Axe: his weapon isn't the machete).
        $"{JasonAnim}/Interaction/AM_Break_Radio_Axe",
        // Picking up throwing knives (Jason_ThrowingKnifePickup's ContextMontage: its KnifePickup driver window).
        $"{JasonAnim}/Interaction/AM_normal_stand_throwingknife_grab",
        // Pulling the driver out (DriverGrab's paired move); stopping a car dead.
        CarGrab[1],
        $"{JasonAnim}/Car/AM_normal_stand_car_stop_slam",
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
    /// Blocks the camera channel (the spring arms' probe).
    public bool BlocksCamera = true;
    /// Blocks traces on WorldDynamic (interactables' occlusion).
    public bool BlocksWorldDynamic = true;
    /// Blocks the Vehicle channel (a car's body).
    public bool BlocksVehicle = true;
    public bool BlocksVisibility = true;
    /// Casts shadows (CastShadow; baked ones included, which f13-game's dynamic sun stands in for).
    public bool CastShadow = true;
    /// Not drawn past this distance from the camera (cm; 0 = always drawn).
    public int CullDistance;
    /// A SplineMeshComponent's spline (component space, cm): its mesh is bent along it (SplineMeshSpline).
    public Newtonsoft.Json.Linq.JObject? Spline;
}
class ActorInfo
{
    public string Level = "", Name = "", Class = "";
    public Xform? Transform;
    /// The actor its root is attached to, and the socket.
    public string? AttachParent, AttachSocket;
    public Newtonsoft.Json.Linq.JObject? Props;
    public List<Newtonsoft.Json.Linq.JObject>? Components;
    /// BlockingVolumes: their convex hulls as world-space point sets (cm).
    public List<float[]>? Hulls;
    /// Whether the hulls block counselors (Pawn) and the camera channel.
    public bool HullsBlockPawn = true, HullsBlockCamera = true, HullsBlockWorldDynamic = true, HullsBlockVehicle = true;
    /// NavMeshBoundsVolumes and the minimap volume: the world-space box their brush covers (cm: min x, y, z, max x, y, z).
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
