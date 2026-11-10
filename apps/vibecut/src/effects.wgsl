struct Params { width: u32, height: u32, horizontal: u32, count: u32 }
@group(0) @binding(0) var<storage, read> src: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read_write> dst: array<vec4<f32>>;
@group(0) @binding(2) var<storage, read> weights: array<f32>;
@group(0) @binding(3) var<uniform> p: Params;
// Reserved common inputs keep the pass interface independent of effect type.
@group(0) @binding(4) var<storage, read> coverage: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read> below: array<vec4<f32>>;
// A sample pass also reads the image its effect instance started with.
@group(0) @binding(6) var<storage, read> inst: array<vec4<f32>>;
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

// Mirrors render::effects::grade_pixel; weights = [gain, exponent, saturation, hue radians].
@compute @workgroup_size(8, 8)
fn grade(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    if px.a <= 0.0 { dst[i] = px; return; }
    var c = max(px.rgb / px.a * weights[0], vec3<f32>(0.0));
    c = 0.18 * pow(c / 0.18, vec3<f32>(weights[1]));
    let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    c = vec3<f32>(l) + (c - vec3<f32>(l)) * weights[2];
    let cs = cos(weights[3]);
    let k = sin(weights[3]) * 0.5773503;
    let off = (1.0 - cs) / 3.0;
    let a = cs + off;
    let o = vec3<f32>(
        a * c.r + (off - k) * c.g + (off + k) * c.b,
        (off + k) * c.r + a * c.g + (off - k) * c.b,
        (off - k) * c.r + (off + k) * c.g + a * c.b);
    dst[i] = vec4<f32>(clamp(o, vec3<f32>(0.0), vec3<f32>(1.0)) * px.a, px.a);
}

fn oetf(x: f32) -> f32 {
    let v = clamp(x, 0.0, 1.0);
    if v <= 0.0031308 { return 12.92 * v; }
    return 1.055 * pow(v, 1.0 / 2.4) - 0.055;
}
fn eotf(x: f32) -> f32 {
    let v = clamp(x, 0.0, 1.0);
    if v <= 0.04045 { return v / 12.92; }
    return pow((v + 0.055) / 1.055, 2.4);
}
fn ramp(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
// Mirrors render::effects::chroma_key_pixel; weights = [key r, key g, key b, intensity, shadow].
@compute @workgroup_size(8, 8)
fn chroma_key(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    if px.a <= 0.0 { dst[i] = px; return; }
    let key = vec3<f32>(weights[0], weights[1], weights[2]);
    let intensity = clamp(weights[3], 0.0, 1.0);
    let shadow = clamp(weights[4], 0.0, 1.0);
    var c = vec3<f32>(oetf(px.r / px.a), oetf(px.g / px.a), oetf(px.b / px.a));
    let y = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
    let cb = (c.b - y) / 1.8556;
    let cr = (c.r - y) / 1.5748;
    let ky = dot(key, vec3<f32>(0.2126, 0.7152, 0.0722));
    let kcb = (key.b - ky) / 1.8556;
    let kcr = (key.r - ky) / 1.5748;
    let mag2 = max(kcb * kcb + kcr * kcr, 1e-6);
    let proj = (cb * kcb + cr * kcr) / mag2;
    let dist = sqrt((cb - proj * kcb) * (cb - proj * kcb) + (cr - proj * kcr) * (cr - proj * kcr)) / sqrt(mag2);
    let tol = 0.08 + 0.5 * intensity;
    let keyness = clamp(proj, 0.0, 1.5) * (1.0 - ramp(0.0, tol, dist));
    let protect = max(ky - y, 0.0) * shadow * 2.0 * min(keyness, 1.0);
    let matte = clamp(1.0 - keyness * (1.0 + 2.0 * intensity) + protect, 0.0, 1.0);
    var dominant = 0u;
    if key.g >= key.r && key.g >= key.b { dominant = 1u; }
    else if key.b >= key.r { dominant = 2u; }
    var limit = 0.0;
    if dominant == 0u { limit = max(c.g, c.b); }
    else if dominant == 1u { limit = max(c.r, c.b); }
    else { limit = max(c.r, c.g); }
    let excess = max(c[dominant] - limit, 0.0);
    c[dominant] = c[dominant] - excess;
    let a = matte * px.a;
    let out = vec3<f32>(eotf(c.r), eotf(c.g), eotf(c.b)) * a;
    dst[i] = vec4<f32>(clamp(out, vec3<f32>(0.0), vec3<f32>(a)), a);
}

fn channel(mode: u32, b: f32, s: f32) -> f32 {
    switch mode {
        case 1u: { return min(b,s); }
        case 2u: { return b*s; }
        case 3u: {
            if b >= 1.0 { return 1.0; }
            if s <= 0.0 { return 0.0; }
            return 1.0-min(1.0,(1.0-b)/s);
        }
        case 4u: { return max(b,s); }
        case 5u: { return b+s-b*s; }
        case 6u: { return min(1.0,b+s); }
        case 7u: {
            if b <= 0.0 { return 0.0; }
            if s >= 1.0 { return 1.0; }
            return min(1.0,b/(1.0-s));
        }
        case 8u: {
            if b <= 0.5 { return 2.0*b*s; }
            return 1.0-2.0*(1.0-b)*(1.0-s);
        }
        case 9u: {
            if s <= 0.5 { return b-(1.0-2.0*s)*b*(1.0-b); }
            var d = sqrt(b);
            if b <= 0.25 { d = ((16.0*b-12.0)*b+4.0)*b; }
            return b+(2.0*s-1.0)*(d-b);
        }
        case 10u: {
            if s <= 0.5 { return 2.0*b*s; }
            return 1.0-2.0*(1.0-b)*(1.0-s);
        }
        case 11u: { return abs(b-s); }
        case 12u: { return b+s-2.0*b*s; }
        default: { return s; }
    }
}
fn lum(c: vec3<f32>) -> f32 { return dot(c,vec3<f32>(0.3,0.59,0.11)); }
fn sat(c: vec3<f32>) -> f32 { return max(c.r,max(c.g,c.b))-min(c.r,min(c.g,c.b)); }
fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let hi = max(c.r,max(c.g,c.b));
    let lo = min(c.r,min(c.g,c.b));
    if hi-lo <= 1e-6 { return vec3<f32>(0.0); }
    return (c-vec3<f32>(lo))*s/(hi-lo);
}
fn set_lum(c: vec3<f32>, level: f32) -> vec3<f32> {
    var o = c+vec3<f32>(level-lum(c));
    let l = lum(o);
    let lo = min(o.r,min(o.g,o.b));
    let hi = max(o.r,max(o.g,o.b));
    if lo < 0.0 { o = vec3<f32>(l)+(o-vec3<f32>(l))*l/max(l-lo,1e-6); }
    if hi > 1.0 { o = vec3<f32>(l)+(o-vec3<f32>(l))*(1.0-l)/max(hi-l,1e-6); }
    return o;
}
fn rgb(mode: u32, b: vec3<f32>, s: vec3<f32>) -> vec3<f32> {
    switch mode {
        case 13u: { return set_lum(set_sat(s,sat(b)),lum(b)); }
        case 14u: { return set_lum(set_sat(b,sat(s)),lum(b)); }
        case 15u: { return set_lum(s,lum(b)); }
        case 16u: { return set_lum(b,lum(s)); }
        default: { return vec3<f32>(channel(mode,b.r,s.r),channel(mode,b.g,s.g),channel(mode,b.b,s.b)); }
    }
}
@compute @workgroup_size(8, 8)
fn blend(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y*p.width+id.x;
    let b = below[i];
    let s = src[i];
    let opacity = weights[0];
    let a = s.a*opacity;
    let mode = u32(weights[1]);
    if a <= 0.0 { dst[i] = b; return; }
    if mode == 0u || b.a <= 0.0 { dst[i] = clamp(s*opacity+b*(1.0-a),vec4<f32>(0.0),vec4<f32>(1.0)); return; }
    let mixed = clamp(rgb(mode,b.rgb/b.a,s.rgb/s.a),vec3<f32>(0.0),vec3<f32>(1.0));
    let alpha = a+b.a*(1.0-a);
    let color = s.rgb*opacity*(1.0-b.a)+b.rgb*(1.0-a)+a*b.a*mixed;
    dst[i] = vec4<f32>(clamp(color,vec3<f32>(0.0),vec3<f32>(alpha)),alpha);
}

