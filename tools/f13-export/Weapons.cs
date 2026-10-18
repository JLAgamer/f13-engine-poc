using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Animation;
using CUE4Parse.UE4.Assets.Exports.StaticMesh;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Assets.Objects;
using CUE4Parse.UE4.Objects.Core.i18N;
using CUE4Parse.UE4.Objects.Engine;
using CUE4Parse.UE4.Objects.UObject;
using Newtonsoft.Json.Linq;

/// A killer weapon (a Jason's WeaponClass or one of his AllowedWeaponClasses: the menu's WeaponSelectionMenu): its
/// FriendlyName and ThumbnailImage (GetKillerWeaponDetails), its ItemMesh3P mesh, WeaponStats.Damage, DefaultGrabKill,
/// and its NeutralAttack / ForwardAttack montages (the light and heavy swings: /Game paths and their takes, with their
/// RecoilMontage and ImpactMontage: a recoil off the world, the weapon stuck), its WeaponStats' stuck and recoil values
/// (SCWeapon's constructor 0x14044bfb0 sets MaximumStuckAngle 15, MaximumRecoilAngle 35) and CombatIdleAsset. Each
/// value is the nearest one up the class chain (PigSplitter and FireAxe inherit some).
record Attack(string Montage, string Take, string Recoil, string[] RecoilTakes, string Impact, string[] ImpactTakes);
record Weapon(string Class, string Name, string Image, UStaticMesh? Mesh, float Damage, string DefaultGrabKill, Attack Light, Attack Heavy, string CombatIdle, JObject Stuck);

static class Weapons
{
    static List<Weapon>? cache;

    static T? ClassValue<T>(UStruct? cls, string name)
    {
        for (; cls != null; cls = cls.SuperStruct?.Load<UStruct>())
            if (cls is UBlueprintGeneratedClass bp && bp.ClassDefaultObject.Load() is { } cdo && cdo.TryGetValue(out T v, name)) return v;
        return default;
    }

    static T? Inherited<T>(UObject component, string name)
    {
        for (UObject? a = component; a != null; a = a.Template?.Load())
            if (a.TryGetValue(out T v, name)) return v;
        return default;
    }

    static string GamePath(string objectPath) => "/Game/" + objectPath.Split('.')[0].TrimStart('/').Replace("SummerCamp/Content/", "").Replace("Game/", "");

