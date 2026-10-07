// Collision data for f13-game's character movement.
//
// collision.json: per exported static mesh (glTF path), its simple collision as convex point sets in mesh space (cm):
// convex elements as cooked (VertexData; the cooked hull faces aren't stored), boxes as their 8 corners, spheres and
// capsules (sphyls) sampled. "ComplexAsSimple" meshes (CTF_UseComplexAsSimple) collide with their render triangles.
// Meshes with neither collide with nothing, as for Unreal's character sweeps (which use simple collision).
//
// Whether a component blocks a counselor comes from its collision profile (the game's DefaultEngine.ini) or its
// custom responses: blocking needs query collision and a Block response on the Pawn channel.
//
// "Surface": the mesh's physical surface type (EPhysicalSurface: SurfaceType1..; 0 the default), from its body setup's
// PhysMaterial, else its first material's (up its parents): footsteps pick their sound by it.
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.StaticMesh;
using CUE4Parse.UE4.Assets.Objects;
using CUE4Parse.UE4.Objects.Core.Math;
using CUE4Parse.UE4.Objects.PhysicsEngine;
using CUE4Parse.UE4.Objects.UObject;

static class Collision
{
    public class MeshShapes
    {
        public List<float[]> Hulls = new();
        public bool ComplexAsSimple;
        public int Surface;
    }

