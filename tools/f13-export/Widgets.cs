// UMG widget trees f13-game draws (ui/<Widget>.json): from the widget Blueprint's WidgetTree, each widget's type and
// name, its properties (object references as {"Path", "Class"}; textures are exported as PNG next to their package
// path, "File" naming the file), its slot in the parent (the slot's properties: canvas LayoutData anchors / offsets /
// alignment, AutoSize, ZOrder; box and overlay padding, alignment, size), and its children in order. A user widget
// (a "..._C" type) names its class ("Class"); that class's tree is exported too. A material brush (UI materials such
// as the stamina ring's radial fill; their graphs are cooked away) keeps its texture samples ("Textures", exported) and
// parameter defaults ("Scalars", "Vectors").
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Exports.Texture;
using CUE4Parse.UE4.Objects.UObject;
using CUE4Parse.UE4.Assets.Objects;
using CUE4Parse.UE4.Objects.Core.Misc;

class WidgetExporter(IFileProvider provider, string outDir, Action<UObject> queue, Func<string, string> toFile)
{
    readonly HashSet<string> done = new(StringComparer.OrdinalIgnoreCase);
    string Dir => Path.Combine(outDir, "ui");

    /// The UI's font faces: the game ships them as loose .ufont files (TrueType), copied to ui/fonts/<face>.ttf.
    public int Fonts(string game)
    {
        var dir = Path.Combine(game, "SummerCamp", "Content", "UI", "Font");
        if (!Directory.Exists(dir)) return 0;
        var outFonts = Path.Combine(Dir, "fonts");
        Directory.CreateDirectory(outFonts);
        var n = 0;
        foreach (var f in Directory.GetFiles(dir, "*.ufont"))
        {
            File.Copy(f, Path.Combine(outFonts, Path.GetFileNameWithoutExtension(f) + ".ttf"), true);
            n++;
        }
        return n;
    }

    /// The textures classes.json's classes name (item icons: ImageOn / ImageOff / HUDPickupIcon), exported; writes
    /// ui/textures.json (texture name -> PNG file) and returns how many.
    public int ClassTextures()
    {
        var file = Path.Combine(outDir, "classes.json");
        if (!File.Exists(file)) return 0;
        var names = System.Text.RegularExpressions.Regex.Matches(File.ReadAllText(file), @"Texture2D'([^']+)'")
            .Select(m => m.Groups[1].Value).ToHashSet(StringComparer.OrdinalIgnoreCase);
        // And the levels' minimap textures (SCMiniMapVolume's MapTexture and KillerMapTexture).
        var levels = Path.Combine(outDir, "levels");
        foreach (var level in Directory.Exists(levels) ? Directory.GetFiles(levels, "*.json") : [])
            foreach (System.Text.RegularExpressions.Match m in System.Text.RegularExpressions.Regex.Matches(File.ReadAllText(level), "\"(?:Killer)?MapTexture\":\\s*\\{\\s*\"AssetPathName\":\\s*\"[^\"]*/([^/\".]+)\\."))
                names.Add(m.Groups[1].Value);
        var index = new SortedDictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        foreach (var key in provider.Files.Keys.Where(k => k.EndsWith(".uasset", StringComparison.OrdinalIgnoreCase)))
        {
            var name = Path.GetFileNameWithoutExtension(key);
            if (!names.Contains(name) || index.ContainsKey(name)) continue;
            var pkg = key[..^".uasset".Length];
            try
            {
                if (provider.LoadPackage(pkg).GetExports().OfType<UTexture2D>().FirstOrDefault() is { } tex)
                {
                    queue(tex);
                    index[name] = pkg + ".png";
                }
            }
            catch (Exception e)
            {
                Console.WriteLine($"texture {name}: {e.Message}");
            }
        }
        Directory.CreateDirectory(Dir);
        File.WriteAllText(Path.Combine(Dir, "textures.json"), JsonConvert.SerializeObject(index, Formatting.Indented));
        return index.Count;
    }

