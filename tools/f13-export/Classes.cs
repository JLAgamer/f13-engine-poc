// Blueprint class descriptions: every component a class spawns with (native default subobjects, construction-script
// nodes, inherited nodes with their per-class overrides), positioned in actor space, plus the class defaults. The
// game needs these for actors that are spawned at runtime (doors, cabinets, items, vehicles), which a level file
// does not contain.
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Component;
using CUE4Parse.UE4.Assets.Exports.Component.StaticMesh;
using CUE4Parse.UE4.Assets.Exports.Component.SkeletalMesh;
using CUE4Parse.UE4.Assets.Exports.SkeletalMesh;
using CUE4Parse.UE4.Assets.Exports.Animation;
using CUE4Parse.UE4.Assets.Exports.StaticMesh;
using CUE4Parse.UE4.Objects.Core.Math;
using CUE4Parse.UE4.Objects.Engine;
using CUE4Parse.UE4.Objects.UObject;

class ClassExporter(DefaultFileProvider provider, Action<UObject> queueMesh, Func<UObject, string> meshFile)
{
    public readonly SortedDictionary<string, JObject> Classes = new(StringComparer.OrdinalIgnoreCase);
    readonly Queue<UObject> pending = new();
    readonly HashSet<string> seen = new(StringComparer.OrdinalIgnoreCase);

    /// Queues a class (a UClass, or a ResolvedObject/FPackageIndex pointing at one) and, transitively, every class it
    /// can spawn.
    public void Add(UObject? cls)
    {
        if (cls is UClass c && seen.Add(c.Name)) pending.Enqueue(c);
    }
    public void AddNative(string name)
    {
        if (seen.Add(name)) Classes[name] = new JObject { ["Name"] = name, ["Native"] = name };
    }
    public void Add(FPackageIndex? index)
    {
        if (index == null || index.IsNull) return;
        UObject? o = null;
        try { o = index.Load(); } catch { }
        if (o is UClass) Add(o);
        else if (!index.Name.EndsWith("_C") && index.Name.Length > 0) AddNative(index.Name); // native class: name only
    }

    public void Run()
    {
        while (pending.Count > 0)
        {
            var cls = pending.Dequeue();
            try { Classes[cls.Name] = Describe((UClass)cls); }
            catch (Exception ex) { Console.WriteLine($"  class {cls.Name}: {ex.Message}"); }
        }
    }

    JObject Describe(UClass cls)
    {
        var info = new JObject { ["Name"] = cls.Name, ["Path"] = cls.GetPathName() };
        // Super chain: Blueprint parents, ending at the first native class.
        var chain = new List<UClass>();
        var supers = new JArray();
        for (UClass? c = cls; c != null;)
        {
            chain.Add(c);
            var sup = c.SuperStruct;
            if (sup == null || sup.IsNull) break;
            UObject? s = null;
            try { s = sup.Load(); } catch { }
            if (s is UBlueprintGeneratedClass bp) { supers.Add(bp.Name); c = bp; }
            else { info["Native"] = sup.Name; break; }
        }
        if (cls is not UBlueprintGeneratedClass) info["Native"] = cls.Name;
        info["Supers"] = supers;

        // Class defaults, base first so that subclasses override.
        var props = new JObject();
        for (var i = chain.Count - 1; i >= 0; i--)
        {
            var cdo = Load(chain[i].ClassDefaultObject);
            if (cdo != null) Merge(props, Sanitize(PropsOf(cdo)));
            // Doors' damage states (SCDoor's TakeDamage swaps the leaf for these): exported as meshes, by file.
            foreach (var dm in new[] { "LightDestructionMesh", "MediumDestructionMesh", "HeavyDestructionMesh" })
                if (cdo != null && Load(cdo.GetOrDefault<FPackageIndex?>(dm)) is USkeletalMesh mesh)
                {
                    queueMesh(mesh);
                    props[dm + "File"] = meshFile(mesh);
                }
        }
        info["Props"] = props;

        if (cls is not UBlueprintGeneratedClass) return info;
        var comps = Components(cls, chain);
        info["Components"] = comps;
        return info;
    }

    record Comp(string Name, UObject Template, string? Parent);

