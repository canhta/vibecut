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

// Mirrors lift_pixel; weights = [lift r, lift g, lift b] in [-1, 1].
@compute @workgroup_size(8, 8)
fn lift(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let c = straight_rgb(px);
    let mapped = clamp(c + vec3<f32>(weights[0], weights[1], weights[2]) * (vec3<f32>(1.0) - c), vec3<f32>(0.0), vec3<f32>(1.0));
    dst[i] = vec4<f32>(mapped * px.a, px.a);
}
// Mirrors tone_gamma_pixel; weights = [gamma r, gamma g, gamma b], 1 neutral.
@compute @workgroup_size(8, 8)
fn tone_gamma(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let c = straight_rgb(px);
    let gamma = max(vec3<f32>(weights[0], weights[1], weights[2]), vec3<f32>(0.25));
    let mapped = clamp(pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0) / gamma), vec3<f32>(0.0), vec3<f32>(1.0));
    dst[i] = vec4<f32>(mapped * px.a, px.a);
}
// Mirrors gain_pixel; weights = [gain r, gain g, gain b], 1 neutral.
@compute @workgroup_size(8, 8)
fn gain(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let c = straight_rgb(px);
    let mapped = clamp(c * vec3<f32>(weights[0], weights[1], weights[2]), vec3<f32>(0.0), vec3<f32>(1.0));
    dst[i] = vec4<f32>(mapped * px.a, px.a);
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
// PixelOp's straight/encoded/premultiplied contract, including tiny positive alpha.
fn straight_rgb(px: vec4<f32>) -> vec3<f32> {
    if px.a <= 0.0 { return vec3<f32>(0.0); }
    return clamp(px.rgb / px.a, vec3<f32>(0.0), vec3<f32>(1.0));
}
fn encoded_rgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(oetf(c.r), oetf(c.g), oetf(c.b));
}
fn decoded_premultiplied(c: vec3<f32>, a: f32) -> vec4<f32> {
    let linear = vec3<f32>(eotf(c.r), eotf(c.g), eotf(c.b));
    return vec4<f32>(clamp(linear * a, vec3<f32>(0.0), vec3<f32>(a)), a);
}
// Mirrors gamma_pixel; weights = [gamma]. Gamma is an encoded-space power.
@compute @workgroup_size(8, 8)
fn gamma(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let encoded = encoded_rgb(straight_rgb(px));
    dst[i] = decoded_premultiplied(pow(encoded, vec3<f32>(max(weights[0], 1e-3))), px.a);
}
// Mirrors invert_pixel; weights = [channel (RGB/R/G/B/Alpha), blend back].
@compute @workgroup_size(8, 8)
fn invert(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let channel = u32(clamp(floor(weights[0] + 0.5), 0.0, 4.0));
    let blend = clamp(weights[1], 0.0, 1.0);
    let c = straight_rgb(px);
    if channel == 4u {
        let a = (1.0 - px.a) * (1.0 - blend) + px.a * blend;
        dst[i] = vec4<f32>(clamp(c * a, vec3<f32>(0.0), vec3<f32>(a)), a);
        return;
    }
    if px.a <= 0.0 { dst[i] = px; return; }
    let e = encoded_rgb(c);
    var mixed = e;
    for (var k = 0u; k < 3u; k++) {
        if channel == 0u || channel == k + 1u {
            let inverted = 1.0 - e[k];
            mixed[k] = inverted + (e[k] - inverted) * blend;
        }
    }
    dst[i] = decoded_premultiplied(mixed, px.a);
}
// Mirrors black_white_pixel; no uniforms. Luma stays in linear light.
@compute @workgroup_size(8, 8)
fn black_white(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let c = straight_rgb(px);
    let l = clamp(0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b, 0.0, 1.0);
    dst[i] = vec4<f32>(vec3<f32>(l * px.a), px.a);
}
// Mirrors channel_mix_pixel; weights = three encoded-space rows [r,g,b,constant].
@compute @workgroup_size(8, 8)
fn channel_mix(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let e = encoded_rgb(straight_rgb(px));
    var mixed = vec3<f32>(0.0);
    for (var k = 0u; k < 3u; k++) {
        let row = k * 4u;
        mixed[k] = clamp(weights[row] * e.r + weights[row + 1u] * e.g
            + weights[row + 2u] * e.b + weights[row + 3u], 0.0, 1.0);
    }
    dst[i] = decoded_premultiplied(mixed, px.a);
}
// Mirrors tint_pixel; weights = encoded black RGB, white RGB, amount.
@compute @workgroup_size(8, 8)
fn tint(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let c = straight_rgb(px);
    let l = oetf(max(0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b, 0.0));
    let black = vec3<f32>(weights[0], weights[1], weights[2]);
    let white = vec3<f32>(weights[3], weights[4], weights[5]);
    let mapped = black + (white - black) * l;
    let e = encoded_rgb(c);
    dst[i] = decoded_premultiplied(e + (mapped - e) * clamp(weights[6], 0.0, 1.0), px.a);
}
// Mirrors levels_pixel; weights = input black/white, output black/white, exponent.
@compute @workgroup_size(8, 8)
fn levels(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let e = encoded_rgb(straight_rgb(px));
    let span = max(weights[1] - weights[0], 1e-3);
    let fraction = clamp((e - vec3<f32>(weights[0])) / span, vec3<f32>(0.0), vec3<f32>(1.0));
    let mapped = vec3<f32>(weights[2]) + pow(fraction, vec3<f32>(weights[4])) * (weights[3] - weights[2]);
    dst[i] = decoded_premultiplied(mapped, px.a);
}
// Mirrors strobe_pixel; weights = encoded RGB, original-picture fraction.
// The evaluator alone gates time. Mixed colour is already linear: do not decode it again.
@compute @workgroup_size(8, 8)
fn strobe(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    let colour = vec3<f32>(eotf(weights[0]), eotf(weights[1]), eotf(weights[2]));
    let mixed = colour + (straight_rgb(px) - colour) * clamp(weights[3], 0.0, 1.0);
    let rgb = clamp(mixed, vec3<f32>(0.0), vec3<f32>(1.0)) * px.a;
    dst[i] = vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(px.a)), px.a);
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
// Mirrors render denoise::reduce_noise_sample; weights = [spatial sigma, range sigma].
// A 5x5 bilateral average of the current image: each tap weighs a spatial gaussian
// times a range gaussian of its encoded straight colour and alpha distance to the
// centre. Taps outside the image are skipped and the weights renormalise.
@compute @workgroup_size(8, 8)
fn reduce_noise(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let centre = src[id.y * p.width + id.x];
    let centre_rgb = encoded_rgb(straight_rgb(centre));
    let spatial = -0.5 / (weights[0] * weights[0]);
    let range = -0.5 / (weights[1] * weights[1]);
    var sum = vec4<f32>(0.0);
    var total = 0.0;
    for (var dy = -2; dy <= 2; dy++) {
        let ty = i32(id.y) + dy;
        if ty < 0 || ty >= i32(p.height) { continue; }
        for (var dx = -2; dx <= 2; dx++) {
            let tx = i32(id.x) + dx;
            if tx < 0 || tx >= i32(p.width) { continue; }
            let tap = src[u32(ty) * p.width + u32(tx)];
            let delta = encoded_rgb(straight_rgb(tap)) - centre_rgb;
            let da = tap.a - centre.a;
            let distance = da * da + dot(delta, delta);
            let weight = exp(f32(dx * dx + dy * dy) * spatial + distance * range);
            sum += weight * tap;
            total += weight;
        }
    }
    var out = centre;
    if total > 0.0 { out = sum / total; }
    let a = clamp(out.a, 0.0, 1.0);
    dst[id.y * p.width + id.x] = clamp(out, vec4<f32>(0.0), vec4<f32>(a));
}

