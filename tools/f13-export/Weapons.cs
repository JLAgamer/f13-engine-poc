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
/// and its NeutralAttack / ForwardAttack montages (the light and heavy swings: /Game paths and their takes) and
/// CombatIdleAsset. Each value is the nearest one up the class chain (PigSplitter and FireAxe inherit some).
record Weapon(string Class, string Name, string Image, UStaticMesh? Mesh, float Damage, string DefaultGrabKill, string Light, string LightTake, string Heavy, string HeavyTake, string CombatIdle);

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
                (string, string) Montage(string prop)
                {
                    var list = ClassValue<FStructFallback[]>(cls, prop) ?? [];
                    var m = list.Select(s => s.GetOrDefault<FPackageIndex?>("AttackMontage")?.Load<UAnimMontage>()).FirstOrDefault(m => m != null);
                    var take = m?.SlotAnimTracks?.SelectMany(s => s.AnimTrack.AnimSegments ?? []).Select(s => s.AnimReference.TryLoad(out UAnimSequence? q) ? q : null).FirstOrDefault(q => q != null)?.Name;
                    return m == null || take == null ? ("", "") : (GamePath(m.GetPathName()), take);
                }
                var (light, lightTake) = Montage("NeutralAttack");
                var (heavy, heavyTake) = Montage("ForwardAttack");
                var item = ClassValue<FPackageIndex>(cls, "ItemMesh3P")?.Load();
                var mesh = item == null ? null : Inherited<FPackageIndex>(item, "StaticMesh")?.Load<UStaticMesh>();
                var stats = ClassValue<FStructFallback>(cls, "WeaponStats");
                var idle = ClassValue<FPackageIndex>(cls, "CombatIdleAsset")?.Load();
                cache.Add(new Weapon(
                    cls.Name,
                    ClassValue<FText>(cls, "FriendlyName")?.Text ?? cls.Name,
                    ClassValue<FPackageIndex>(cls, "ThumbnailImage")?.Load<UTexture2D>()?.GetPathName().Split('.')[0].TrimStart('/').Replace("Game/", "SummerCamp/Content/") ?? "",
                    mesh,
                    stats?.GetOrDefault("Damage", 20f) ?? 20f,
                    ClassValue<FPackageIndex>(cls, "DefaultGrabKill")?.Name ?? "",
                    light, lightTake, heavy, heavyTake,
                    idle?.Name ?? ""));
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
                ["LightTake"] = w.LightTake,
                ["HeavyTake"] = w.HeavyTake,
                ["CombatIdle"] = w.CombatIdle,
            });
        }
        File.WriteAllText(System.IO.Path.Combine(outDir, "weapons.json"), a.ToString());
    }
}
