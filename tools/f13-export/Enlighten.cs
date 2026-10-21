// `enlighten`: the map's Enlighten precomputed data, for f13-engine-poc's solve (f13-data enlighten.rs, f13-game enlighten.rs),
// into <out>/enlighten/: read from the map's own packages (SummerCamp/Content/Maps/CrystalLake_Small/*.umap / .uexp, as
// UE 4.18 cooked them: a minimal reader of their names, imports, exports and tagged properties, since the Enlighten
// objects' native data follows their tags), after `mvp` (its levels' placements).
// - index.json: per system (EnlightenSystem) its package, name, SystemGuid, output size, AtlasAllocations and
//   primitives; per package its EnlightenAtlas's size, primitives and instanced primitives' per-instance UVScaleBias;
//   per probe set (EnlightenProbeSet) its probe count and octree origin and size (RadProbeSetCore from native +0x34:
//   +0x10, +0x50, +0x60);
// - per system its native data's Geomerics blocks GESG (transport), GEIW (input points, its 0x20 prefix, through its
//   cluster table GEDG), GERD (resampling), GEVS (directional visibility) and its saved albedo (.alb: 8-bit rgba per
//   cluster after GEVS's 0x20-byte header);
// - per probe set its GESH, GEPO and GEPV blocks;
// - <package>.prims.bin: per placed primitive its atlas UVs (u16 u, u16 v), from the atlas's records or else its mesh's
//   UEnlightenUVBufferCollection entry (LOD 0, the placement's scale) placed by its system's allocation;
// - <package>.inst.bin: per instanced component its instances' UVScaleBias (4 f32), in its instance order;
// - CrystalLake_Small_Terrain.land.json: per landscape component cell (6300 cm from -18900) the affine world XY (cm) to
//   atlas UV, least squares from its clusters' mean points (GEIW) to the output pixels each resamples (GERD).
// (A port of F13-decomp/enlighten's extract.py, prims.py, instances.py and landfit.py.)
using System.Text;
using Newtonsoft.Json.Linq;

static class Enlighten
{
    const string Map = "SummerCamp/Content/Maps/CrystalLake_Small";
    const string Terrain = "CrystalLake_Small_Terrain";
    /// A landscape component's cell: 63 quads x 100 cm, the grid's origin.
    const double Cell = 6300.0, Origin = -18900.0;

    // ---- A cooked UE 4.18 package (read-only).

    sealed class Package
    {
        public readonly byte[] Ua;
        readonly string path;
        readonly int headerSize;
        public readonly List<string> Names = [];
        public readonly List<string> Imports = []; // (each import's object name)
        public readonly List<Export> Exports = [];

        public sealed record Export(string Cls, int Outer, string Name, long Size, long Off);

        public Package(string umap)
        {
            path = umap;
            Ua = File.ReadAllBytes(umap);
            var o = 4;
            var legacy = I32(Ua, o); o += 4;
            if (legacy != -4) o += 4;
            o += 8;
            var custom = I32(Ua, o); o += 4 + custom * 20;
            headerSize = I32(Ua, o); o += 4;
            (_, o) = FString(Ua, o);
            o += 4; // package flags
            int nameCount = I32(Ua, o), nameOff = I32(Ua, o + 4); o += 8;
            o += 8; // gatherable text data
            int exportCount = I32(Ua, o), exportOff = I32(Ua, o + 4), importCount = I32(Ua, o + 8), importOff = I32(Ua, o + 12);
            var p = nameOff;
            for (var i = 0; i < nameCount; i++)
            {
                (var s, p) = FString(Ua, p); p += 4; // (its hashes)
                Names.Add(s);
            }
            p = importOff;
            for (var i = 0; i < importCount; i++)
            {
                Imports.Add(Name(BitConverter.ToInt64(Ua, p + 20)));
                p += 28;
            }
            p = exportOff;
            for (var i = 0; i < exportCount; i++)
            {
                int cls = I32(Ua, p), outer = I32(Ua, p + 12);
                var name = BitConverter.ToInt64(Ua, p + 16);
                long size = BitConverter.ToInt64(Ua, p + 28), off = BitConverter.ToInt64(Ua, p + 36);
                Exports.Add(new Export(Ref(cls), outer, Name(name), size, off));
                p += 104;
            }
        }

