#define_import_path bevy_pbr::irradiance_volume

#import bevy_pbr::light_probe::{light_probe_iterator_new, light_probe_iterator_next}
#import bevy_pbr::mesh_view_bindings::{
    irradiance_volumes,
    irradiance_volume,
    irradiance_volume_sampler,
    light_probes,
};
#import bevy_pbr::clustered_forward::ClusterableObjectIndexRanges

#ifdef LIGHTMAP
#import bevy_pbr::mesh_view_types::LIGHT_PROBE_FLAG_AFFECTS_LIGHTMAPPED_MESH_DIFFUSE;
#endif

#ifdef IRRADIANCE_VOLUMES_ARE_USABLE

// See:
// https://advances.realtimerendering.com/s2006/Mitchell-ShadingInValvesSourceEngine.pdf
// Slide 28, "Ambient Cube Basis"
fn irradiance_volume_light(
    world_position: vec3<f32>,
    N: vec3<f32>,
    clusterable_object_index_ranges: ptr<function, ClusterableObjectIndexRanges>,
) -> vec3<f32> {
    // Find all irradiance volumes that contain the fragment. We're going to
    // accumulate all the irradiance from them in a weighted sum.
    var iterator = light_probe_iterator_new(
        world_position,
        /*is_irradiance_volume=*/ true,
        clusterable_object_index_ranges,
    );

    var total_weight = 0.0;
    var total_irradiance = vec3(0.0);

    while (true) {
        var query_result = light_probe_iterator_next(&iterator);

        // If there was no irradiance volume found, we're done.
        if (query_result.texture_index < 0) {
            break;
        }

        // If we're lightmapped, and the irradiance volume contributes no diffuse
        // light, then bail out.
    #ifdef LIGHTMAP
        if ((query_result.flags & LIGHT_PROBE_FLAG_AFFECTS_LIGHTMAPPED_MESH_DIFFUSE) == 0u) {
            continue;
        }
    #endif  // LIGHTMAP

    #ifdef MULTIPLE_LIGHT_PROBES_IN_ARRAY
        let irradiance_volume_texture = irradiance_volumes[query_result.texture_index];
    #else
        let irradiance_volume_texture = irradiance_volume;
    #endif

        let atlas_resolution = vec3<f32>(textureDimensions(irradiance_volume_texture));
        let resolution = vec3<f32>(textureDimensions(irradiance_volume_texture) / vec3(1u, 2u, 3u));

        // Make sure to clamp to the edges to avoid texture bleed.
        var unit_pos = (query_result.light_from_world * vec4(world_position, 1.0f)).xyz;
        let stp = clamp((unit_pos + 0.5) * resolution, vec3(0.5f), resolution - vec3(0.5f));
        let uvw = stp / atlas_resolution;

        // f13-rs: the volume holds Enlighten's L1 probes (f13-game enlighten.rs ProbeVolume): the +X slot their L0,
        // -X / +Y / -Y their L1 along x / y / z (signed), and the game's movable base pass (M_Prop_Master s001)
        // reconstructs each channel per pixel: r = |L1| / L0, q = saturate(dot(N, L1 / |L1|) / 2 + 1/2),
        // a = (1 - r) / (1 + r), p = 1 + 2r: L0 (a + (1 - a)(p + 1) q^p) (L0 alone with no L1).
        let l0 = textureSampleLevel(irradiance_volume_texture, irradiance_volume_sampler, uvw, 0.0).rgb;
        let lx = textureSampleLevel(irradiance_volume_texture, irradiance_volume_sampler, uvw + vec3(0.0f, 0.5f, 0.0f), 0.0).rgb;
        let ly = textureSampleLevel(irradiance_volume_texture, irradiance_volume_sampler, uvw + vec3(0.0f, 0.0f, 1.0f / 3.0f), 0.0).rgb;
        let lz = textureSampleLevel(irradiance_volume_texture, irradiance_volume_sampler, uvw + vec3(0.0f, 0.5f, 1.0f / 3.0f), 0.0).rgb;
        var rgb = vec3(0.0);
        for (var c = 0; c < 3; c += 1) {
            let L0 = l0[c];
            let L1 = vec3(lx[c], ly[c], lz[c]);
            let len = length(L1);
            var v = L0;
            if (len != 0.0 && L0 > 0.0) {
                let r = len / L0;
                let q = saturate(dot(N, L1 / len) * 0.5 + 0.5);
                let a = (1.0 - r) / (1.0 + r);
                let p = 1.0 + 2.0 * r;
                v = L0 * (a + (1.0 - a) * (p + 1.0) * pow(q, p));
            }
            rgb[c] = max(v, 0.0);
        }
        total_irradiance += rgb * query_result.intensity * query_result.weight;

        total_weight += query_result.weight;
    }

    if (total_weight != 0.0) {
        total_irradiance /= total_weight;
    }

    return total_irradiance;
}

#endif  // IRRADIANCE_VOLUMES_ARE_USABLE
