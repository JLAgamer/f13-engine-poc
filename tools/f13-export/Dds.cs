// DDS copies of the exported textures: the game's own block-compressed data (BC1/BC3/BC5...) with its full mip chain,
// wrapped in a DX10 DDS header. Uncompressed PNGs without mips cost several times the GPU memory and shimmer at a
// distance; f13-game prefers the .dds when there is one.
using CUE4Parse.FileProvider;
using CUE4Parse.UE4.Assets.Exports.Texture;

static class DdsExporter
{
    /// Writes <file>.dds next to every exported texture PNG under <root>/SummerCamp. Returns (written, skipped).
    public static (int, int) WriteAll(IFileProvider provider, string root)
    {
        var dir = Path.Combine(root, "SummerCamp");
        if (!Directory.Exists(dir)) return (0, 0);
        int written = 0, skipped = 0;
        var formats = new Dictionary<string, int>();
        // Textures that don't wrap (AddressX/Y TA_Clamp or TA_Mirror): texture_address.json, path -> [X, Y].
        var address = new SortedDictionary<string, string[]>();
        // PNGs, and the float textures' Radiance .hdr (no alpha: the .dds keeps it).
        foreach (var png in Directory.EnumerateFiles(dir, "*.png", SearchOption.AllDirectories).Concat(Directory.EnumerateFiles(dir, "*.hdr", SearchOption.AllDirectories)))
        {
            var rel = Path.GetRelativePath(root, png).Replace(Path.DirectorySeparatorChar, '/')[..^".png".Length];
            UTexture2D? tex;
            UTextureCube? cube = null;
            try
            {
                var exports = provider.LoadPackage(rel).GetExports().ToList();
                tex = exports.OfType<UTexture2D>().FirstOrDefault();
                cube = exports.OfType<UTextureCube>().FirstOrDefault();
            }
            catch { tex = null; }
            if (tex == null && cube != null)
            {
                // A cubemap (CUE4Parse writes it as a lat-long .hdr): its six faces as a DDS cube.
                if (WriteCube(cube, Path.ChangeExtension(png, ".dds"))) written++;
                else skipped++;
                continue;
            }
            if (tex == null) { skipped++; continue; }
            var props = Newtonsoft.Json.Linq.JObject.Parse(Newtonsoft.Json.JsonConvert.SerializeObject(tex))["Properties"];
            var ax = props?["AddressX"]?.ToString() ?? "TA_Wrap";
            var ay = props?["AddressY"]?.ToString() ?? "TA_Wrap";
            if (ax != "TA_Wrap" || ay != "TA_Wrap") address[rel] = [ax, ay];
            var key = tex.Format.ToString();
            formats[key] = formats.GetValueOrDefault(key) + 1;
            if (Write(tex, Path.ChangeExtension(png, ".dds"))) written++;
            else skipped++;
        }
        File.WriteAllText(Path.Combine(root, "texture_address.json"), Newtonsoft.Json.JsonConvert.SerializeObject(address, Newtonsoft.Json.Formatting.Indented));
        Console.WriteLine($"dds: {written} written, {skipped} skipped; formats {string.Join(", ", formats.OrderByDescending(f => f.Value).Select(f => $"{f.Key} {f.Value}"))}");
        return (written, skipped);
    }

    static uint? Dxgi(EPixelFormat f, bool srgb) => f switch
    {
        EPixelFormat.PF_DXT1 => srgb ? 72u : 71u,     // BC1
        EPixelFormat.PF_DXT3 => srgb ? 75u : 74u,     // BC2
        EPixelFormat.PF_DXT5 => srgb ? 78u : 77u,     // BC3
        EPixelFormat.PF_BC4 => 80u,
        EPixelFormat.PF_BC5 => 83u,
        EPixelFormat.PF_BC7 => srgb ? 99u : 98u,
        EPixelFormat.PF_B8G8R8A8 => srgb ? 91u : 87u,
        EPixelFormat.PF_G8 => 61u,                    // R8_UNORM
        EPixelFormat.PF_FloatRGBA => 10u,             // R16G16B16A16_FLOAT
        _ => null,
    };