        public string Name(long v)
        {
            int i = (int)(v & 0xFFFFFFFF), n = (int)(v >> 32);
            var s = i < Names.Count ? Names[i] : $"?{i}";
            return n == 0 ? s : $"{s}_{n - 1}";
        }

        public string Ref(int i) => i < 0 ? Imports[-i - 1] : i > 0 ? "export:" + (i - 1) : "None";

        /// An export's serialized bytes (in the .uexp, its offset past the header).
        public byte[] Data(Export e)
        {
            using var ux = File.OpenRead(Path.ChangeExtension(path, ".uexp"));
            ux.Seek(e.Off - headerSize, SeekOrigin.Begin);
            var b = new byte[e.Size];
            ux.ReadExactly(b);
            return b;
        }

        /// UE 4.18 tagged properties from `o`: (name, value bytes) each, and the offset past their None.
        public (List<(string Name, byte[] Value)>, int) Tags(byte[] d, int o = 0)
        {
            var output = new List<(string, byte[])>();
            while (true)
            {
                var name = Name(BitConverter.ToInt64(d, o)); o += 8;
                if (name == "None") return (output, o);
                var type = Name(BitConverter.ToInt64(d, o)); o += 8;
                var size = I32(d, o); o += 8;
                switch (type)
                {
                    case "StructProperty": o += 8 + 16; break;
                    case "BoolProperty": o += 1; break;
                    case "ByteProperty" or "EnumProperty" or "ArrayProperty" or "SetProperty": o += 8; break;
                    case "MapProperty": o += 16; break;
                }
                o += d[o] != 0 ? 17 : 1;
                output.Add((name, d[o..(o + size)])); o += size;
            }
        }

        /// An ArrayProperty of structs: each element's tags.
        public List<List<(string Name, byte[] Value)>> StructArray(byte[]? v)
        {
            var output = new List<List<(string, byte[])>>();
            if (v == null) return output;
            var n = I32(v, 0);
            var p = 4 + 24 + 8 + 16 + 1;
            for (var i = 0; i < n; i++)
            {
                (var t, p) = Tags(v, p);
                output.Add(t);
            }
            return output;
        }
    }

    static int I32(byte[] b, int o) => BitConverter.ToInt32(b, o);
    static uint U32(byte[] b, int o) => BitConverter.ToUInt32(b, o);

    static (string, int) FString(byte[] b, int o)
    {
        var n = I32(b, o); o += 4;
        if (n == 0) return ("", o);
        if (n > 0) return (Encoding.Latin1.GetString(b, o, n - 1), o + n);
        return (Encoding.Unicode.GetString(b, o, -2 * n - 2), o - 2 * n);
    }

    static byte[]? Prop(List<(string Name, byte[] Value)> t, string name) => t.FirstOrDefault(x => x.Name == name).Value;
    static string Guid(byte[] v) => Convert.ToHexString(v).ToLowerInvariant();

    static int Find(byte[] b, byte[] pattern, int from = 0)
    {
        if (from < 0) from = 0;
        var i = b.AsSpan(from).IndexOf(pattern);
        return i < 0 ? -1 : i + from;
    }

    static readonly byte[] GESG = "GESG"u8.ToArray(), GERD = "GERD"u8.ToArray(), GEDG = "GEDG"u8.ToArray(), GEVS = "GEVS"u8.ToArray(),
        GEIW = "GEIW"u8.ToArray(), GESH = "GESH"u8.ToArray(), GEPO = "GEPO"u8.ToArray(), GEPV = "GEPV"u8.ToArray();

    /// The first GEIW that is the workspace's (its point count after it plausible: a "GE.." in quantized data isn't).
    static int FindGeiw(byte[] native, int from)
    {
        var j = Find(native, GEIW, from);
        while (j >= 0 && !(I32(native, j + 4) is > 0 and < 10_000_000)) j = Find(native, GEIW, j + 4);
        return j;
    }

    /// A Geomerics block from its magic (minus a prefix before the magic) to the next block (a known magic with its
    /// version) or the end.
    static byte[]? Block(byte[] native, byte[] magic, int prefix = 0, byte[]? through = null)
    {
        var i = Find(native, magic);
        if (i < 0) return null;
        var start = i - prefix;
        var ends = new List<int>();
        foreach (var (m, v) in new[] { (GESG, 37u), (GERD, 2u), (GEDG, 12u), (GEVS, 5u) })
        {
            if (through != null && m.SequenceEqual(through)) continue;
            var j = Find(native, m, i + 4);
            while (j >= 0 && U32(native, j + 4) != v) j = Find(native, m, j + 4);
            if (j > i) ends.Add(j);
        }
        var g = FindGeiw(native, i + 4);
        if (g > i) ends.Add(g - 0x20);
        return native[start..(ends.Count > 0 ? ends.Min() : native.Length)];
    }