    JArray Components(UClass cls, List<UClass> chain)
    {
        var comps = new Dictionary<string, Comp>(StringComparer.OrdinalIgnoreCase);
        var order = new List<string>();
        void Put(string name, UObject t, string? parent)
        {
            if (comps.ContainsKey(name)) return;
            comps[name] = new Comp(name, t, parent);
            order.Add(name);
        }

        // Native default subobjects of the most derived class default object (each CDO serializes its own copies).
        var cdo = Load(cls.ClassDefaultObject);
        string? root = null;
        if (cdo != null)
        {
            root = cdo.GetOrDefault<FPackageIndex?>("RootComponent")?.Name;
            foreach (var e in cdo.Owner!.GetExports())
            {
                if (e.Outer?.Name.Text != cdo.Name || !IsComponent(e)) continue;
                Put(e.Name, e, e.GetOrDefault<FPackageIndex?>("AttachParent")?.Name);
            }
        }

        // Construction-script nodes of this class and its Blueprint parents; a subclass may override an inherited
        // node's template through its InheritableComponentHandler.
        var overrides = new Dictionary<string, UObject>(StringComparer.OrdinalIgnoreCase);
        foreach (var c in chain.OfType<UBlueprintGeneratedClass>())
        {
            var ich = Load(c.InheritableComponentHandler);
            foreach (var rec in ich?.GetOrDefault<CUE4Parse.UE4.Assets.Objects.FStructFallback[]>("Records", []) ?? [])
            {
                var key = rec.GetOrDefault<CUE4Parse.UE4.Assets.Objects.FStructFallback?>("ComponentKey");
                var name = key?.GetOrDefault<FName>("SCSVariableName").Text;
                var t = Load(rec.GetOrDefault<FPackageIndex?>("ComponentTemplate"));
                if (name != null && t != null) overrides.TryAdd(name, t);
            }
        }
        foreach (var c in chain.OfType<UBlueprintGeneratedClass>())
        {
            var scs = Load(c.SimpleConstructionScript);
            if (scs == null) continue;
            var nodes = scs.GetOrDefault<FPackageIndex[]>("RootNodes", []).ToList();
            var defRoot = scs.GetOrDefault<FPackageIndex?>("DefaultSceneRootNode");
            if (defRoot != null && !defRoot.IsNull) nodes.Add(defRoot);
            var visited = new HashSet<string>();
            void Walk(FPackageIndex idx, string? parent)
            {
                var node = Load(idx);
                if (node == null || !visited.Add(node.Name)) return;
                var name = node.GetOrDefault<FName>("InternalVariableName").Text;
                var t = overrides.GetValueOrDefault(name) ?? Load(node.GetOrDefault<FPackageIndex?>("ComponentTemplate"));
                parent ??= node.GetOrDefault<FName>("ParentComponentOrVariableName").Text is { Length: > 0 } p && p != "None" ? p : null;
                if (t != null) Put(name, t, parent);
                foreach (var child in node.GetOrDefault<FPackageIndex[]>("ChildNodes", [])) Walk(child, name);
            }
            foreach (var n in nodes) Walk(n, null);
        }

        // An actor without a native root uses its first construction-script root.
        root ??= order.FirstOrDefault(n => comps[n].Parent == null && IsScene(comps[n].Template));

        var local = new Dictionary<string, FTransform>(StringComparer.OrdinalIgnoreCase);
        FTransform ActorSpace(string name, int depth = 0)
        {
            if (local.TryGetValue(name, out var t)) return t;
            var c = comps[name];
            var rel = Relative(c.Template);
            // The root's own relative transform is applied when the actor spawns; actor space starts at the root.
            if (name.Equals(root, StringComparison.OrdinalIgnoreCase) || depth > 32) t = FTransform.Identity;
            else if (c.Parent != null && comps.ContainsKey(c.Parent)) t = rel * ActorSpace(c.Parent, depth + 1);
            else if (root != null && comps.ContainsKey(root)) t = rel; // attached to the root by default
            else t = rel;
            local[name] = t;
            return t;
        }

        var arr = new JArray();
        foreach (var name in order)
        {
            var c = comps[name];
            var j = Describe(c.Template, name, c.Parent);
            if (j == null) continue;
            if (IsScene(c.Template)) j["Local"] = Xf(ActorSpace(name));
            if (name.Equals(root, StringComparison.OrdinalIgnoreCase))
            {
                j["Root"] = true;
                j["RootRelative"] = Xf(Relative(c.Template));
            }
            arr.Add(j);
        }
        return arr;
    }