// Mirrors render matte::sample; weights = the affine map [a, b, c, d, e, f] from the
// pixel centre to the 192x192 matte plane's cell coordinate (u = a px + b py + c,
// v = d px + e py + f). The plane rides in the instance input: cell i is texel i's
// alpha. Bilinear with edge cells clamped; the layer is scaled by the coverage.
@compute @workgroup_size(8, 8)
fn person_matte(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    let u = weights[0] * px + weights[1] * py + weights[2];
    let v = weights[3] * px + weights[4] * py + weights[5];
    let u0 = floor(u);
    let v0 = floor(v);
    let fx = u - u0;
    let fy = v - v0;
    let left = u32(clamp(u0, 0.0, 191.0));
    let right = u32(clamp(u0 + 1.0, 0.0, 191.0));
    let top = u32(clamp(v0, 0.0, 191.0));
    let bottom = u32(clamp(v0 + 1.0, 0.0, 191.0));
    let upper = inst[top * 192u + left].a * (1.0 - fx) + inst[top * 192u + right].a * fx;
    let lower = inst[bottom * 192u + left].a * (1.0 - fx) + inst[bottom * 192u + right].a * fx;
    let m = clamp(upper * (1.0 - fy) + lower * fy, 0.0, 1.0);
    let out = src[id.y * p.width + id.x] * m;
    let a = clamp(out.a, 0.0, 1.0);
    dst[id.y * p.width + id.x] = clamp(out, vec4<f32>(0.0), vec4<f32>(a));
}

// ---- The E1 spatial and E2 whole-image entries. Each mirrors the render reference it
// names, keeping its operation order; the helpers below mirror render's effects.rs,
// effects/image_ops.rs and effects/whole_image.rs.

const TAU: f32 = 6.2831855;
const PI: f32 = 3.1415927;
const FRAC_PI_2: f32 = 1.5707964;
const EDGE_CLAMP: u32 = 0u;
const EDGE_WRAP: u32 = 1u;
const EDGE_TRANSPARENT: u32 = 2u;

// render's `luma`, summed left to right.
fn luma709(c: vec3<f32>) -> f32 {
    return 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
}
fn max3(c: vec3<f32>) -> f32 {
    return max(max(c.r, c.g), c.b);
}
// whole_image's `smooth`: render's smoothstep with a degenerate edge pair widened.
fn smooth_edge(e0: f32, e1: f32, x: f32) -> f32 {
    return ramp(e0, max(e1, e0 + 1e-6), x);
}
// f32::round: halves away from zero (WGSL round takes them to even).
fn round_away(x: f32) -> f32 {
    let a = abs(x);
    let f = floor(a);
    let r = select(f, f + 1.0, a - f >= 0.5);
    return select(r, -r, x < 0.0);
}
// f32::powf for x >= 0 (WGSL leaves pow(0, y) undefined).
fn powf(x: f32, y: f32) -> f32 {
    if x == 0.0 {
        if y == 0.0 { return 1.0; }
        if y > 0.0 { return 0.0; }
        return 3.0e38;
    }
    return pow(x, y);
}
// f32::powi as the Rust runtime evaluates it: square and multiply.
fn powi(x: f32, n: i32) -> f32 {
    var base = x;
    var e = n;
    var r = 1.0;
    loop {
        if (e & 1) != 0 { r = r * base; }
        e = e / 2;
        if e == 0 { break; }
        base = base * base;
    }
    if n < 0 { return 1.0 / r; }
    return r;
}
// render's `premultiply`: straight colour clamped, scaled by alpha, kept within it.
fn premultiply(c: vec3<f32>, a: f32) -> vec4<f32> {
    let scaled = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)) * a;
    return vec4<f32>(clamp(scaled, vec3<f32>(0.0), vec3<f32>(a)), a);
}
fn decoded_rgb(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(eotf(c.r), eotf(c.g), eotf(c.b));
}
// The CPU sample pass exit (`clamp_pixel`): alpha into 0..1, colour into 0..alpha.
fn sample_exit(v: vec4<f32>) -> vec4<f32> {
    let a = clamp(v.a, 0.0, 1.0);
    return vec4<f32>(clamp(v.rgb, vec3<f32>(0.0), vec3<f32>(a)), a);
}
// The current image's texel at a coordinate clamped into the picture.
fn clamped_src(x: i32, y: i32) -> vec4<f32> {
    let sx = clamp(x, 0, i32(p.width) - 1);
    let sy = clamp(y, 0, i32(p.height) - 1);
    return src[u32(sy) * p.width + u32(sx)];
}
fn edge_axis(i: i32, n: i32, edge: u32) -> i32 {
    if n <= 1 && edge != EDGE_TRANSPARENT { return 0; }
    if edge == EDGE_CLAMP { return clamp(i, 0, n - 1); }
    if edge == EDGE_WRAP { return ((i % n) + n) % n; }
    return i;
}
// One texel of the current image, or of the instance input when `from_input`.
fn edge_texel(from_input: bool, x: i32, y: i32, edge: u32) -> vec4<f32> {
    let w = i32(p.width);
    let h = i32(p.height);
    let sx = edge_axis(x, w, edge);
    let sy = edge_axis(y, h, edge);
    if sx < 0 || sy < 0 || sx >= w || sy >= h { return vec4<f32>(0.0); }
    let i = u32(sy) * p.width + u32(sx);
    if from_input { return inst[i]; }
    return src[i];
}
// image_ops::sample: texel-index bilinear with an edge mode, clamped to 0..1. A
// clamp-edge coordinate is first limited to [-1, extent], where its neighbours are
// equal so the result is unchanged, and a transparent one beyond its footprint is
// zero; both keep the integer conversion in range.
fn edge_sample(from_input: bool, x: f32, y: f32, edge: u32) -> vec4<f32> {
    var sx = x;
    var sy = y;
    if edge == EDGE_TRANSPARENT {
        if x <= -1.0 || y <= -1.0 || x >= f32(p.width) || y >= f32(p.height) { return vec4<f32>(0.0); }
    } else if edge == EDGE_CLAMP {
        sx = clamp(x, -1.0, f32(p.width));
        sy = clamp(y, -1.0, f32(p.height));
    }
    let fx = floor(sx);
    let fy = floor(sy);
    let tx = sx - fx;
    let ty = sy - fy;
    let ix = i32(fx);
    let iy = i32(fy);
    let a = edge_texel(from_input, ix, iy, edge);
    let b = edge_texel(from_input, ix + 1, iy, edge);
    let c = edge_texel(from_input, ix, iy + 1, edge);
    let d = edge_texel(from_input, ix + 1, iy + 1, edge);
    let top = a + (b - a) * tx;
    let bottom = c + (d - c) * tx;
    return clamp(top + (bottom - top) * ty, vec4<f32>(0.0), vec4<f32>(1.0));
}
fn cur_sample(x: f32, y: f32, edge: u32) -> vec4<f32> {
    return edge_sample(false, x, y, edge);
}
fn in_sample(x: f32, y: f32, edge: u32) -> vec4<f32> {
    return edge_sample(true, x, y, edge);
}
// The light composite shared by the glow, ray and flare references: light added to
// the instance input, its coverage raised by the brightest light channel.
fn add_light(base: vec4<f32>, light: vec3<f32>) -> vec4<f32> {
    let alpha = base.a + (1.0 - base.a) * min(max3(light), 1.0);
    return vec4<f32>(max(base.rgb + light, vec3<f32>(0.0)), alpha);
}
// A shadow over the instance input, or the shadow alone when `only`.
fn over_shadow(shadow: vec4<f32>, base: vec4<f32>, only: f32) -> vec4<f32> {
    if only > 0.5 { return shadow; }
    return shadow + base * (1.0 - shadow.a);
}