    /// The system's saved albedo buffer (after GEVS): a 0x20-byte header (u32 the system guid's first, 0, 0, the
    /// cluster count, 0x20, 0, 0) then an 8-bit rgba per cluster.
    static byte[]? Albedo(byte[] native)
    {
        var gi = FindGeiw(native, 0);
        if (gi < 0) return null;
        var n = I32(native, gi + 8);
        var key = new byte[16];
        BitConverter.TryWriteBytes(key.AsSpan(8), n);
        BitConverter.TryWriteBytes(key.AsSpan(12), 0x20);
        var h = Find(native, key, Math.Max(Find(native, GEVS), 0));
        return h >= 4 ? native[(h + 24)..(h + 24 + 4 * n)] : null;
    }

    // ---- The atlas's records (UEnlightenAtlasBase::Serialize 0x1409cbca0; the archive version 0x1e).

    sealed class Reader(byte[] b, int o)
    {
        public int O = o;
        public uint U32() { var v = BitConverter.ToUInt32(b, O); O += 4; return v; }
        public int I32() { var v = BitConverter.ToInt32(b, O); O += 4; return v; }
        public void F6() => O += 24;
        public byte[] Bytes(int n) { var v = b[O..(O + n)]; O += n; return v; }
        public void FString() { var n = I32(); O += n < 0 ? -2 * n : n; }
    }

    /// One record (0x1414106b0: u32 type, then type 0 0x141427fd0, 1 0x141428160, 2 0x141427f00): its UV sets' buffers.
    static List<byte[]?> Record(Reader r, int v)
    {
        var t = r.U32();
        if (t > 2) throw new InvalidDataException($"record type {t} at {r.O - 4}");
        r.FString(); r.F6(); r.U32(); r.U32();
        if (v > 0x17) r.U32();
        if (v > 0x1d) r.U32();
        var uvs = new List<byte[]?>();
        if (t == 1)
        {
            r.U32();
            var n = r.U32();
            for (var i = 0; i < n; i++)
            {
                byte[]? buf = null;
                if (r.U32() != 0)
                {
                    int es = (int)r.U32(), count = (int)r.U32();
                    buf = r.Bytes(es * count);
                }
                r.I32();
                if (v > 0x1a) r.F6();
                uvs.Add(buf);
            }
        }
        else if (t == 2)
        {
            r.F6();
            if (v is > 0x14 and < 0x19) throw new InvalidDataException("version with an extra array");
            if (v is > 0x14 and < 0x1f) r.O += 4;
        }
        return uvs;
    }

    /// A package's components' EnlightenId by their path (outers inward, the outermost dropped), its atlas's primitive
    /// ids and their records (null: no atlas, or none serialized).
    static (Dictionary<string, string> Eid, List<string> Ids, List<List<byte[]?>> Records)? Resolve(Package p)
    {
        string PathOf(int i)
        {
            var parts = new List<string>();
            while (i >= 0)
            {
                var e = p.Exports[i];
                parts.Add(e.Name);
                i = e.Outer - 1;
            }
            parts.Reverse();
            return parts.Count > 1 ? string.Join(".", parts.Skip(1)) : parts[0];
        }
        var eid = new Dictionary<string, string>();
        for (var i = 0; i < p.Exports.Count; i++)
        {
            if (!p.Exports[i].Cls.Contains("Component")) continue;
            try
            {
                var (t, _) = p.Tags(p.Data(p.Exports[i]));
                if (Prop(t, "EnlightenId") is { } g) eid[PathOf(i)] = Guid(g);
            }
            catch (Exception) { }
        }
        var atlas = p.Exports.FirstOrDefault(e => e.Cls == "EnlightenAtlas");
        if (atlas == null) return null;
        var d = p.Data(atlas);
        var (at, o) = p.Tags(d);
        if (Prop(at, "SerializedData") is not { } sd) return null;
        var (st, _) = p.Tags(sd);
        var ids = p.StructArray(Prop(st, "Primitives")).Select(x => Guid(Prop(x, "PrimitiveId")!)).ToList();
        var native = d[o..];
        var records = new List<List<byte[]?>>();
        if (native.Length > 8)
        {
            var r = new Reader(native, 4);
            for (var k = 0; k < ids.Count; k++) records.Add(Record(r, 0x1e));
        }
        return (eid, ids, records);
    }