    static bool Write(UTexture2D tex, string path)
    {
        var pd = tex.PlatformData;
        if (pd == null || Dxgi(tex.Format, tex.SRGB) is not { } dxgi) return false;
        // Keep the chain down to the first missing level (cooked textures normally have them all).
        var mips = pd.Mips.TakeWhile(m => m.BulkData?.Data is { Length: > 0 }).ToList();
        if (mips.Count == 0) return false;
        using var w = new BinaryWriter(File.Create(path));
        w.Write(0x20534444u); // "DDS "
        w.Write(124u);
        w.Write(0x1u | 0x2u | 0x4u | 0x1000u | 0x20000u | 0x80000u); // caps, height, width, pixelformat, mipmapcount, linearsize
        w.Write((uint)mips[0].SizeY);
        w.Write((uint)mips[0].SizeX);
        w.Write((uint)mips[0].BulkData!.Data!.Length);
        w.Write(0u); // depth
        w.Write((uint)mips.Count);
        for (var i = 0; i < 11; i++) w.Write(0u);
        // DDS_PIXELFORMAT: FourCC "DX10"
        w.Write(32u);
        w.Write(0x4u);
        w.Write(0x30315844u);
        for (var i = 0; i < 5; i++) w.Write(0u);
        w.Write(0x1000u | 0x8u | 0x400000u); // texture, complex, mipmap
        for (var i = 0; i < 4; i++) w.Write(0u);
        // DDS_HEADER_DXT10
        w.Write(dxgi);
        w.Write(3u); // TEXTURE2D
        w.Write(0u);
        w.Write(1u); // array size
        w.Write(0u);
        foreach (var m in mips) w.Write(m.BulkData!.Data!);
        return true;
    }

    /// A cube texture as a DDS cubemap. Unreal keeps each mip's six faces together (+X, -X, +Y, -Y, +Z, -Z, as D3D);
    /// a DDS stores each face with its whole mip chain.
    static bool WriteCube(UTextureCube cube, string path)
    {
        var pd = cube.PlatformData;
        if (pd == null || Dxgi(cube.Format, cube.SRGB) is not { } dxgi) return false;
        var mips = pd.Mips.TakeWhile(m => m.BulkData?.Data is { Length: > 0 }).ToList();
        if (mips.Count == 0 || mips.Any(m => m.BulkData!.Data!.Length % 6 != 0)) return false;
        using var w = new BinaryWriter(File.Create(path));
        w.Write(0x20534444u); // "DDS "
        w.Write(124u);
        w.Write(0x1u | 0x2u | 0x4u | 0x1000u | 0x20000u | 0x80000u);
        w.Write((uint)mips[0].SizeY);
        w.Write((uint)mips[0].SizeX);
        w.Write((uint)(mips[0].BulkData!.Data!.Length / 6));
        w.Write(0u);
        w.Write((uint)mips.Count);
        for (var i = 0; i < 11; i++) w.Write(0u);
        w.Write(32u);
        w.Write(0x4u);
        w.Write(0x30315844u);
        for (var i = 0; i < 5; i++) w.Write(0u);
        w.Write(0x1000u | 0x8u | 0x400000u); // texture, complex, mipmap
        w.Write(0x200u | 0xFC00u); // caps2: cubemap, all six faces
        for (var i = 0; i < 3; i++) w.Write(0u);
        w.Write(dxgi);
        w.Write(3u); // TEXTURE2D
        w.Write(0x4u); // misc: TEXTURECUBE
        w.Write(6u); // array size: six faces, as ddsfile (f13-game's loader) counts a cube's layers
        w.Write(0u);
        for (var face = 0; face < 6; face++)
            foreach (var m in mips)
            {
                var data = m.BulkData!.Data!;
                var size = data.Length / 6;
                w.Write(data, face * size, size);
            }
        return true;
    }
}