// ---- whole_image noise. The reference hashes in u64; here a u64 is (low, high) u32.
fn mul_wide(a: u32, b: u32) -> vec2<u32> {
    let a0 = a & 0xFFFFu;
    let a1 = a >> 16u;
    let b0 = b & 0xFFFFu;
    let b1 = b >> 16u;
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = p01 + p10;
    let mid_carry = select(0u, 0x10000u, mid < p01);
    let lo = p00 + (mid << 16u);
    let lo_carry = select(0u, 1u, lo < p00);
    return vec2<u32>(lo, p11 + (mid >> 16u) + mid_carry + lo_carry);
}
// Wrapping u64 multiplication.
fn mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let w = mul_wide(a.x, b.x);
    return vec2<u32>(w.x, w.y + a.x * b.y + a.y * b.x);
}
// `i64 as u64`: two's complement sign extension.
fn widen(v: i32) -> vec2<u32> {
    return vec2<u32>(bitcast<u32>(v), select(0u, 0xFFFFFFFFu, v < 0));
}
// h ^ (h >> n) for 0 < n < 32.
fn xor_shift_right(h: vec2<u32>, n: u32) -> vec2<u32> {
    return h ^ vec2<u32>((h.x >> n) | (h.y << (32u - n)), h.y >> n);
}
// whole_image::hash_u; seeds stay below 2^32.
fn hash_u(x: i32, y: i32, z: i32, seed: u32) -> vec2<u32> {
    var h = mul64(widen(x), vec2<u32>(0x7F4A7C15u, 0x9E3779B9u))
        ^ mul64(widen(y), vec2<u32>(0x27D4EB4Fu, 0xC2B2AE3Du))
        ^ mul64(widen(z), vec2<u32>(0x9E3779F9u, 0x165667B1u))
        ^ mul64(vec2<u32>(seed, 0u), vec2<u32>(0x165667C5u, 0x27D4EB2Fu));
    h = xor_shift_right(h, 31u);
    h = mul64(h, vec2<u32>(0x1CE4E5B9u, 0xBF58476Du));
    h = xor_shift_right(h, 29u);
    h = mul64(h, vec2<u32>(0x133111EBu, 0x94D049BBu));
    return vec2<u32>(h.x ^ h.y, h.y);
}
// whole_image::hash01: the top 24 bits over 2^24.
fn hash01(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    return f32(hash_u(x, y, z, seed).y >> 8u) / 16777216.0;
}
fn fade(t: f32) -> f32 {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}
// whole_image::grad3 on the hash modulo 12 (2^32 is 4 modulo 12).
fn grad3(h: vec2<u32>, x: f32, y: f32, z: f32) -> f32 {
    switch ((h.y % 12u) * 4u + h.x % 12u) % 12u {
        case 0u: { return x + y; }
        case 1u: { return -x + y; }
        case 2u: { return x - y; }
        case 3u: { return -x - y; }
        case 4u: { return x + z; }
        case 5u: { return -x + z; }
        case 6u: { return x - z; }
        case 7u: { return -x - z; }
        case 8u: { return y + z; }
        case 9u: { return -y + z; }
        case 10u: { return y - z; }
        default: { return -y - z; }
    }
}
fn lerp1(a: f32, b: f32, t: f32) -> f32 {
    return a + (b - a) * t;
}
// whole_image::noise3.
fn noise3(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let x0 = floor(x);
    let y0 = floor(y);
    let z0 = floor(z);
    let xf = x - x0;
    let yf = y - y0;
    let zf = z - z0;
    let xi = i32(x0);
    let yi = i32(y0);
    let zi = i32(z0);
    let u = fade(xf);
    let v = fade(yf);
    let w = fade(zf);
    let g000 = grad3(hash_u(xi, yi, zi, seed), xf, yf, zf);
    let g100 = grad3(hash_u(xi + 1, yi, zi, seed), xf - 1.0, yf, zf);
    let g010 = grad3(hash_u(xi, yi + 1, zi, seed), xf, yf - 1.0, zf);
    let g110 = grad3(hash_u(xi + 1, yi + 1, zi, seed), xf - 1.0, yf - 1.0, zf);
    let g001 = grad3(hash_u(xi, yi, zi + 1, seed), xf, yf, zf - 1.0);
    let g101 = grad3(hash_u(xi + 1, yi, zi + 1, seed), xf - 1.0, yf, zf - 1.0);
    let g011 = grad3(hash_u(xi, yi + 1, zi + 1, seed), xf, yf - 1.0, zf - 1.0);
    let g111 = grad3(hash_u(xi + 1, yi + 1, zi + 1, seed), xf - 1.0, yf - 1.0, zf - 1.0);
    let x00 = lerp1(g000, g100, u);
    let x10 = lerp1(g010, g110, u);
    let x01 = lerp1(g001, g101, u);
    let x11 = lerp1(g011, g111, u);
    return lerp1(lerp1(x00, x10, v), lerp1(x01, x11, v), w) * 0.9;
}
// whole_image::fbm; octave seeds add to `seed` as the reference's wrapping add does.
fn fbm(x: f32, y: f32, z: f32, octaves: f32, seed: u32) -> f32 {
    let n = clamp(octaves, 1.0, 10.0);
    var sum = 0.0;
    var amp = 1.0;
    var freq = 1.0;
    var norm = 0.0;
    var octave = 0u;
    while f32(octave) < n {
        let k = min(n - f32(octave), 1.0);
        sum += noise3(x * freq, y * freq, z * freq + f32(octave) * 17.13, seed + octave) * amp * k;
        norm += amp * k;
        amp *= 0.5;
        freq *= 2.0;
        octave++;
    }
    return sum / max(norm, 1e-6);
}

// ---- 265 pixel entries.

// Mirrors whole_image::highlights; weights = [threshold, softness].
@compute @workgroup_size(8, 8)
fn highlights(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    if px.a <= 1e-6 { dst[i] = vec4<f32>(0.0); return; }
    let threshold = clamp(weights[0], 0.0, 1.0);
    let soft = max(weights[1], 1e-3);
    let level = luma709(encoded_rgb(straight_rgb(px)));
    dst[i] = px * smooth_edge(threshold, threshold + soft, level);
}
// Mirrors whole_image::alpha_plane; no uniforms.
@compute @workgroup_size(8, 8)
fn alpha_plane(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let a = src[i].a;
    dst[i] = vec4<f32>(a, a, a, 1.0);
}
// Mirrors whole_image::color_key; weights = encoded key RGB, tolerance, softness, thin.
@compute @workgroup_size(8, 8)
fn color_key(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    if px.a <= 0.0 { dst[i] = px; return; }
    let soft = max(weights[4], 1e-4);
    let colour = straight_rgb(px);
    let e = encoded_rgb(colour);
    let key = encoded_rgb(vec3<f32>(weights[0], weights[1], weights[2]));
    let y = luma709(e);
    let cb = (e.b - y) / 1.8556;
    let cr = (e.r - y) / 1.5748;
    let ky = luma709(key);
    let kcb = (key.b - ky) / 1.8556;
    let kcr = (key.r - ky) / 1.5748;
    let dist = sqrt((cb - kcb) * (cb - kcb) + (cr - kcr) * (cr - kcr)) + abs(y - ky) * 0.15;
    let tolerance = max(weights[3] + weights[5] / 5.0 * 0.1, 0.0);
    let a = px.a * clamp((dist - tolerance) / soft, 0.0, 1.0);
    dst[i] = vec4<f32>(colour * a, a);
}
// Mirrors whole_image::luma_key; weights = [threshold, cutoff, invert].
@compute @workgroup_size(8, 8)
fn luma_key(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = src[i];
    if px.a <= 0.0 { dst[i] = px; return; }
    let threshold = weights[0];
    let cutoff = weights[1];
    let colour = straight_rgb(px);
    var level = clamp(luma709(encoded_rgb(colour)), 0.0, 1.0);
    if weights[2] > 0.5 { level = 1.0 - level; }
    var matte = (level - cutoff) / max(threshold - cutoff, 1e-3);
    if level <= cutoff {
        matte = 0.0;
    } else if level >= max(threshold, cutoff + 1e-3) {
        matte = 1.0;
    }
    let a = px.a * clamp(matte, 0.0, 1.0);
    dst[i] = vec4<f32>(colour * a, a);
}