    /// One component: type, mesh, shape, spawner list and other gameplay properties (merged over its archetypes).
    public JObject? Describe(UObject t, string name, string? parent)
    {
        var type = t.ExportType;
        var j = new JObject { ["Name"] = name, ["Type"] = type };
        if (parent != null) j["Parent"] = parent;
        switch (t)
        {
            case UStaticMeshComponent:
            {
                var mesh = Load(t.GetOrDefault<FPackageIndex?>("StaticMesh")) as UStaticMesh;
                if (mesh != null) { queueMesh(mesh); j["Mesh"] = meshFile(mesh); }
                j["Materials"] = new JArray(t.GetOrDefault<FPackageIndex[]>("OverrideMaterials", []).Select(m => m?.ResolvedObject?.GetPathName()));
                if (mesh != null && MeshMaterials.OverrideMap(mesh, t.GetOrDefault<FPackageIndex[]>("OverrideMaterials", []), queueMesh) is { } map)
                    j["MaterialMap"] = JObject.FromObject(map);
                j["Hidden"] = t.GetOrDefault("bHiddenInGame", false) || !t.GetOrDefault("bVisible", true);
                j["BlocksPawn"] = Collision.BlocksPawn(t);
                j["BlocksCamera"] = Collision.BlocksCamera(t);
                j["QueriedByWorldDynamic"] = Collision.QueriedByWorldDynamic(t);
                return j;
            }
            case USkeletalMeshComponent:
            case not null when type == "DestructibleComponent": // door leaves: a UDestructibleMesh is a skeletal mesh
            {
                var mesh = Load(t.GetOrDefault<FPackageIndex?>("SkeletalMesh")) as USkeletalMesh;
                if (mesh != null) { queueMesh(mesh); j["Mesh"] = meshFile(mesh); }
                if (mesh != null && Sockets(mesh) is { Count: > 0 } sockets) j["Sockets"] = sockets;
                j["Hidden"] = t.GetOrDefault("bHiddenInGame", false) || !t.GetOrDefault("bVisible", true);
                return j;
            }
        }
        var props = new JObject();
        var archetypes = new List<UObject>();
        for (UObject? a = t; a != null && archetypes.Count < 16; a = a.Template?.Load()) archetypes.Add(a);
        for (var i = archetypes.Count - 1; i >= 0; i--) Merge(props, Sanitize(PropsOf(archetypes[i])));
        foreach (var p in new[] { "AttachParent", "RelativeLocation", "RelativeRotation", "RelativeScale3D", "CreationMethod", "bNetAddressable" })
            props.Remove(p);
        // Shape defaults from the engine constructors, for shapes that keep them.
        if (type.Contains("SphereComponent")) props["SphereRadius"] ??= 32f;
        if (type.Contains("BoxComponent")) props["BoxExtent"] ??= new JObject { ["X"] = 32f, ["Y"] = 32f, ["Z"] = 32f };
        if (type.Contains("CapsuleComponent")) { props["CapsuleRadius"] ??= 22f; props["CapsuleHalfHeight"] ??= 44f; }
        // Shapes that block counselors, as convex point sets in component space (cm). Shapes keeping their native
        // constructor's profile (not in the export) are taken as non-blocking (triggers, volumes).
        // Spacer capsules (doorways, windows, car seats) only block while an interaction pushes characters clear.
        if (!type.Contains("SpacerCapsule") && Collision.ShapeComponent(t, FVector.OneVector) is { } hull && Collision.BlocksPawn(t, "OverlapAllDynamic"))
        {
            j["Hull"] = new JArray(hull);
            j["HullBlocksCamera"] = Collision.BlocksCamera(t, "OverlapAllDynamic");
        }
        // Shapes an overlap query on WorldDynamic finds (a barricade's BeginPlay finds its door that way).
        if (Collision.ShapeComponent(t, FVector.OneVector) is { } shape && Collision.QueriedByWorldDynamic(t, "OverlapAllDynamic"))
            j["QueryHull"] = new JArray(shape);
        foreach (var cls in t.GetOrDefault<FPackageIndex[]>("ActorList", [])) Add(cls);
        Add(t.GetOrDefault<FPackageIndex?>("ChildActorClass"));
        foreach (var cls in t.GetOrDefault<FPackageIndex[]>("RequiredPartClasses", [])) Add(cls);
        if (props.Count > 0) j["Props"] = props;
        return j;
    }