    /// The package's UEnlightenUVBufferCollection: mesh name -> its entries (their scale, LOD 0's packed UVs, u16 in
    /// 1/32768 of the system's output).
    static Dictionary<string, List<(float[] Scale, byte[] Data)>> Collection(Package p)
    {
        var output = new Dictionary<string, List<(float[], byte[])>>();
        var e = p.Exports.FirstOrDefault(x => x.Cls == "EnlightenUVBufferCollection");
        if (e == null) return output;
        var d = p.Data(e);
        var (_, o) = p.Tags(d);
        var n = d[o..];
        if (n.Length <= 8) return output;
        var count = U32(n, 4);
        var q = 8;
        for (var i = 0; i < count; i++)
        {
            int es = I32(n, q), num = I32(n, q + 4);
            var data = n[(q + 8)..(q + 8 + es * num)];
            q += 8 + es * num;
            var reference = I32(n, q);
            var lod = U32(n, q + 4);
            float[] scale = [BitConverter.ToSingle(n, q + 9), BitConverter.ToSingle(n, q + 13), BitConverter.ToSingle(n, q + 17)];
            q += 8 + 1 + 12 + 4;
            if (lod == 0 && reference < 0 && -reference <= p.Imports.Count)
            {
                var mesh = p.Ref(reference);
                if (!output.TryGetValue(mesh, out var list)) output[mesh] = list = [];
                list.Add((scale, data));
            }
        }
        return output;
    }