    /// Exports the widget Blueprint at `path` (and the user widgets it uses). Returns the widgets written.
    public int Export(string path)
    {
        Directory.CreateDirectory(Dir);
        var name = path.Split('/')[^1].Split('.')[0];
        if (!done.Add(name)) return 0;
        var exports = provider.LoadPackage(toFile(path)).GetExports().ToList();
        var cls = exports.FirstOrDefault(e => e.ExportType == "WidgetBlueprintGeneratedClass");
        var tree = cls?.GetOrDefault<FPackageIndex?>("WidgetTree")?.Load<UObject>();
        var root = tree?.GetOrDefault<FPackageIndex?>("RootWidget")?.Load<UObject>();
        if (root == null)
        {
            Console.WriteLine($"widget {path}: no widget tree");
            return 0;
        }
        var nested = new List<string>();
        var j = new JObject { ["Name"] = name, ["Root"] = Node(root, null, nested) };
        // The class's defaults (its variables' values that its graphs use: sizes, times, curves).
        if (exports.FirstOrDefault(e => e.Name.StartsWith("Default__")) is { } cdo)
            j["Defaults"] = Props(cdo, "WidgetTree", "UberGraphFrame");
        // Its animations (UWidgetAnimation): each bound widget's animated property and its channels' curves.
        var anims = new JObject();
        foreach (var a in cls!.GetOrDefault<FPackageIndex[]>("Animations", []))
        {
            if (a.Load<UObject>() is not { } wa || wa.GetOrDefault<FPackageIndex?>("MovieScene")?.Load<UObject>() is not { } scene) continue;
            var sp = JObject.Parse(JsonConvert.SerializeObject(scene))["Properties"] as JObject ?? new JObject();
            var names = new Dictionary<string, string>();
            foreach (var b in JObject.Parse(JsonConvert.SerializeObject(wa))["Properties"]?["AnimationBindings"] ?? new JArray())
                names[b["AnimationGuid"]!.ToString()] = b["WidgetName"]!.ToString();
            var tracks = new JArray();
            foreach (var ob in scene.GetOrDefault<UScriptArray?>("ObjectBindings")?.Properties ?? [])
            {
                if (ob.GetValue(typeof(FStructFallback)) is not FStructFallback binding) continue;
                var guid = binding.GetOrDefault<FGuid>("ObjectGuid").ToString(EGuidFormats.UniqueObjectGuid);
                foreach (var ti in binding.GetOrDefault<FPackageIndex[]>("Tracks", []))
                {
                    if (ti.Load<UObject>() is not { } track) continue;
                    var sections = new JArray();
                    foreach (var si in track.GetOrDefault<FPackageIndex[]>("Sections", []))
                    {
                        if (si.Load<UObject>() is not { } section) continue;
                        var props = JObject.Parse(JsonConvert.SerializeObject(section))["Properties"] as JObject ?? new JObject();
                        var channels = new JObject();
                        foreach (var p in props.Properties())
                            if (p.Value is JObject c && (c["Keys"] != null || c["DefaultValue"] != null)) channels[p.Name] = c;
                        sections.Add(new JObject { ["Type"] = section.ExportType, ["Start"] = props["StartTime"] ?? 0, ["End"] = props["EndTime"], ["Channels"] = channels });
                    }
                    tracks.Add(new JObject
                    {
                        ["Widget"] = names.GetValueOrDefault(guid, guid),
                        ["Type"] = track.ExportType,
                        ["Property"] = track.GetOrDefault<FName>("PropertyName").Text,
                        ["Sections"] = sections,
                    });
                }
            }
            anims[wa.Name.Replace("_INST", "")] = new JObject { ["Length"] = sp["PlaybackRange"]?["UpperBound"]?["Value"], ["Tracks"] = tracks };
        }
        if (anims.Count > 0) j["Animations"] = anims;
        // Textures its graphs set (SetBrushFromTexture's FindObject): the package's Texture2D imports, by name.
        var scriptTextures = new JObject();
        if (provider.LoadPackage(toFile(path)) is CUE4Parse.UE4.Assets.Package pkg)
            foreach (var imp in pkg.ImportMap.Where(i => i.ClassName.Text == "Texture2D"))
            {
                if (!imp.OuterIndex.IsImport) continue;
                var outer = pkg.ImportMap[-imp.OuterIndex.Index - 1].ObjectName.Text;
                try
                {
                    if (provider.LoadPackage(toFile(outer)).GetExports().OfType<UTexture2D>().FirstOrDefault() is { } tex)
                    {
                        queue(tex);
                        scriptTextures[imp.ObjectName.Text] = tex.GetPathName().Split('.')[0] + ".png";
                    }
                }
                catch (Exception e)
                {
                    Console.WriteLine($"texture {outer}: {e.Message}");
                }
            }
        if (scriptTextures.Count > 0) j["Textures"] = scriptTextures;
        File.WriteAllText(Path.Combine(Dir, name + ".json"), j.ToString(Formatting.Indented));
        var n = 1;
        foreach (var p in nested.Distinct()) n += Export(p);
        return n;
    }

    JObject Node(UObject w, UObject? slot, List<string> nested)
    {
        var j = new JObject { ["Type"] = w.ExportType, ["Name"] = w.Name, ["Props"] = Props(w, "Slots", "Slot") };
        if (slot != null) j["Slot"] = new JObject { ["Type"] = slot.ExportType, ["Props"] = Props(slot, "Content", "Parent") };
        // A user widget: its class's own tree.
        if (w.ExportType.EndsWith("_C") && w.Class?.GetPathName() is { } classPath)
        {
            j["Class"] = w.ExportType[..^2];
            nested.Add("/" + classPath.Replace("SummerCamp/Content/", "Game/").Split('.')[0]);
        }
        var children = new JArray();
        foreach (var s in w.GetOrDefault<FPackageIndex[]>("Slots", []))
        {
            if (s.Load<UObject>() is not { } sl) continue;
            if (sl.GetOrDefault<FPackageIndex?>("Content")?.Load<UObject>() is { } child) children.Add(Node(child, sl, nested));
        }
        j["Children"] = children;
        return j;
    }

    JObject Props(UObject o, params string[] skip)
    {
        var p = JObject.Parse(JsonConvert.SerializeObject(o))["Properties"] as JObject ?? new JObject();
        foreach (var s in skip) p.Remove(s);
        return (JObject)Flatten(p);
    }