// ---- 265 warps.

// Mirrors whole_image::twirl; weights = [centre x, centre y, angle, radius].
@compute @workgroup_size(8, 8)
fn twirl(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let cx = weights[0];
    let cy = weights[1];
    let radius = weights[3];
    let dx = f32(id.x) + 0.5 - cx;
    let dy = f32(id.y) + 0.5 - cy;
    let d = sqrt(dx * dx + dy * dy);
    var out = src[i];
    if radius > 0.0 && d < radius {
        let k = 1.0 - d / radius;
        let a = weights[2] * k * k;
        let s = sin(a);
        let c = cos(a);
        out = cur_sample(cx + dx * c - dy * s - 0.5, cy + dx * s + dy * c - 0.5, EDGE_CLAMP);
    }
    dst[i] = sample_exit(out);
}
// Mirrors whole_image::spherize; weights = [centre x, centre y, radius].
@compute @workgroup_size(8, 8)
fn spherize(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let cx = weights[0];
    let cy = weights[1];
    let radius = weights[2];
    let dx = f32(id.x) + 0.5 - cx;
    let dy = f32(id.y) + 0.5 - cy;
    let d = sqrt(dx * dx + dy * dy);
    var out = src[i];
    if !(radius < 0.5 || d >= radius || d < 1e-9) {
        let n = d / radius;
        let k = 1.0 - 0.5 * sqrt(max(1.0 - n * n, 0.0));
        out = cur_sample(cx + dx * k - 0.5, cy + dy * k - 0.5, EDGE_CLAMP);
    }
    dst[i] = sample_exit(out);
}
// Mirrors whole_image::magnify; weights = [centre x, centre y, magnification, size,
// feather, shape].
@compute @workgroup_size(8, 8)
fn magnify(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let cx = weights[0];
    let cy = weights[1];
    let size = weights[3];
    let feather = weights[4];
    let dx = f32(id.x) + 0.5 - cx;
    let dy = f32(id.y) + 0.5 - cy;
    var d = sqrt(dx * dx + dy * dy);
    if weights[5] > 0.5 { d = max(abs(dx), abs(dy)); }
    var k = clamp(size - d + 0.5, 0.0, 1.0);
    if feather > 0.0 { k = clamp((size - d) / feather, 0.0, 1.0); }
    let base = src[i];
    var out = base;
    if k > 0.0 {
        let m = max(weights[2] / 100.0, 0.01);
        let s = cur_sample(cx + dx / m - 0.5, cy + dy / m - 0.5, EDGE_CLAMP);
        out = base + (s - base) * k;
    }
    dst[i] = sample_exit(out);
}
// Mirrors whole_image::wave_warp; weights = [height, width, direction, phase].
@compute @workgroup_size(8, 8)
fn wave_warp(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    let ux = sin(weights[2]);
    let uy = -cos(weights[2]);
    let along = px * ux + py * uy;
    let offset = weights[0] * sin(along / max(weights[1], 1e-3) * TAU - weights[3]);
    dst[i] = sample_exit(cur_sample(px - uy * offset - 0.5, py + ux * offset - 0.5, EDGE_CLAMP));
}
// Mirrors whole_image::lens_distortion; weights = [curvature, centre x, centre y, norm].
@compute @workgroup_size(8, 8)
fn lens_distortion(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let cx = weights[1];
    let cy = weights[2];
    let norm = weights[3];
    let dx = (f32(id.x) + 0.5 - cx) / norm;
    let dy = (f32(id.y) + 0.5 - cy) / norm;
    let scale = 1.0 - weights[0] * (dx * dx + dy * dy);
    dst[i] = sample_exit(cur_sample(cx + dx * scale * norm - 0.5, cy + dy * scale * norm - 0.5, EDGE_CLAMP));
}
// One displacement channel of whole_image::turbulent_displace.
fn turbulence(x: f32, y: f32, channel_index: f32, evolution: f32, complexity: f32) -> f32 {
    return fbm(x + channel_index * 31.7, y, evolution + channel_index * 3.1, complexity, 0u);
}
// Mirrors whole_image::turbulent_displace; weights = [kind, amount, size, offset x,
// offset y, complexity, evolution, coarse].
@compute @workgroup_size(8, 8)
fn turbulent_displace(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let kind = i32(round_away(weights[0]));
    let amount = weights[1];
    var complexity = weights[5];
    if weights[7] > 0.5 { complexity = min(complexity, 3.0); }
    let evolution = weights[6];
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    let size = max(weights[2], 0.5);
    let u = px / size + weights[3];
    let v = py / size + weights[4];
    var d = vec2<f32>(0.0);
    switch kind {
        case 0, 3: {
            d = vec2<f32>(turbulence(u, v, 0.0, evolution, complexity), turbulence(u, v, 1.0, evolution, complexity));
        }
        case 1, 4, 2, 5: {
            let eps: f32 = 0.05;
            let gx = (turbulence(u + eps, v, 0.0, evolution, complexity) - turbulence(u - eps, v, 0.0, evolution, complexity)) / (2.0 * eps);
            let gy = (turbulence(u, v + eps, 0.0, evolution, complexity) - turbulence(u, v - eps, 0.0, evolution, complexity)) / (2.0 * eps);
            if kind == 1 || kind == 4 {
                d = vec2<f32>(gx * 0.3, gy * 0.3);
            } else {
                d = vec2<f32>(-gy * 0.3, gx * 0.3);
            }
        }
        case 6: {
            d = vec2<f32>(0.0, turbulence(u, v, 0.0, evolution, complexity));
        }
        case 7: {
            d = vec2<f32>(turbulence(u, v, 0.0, evolution, complexity), 0.0);
        }
        default: {
            let q = turbulence(u, v, 0.0, evolution, complexity);
            d = vec2<f32>(q, q);
        }
    }
    dst[i] = sample_exit(cur_sample(px - d.x * amount - 0.5, py - d.y * amount - 0.5, EDGE_CLAMP));
}
// Mirrors whole_image::corner_pin; weights = the inverse homography (9), width, height.
@compute @workgroup_size(8, 8)
fn corner_pin(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    let wv = weights[6] * px + weights[7] * py + weights[8];
    var out = vec4<f32>(0.0);
    if abs(wv) >= 1e-12 {
        let u = (weights[0] * px + weights[1] * py + weights[2]) / wv;
        let v = (weights[3] * px + weights[4] * py + weights[5]) / wv;
        let margin: f32 = 1e-3;
        let hi = 1.0 + margin;
        if u >= -margin && u <= hi && v >= -margin && v <= hi {
            out = in_sample(u * weights[9] - 0.5, v * weights[10] - 0.5, EDGE_TRANSPARENT);
        }
    }
    dst[i] = sample_exit(out);
}
// Mirrors whole_image::basic_3d; weights = [swivel, tilt, distance, focal, width, height].
@compute @workgroup_size(8, 8)
fn basic_3d(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let focal = weights[3];
    let width = weights[4];
    let height = weights[5];
    let qx = f32(id.x) + 0.5 - width / 2.0;
    let qy = f32(id.y) + 0.5 - height / 2.0;
    let ss = sin(weights[0]);
    let cs = cos(weights[0]);
    let st = sin(weights[1]);
    let ct = cos(weights[1]);
    let n = vec3<f32>(ss * ct, -st, cs * ct);
    let uu = vec3<f32>(cs, 0.0, -ss);
    let vv = vec3<f32>(ss * st, ct, cs * st);
    let z0 = focal + weights[2] * 10.0;
    let dir = vec3<f32>(qx, qy, focal);
    let denom = n.x * dir.x + n.y * dir.y + n.z * dir.z;
    var out = vec4<f32>(0.0);
    if abs(denom) >= 1e-9 {
        let t = (n.z * z0) / denom;
        if t > 0.0 {
            let hit = vec3<f32>(dir.x * t, dir.y * t, dir.z * t - z0);
            let su = hit.x * uu.x + hit.y * uu.y + hit.z * uu.z;
            let sv = hit.x * vv.x + hit.y * vv.y + hit.z * vv.z;
            out = in_sample(su + width / 2.0 - 0.5, sv + height / 2.0 - 0.5, EDGE_TRANSPARENT);
        }
    }
    dst[i] = sample_exit(out);
}