    public static void Write(string game, string outDir)
    {
        var dir = Path.Combine(outDir, "enlighten");
        Directory.CreateDirectory(dir);
        var maps = Path.Combine(game, Map);
        var systems = new JArray();
        var atlases = new JObject();
        var probeSets = new JArray();
        var packages = new Dictionary<string, Package>();
        foreach (var f in Directory.GetFiles(maps, "*.umap").Order(StringComparer.Ordinal))
        {
            var p = new Package(f);
            var pkg = Path.GetFileNameWithoutExtension(f);
            packages[pkg] = p;
            foreach (var e in p.Exports)
            {
                if (e.Cls == "EnlightenAtlas")
                {
                    var d = p.Data(e);
                    var (t, _) = p.Tags(d);
                    if (Prop(t, "SerializedData") is not { } sd)
                    {
                        atlases[pkg] = new JObject { ["width"] = 0, ["height"] = 0, ["primitives"] = new JArray(), ["instanced"] = new JArray() };
                        continue;
                    }
                    var (st, _) = p.Tags(sd);
                    var instanced = new JArray(p.StructArray(Prop(st, "InstancedPrimitives")).Select(ip => new JObject
                    {
                        ["primitive"] = Guid(Prop(ip, "PrimitiveId")!),
                        ["uv_scale_bias"] = new JArray(p.StructArray(Prop(ip, "Instances")).Select(it =>
                        {
                            var v = Prop(it, "UVScaleBias")!;
                            return new JArray(Enumerable.Range(0, 4).Select(k => (double)BitConverter.ToSingle(v, 4 * k)));
                        })),
                    }));
                    atlases[pkg] = new JObject
                    {
                        ["width"] = I32(Prop(t, "Width")!, 0),
                        ["height"] = I32(Prop(t, "Height")!, 0),
                        ["primitives"] = new JArray(p.StructArray(Prop(st, "Primitives")).Select(x => Guid(Prop(x, "PrimitiveId")!))),
                        ["instanced"] = instanced,
                    };
                }
                if (e.Cls == "EnlightenProbeSet")
                {
                    var d = p.Data(e);
                    var (_, o) = p.Tags(d);
                    var native = d[o..];
                    int i = Find(native, GESH), j = Find(native, GEPO), k = Find(native, GEPV);
                    if (Math.Min(i, Math.Min(j, k)) < 0) continue;
                    var name = $"{pkg}.{e.Name}";
                    File.WriteAllBytes(Path.Combine(dir, $"{name}.gesh.bin"), native[i..j]);
                    File.WriteAllBytes(Path.Combine(dir, $"{name}.gepo.bin"), native[j..k]);
                    File.WriteAllBytes(Path.Combine(dir, $"{name}.gepv.bin"), native[k..]);
                    const int core = 0x34;
                    probeSets.Add(new JObject
                    {
                        ["package"] = pkg,
                        ["name"] = e.Name,
                        ["probes"] = I32(native, core + 0x10),
                        ["origin"] = new JArray(Enumerable.Range(0, 3).Select(a => (double)BitConverter.ToSingle(native, core + 0x50 + 4 * a))),
                        ["size"] = (double)BitConverter.ToSingle(native, core + 0x60),
                    });
                    continue;
                }
                if (e.Cls != "EnlightenSystem") continue;
                {
                    var d = p.Data(e);
                    var (t, o) = p.Tags(d);
                    var native = d[o..];
                    var alloc = new JArray(p.StructArray(Prop(t, "AtlasAllocations")).Select(a => new JObject(a.Select(x => new JProperty(x.Name, U32(x.Value, 0))))));
                    var st = Prop(t, "SerializedData") is { } sd ? p.Tags(sd).Item1 : [];
                    var prims = new JArray(p.StructArray(Prop(st, "Primitives")).Select(x => Guid(Prop(x, "PrimitiveId")!)));
                    var gesg = Block(native, GESG);
                    var geiw = Block(native, GEIW, 0x20, GEDG);
                    var gerd = Block(native, GERD);
                    var alb = Albedo(native);
                    var gevs = Block(native, GEVS);
                    var name = $"{pkg}.{e.Name}";
                    foreach (var (tag, b) in new[] { ("gesg", gesg), ("geiw", geiw), ("gerd", gerd), ("alb", alb), ("gevs", gevs) })
                        if (b is { Length: > 0 }) File.WriteAllBytes(Path.Combine(dir, $"{name}.{tag}.bin"), b);
                    systems.Add(new JObject
                    {
                        ["package"] = pkg,
                        ["name"] = e.Name,
                        ["guid"] = Guid(Prop(t, "SystemGuid")!),
                        ["width"] = Prop(t, "Width") is { } w ? I32(w, 0) : 0,
                        ["height"] = Prop(t, "Height") is { } h ? I32(h, 0) : 0,
                        ["atlas"] = alloc,
                        ["primitives"] = prims,
                        ["gesg"] = gesg is { Length: > 0 },
                        ["geiw"] = geiw is { Length: > 0 },
                    });
                }
            }
        }
        var index = new JObject { ["systems"] = systems, ["atlases"] = atlases };
        if (probeSets.Count > 0) index["probe_sets"] = probeSets;
        File.WriteAllText(Path.Combine(dir, "index.json"), index.ToString(Newtonsoft.Json.Formatting.Indented));
        Console.WriteLine($"enlighten: {systems.Count} systems, {systems.Count(s => (bool)s["gesg"]!)} with GESG, {systems.Count(s => (bool)s["geiw"]!)} with GEIW; {atlases.Count} atlases, {probeSets.Count} probe sets");

        // Every level file's placements by their level (the main map's file holds its sublevels').
        var placements = new SortedDictionary<string, List<JToken>>(StringComparer.Ordinal);
        foreach (var f in Directory.GetFiles(Path.Combine(outDir, "levels"), "*.json").Order(StringComparer.Ordinal))
            foreach (var pl in JObject.Parse(File.ReadAllText(f))["Placements"] ?? new JArray())
            {
                var level = pl["Level"]?.ToString() ?? "";
                if (!placements.TryGetValue(level, out var list)) placements[level] = list = [];
                list.Add(pl);
            }
        static string Comp(JToken pl) => pl["Component"]!.ToString().Split(':')[^1];
        foreach (var (pk, pls) in placements)
        {
            if (!packages.TryGetValue(pk, out var p)) continue;
            var resolved = Resolve(p);
            if (resolved is not var (eid, ids, records)) continue;
            Primitives(dir, pk, p, eid, ids, records, pls, index);
            Instances(dir, pk, eid, pls.Where(pl => pl["Instances"] is JArray { Count: > 0 }).ToList(), index);
        }
        Land(dir, index);
    }