    /// A PhysicalMaterial's SurfaceType number (0: none or the default).
    public static int SurfaceOf(UObject? physMat)
    {
        if (physMat == null) return 0;
        var s = Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(physMat))["Properties"]?["SurfaceType"]?.ToString();
        var m = s == null ? null : System.Text.RegularExpressions.Regex.Match(s, @"SurfaceType(\d+)");
        return m is { Success: true } ? int.Parse(m.Groups[1].Value) : 0;
    }

    /// A material's PhysMaterial, looked up its parents.
    public static UObject? MaterialPhysMat(UObject? material)
    {
        for (var depth = 0; material != null && depth < 8; depth++)
        {
            if (material.GetOrDefault<FPackageIndex?>("PhysMaterial")?.Load<UObject>() is { } pm) return pm;
            material = material.GetOrDefault<FPackageIndex?>("Parent")?.Load<UObject>();
        }
        return null;
    }

    /// Profile name -> (query collision enabled, Pawn channel response).
    static readonly Dictionary<string, (bool Query, string Pawn)> profiles = new(StringComparer.OrdinalIgnoreCase);
    static readonly HashSet<string> unknown = new(StringComparer.OrdinalIgnoreCase);
    /// How often each profile decided BlocksPawn (summary for the log).
    public static readonly SortedDictionary<string, int> Used = new();

    /// Reads the collision profiles from the game's DefaultEngine.ini ("+Profiles=" lines).
    public static void LoadProfiles(string game)
    {
        var ini = Path.Combine(game, "SummerCamp", "Config", "DefaultEngine.ini");
        if (!File.Exists(ini)) { Console.WriteLine($"no {ini}: every collision profile is treated as blocking"); return; }
        foreach (var line in File.ReadLines(ini))
        {
            if (!line.StartsWith("+Profiles=")) continue;
            var name = Field(line, "Name");
            if (name == null) continue;
            var enabled = Field(line, "CollisionEnabled") ?? "QueryAndPhysics";
            var pawn = System.Text.RegularExpressions.Regex.Match(line, "\\(Channel=\"Pawn\",Response=(ECR_\\w+)\\)");
            profiles[name] = (enabled.StartsWith("Query"), pawn.Success ? pawn.Groups[1].Value : "ECR_Block");
        }
    }

    static string? Field(string line, string key)
    {
        var m = System.Text.RegularExpressions.Regex.Match(line, key + "=\"?([A-Za-z_]+)\"?");
        return m.Success ? m.Groups[1].Value : null;
    }

    /// Whether a primitive component blocks a counselor (Pawn). `defaultProfile` is the component class's constructor
    /// default (BlockAll for static meshes) for components that keep it.
    public static bool BlocksPawn(UObject component, string defaultProfile = "BlockAll")
    {
        var bi = component.GetOrDefault<FStructFallback?>("BodyInstance");
        var profile = bi?.GetOrDefault<FName>("CollisionProfileName").Text;
        if (string.IsNullOrEmpty(profile) || profile == "None") profile = defaultProfile;
        Used[profile] = Used.GetValueOrDefault(profile) + 1;
        if (profile == "Custom")
        {
            var enabled = bi?.GetOrDefault<string>("CollisionEnabled") ?? "ECollisionEnabled::QueryAndPhysics";
            if (!enabled.Contains("Query")) return false;
            foreach (var r in bi?.GetOrDefault<FStructFallback?>("CollisionResponses")?.GetOrDefault<FStructFallback[]>("ResponseArray", []) ?? [])
                if (r.GetOrDefault<FName>("Channel").Text == "Pawn") return r.GetOrDefault<string>("Response") == "ECR_Block";
            return true; // the Pawn channel's default response is Block
        }
        if (profiles.TryGetValue(profile, out var p)) return p.Query && p.Pawn == "ECR_Block";
        if (unknown.Add(profile)) Console.WriteLine($"  collision profile {profile} not in DefaultEngine.ini: treated as blocking");
        return true;
    }

    public static MeshShapes? Shapes(UStaticMesh mesh)
    {
        if (mesh.BodySetup?.Load() is not UBodySetup body) return null;
        var result = new MeshShapes
        {
            ComplexAsSimple = body.GetOrDefault<string>("CollisionTraceFlag")?.EndsWith("UseComplexAsSimple") == true,
            Surface = SurfaceOf(body.GetOrDefault<FPackageIndex?>("PhysMaterial")?.Load<UObject>()
                ?? MaterialPhysMat(mesh.StaticMaterials?.FirstOrDefault()?.MaterialInterface?.Load())),
        };
        var agg = body.AggGeom;
        if (agg != null)
        {
            foreach (var c in agg.ConvexElems ?? [])
                if (c.VertexData is { Length: >= 4 } v) result.Hulls.Add(Flat(v.Select(p => c.Transform.TransformPosition(p))));
            foreach (var b in agg.BoxElems ?? [])
            {
                var q = b.Rotation.Quaternion();
                var corners = new List<FVector>();
                for (var i = 0; i < 8; i++)
                    corners.Add(b.Center + q.RotateVector(new FVector((i & 1) != 0 ? b.X / 2 : -b.X / 2, (i & 2) != 0 ? b.Y / 2 : -b.Y / 2, (i & 4) != 0 ? b.Z / 2 : -b.Z / 2)));
                result.Hulls.Add(Flat(corners));
            }
            foreach (var s in agg.SphereElems ?? [])
                result.Hulls.Add(Flat(Sphere(s.Radius, 0).Select(p => s.Center + p)));
            foreach (var s in agg.SphylElems ?? [])
            {
                var q = s.Rotation.Quaternion();
                result.Hulls.Add(Flat(Sphere(s.Radius, s.Length / 2).Select(p => s.Center + q.RotateVector(p))));
            }
        }
        return result.Hulls.Count > 0 || result.ComplexAsSimple ? result : null;
    }

    /// Box, sphere and capsule components (cm, component space): their points as a convex point set.
    public static float[]? ShapeComponent(UObject c, FVector scale)
    {
        var type = c.ExportType;
        IEnumerable<FVector>? points = null;
        if (type.Contains("BoxComponent"))
        {
            var e = c.GetOrDefault("BoxExtent", new FVector(32, 32, 32));
            points = Enumerable.Range(0, 8).Select(i => new FVector((i & 1) != 0 ? e.X : -e.X, (i & 2) != 0 ? e.Y : -e.Y, (i & 4) != 0 ? e.Z : -e.Z));
        }
        else if (type.Contains("SphereComponent")) points = Sphere(c.GetOrDefault("SphereRadius", 32f), 0);
        else if (type.Contains("CapsuleComponent"))
        {
            var r = c.GetOrDefault("CapsuleRadius", 22f);
            points = Sphere(r, Math.Max(c.GetOrDefault("CapsuleHalfHeight", 44f) - r, 0));
        }
        return points == null ? null : Flat(points.Select(p => p * scale));
    }

    /// Points on a sphere of `radius` (16 around, 8 rings), split `halfLength` up and down along Z (a capsule).
    static IEnumerable<FVector> Sphere(float radius, float halfLength)
    {
        yield return new FVector(0, 0, radius + halfLength);
        yield return new FVector(0, 0, -radius - halfLength);
        for (var ring = 1; ring < 8; ring++)
        {
            var phi = Math.PI * ring / 8;
            var z = (float)(radius * Math.Cos(phi));
            var rr = (float)(radius * Math.Sin(phi));
            var offset = z >= 0 ? halfLength : -halfLength;
            for (var k = 0; k < 16; k++)
            {
                var th = 2 * Math.PI * k / 16;
                yield return new FVector((float)(rr * Math.Cos(th)), (float)(rr * Math.Sin(th)), z + offset);
                if (ring == 4 && halfLength > 0) yield return new FVector((float)(rr * Math.Cos(th)), (float)(rr * Math.Sin(th)), -offset);
            }
        }
    }

    static float[] Flat(IEnumerable<FVector> points) => points.SelectMany(p => new[] { p.X, p.Y, p.Z }).ToArray();
}