// ---- 265 stylize.

// Mirrors whole_image::brush_strokes; weights = [angle, size, length, cell, steps].
@compute @workgroup_size(8, 8)
fn brush_strokes(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let size = weights[1];
    let len = weights[2];
    let cell = weights[3];
    let steps = u32(clamp(round_away(weights[4]), 2.0, 24.0));
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    let cxi = i32(floor(px / cell));
    let cyi = i32(floor(py / cell));
    let jitter = (hash01(cxi, cyi, 1, 77u) - 0.5) * 0.5 * FRAC_PI_2;
    let a = weights[0] + jitter;
    let dx = cos(a);
    let dy = -sin(a);
    let nx = -dy;
    let ny = dx;
    var acc = vec4<f32>(0.0);
    var count = 0.0;
    for (var s = 0u; s < steps; s++) {
        let t = (f32(s) / f32(steps - 1u) - 0.5) * len;
        for (var k = 0u; k < 3u; k++) {
            let side = f32(k) * 0.5 - 0.5;
            acc += in_sample(px + dx * t + nx * side * size - 0.5, py + dy * t + ny * side * size - 0.5, EDGE_CLAMP);
            count += 1.0;
        }
    }
    let mean = acc / max(count, 1.0);
    let perpendicular = px * nx + py * ny;
    let bristle = 1.0 + 0.1 * noise3(perpendicular / max(size, 1.0), (px * dx + py * dy) / max(len, 1.0), f32(cxi) * 0.37, 5u);
    let cover = min(0.75 + 0.25 * hash01(cxi, cyi, 3, 77u), 1.0);
    dst[i] = sample_exit(vec4<f32>(mean.rgb * bristle, mean.a) * cover);
}
// Mirrors whole_image::roughen_edges; the current image is the blurred alpha plane.
// weights = [kind, edge RGB, border, scale, complexity, coarse].
@compute @workgroup_size(8, 8)
fn roughen_edges(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let kind = i32(round_away(weights[0]));
    let edge = vec3<f32>(weights[1], weights[2], weights[3]);
    let scale = weights[5];
    var complexity = weights[6];
    if weights[7] > 0.5 { complexity = min(complexity, 3.0); }
    let centre = inst[i];
    if centre.a <= 1e-6 { dst[i] = sample_exit(centre); return; }
    let d = clamp(src[i].r, 0.0, 1.0);
    let u = f32(id.x) / max(scale, 0.5);
    let v = f32(id.y) / max(scale, 0.5);
    var n = fbm(u, v, 0.0, complexity, 0u);
    if kind == 3 { n = 1.0 - 2.0 * abs(n); }
    var k = clamp((d - 0.5 + n * 0.5) * 21.0 + 0.5, 0.0, 1.0);
    if kind == 2 { k = select(0.0, 1.0, k > 0.5); }
    if kind >= 4 && kind <= 7 {
        let hole = fbm(u * 2.3 + 11.0, v * 2.3, 0.0, complexity, 1u);
        if d < 0.97 && hole > 0.25 { k *= 1.0 - smooth_edge(0.25, 0.4, hole) * (1.0 - d); }
    }
    var out = centre;
    if (kind == 1 || kind == 5 || kind == 7) && d < 0.97 {
        let t = min((1.0 - d) * 1.5, 1.0);
        let c = straight_rgb(centre);
        out = premultiply(c + (edge - c) * t, centre.a);
    }
    if kind == 6 || kind == 7 {
        let c = straight_rgb(out);
        let level = select(0.0, 1.0, oetf(max(luma709(c), 0.0)) > 0.5);
        out = premultiply(vec3<f32>(level), out.a);
    }
    dst[i] = sample_exit(out * k);
}
// Mirrors whole_image::camera_shake; weights = [offset x, offset y, scale, angle], the
// frame's shake evaluated once by the reference's evaluator.
@compute @workgroup_size(8, 8)
fn camera_shake(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let w = f32(p.width);
    let h = f32(p.height);
    let qx = (f32(id.x) + 0.5 - w / 2.0 - weights[0]) / weights[2];
    let qy = (f32(id.y) + 0.5 - h / 2.0 - weights[1]) / weights[2];
    let s = sin(weights[3]);
    let c = cos(weights[3]);
    let rx = qx * c - qy * s;
    let ry = qx * s + qy * c;
    dst[i] = sample_exit(in_sample(rx + w / 2.0 - 0.5, ry + h / 2.0 - 0.5, EDGE_CLAMP));
}

// ---- 265 glows and light.