    /// The mesh's sockets (its skeleton's, the mesh's own winning) in component space, through their bones'
    /// reference pose: where seats attach (SCVehicleSeatComponent's AttachSocket).
    static JObject Sockets(USkeletalMesh mesh)
    {
        var skeleton = mesh.Skeleton.Load<USkeleton>();
        var bones = mesh.ReferenceSkeleton.FinalRefBoneInfo;
        var pose = mesh.ReferenceSkeleton.FinalRefBonePose;
        var j = new JObject();
        var all = (skeleton?.GetOrDefault<FPackageIndex[]>("Sockets", []) ?? []).Concat(mesh.GetOrDefault<FPackageIndex[]>("Sockets", []));
        foreach (var s in all.Select(s => s.Load()))
        {
            if (s == null) continue;
            var bone = s.GetOrDefault<FName>("BoneName").Text;
            var i = Array.FindIndex(bones, b => b.Name.Text == bone);
            var xf = new FTransform(s.GetOrDefault<FRotator>("RelativeRotation").Quaternion(), s.GetOrDefault<FVector>("RelativeLocation"), s.GetOrDefault("RelativeScale", FVector.OneVector));
            for (; i >= 0; i = bones[i].ParentIndex) xf = xf * pose[i];
            j[s.GetOrDefault<FName>("SocketName").Text] = Xf(xf);
        }
        return j;
    }

    static bool IsComponent(UObject e) => e.ExportType.EndsWith("Component") || e is UActorComponent || e.ExportType is "SCSplineCamera";
    static bool IsScene(UObject e) => e is USceneComponent || e.GetOrDefault<FPackageIndex?>("AttachParent") != null
        || e.ExportType.EndsWith("Component") && !e.ExportType.Contains("Movement") && !e.ExportType.Contains("Inventory")
        || e.ExportType == "SCSplineCamera"; // a CameraComponent (interaction and kill cameras)

    public static FTransform Relative(UObject t)
    {
        var loc = t.GetOrDefault("RelativeLocation", FVector.ZeroVector);
        var rot = t.GetOrDefault("RelativeRotation", FRotator.ZeroRotator);
        var scale = t.GetOrDefault("RelativeScale3D", FVector.OneVector);
        return new FTransform(rot.Quaternion(), loc, scale);
    }

    public static JObject Xf(FTransform t) => new()
    {
        ["T"] = new JArray(t.Translation.X, t.Translation.Y, t.Translation.Z),
        ["R"] = new JArray(t.Rotation.X, t.Rotation.Y, t.Rotation.Z, t.Rotation.W),
        ["S"] = new JArray(t.Scale3D.X, t.Scale3D.Y, t.Scale3D.Z),
    };

    static UObject? Load(FPackageIndex? i)
    {
        if (i == null || i.IsNull) return null;
        try { return i.Load(); } catch { return null; }
    }

    public static JObject PropsOf(UObject o) =>
        JObject.Parse(JsonConvert.SerializeObject(o))["Properties"] as JObject ?? new JObject();

    static void Merge(JObject into, JObject from)
    {
        foreach (var p in from.Properties()) into[p.Name] = p.Value;
    }

    /// Keeps values the game logic can use: scalars, enums, text, structs of those, and class/asset references
    /// (as their names). References to subobjects (components, default objects) are dropped.
    public static JObject Sanitize(JObject o)
    {
        var r = new JObject();
        foreach (var p in o.Properties())
        {
            var v = SanitizeToken(p.Value);
            if (v != null) r[p.Name] = v;
        }
        return r;
    }

    static JToken? SanitizeToken(JToken t)
    {
        switch (t)
        {
            case JValue v: return v.Type == JTokenType.Null ? null : v;
            case JArray a:
            {
                var r = new JArray();
                foreach (var x in a) { var s = SanitizeToken(x); if (s != null) r.Add(s); }
                return r;
            }
            case JObject o when o["ObjectName"] is JValue name:
            {
                var s = (string?)name ?? "";
                var m = System.Text.RegularExpressions.Regex.Match(s, @"^(\w+)'(.*)'$");
                if (!m.Success) return null;
                var (kind, path) = (m.Groups[1].Value, m.Groups[2].Value);
                // Another actor in the level (e.g. a fuse box's PowerGrid volume): by name.
                var actor = System.Text.RegularExpressions.Regex.Match(path, @"^[^:]+:PersistentLevel\.([^.:]+)$");
                if (actor.Success) return $"{kind}'{actor.Groups[1].Value}'";
                if (path.Contains("Default__") || path.Contains(':') || path.Contains('.')) return null; // subobject
                return kind.EndsWith("Class") ? path : $"{kind}'{path}'";
            }
            case JObject o:
                return Sanitize(o) is { Count: > 0 } so ? so : null;
        }
        return null;
    }
}
