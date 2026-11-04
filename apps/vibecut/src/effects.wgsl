struct Params { width: u32, height: u32, horizontal: u32, count: u32 }
@group(0) @binding(0) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> dst: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> weights: array<f32>;
@group(0) @binding(3) var<uniform> p: Params;
// Reserved common inputs keep the pass interface independent of effect type.
@group(0) @binding(4) var<storage, read> coverage: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read> below: array<vec4<f32>>;
@compute @workgroup_size(8, 8)
fn gaussian(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    var sum = vec4<f32>(0.0);
    for (var t = 0u; t < p.count; t++) {
        let d = i32(t) - i32(p.count / 2u);
        var x = i32(id.x);
        var y = i32(id.y);
        if p.horizontal != 0u { x = clamp(x + d, 0, i32(p.width) - 1); }
        else { y = clamp(y + d, 0, i32(p.height) - 1); }
        sum += src[u32(y) * p.width + u32(x)] * weights[t];
    }
    let a = clamp(sum.a, 0.0, 1.0);
    dst[id.y * p.width + id.x] = clamp(sum, vec4<f32>(0.0), vec4<f32>(a));
}