    /// <package>.prims.bin: u32 count, per primitive u16 length + its component path, u32 vertex count and its packed
    /// (u16 u, u16 v) atlas UVs (its record's first UV set, LOD 0, in Unreal's vertex order).
    static void Primitives(string dir, string pk, Package p, Dictionary<string, string> eid, List<string> ids, List<List<byte[]?>> records, List<JToken> pls, JObject index)
    {
        if (records.Count == 0) return;
        var byId = new Dictionary<string, int>();
        for (var k = 0; k < ids.Count; k++) byId[ids[k]] = k;
        var rows = new List<(string, byte[])>();
        foreach (var (comp, g) in eid)
            if (byId.TryGetValue(g, out var k) && records[k] is [{ Length: > 0 } uvs, ..])
                rows.Add((comp, uvs));
        // The primitives whose UVs are their mesh's (UEnlightenUVBufferCollection, LOD 0: local to their system's
        // output), placed by their system's atlas allocation: atlas uv = (base + uv x size) / the atlas's size.
        var atlas = index["atlases"]![pk] as JObject;
        var sysOf = new Dictionary<string, JToken>();
        foreach (var s in index["systems"]!)
        {
            if (s["package"]!.ToString() != pk || s["atlas"] is not JArray { Count: > 0 }) continue;
            foreach (var g in s["primitives"]!) sysOf[g.ToString()] = s;
        }
        var collection = Collection(p);
        var meshOf = new Dictionary<string, string>();
        var scaleOf = new Dictionary<string, double[]>();
        foreach (var pl in pls)
        {
            var c = pl["Component"]!.ToString().Split(':')[^1];
            meshOf[c] = (pl["Mesh"]?.ToString() ?? "").Split('.')[^1];
            scaleOf[c] = pl["Transform"] is JObject { } tr && tr["S"] is JArray { Count: 3 } sc ? sc.Select(x => (double)x).ToArray() : [1.0, 1.0, 1.0];
        }
        var have = rows.Select(r => r.Item1).ToHashSet();
        var added = 0;
        foreach (var (comp, g) in eid)
        {
            if (have.Contains(comp) || !sysOf.TryGetValue(g, out var s) || atlas == null || !atlas.HasValues) continue;
            if (!meshOf.TryGetValue(comp, out var mesh) || !collection.TryGetValue(mesh, out var entries) || entries.Count == 0) continue;
            // (The entry at the placement's scale.)
            var sc = scaleOf.GetValueOrDefault(comp, [1.0, 1.0, 1.0]);
            var buf = entries.MinBy(e => Enumerable.Range(0, 3).Sum(a => Math.Pow(e.Scale[a] - sc[a], 2))).Data;
            var a0 = s["atlas"]![0]!;
            double baseX = (double)a0["AtlasBaseX"]!, baseY = (double)a0["AtlasBaseY"]!, w = (double)a0["Width"]!, h = (double)a0["Height"]!;
            double aw = (double)atlas["width"]!, ah = (double)atlas["height"]!;
            var output = new byte[buf.Length / 4 * 4];
            for (var i = 0; i + 4 <= buf.Length; i += 4)
            {
                int u = BitConverter.ToUInt16(buf, i), v = BitConverter.ToUInt16(buf, i + 2);
                var au = (baseX + u / 32768.0 * w) / aw;
                var av = (baseY + v / 32768.0 * h) / ah;
                BitConverter.TryWriteBytes(output.AsSpan(i), (ushort)Math.Min(65535, Math.Round(au * 65535)));
                BitConverter.TryWriteBytes(output.AsSpan(i + 2), (ushort)Math.Min(65535, Math.Round(av * 65535)));
            }
            rows.Add((comp, output));
            added++;
        }
        using var wr = new BinaryWriter(File.Create(Path.Combine(dir, pk + ".prims.bin")));
        wr.Write((uint)rows.Count);
        foreach (var (comp, data) in rows)
        {
            var c = Encoding.UTF8.GetBytes(comp);
            wr.Write((ushort)c.Length); wr.Write(c); wr.Write((uint)(data.Length / 4)); wr.Write(data);
        }
        Console.WriteLine($"enlighten: {pk} {rows.Count} primitives with atlas UVs, {added} of them through their mesh's buffer");
    }