// Encoded linear luma times coverage of one clamped neighbour, as sobel_edges reads it.
fn sobel_level(x: i32, y: i32) -> f32 {
    let q = clamped_src(x, y);
    return oetf(clamp(luma709(q.rgb), 0.0, 1.0)) * min(q.a, 1.0);
}
// Mirrors whole_image::sobel_edges; weights = [threshold].
@compute @workgroup_size(8, 8)
fn sobel_edges(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let threshold = clamp(weights[0], 0.0, 1.0);
    let x = i32(id.x);
    let y = i32(id.y);
    let gx = sobel_level(x + 1, y - 1) + 2.0 * sobel_level(x + 1, y) + sobel_level(x + 1, y + 1)
        - sobel_level(x - 1, y - 1) - 2.0 * sobel_level(x - 1, y) - sobel_level(x - 1, y + 1);
    let gy = sobel_level(x - 1, y + 1) + 2.0 * sobel_level(x, y + 1) + sobel_level(x + 1, y + 1)
        - sobel_level(x - 1, y - 1) - 2.0 * sobel_level(x, y - 1) - sobel_level(x + 1, y - 1);
    let level = smooth_edge(threshold, threshold + 0.1, sqrt(gx * gx + gy * gy) / 4.0);
    dst[i] = sample_exit(vec4<f32>(level));
}
// Mirrors whole_image::glow_add; the current image is the blurred light. weights =
// [colour RGB, intensity, saturation, tint, screen, only].
@compute @workgroup_size(8, 8)
fn glow_add(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let saturation = weights[4];
    var colour = src[i].rgb;
    if weights[5] > 0.5 {
        let l = max(luma709(colour), 0.0);
        colour = vec3<f32>(l * weights[0] * 1.5, l * weights[1] * 1.5, l * weights[2] * 1.5);
    } else if abs(saturation - 1.0) > 1e-4 {
        let l = luma709(colour);
        colour = vec3<f32>(l) + (colour - vec3<f32>(l)) * saturation;
    }
    let light = max(colour * weights[3], vec3<f32>(0.0));
    if weights[7] > 0.5 {
        dst[i] = sample_exit(vec4<f32>(light, min(max3(light), 1.0)));
        return;
    }
    let base = inst[i];
    var out = add_light(base, light);
    if weights[6] > 0.5 {
        let add = min(light, vec3<f32>(1.0));
        out = vec4<f32>(max(base.rgb + add - min(base.rgb, vec3<f32>(1.0)) * add, vec3<f32>(0.0)), out.a);
    }
    dst[i] = sample_exit(out);
}
// Mirrors whole_image::glint_rays; the current image is the highlights. weights =
// [colour RGB, intensity, length, rotation, rays, colorize, steps].
@compute @workgroup_size(8, 8)
fn glint_rays(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let colour = vec3<f32>(weights[0], weights[1], weights[2]);
    let len = weights[4];
    let rays = u32(clamp(round_away(weights[6]), 2.0, 8.0));
    let colorize = weights[7];
    let steps = u32(clamp(round_away(weights[8]), 2.0, 64.0));
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    var total = vec4<f32>(0.0);
    for (var ray = 0u; ray < rays; ray++) {
        let a = weights[5] + TAU * f32(ray) / f32(rays);
        let dx = cos(a) * len * 0.35;
        let dy = sin(a) * len * 0.35;
        let decay = exp(-1.0 / max(len, 1.0));
        var weight_sum = decay * (1.0 - powi(decay, i32(steps))) / (1.0 - decay);
        if abs(1.0 - decay) < 1e-6 { weight_sum = f32(steps); }
        weight_sum = max(weight_sum, 1e-6);
        var weight = 1.0;
        for (var k = 1u; k <= steps; k++) {
            weight *= decay;
            let t = f32(k) * len / f32(steps);
            total += cur_sample(px - dx * t - 0.5, py - dy * t - 0.5, EDGE_CLAMP) * weight / weight_sum;
        }
    }
    total = total / f32(rays);
    if colorize > 0.0 {
        let l = max(luma709(total.rgb), 0.0);
        let tinted = vec3<f32>(l * colour.r * 1.5, l * colour.g * 1.5, l * colour.b * 1.5);
        total = vec4<f32>(total.rgb + (tinted - total.rgb) * clamp(colorize, 0.0, 1.0), total.a);
    }
    dst[i] = sample_exit(add_light(inst[i], total.rgb * weights[3]));
}
// Mirrors whole_image::radial_rays; the current image is the highlights. weights =
// [centre x, centre y, length, intensity, colour RGB, only, steps].
@compute @workgroup_size(8, 8)
fn radial_rays(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let cx = weights[0];
    let cy = weights[1];
    let len = weights[2];
    let intensity = weights[3];
    let steps = u32(clamp(round_away(weights[8]), 4.0, 64.0));
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    var acc = vec4<f32>(0.0);
    var weight_sum = 0.0;
    for (var k = 0u; k < steps; k++) {
        let t = f32(k) / f32(steps) * len;
        let q = cur_sample(px + (cx - px) * t - 0.5, py + (cy - py) * t - 0.5, EDGE_CLAMP);
        let weight = 1.0 - f32(k) / f32(steps) * 0.6;
        acc += q * weight;
        weight_sum += weight;
    }
    let glow = acc / max(weight_sum, 1e-6);
    let light = vec3<f32>(
        glow.r * weights[4] * intensity * 2.5,
        glow.g * weights[5] * intensity * 2.5,
        glow.b * weights[6] * intensity * 2.5);
    if weights[7] > 0.5 {
        dst[i] = sample_exit(vec4<f32>(light, min(max3(light), 1.0)));
        return;
    }
    dst[i] = sample_exit(add_light(inst[i], light));
}
fn leak_overlay(c: f32, l: f32) -> f32 {
    let s = min(c + l + 0.5, 1.0);
    if c < 0.5 { return 2.0 * c * s; }
    return 1.0 - 2.0 * (1.0 - c) * (1.0 - s);
}
// Mirrors whole_image::light_leaks; weights = [colour 1 RGB, colour 2 RGB, intensity,
// scale, direction, phase, seed, mode].
@compute @workgroup_size(8, 8)
fn light_leaks(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let c1 = vec3<f32>(weights[0], weights[1], weights[2]);
    let c2 = vec3<f32>(weights[3], weights[4], weights[5]);
    let scale = weights[7];
    let phase = weights[9];
    let seed = u32(max(round_away(weights[10]), 0.0));
    let mode = u32(max(round_away(weights[11]), 0.0));
    let base = inst[i];
    let w = f32(p.width);
    let h = f32(p.height);
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    let dx = cos(weights[8]);
    let dy = sin(weights[8]);
    let along = ((px / w - 0.5) * dx + (py / h - 0.5) * dy) * 2.0;
    let bias = smooth_edge(-0.2, 1.0, -along);
    let u = px / scale;
    let v = py / scale;
    let n = fbm(u + dx * phase * 0.3, v + dy * phase * 0.3, phase * 0.15, 3.0, seed) * 0.5 + 0.5;
    let m = fbm(u * 0.7 + 5.2, v * 0.7, phase * 0.1 + 9.0, 2.0, seed) * 0.5 + 0.5;
    let k = smooth_edge(0.35, 0.85, n) * bias * weights[6];
    let light = (c1 + (c2 - c1) * m) * k;
    let colour = straight_rgb(base);
    var blended = vec3<f32>(1.0) - (vec3<f32>(1.0) - min(colour, vec3<f32>(1.0))) * (vec3<f32>(1.0) - min(light, vec3<f32>(1.0)));
    if mode == 1u {
        blended = colour + light;
    } else if mode == 2u {
        blended = vec3<f32>(leak_overlay(colour.r, light.r), leak_overlay(colour.g, light.g), leak_overlay(colour.b, light.b));
    }
    dst[i] = sample_exit(premultiply(blended, base.a + (1.0 - base.a) * min(k, 1.0)));
}
// One ghost ring of whole_image::lens_flare added to `light`.
fn flare_ghost(light: vec3<f32>, pos: vec2<f32>, centre: vec2<f32>, mid: vec2<f32>, scale: f32, t: f32, radius: f32, hue: vec3<f32>, factor: f32) -> vec3<f32> {
    let gx = centre.x + (mid.x - centre.x) * 2.0 * t;
    let gy = centre.y + (mid.y - centre.y) * 2.0 * t;
    let ex = pos.x - gx;
    let ey = pos.y - gy;
    let dd = sqrt(ex * ex + ey * ey) / scale;
    let ring = (max(1.0 - abs((dd - radius) / 0.01), 0.0) * 0.12 + select(0.0, 0.05, dd < radius)) * factor;
    return light + hue * ring;
}
// Mirrors whole_image::lens_flare; weights = [centre x, centre y, brightness, blend].
@compute @workgroup_size(8, 8)
fn lens_flare(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let centre = vec2<f32>(weights[0], weights[1]);
    let w = f32(p.width);
    let h = f32(p.height);
    let pos = vec2<f32>(f32(id.x) + 0.5, f32(id.y) + 0.5);
    let scale = max(w, h);
    let ex = pos.x - centre.x;
    let ey = pos.y - centre.y;
    let d = sqrt(ex * ex + ey * ey) / scale;
    var light = vec3<f32>(0.35 * exp(-d * 18.0) + 0.08 * exp(-d * 3.0));
    let mid = vec2<f32>(w / 2.0, h / 2.0);
    light = flare_ghost(light, pos, centre, mid, scale, 0.35, 0.05, vec3<f32>(0.4, 0.6, 1.0), 1.0);
    light = flare_ghost(light, pos, centre, mid, scale, 0.77, 0.08, vec3<f32>(1.0, 0.5, 0.3), 0.8);
    light = flare_ghost(light, pos, centre, mid, scale, 1.19, 0.03, vec3<f32>(0.9, 0.4, 1.0), 1.0);
    let gain_factor = weights[2] * (1.0 - weights[3]);
    dst[i] = sample_exit(add_light(inst[i], max(light * gain_factor, vec3<f32>(0.0))));
}
// Mirrors whole_image::rgb_split; weights = [mode, amount, angle, centre x, centre y,
// max distance, blend].
@compute @workgroup_size(8, 8)
fn rgb_split(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let amount = weights[1];
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    var ox = cos(weights[2]) * amount;
    var oy = sin(weights[2]) * amount;
    if u32(max(round_away(weights[0]), 0.0)) == 1u {
        ox = (px - weights[3]) / max(weights[5], 1.0) * amount;
        oy = (py - weights[4]) / max(weights[5], 1.0) * amount;
    }
    let r = in_sample(px - ox - 0.5, py - oy - 0.5, EDGE_CLAMP);
    let g = inst[i];
    let b = in_sample(px + ox - 0.5, py + oy - 0.5, EDGE_CLAMP);
    let mixed = vec4<f32>(r.r, g.g, b.b, max(max(r.a, g.a), b.a));
    dst[i] = sample_exit(mixed + (g - mixed) * weights[6]);
}