    /// The weapons the given Jason Blueprints carry or allow, each once, in first-seen order.
    public static List<Weapon> All(IFileProvider provider, IEnumerable<string> jasons)
    {
        if (cache != null) return cache;
        cache = new();
        var paths = new List<string>();
        foreach (var bp in jasons)
        {
            var cls = provider.LoadPackageObject<UBlueprintGeneratedClass>(bp + "." + bp.Split('/')[^1] + "_C");
            var own = ClassValue<FSoftObjectPath>(cls, "WeaponClass").AssetPathName.Text;
            foreach (var p in new[] { own }.Concat((ClassValue<FSoftObjectPath[]>(cls, "AllowedWeaponClasses") ?? []).Select(s => s.AssetPathName.Text)))
                if (!string.IsNullOrEmpty(p) && p != "None" && !paths.Contains(p)) paths.Add(p);
        }
        foreach (var p in paths)
        {
            try
            {
                var cls = provider.LoadPackageObject<UBlueprintGeneratedClass>(p);
                static string[] Takes(UAnimMontage? m) =>
                    m?.SlotAnimTracks?.SelectMany(s => s.AnimTrack.AnimSegments ?? []).Select(s => s.AnimReference.TryLoad(out UAnimSequence? q) ? q?.Name : null).OfType<string>().ToArray() ?? [];
                Attack Montage(string prop)
                {
                    // The first entry with an AttackMontage (the male one where there are two).
                    var list = ClassValue<FStructFallback[]>(cls, prop) ?? [];
                    var s = list.FirstOrDefault(s => s.GetOrDefault<FPackageIndex?>("AttackMontage")?.Load<UAnimMontage>() != null);
                    UAnimMontage? M(string name) => s?.GetOrDefault<FPackageIndex?>(name)?.Load<UAnimMontage>();
                    var (m, r, i) = (M("AttackMontage"), M("RecoilMontage"), M("ImpactMontage"));
                    var take = Takes(m).FirstOrDefault();
                    return new Attack(m == null || take == null ? "" : GamePath(m.GetPathName()), take ?? "",
                        r == null ? "" : GamePath(r.GetPathName()), Takes(r), i == null ? "" : GamePath(i.GetPathName()), Takes(i));
                }
                var light = Montage("NeutralAttack");
                var heavy = Montage("ForwardAttack");
                var item = ClassValue<FPackageIndex>(cls, "ItemMesh3P")?.Load();
                var mesh = item == null ? null : Inherited<FPackageIndex>(item, "StaticMesh")?.Load<UStaticMesh>();
                var stats = ClassValue<FStructFallback>(cls, "WeaponStats");
                var idle = ClassValue<FPackageIndex>(cls, "CombatIdleAsset")?.Load();
                var stuck = new JObject
                {
                    ["StuckChance"] = stats?.GetOrDefault("StuckChance", 0) ?? 0,
                    ["StuckSurfaceTypes"] = new JArray((stats?.GetOrDefault<FName[]>("StuckSurfaceTypes", []) ?? [])
                        .Select(n => System.Text.RegularExpressions.Regex.Match(n.Text, @"SurfaceType(\d+)") is { Success: true } m ? int.Parse(m.Groups[1].Value) : 0)),
                    ["MinimumSurfacePenetration"] = stats?.GetOrDefault("MinimumSurfacePenetration", 0f) ?? 0f,
                    ["MaximumStuckAngle"] = stats?.GetOrDefault("MaximumStuckAngle", 15f) ?? 15f,
                    ["MaximumRecoilAngle"] = stats?.GetOrDefault("MaximumRecoilAngle", 35f) ?? 35f,
                };
                cache.Add(new Weapon(
                    cls.Name,
                    ClassValue<FText>(cls, "FriendlyName")?.Text ?? cls.Name,
                    ClassValue<FPackageIndex>(cls, "ThumbnailImage")?.Load<UTexture2D>()?.GetPathName().Split('.')[0].TrimStart('/').Replace("Game/", "SummerCamp/Content/") ?? "",
                    mesh,
                    stats?.GetOrDefault("Damage", 20f) ?? 20f,
                    ClassValue<FPackageIndex>(cls, "DefaultGrabKill")?.Name ?? "",
                    light, heavy,
                    idle?.Name ?? "", stuck));
            }
            catch (Exception e)
            {
                Console.WriteLine($"weapon {p}: {e.Message}");
            }
        }
        Console.WriteLine($"weapons: {cache.Count}");
        return cache;
    }

    /// weapons.json (f13-game killer_profile.rs, jason.rs), the meshes and thumbnails queued.
    public static void Write(IFileProvider provider, string outDir, IEnumerable<string> jasons, Action<UObject> queue)
    {
        var a = new JArray();
        foreach (var w in All(provider, jasons))
        {
            if (w.Mesh != null) queue(w.Mesh);
            if (!string.IsNullOrEmpty(w.Image) && provider.TryLoadPackageObject<UTexture2D>(w.Image.Replace("SummerCamp/Content/", "/Game/") + "." + w.Image.Split('/')[^1], out var tex)) queue(tex);
            a.Add(new JObject
            {
                ["Class"] = w.Class,
                ["Name"] = w.Name,
                ["Image"] = string.IsNullOrEmpty(w.Image) ? "" : w.Image + ".png",
                ["Mesh"] = w.Mesh == null ? "" : new CUE4Parse_Conversion.Exporters.StaticMeshExporter(w.Mesh).SavePath + ".glb",
                ["Damage"] = w.Damage,
                ["DefaultGrabKill"] = w.DefaultGrabKill,
                ["LightTake"] = w.Light.Take,
                ["HeavyTake"] = w.Heavy.Take,
                ["LightRecoil"] = new JArray(w.Light.RecoilTakes),
                ["LightImpact"] = new JArray(w.Light.ImpactTakes),
                ["HeavyRecoil"] = new JArray(w.Heavy.RecoilTakes),
                ["HeavyImpact"] = new JArray(w.Heavy.ImpactTakes),
                ["Stuck"] = w.Stuck,
                ["CombatIdle"] = w.CombatIdle,
            });
        }
        File.WriteAllText(System.IO.Path.Combine(outDir, "weapons.json"), a.ToString());
    }
}