    /// <package>.inst.bin: u32 count, per component u16 length + its path, u32 instance count and its instances'
    /// (scale u, scale v, bias u, bias v) f32s, in its instance order (the atlas's InstancedPrimitives by its EnlightenId).
    static void Instances(string dir, string pk, Dictionary<string, string> eid, List<JToken> pls, JObject index)
    {
        var inst = new Dictionary<string, JArray>();
        foreach (var i in (index["atlases"]![pk]?["instanced"] as JArray) ?? []) inst[i["primitive"]!.ToString()] = (JArray)i["uv_scale_bias"]!;
        if (inst.Count == 0 || pls.Count == 0) return;
        var rows = new List<(string, JArray)>();
        int missing = 0, mismatch = 0;
        foreach (var pl in pls)
        {
            var comp = pl["Component"]!.ToString().Split(':')[^1];
            if (!eid.TryGetValue(comp, out var g) || !inst.TryGetValue(g, out var uvs)) { missing++; continue; }
            if (uvs.Count != ((JArray)pl["Instances"]!).Count) { mismatch++; continue; }
            rows.Add((comp, uvs));
        }
        using var w = new BinaryWriter(File.Create(Path.Combine(dir, pk + ".inst.bin")));
        w.Write((uint)rows.Count);
        foreach (var (comp, uvs) in rows)
        {
            var c = Encoding.UTF8.GetBytes(comp);
            w.Write((ushort)c.Length); w.Write(c); w.Write((uint)uvs.Count);
            foreach (var s in uvs) foreach (var x in s) w.Write((float)(double)x);
        }
        Console.WriteLine($"enlighten: {pk} {rows.Count} instanced components with their instances' UVs; {missing} not in the atlas, {mismatch} with other instance counts");
    }

    /// GEIW (from its 0x20 prefix): each cluster's points' mean (UE cm), null without points.
    static List<double[]?> GeiwClusters(byte[] g)
    {
        int clusters = I32(g, 0x28), oPts = I32(g, 0x30), oBox = I32(g, 0x34), oCnt = I32(g, 0x40);
        var output = new List<double[]?>();
        var packet = 0;
        for (var c = 0; c < clusters; c++)
        {
            var mn = Enumerable.Range(0, 3).Select(a => (double)BitConverter.ToSingle(g, oBox + 32 * c + 4 * a)).ToArray();
            var mx = Enumerable.Range(0, 3).Select(a => (double)BitConverter.ToSingle(g, oBox + 32 * c + 16 + 4 * a)).ToArray();
            int n = g[oCnt + 0x10 + c], count = 0;
            var sum = new double[3];
            for (var i = 0; i < n; i++)
            {
                var q = oPts + 16 * packet; packet++;
                for (var j = 0; j < 4; j++, count++)
                    for (var a = 0; a < 3; a++) sum[a] += mn[a] + (mx[a] - mn[a]) * g[q + 4 * j + a] / 255.0;
            }
            output.Add(count > 0 ? sum.Select(x => x / count).ToArray() : null);
        }
        return output;
    }

    /// GERD: each cluster's resampled output pixels (x, y, weight).
    static List<List<(int X, int Y, double W)>> GerdClusters(byte[] b)
    {
        var n = (int)U32(b, 8);
        var p = (0x20 + 2 * n + 7) & ~7;
        var output = new List<List<(int, int, double)>>();
        for (var c = 0; c < n; c++)
        {
            var k = BitConverter.ToUInt16(b, 0x20 + 2 * c);
            var s = new List<(int, int, double)>();
            for (var i = 0; i < k; i++, p += 8) s.Add((BitConverter.ToUInt16(b, p), BitConverter.ToUInt16(b, p + 2), BitConverter.ToSingle(b, p + 4)));
            output.Add(s);
        }
        return output;
    }