// Exact texel of the instance input image; transparent outside the picture.
fn inst_texel(x: i32, y: i32) -> vec4<f32> {
    if x < 0 || y < 0 || x >= i32(p.width) || y >= i32(p.height) { return vec4<f32>(0.0); }
    return inst[u32(y) * p.width + u32(x)];
}
// Texel-index bilinear sample: integers address exact texels, as image_ops::sample.
fn inst_sample(x: f32, y: f32) -> vec4<f32> {
    // Geometry admission bounds coordinates; outside the transparent footprint
    // return before converting or adding neighbours, including singleton axes.
    if x <= -1.0 || y <= -1.0 || x >= f32(p.width) || y >= f32(p.height) { return vec4<f32>(0.0); }
    let ix = floor(x);
    let iy = floor(y);
    let tx = x - ix;
    let ty = y - iy;
    let i = vec2<i32>(i32(ix), i32(iy));
    let a = inst_texel(i.x, i.y);
    let b = inst_texel(i.x + 1, i.y);
    let c = inst_texel(i.x, i.y + 1);
    let d = inst_texel(i.x + 1, i.y + 1);
    let top = a + (b - a) * tx;
    let bottom = c + (d - c) * tx;
    return clamp(top + (bottom - top) * ty, vec4<f32>(0.0), vec4<f32>(1.0));
}
// Mirrors render::effects::mirror; weights = [centre x, centre y, unit normal x, unit normal y].
@compute @workgroup_size(8, 8)
fn mirror(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    let centre = vec2<f32>(weights[0], weights[1]);
    let dir = vec2<f32>(weights[2], weights[3]);
    let d = dot(vec2<f32>(px, py) - centre, dir);
    var out = src[id.y * p.width + id.x];
    if d > 0.0 { out = inst_sample(px - 2.0 * d * dir.x - 0.5, py - 2.0 * d * dir.y - 0.5); }
    let a = clamp(out.a, 0.0, 1.0);
    dst[id.y * p.width + id.x] = clamp(out, vec4<f32>(0.0), vec4<f32>(a));
}
