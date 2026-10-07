// The counselor minimap (SC_MiniMap_Widget: MiniMap_Mat's texture turned about its Center, in the RetainerBox's
// CircleMask): the level's map around her, turned so the camera's forward points up, inside the mask's radius.
// view: her map UV, the span of the map across the widget, the mask radius; dir: the camera's forward in map UV.
#import bevy_ui::ui_vertex_output::UiVertexOutput

@group(1) @binding(0) var<uniform> view: vec4<f32>;
@group(1) @binding(1) var<uniform> dir: vec4<f32>;
@group(1) @binding(2) var map: texture_2d<f32>;
@group(1) @binding(3) var map_sampler: sampler;

@fragment
fn fragment(in: UiVertexOutput) -> @location(0) vec4<f32> {
    let d = in.uv - vec2<f32>(0.5, 0.5);
    if (length(d) > view.w) {
        return vec4<f32>(0.0);
    }
    let f = normalize(dir.xy);
    let r = vec2<f32>(-f.y, f.x);
    let uv = view.xy + (r * d.x - f * d.y) * view.z;
    let c = textureSample(map, map_sampler, uv);
    let inside = all(uv >= vec2<f32>(0.0)) && all(uv <= vec2<f32>(1.0));
    return vec4<f32>(c.rgb, c.a * select(0.0, 1.0, inside));
}