    // Object references to {"Path", "Class"}; textures exported, with their PNG "File".
    JToken Flatten(JToken t)
    {
        switch (t)
        {
            case JObject o when o["ObjectName"] is { } n && o["ObjectPath"] is { } p:
            {
                var name = n.ToString();
                var path = p.ToString();
                var r = new JObject { ["Name"] = name, ["Path"] = path };
                if (name.StartsWith("Material'") || name.StartsWith("MaterialInstanceConstant'"))
                {
                    try
                    {
                        if (provider.LoadPackage(path.Split('.')[0]).GetExports().ToList() is { } mexp)
                        {
                            var textures = new JArray();
                            var scalars = new JObject();
                            var vectors = new JObject();
                            // A texture parameter's default, by its name (kept apart: "Textures" is the samples').
                            var textureParams = new JObject();
                            foreach (var e in mexp)
                            {
                                if (e.ExportType == "MaterialExpressionTextureSampleParameter2D" && e.GetOrDefault<FPackageIndex?>("Texture")?.Load<UTexture2D>() is { } ptexd)
                                {
                                    queue(ptexd);
                                    textureParams[e.GetOrDefault<FName>("ParameterName").Text] = ptexd.GetPathName().Split('.')[0] + ".png";
                                    continue;
                                }
                                if (e.ExportType == "MaterialExpressionTextureSample" && e.GetOrDefault<FPackageIndex?>("Texture")?.Load<UTexture2D>() is { } tex)
                                {
                                    queue(tex);
                                    textures.Add(tex.GetPathName().Split('.')[0] + ".png");
                                }
                                else if (e.ExportType == "MaterialExpressionScalarParameter")
                                    scalars[e.GetOrDefault<FName>("ParameterName").Text] = e.GetOrDefault("DefaultValue", 0f);
                                else if (e.ExportType == "MaterialExpressionVectorParameter" && JObject.Parse(JsonConvert.SerializeObject(e))["Properties"]?["DefaultValue"] is { } dv)
                                    vectors[e.GetOrDefault<FName>("ParameterName").Text] = new JArray(dv["R"], dv["G"], dv["B"], dv["A"]);
                                else if (e.ExportType == "MaterialInstanceConstant" && JObject.Parse(JsonConvert.SerializeObject(e))["Properties"] is JObject mp)
                                {
                                    // An instance: its parameter values (the parent's graph is cooked away too).
                                    r["Parent"] = mp["Parent"]?["ObjectName"]?.ToString().Split('\'')[1];
                                    foreach (var tp in mp["TextureParameterValues"] ?? new JArray())
                                        if (tp["ParameterValue"]?["ObjectPath"]?.ToString() is { } tpath
                                            && provider.LoadPackage(tpath.Split('.')[0]).GetExports().OfType<UTexture2D>().FirstOrDefault() is { } ptex)
                                        {
                                            queue(ptex);
                                            textures.Add(tpath.Split('.')[0] + ".png");
                                        }
                                    foreach (var sp in mp["ScalarParameterValues"] ?? new JArray())
                                        scalars[sp["ParameterName"]!.ToString()] = sp["ParameterValue"];
                                    foreach (var vp in mp["VectorParameterValues"] ?? new JArray())
                                        if (vp["ParameterValue"] is { } v)
                                            vectors[vp["ParameterName"]!.ToString()] = new JArray(v["R"], v["G"], v["B"], v["A"]);
                                }
                            }
                            r["Textures"] = textures;
                            r["Scalars"] = scalars;
                            r["Vectors"] = vectors;
                            if (textureParams.Count > 0) r["TextureParameters"] = textureParams;
                        }
                    }
                    catch (Exception e)
                    {
                        Console.WriteLine($"material {path}: {e.Message}");
                    }
                }
                if (name.StartsWith("CurveFloat'"))
                {
                    // A float curve: its keys (FRichCurve).
                    try
                    {
                        if (provider.LoadPackage(path.Split('.')[0]).GetExports().FirstOrDefault(e => e.ExportType == "CurveFloat") is { } curve)
                            r["FloatCurve"] = JObject.Parse(JsonConvert.SerializeObject(curve))["Properties"]?["FloatCurve"];
                    }
                    catch (Exception e)
                    {
                        Console.WriteLine($"curve {path}: {e.Message}");
                    }
                }
                if (name.StartsWith("Texture2D'"))
                {
                    var pkg = path.Split('.')[0];
                    try
                    {
                        if (provider.LoadPackage(pkg).GetExports().OfType<UTexture2D>().FirstOrDefault() is { } tex)
                        {
                            queue(tex);
                            r["File"] = pkg + ".png";
                        }
                    }
                    catch (Exception e)
                    {
                        Console.WriteLine($"texture {pkg}: {e.Message}");
                    }
                }
                return r;
            }
            case JObject o:
                return new JObject(o.Properties().Select(x => new JProperty(x.Name, Flatten(x.Value))));
            case JArray a:
                return new JArray(a.Select(Flatten));
            default:
                return t.DeepClone();
        }
    }
}