// ---- 265 overlay.

// Mirrors whole_image::drop_shadow; the current image is the (blurred) item. weights =
// [encoded colour RGB, opacity, dx, dy, only].
@compute @workgroup_size(8, 8)
fn drop_shadow(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let colour = decoded_rgb(vec3<f32>(weights[0], weights[1], weights[2]));
    let shifted = cur_sample(f32(id.x) + 0.5 - weights[4] - 0.5, f32(id.y) + 0.5 - weights[5] - 0.5, EDGE_CLAMP);
    let alpha = shifted.a * weights[3];
    dst[i] = sample_exit(over_shadow(vec4<f32>(colour * alpha, alpha), inst[i], weights[6]));
}
// Mirrors whole_image::long_shadow; weights = [angle, length, reach, encoded colour RGB,
// opacity, fade, only, steps].
@compute @workgroup_size(8, 8)
fn long_shadow(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let colour = decoded_rgb(vec3<f32>(weights[3], weights[4], weights[5]));
    let len = weights[1];
    let reach = weights[2];
    let steps = u32(clamp(round_away(weights[9]), 2.0, 64.0));
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    let dx = sin(weights[0]);
    let dy = -cos(weights[0]);
    var alpha = 0.0;
    for (var k = 1u; k <= steps; k++) {
        let dist = reach * f32(k) / f32(steps);
        let q = in_sample(px - dx * dist - 0.5, py - dy * dist - 0.5, EDGE_TRANSPARENT);
        if q.a > 0.5 {
            var f = 1.0;
            if weights[7] > 0.5 { f = 1.0 - dist / max(len, 1e-3); }
            alpha = clamp(f, 0.0, 1.0) * weights[6];
            break;
        }
    }
    dst[i] = sample_exit(over_shadow(vec4<f32>(colour * alpha, alpha), inst[i], weights[8]));
}
// Mirrors whole_image::gradient; weights = [start x, start y, end x, end y, encoded
// start RGB, encoded end RGB, shape, gamma].
@compute @workgroup_size(8, 8)
fn gradient(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let sx = weights[0];
    let sy = weights[1];
    let first = decoded_rgb(vec3<f32>(weights[4], weights[5], weights[6]));
    let last = decoded_rgb(vec3<f32>(weights[7], weights[8], weights[9]));
    let shape = u32(max(round_away(weights[10]), 0.0));
    let vx = weights[2] - sx;
    let vy = weights[3] - sy;
    let l2 = max(vx * vx + vy * vy, 1e-6);
    let l = sqrt(l2);
    let dx = f32(id.x) + 0.5 - sx;
    let dy = f32(id.y) + 0.5 - sy;
    let along = (dx * vx + dy * vy) / l2;
    let across = (-dx * vy + dy * vx) / l2;
    var t = along;
    if shape == 1u {
        t = sqrt(dx * dx + dy * dy) / l;
    } else if shape == 2u {
        t = abs(along);
    } else if shape == 3u {
        t = abs(along) + abs(across);
    }
    t = powf(clamp(t, 0.0, 1.0), weights[11]);
    dst[i] = sample_exit(premultiply(first + (last - first) * t, 1.0));
}
// Mirrors whole_image::four_color_gradient; weights = four points, then four encoded
// RGB colours.
@compute @workgroup_size(8, 8)
fn four_color_gradient(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let px = f32(id.x) + 0.5;
    let py = f32(id.y) + 0.5;
    var acc = vec3<f32>(0.0);
    var weight_sum = 0.0;
    for (var k = 0u; k < 4u; k++) {
        let ex = px - weights[2u * k];
        let ey = py - weights[2u * k + 1u];
        let d = max(sqrt(ex * ex + ey * ey), 1.0);
        let weight = 1.0 / powi(d, 2);
        let c = vec3<f32>(weights[8u + 3u * k], weights[9u + 3u * k], weights[10u + 3u * k]);
        acc = acc + c * weight;
        weight_sum += weight;
    }
    dst[i] = sample_exit(premultiply(acc / max(weight_sum, 1e-6), 1.0));
}

// ---- 264 spatial entries.

