// The level's cooked Recast navmeshes (ARecastNavMesh actors: one per agent in DefaultEngine.ini's SupportedAgents).
// UE 4.18 serializes them after the actor's tagged properties: an int32 (no object GUID), NavMeshVersion, the size of
// what follows, then FPImplRecastNavMesh::Serialize: the tile count, dtNavMeshParams (origin, tile width/height,
// max tiles, max polys) and per tile its dtTileRef, its in-memory size, the tile (SerializeRecastMeshTile: counts,
// dtMeshHeader field by field, vertices, polygons, detail meshes/vertices/triangles, BV tree, off-mesh connections
// and segments, clusters, poly clusters) and its compressed tile cache layer (skipped here).
// Writes navmesh-<agent>.json: per tile its grid cell and layer, its vertices (Recast space: Unreal (-X, Z, -Y), cm)
// and polygons (vertex indices, neighbours per edge: 0 none, 1.. a polygon of the tile, 0x8000 | side the tile's
// border, area).
using System.Text.Json;
using CUE4Parse.FileProvider;

static class NavMeshExporter
{
    const int VertsPerPoly = 6;

    public class Tile
    {
        public int X, Y, Layer;
        public float[] Verts = [];
        public List<Poly> Polys = new();
    }

    public class Poly
    {
        public ushort[] V = [];
        public ushort[] N = [];
        public int Area;
    }

    public class Mesh
    {
        public string Agent = "";
        public float AgentRadius, AgentHeight, MaxClimb;
        public float TileWidth, TileHeight;
        public float[] Origin = [];
        public List<Tile> Tiles = new();
    }

    public static void Write(IFileProvider provider, string outDir, string map)
    {
        var file = map.TrimStart('/').Replace("Game/", "SummerCamp/Content/") + ".umap";
        var pkg = (CUE4Parse.UE4.Assets.Package)provider.LoadPackage(file);
        var bytes = provider.SavePackage(file).OrderBy(f => f.Key.EndsWith(".uexp")).SelectMany(f => f.Value).ToArray();
        var none = Array.FindIndex(pkg.NameMap, n => n.Name == "None");
        foreach (var e in pkg.ExportMap.Where(x => x.ClassName == "RecastNavMesh"))
        {
            var data = bytes.AsSpan((int)e.SerialOffset, (int)e.SerialSize).ToArray();
            Mesh mesh;
            try { mesh = Parse(data, none, e.ObjectName.Text); }
            catch (Exception ex) { Console.WriteLine($"skipped: {ex.Message}"); continue; }
            var agent = e.ObjectName.Text.Replace("RecastNavMesh-", "");
            mesh.Agent = agent;
            var path = Path.Combine(outDir, $"navmesh-{agent}.json");
            File.WriteAllText(path, JsonSerializer.Serialize(mesh, new JsonSerializerOptions { IncludeFields = true }));
            Console.WriteLine($"{path}: {mesh.Tiles.Count} tiles, {mesh.Tiles.Sum(t => t.Polys.Count)} polygons");
        }
    }

    static Mesh Parse(byte[] b, int noneIndex, string name)
    {
        // The tagged properties end with the name None; the native data follows (after a 4-byte zero).
        var start = -1;
        for (var i = 0; i + 20 <= b.Length; i++)
            if (BitConverter.ToInt32(b, i) == noneIndex && BitConverter.ToInt32(b, i + 4) == 0 && BitConverter.ToInt32(b, i + 8) == 0
                && BitConverter.ToUInt32(b, i + 16) == b.Length - (i + 16)) { start = i + 12; break; } // the size counts itself
        if (start < 0) throw new Exception($"{name}: no navmesh data found");
        var r = new BinaryReader(new MemoryStream(b, start, b.Length - start));
        var version = r.ReadUInt32();
        if (version != 13) throw new Exception($"{name}: navmesh version {version} (expected 13)");
        r.ReadUInt32(); // size of the rest
        var mesh = new Mesh();
        var tiles = r.ReadInt32();
        mesh.Origin = [r.ReadSingle(), r.ReadSingle(), r.ReadSingle()];
        mesh.TileWidth = r.ReadSingle();
        mesh.TileHeight = r.ReadSingle();
        r.ReadInt32(); r.ReadInt32(); // max tiles, max polys
        for (var t = 0; t < tiles; t++)
        {
            r.ReadUInt64(); r.ReadInt32(); // tile ref, in-memory size
            for (var i = 0; i < 10; i++) r.ReadInt32(); // counts (repeated in the header)
            if (r.ReadInt32() != 0x444E4156) throw new Exception($"{name}: tile {t}: bad magic");
            r.ReadInt32(); // version
            var tile = new Tile { X = r.ReadInt32(), Y = r.ReadInt32(), Layer = r.ReadInt32() };
            r.ReadInt32(); // userId
            int polyCount = r.ReadInt32(), vertCount = r.ReadInt32(); r.ReadInt32();
            int detailMeshes = r.ReadInt32(), detailVerts = r.ReadInt32(), detailTris = r.ReadInt32(), bvNodes = r.ReadInt32(), offMesh = r.ReadInt32();
            r.ReadInt32(); // offMeshBase
            mesh.AgentHeight = r.ReadSingle(); mesh.AgentRadius = r.ReadSingle(); mesh.MaxClimb = r.ReadSingle();
            for (var i = 0; i < 7; i++) r.ReadSingle(); // bmin, bmax, bvQuantFactor
            var clusters = r.ReadInt32();
            var offMeshSegs = r.ReadInt32(); r.ReadInt32(); r.ReadInt32();
            if (offMesh != 0 || offMeshSegs != 0) throw new Exception($"{name}: tile {t}: off-mesh connections are not handled");
            tile.Verts = new float[vertCount * 3];
            for (var i = 0; i < tile.Verts.Length; i++) tile.Verts[i] = r.ReadSingle();
            for (var p = 0; p < polyCount; p++)
            {
                r.ReadUInt32(); // firstLink
                var v = new ushort[VertsPerPoly]; var n = new ushort[VertsPerPoly];
                for (var i = 0; i < VertsPerPoly; i++) v[i] = r.ReadUInt16();
                for (var i = 0; i < VertsPerPoly; i++) n[i] = r.ReadUInt16();
                r.ReadUInt16(); // flags
                int count = r.ReadByte(), areaAndType = r.ReadByte();
                tile.Polys.Add(new Poly { V = v[..count], N = n[..count], Area = areaAndType & 0x3f });
            }
            r.BaseStream.Seek(detailMeshes * 10 + detailVerts * 12 + detailTris * 4 + bvNodes * 16 + clusters * 12 + polyCount * 2, SeekOrigin.Current);
            var layerSize = r.ReadInt32();
            r.BaseStream.Seek(layerSize, SeekOrigin.Current);
            mesh.Tiles.Add(tile);
        }
        if (r.BaseStream.Position != r.BaseStream.Length) throw new Exception($"{name}: {r.BaseStream.Length - r.BaseStream.Position} bytes left over");
        return mesh;
    }
}