    /// The least squares M (2 x 3) with M . (x, y, 1) ~ (px, py), by the normal equations; and each fit's p95 residual
    /// (the larger axis's, as numpy's abs(A M - B) flattened).
    static (double[][] M, double P95) Fit(List<double[]> a, List<double[]> b)
    {
        var ata = new double[3, 3];
        var atb = new double[3, 2];
        for (var i = 0; i < a.Count; i++)
            for (var r = 0; r < 3; r++)
            {
                for (var c = 0; c < 3; c++) ata[r, c] += a[i][r] * a[i][c];
                for (var c = 0; c < 2; c++) atb[r, c] += a[i][r] * b[i][c];
            }
        // Gaussian elimination with partial pivoting on [ata | atb].
        var m = new double[3, 5];
        for (var r = 0; r < 3; r++) { for (var c = 0; c < 3; c++) m[r, c] = ata[r, c]; m[r, 3] = atb[r, 0]; m[r, 4] = atb[r, 1]; }
        for (var col = 0; col < 3; col++)
        {
            var piv = Enumerable.Range(col, 3 - col).MaxBy(r => Math.Abs(m[r, col]));
            for (var c = 0; c < 5; c++) (m[col, c], m[piv, c]) = (m[piv, c], m[col, c]);
            for (var r = 0; r < 3; r++)
            {
                if (r == col || m[col, col] == 0) continue;
                var f = m[r, col] / m[col, col];
                for (var c = 0; c < 5; c++) m[r, c] -= f * m[col, c];
            }
        }
        var x = new double[2][];
        for (var k = 0; k < 2; k++) x[k] = [m[0, 3 + k] / m[0, 0], m[1, 3 + k] / m[1, 1], m[2, 3 + k] / m[2, 2]];
        var res = new List<double>();
        for (var i = 0; i < a.Count; i++)
            for (var k = 0; k < 2; k++) res.Add(Math.Abs(x[k][0] * a[i][0] + x[k][1] * a[i][1] + x[k][2] - b[i][k]));
        res.Sort();
        // numpy's percentile: linear between the ranks.
        var rank = 0.95 * (res.Count - 1);
        var lo = (int)Math.Floor(rank);
        var p95 = res.Count == 0 ? 0 : res[lo] + (res[Math.Min(lo + 1, res.Count - 1)] - res[lo]) * (rank - lo);
        return (x, p95);
    }

    /// <Terrain>.land.json: per landscape component cell its atlas UV from world XY: uv = m . (x, y, 1) (UE cm), the
    /// system's fitted pixel map placed by its atlas allocation.
    static void Land(string dir, JObject index)
    {
        if (index["atlases"]![Terrain] is not JObject atlas) return;
        double aw = (double)atlas["width"]!, ah = (double)atlas["height"]!;
        var cells = new JArray();
        foreach (var s in index["systems"]!)
        {
            if (s["package"]!.ToString() != Terrain || !(bool)s["gesg"]!) continue;
            var name = s["name"]!.ToString();
            string geiwFile = Path.Combine(dir, $"{Terrain}.{name}.geiw.bin"), gerdFile = Path.Combine(dir, $"{Terrain}.{name}.gerd.bin");
            if (!File.Exists(geiwFile) || !File.Exists(gerdFile)) continue;
            var w = GeiwClusters(File.ReadAllBytes(geiwFile));
            var r = GerdClusters(File.ReadAllBytes(gerdFile));
            var groups = new SortedDictionary<(int, int), (List<double[]> A, List<double[]> B)>();
            for (var c = 0; c < Math.Min(w.Count, r.Count); c++)
            {
                if (w[c] is not { } pt || r[c].Count == 0) continue;
                var tw = r[c].Sum(x => x.W);
                var px = r[c].Sum(x => (x.X + 0.5) * x.W) / tw;
                var py = r[c].Sum(x => (x.Y + 0.5) * x.W) / tw;
                var key = ((int)Math.Floor((pt[0] - Origin) / Cell), (int)Math.Floor((pt[1] - Origin) / Cell));
                if (!groups.TryGetValue(key, out var g)) groups[key] = g = ([], []);
                g.A.Add([pt[0], pt[1], 1.0]);
                g.B.Add([px, py]);
            }
            var a0 = s["atlas"]![0]!;
            double baseX = (double)a0["AtlasBaseX"]!, baseY = (double)a0["AtlasBaseY"]!;
            foreach (var (key, (a, b)) in groups)
            {
                if (a.Count < 6) continue;
                var (m, p95) = Fit(a, b);
                cells.Add(new JObject
                {
                    ["cell"] = new JArray(key.Item1, key.Item2),
                    ["system"] = name,
                    ["m"] = new JArray(
                        new JArray(m[0][0] / aw, m[0][1] / aw, (m[0][2] + baseX) / aw),
                        new JArray(m[1][0] / ah, m[1][1] / ah, (m[1][2] + baseY) / ah)),
                    ["residual_p95_px"] = p95,
                });
            }
        }
        File.WriteAllText(Path.Combine(dir, Terrain + ".land.json"), new JObject { ["origin"] = Origin, ["size"] = Cell, ["cells"] = cells }.ToString(Newtonsoft.Json.Formatting.Indented));
        Console.WriteLine($"enlighten: {Terrain} {cells.Count} landscape cells");
    }
}