// A Pixelate tile centre of the instance input, clamped into the picture.
fn tile_centre(ti: f32, tj: f32, bw: f32, bh: f32) -> vec4<f32> {
    let x = clamp((ti + 0.5) * bw - 0.5, 0.0, f32(p.width) - 1.0);
    let y = clamp((tj + 0.5) * bh - 0.5, 0.0, f32(p.height) - 1.0);
    return in_sample(x, y, EDGE_CLAMP);
}
// Mirrors render effects::pixelate_sample; weights = [block width, block height, softness].
@compute @workgroup_size(8, 8)
fn pixelate(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let bw = weights[0];
    let bh = weights[1];
    let softness = weights[2];
    let tiles_x = max(ceil(f32(p.width) / bw), 1.0);
    let tiles_y = max(ceil(f32(p.height) / bh), 1.0);
    let ix = clamp(floor(f32(id.x) / bw), 0.0, tiles_x - 1.0);
    let iy = clamp(floor(f32(id.y) / bh), 0.0, tiles_y - 1.0);
    let nearest = tile_centre(ix, iy, bw, bh);
    if softness <= 0.0 { dst[i] = sample_exit(nearest); return; }
    let rx = (f32(id.x) + 0.5) / bw - 0.5;
    let ry = (f32(id.y) + 0.5) / bh - 0.5;
    let i0 = floor(rx);
    let j0 = floor(ry);
    let fx = rx - i0;
    let fy = ry - j0;
    let mx = tiles_x - 1.0;
    let my = tiles_y - 1.0;
    let t00 = tile_centre(clamp(i0, 0.0, mx), clamp(j0, 0.0, my), bw, bh);
    let t10 = tile_centre(clamp(i0 + 1.0, 0.0, mx), clamp(j0, 0.0, my), bw, bh);
    let t01 = tile_centre(clamp(i0, 0.0, mx), clamp(j0 + 1.0, 0.0, my), bw, bh);
    let t11 = tile_centre(clamp(i0 + 1.0, 0.0, mx), clamp(j0 + 1.0, 0.0, my), bw, bh);
    let top = t00 + (t10 - t00) * fx;
    let bottom = t01 + (t11 - t01) * fx;
    let lattice = top + (bottom - top) * fy;
    dst[i] = sample_exit(nearest + (lattice - nearest) * softness);
}
// Mirrors render effects::detail_boost_sample; the current image is the blurred copy.
// weights = [gain, threshold].
@compute @workgroup_size(8, 8)
fn detail_boost(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let threshold = weights[1];
    let original = inst[i];
    let difference = original.rgb - src[i].rgb;
    var keep = abs(difference) >= vec3<f32>(threshold * original.a);
    if threshold <= 0.0 { keep = vec3<bool>(true); }
    let boosted = original.rgb + select(vec3<f32>(0.0), weights[0] * difference, keep);
    dst[i] = sample_exit(vec4<f32>(boosted, original.a));
}
// f32::rem_euclid.
fn rem_euclid(x: f32, y: f32) -> f32 {
    let r = x - y * trunc(x / y);
    if r < 0.0 { return r + abs(y); }
    return r;
}
// spatial::iris_radius: the circumradius-1 polygon radius in direction `theta`.
fn iris_radius(sides: u32, theta: f32, rotation: f32) -> f32 {
    if sides == 0u { return 1.0; }
    let segment = TAU / f32(sides);
    let a = rem_euclid(theta - rotation, segment) - segment / 2.0;
    return cos(PI / f32(sides)) / cos(a);
}
// spatial::lens_tap: a clamped bilinear tap with the highlight boost.
fn lens_tap(x: f32, y: f32, boost: f32, threshold: f32) -> vec4<f32> {
    let q = cur_sample(x, y, EDGE_CLAMP);
    if boost <= 0.0 || q.a <= 1e-6 { return q; }
    let l = luma709(encoded_rgb(straight_rgb(q)));
    var t = clamp((l - threshold) / (1.0 - threshold), 0.0, 1.0);
    if threshold >= 1.0 { t = select(0.0, 1.0, l >= 1.0); }
    let k = 1.0 + boost * t * t * (3.0 - 2.0 * t);
    return vec4<f32>(q.rgb * k, q.a);
}
// Mirrors spatial::lens_blur, the direct gather; the adapter refuses the discs the CPU
// reduces (render effects::shader_computes). weights = [sides, rotation, radius x,
// radius y, rings, boost, threshold].
@compute @workgroup_size(8, 8)
fn lens_blur(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let sides = u32(max(weights[0], 0.0));
    let rotation = weights[1];
    let rx = weights[2];
    let ry = weights[3];
    let rings = u32(max(weights[4], 0.0));
    let boost = weights[5];
    let threshold = weights[6];
    let cx = f32(id.x);
    let cy = f32(id.y);
    var sum = lens_tap(cx, cy, boost, threshold);
    var count = 1.0;
    for (var ring = 1u; ring <= rings; ring++) {
        let taps = max(ring * 8u, 6u);
        let radius = f32(ring) / f32(rings);
        for (var k = 0u; k < taps; k++) {
            let theta = TAU * (f32(k) + 0.5 * f32(ring % 2u)) / f32(taps);
            let reach = radius * iris_radius(sides, theta, rotation);
            sum += lens_tap(cx + cos(theta) * reach * rx, cy + sin(theta) * reach * ry, boost, threshold);
            count += 1.0;
        }
    }
    dst[i] = sample_exit(sum / count);
}
// Mirrors spatial::wrap_shift; weights = [dx, dy, blend].
@compute @workgroup_size(8, 8)
fn wrap_shift(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let shifted = cur_sample(f32(id.x) - weights[0], f32(id.y) - weights[1], EDGE_WRAP);
    dst[i] = sample_exit(shifted + (src[i] - shifted) * weights[2]);
}
// Mirrors render effects::streak_sample; weights = [dx, dy, steps].
@compute @workgroup_size(8, 8)
fn streak_blur(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let steps = weights[2];
    var sum = vec4<f32>(0.0);
    for (var k = 0u; k < u32(max(steps, 0.0)); k++) {
        let t = f32(k) / (steps - 1.0) - 0.5;
        let sx = clamp(f32(id.x) + weights[0] * t, 0.0, f32(p.width) - 1.0);
        let sy = clamp(f32(id.y) + weights[1] * t, 0.0, f32(p.height) - 1.0);
        sum += cur_sample(sx, sy, EDGE_CLAMP) / steps;
    }
    dst[i] = sample_exit(sum);
}
// Linear luma of one clamped premultiplied neighbour, as edge_lines_sample reads it.
fn edge_luma(x: i32, y: i32) -> f32 {
    return luma709(clamped_src(x, y).rgb);
}
// Mirrors render effects::edge_lines_sample; weights = [invert, blend].
@compute @workgroup_size(8, 8)
fn edge_lines(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let x = i32(id.x);
    let y = i32(id.y);
    let gx = edge_luma(x + 1, y - 1) + 2.0 * edge_luma(x + 1, y) + edge_luma(x + 1, y + 1)
        - edge_luma(x - 1, y - 1) - 2.0 * edge_luma(x - 1, y) - edge_luma(x - 1, y + 1);
    let gy = edge_luma(x - 1, y + 1) + 2.0 * edge_luma(x, y + 1) + edge_luma(x + 1, y + 1)
        - edge_luma(x - 1, y - 1) - 2.0 * edge_luma(x, y - 1) - edge_luma(x + 1, y - 1);
    var m = min(sqrt(gx * gx + gy * gy), 1.0);
    if weights[0] <= 0.5 { m = 1.0 - m; }
    let centre = src[i];
    let mix_back = clamp(weights[1], 0.0, 1.0);
    let level = m * centre.a * (1.0 - mix_back);
    dst[i] = sample_exit(vec4<f32>(vec3<f32>(level) + centre.rgb * mix_back, centre.a));
}
// render effects::straight_sample: transparent neighbours contribute zero.
fn loose_straight(q: vec4<f32>) -> vec3<f32> {
    if q.a <= 1e-6 { return vec3<f32>(0.0); }
    return q.rgb / q.a;
}
// Mirrors render effects::colour_relief_sample; weights = [dx, dy, contrast, blend].
@compute @workgroup_size(8, 8)
fn colour_relief(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.width || id.y >= p.height { return; }
    let i = id.y * p.width + id.x;
    let centre = src[i];
    let alpha = centre.a;
    if alpha <= 1e-6 { dst[i] = sample_exit(centre); return; }
    let wmax = f32(p.width) - 1.0;
    let hmax = f32(p.height) - 1.0;
    let plus = cur_sample(clamp(f32(id.x) + weights[0], 0.0, wmax), clamp(f32(id.y) + weights[1], 0.0, hmax), EDGE_CLAMP);
    let minus = cur_sample(clamp(f32(id.x) - weights[0], 0.0, wmax), clamp(f32(id.y) - weights[1], 0.0, hmax), EDGE_CLAMP);
    let difference = luma709(loose_straight(plus)) - luma709(loose_straight(minus));
    let e = encoded_rgb(centre.rgb / alpha);
    let relief = clamp(e + 2.0 * weights[2] * difference, vec3<f32>(0.0), vec3<f32>(1.0));
    let mixed = relief + (e - relief) * clamp(weights[3], 0.0, 1.0);
    dst[i] = sample_exit(decoded_premultiplied(mixed, alpha));
}
