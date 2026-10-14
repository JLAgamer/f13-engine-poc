using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Animation;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Objects.Core.i18N;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Objects.Engine;
using Newtonsoft.Json.Linq;

/// One grab kill Jason_Select_Widget's Kill_Select_Widget offers: its class, the weapon it needs (none: a hand kill),
/// its KillInfo's KillName, its FriendlyName and Description, its ContextKillHUDIcon, and its two special moves'
/// ContextMontage (his: InteractorSpecialClass, hers: VictimSpecialClass) as /Game paths with the take each plays (the
/// montage's first sequence: the key the character export writes it under).
record GrabKill(string Class, string Path, string Weapon, string KillName, string Name, string Description, string Icon, string His, string HisTake, string Hers, string HersTake);

static class GrabKills
{
    static List<GrabKill>? cache;

    /// Kill_Select_Widget's NonWeaponGrabKills and WeaponGrabKills (its CDO), in that order; a kill whose montages
    /// don't resolve is left out (logged).
    public static List<GrabKill> All(IFileProvider provider)
    {
        if (cache != null) return cache;
        cache = new();
        var widget = provider.LoadPackage("SummerCamp/Content/UI/Menu/Character/Killer/Kill_Select_Widget").GetExports()
            .FirstOrDefault(e => e.Name.StartsWith("Default__"));
        foreach (var list in new[] { "NonWeaponGrabKills", "WeaponGrabKills" })
            foreach (var k in widget?.GetOrDefault<FPackageIndex[]>(list, []) ?? [])
            {
                try
                {
                    if (Resolve(provider, k) is { } g) cache.Add(g);
                }
                catch (Exception e)
                {
                    Console.WriteLine($"grab kill {k.Name}: {e.Message}");
                }
            }
        Console.WriteLine($"grab kills: {cache.Count}");
        return cache;
    }

    static string GamePath(string objectPath) => "/Game/" + objectPath.Split('.')[0].TrimStart('/').Replace("SummerCamp/Content/", "").Replace("Game/", "");

    static GrabKill? Resolve(IFileProvider provider, FPackageIndex k)
    {
        if (k.Load<UBlueprintGeneratedClass>() is not { } cls || cls.ClassDefaultObject.Load() is not { } cdo) return null;
        var path = cls.GetPathName().Split('.')[0].TrimStart('/');
        UObject? Comp(string name) => cdo.GetOrDefault<FPackageIndex?>(name)?.Load();
        var component = Comp("ContextKillComponent");
        (string, string)? Montage(string prop)
        {
            if (component?.GetOrDefault<FPackageIndex?>(prop)?.Load<UBlueprintGeneratedClass>()?.ClassDefaultObject.Load() is not { } move) return null;
            if (move.GetOrDefault<FPackageIndex?>("ContextMontage")?.Load<UAnimMontage>() is not { } montage) return null;
            var take = montage.SlotAnimTracks?.SelectMany(s => s.AnimTrack.AnimSegments ?? []).Select(s => s.AnimReference.TryLoad(out UAnimSequence? q) ? q : null).FirstOrDefault(q => q != null)?.Name;
            return take == null ? null : (GamePath(montage.GetPathName()), take);
        }
        var his = Montage("InteractorSpecialClass");
        var hers = Montage("VictimSpecialClass");
        if (his == null || hers == null)
        {
            Console.WriteLine($"grab kill {cls.Name}: no montages");
            return null;
        }
        var weapon = cdo.GetOrDefault<FSoftObjectPath>("WeaponClass").AssetPathName.Text;
        weapon = string.IsNullOrEmpty(weapon) || weapon == "None" ? "" : weapon.Split('.')[^1];
        var killInfo = component?.GetOrDefault<FPackageIndex?>("KillInfo")?.Load<UBlueprintGeneratedClass>()?.ClassDefaultObject.Load();
        var icon = cdo.GetOrDefault<FPackageIndex?>("ContextKillHUDIcon")?.Load<UTexture2D>();
        return new GrabKill(
            cls.Name, path, weapon,
            Str(killInfo, "KillName") ?? cls.Name,
            cdo.GetOrDefault<FText?>("FriendlyName")?.Text ?? "",
            cdo.GetOrDefault<FText?>("Description")?.Text ?? "",
            icon?.Name ?? "",
            his.Value.Item1, his.Value.Item2, hers.Value.Item1, hers.Value.Item2);
    }

    /// A string property, whichever of FString, FName or FText it is.
    static string? Str(UObject? o, string name)
    {
        if (o == null) return null;
        foreach (var p in o.Properties)
            if (p.Name.Text == name)
                return p.Tag?.GenericValue switch
                {
                    string t => t,
                    FName n => n.Text,
                    FText x => x.Text,
                    var v => v?.ToString(),
                };
        return null;
    }

    /// grab_kills.json: the list, as the game reads it (jason.rs).
    public static void Write(IFileProvider provider, string outDir)
    {
        var a = new JArray();
        foreach (var g in All(provider))
            a.Add(new JObject
            {
                ["Class"] = g.Class, ["Weapon"] = g.Weapon, ["KillName"] = g.KillName, ["Name"] = g.Name,
                ["Description"] = g.Description, ["Icon"] = g.Icon, ["HisTake"] = g.HisTake, ["HersTake"] = g.HersTake,
            });
        File.WriteAllText(System.IO.Path.Combine(outDir, "grab_kills.json"), a.ToString());
    }
}
