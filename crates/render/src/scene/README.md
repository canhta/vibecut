# Working pixels and scene

`scene/` applies the per-item visual stages (transform, blend, mask, effects, color, caption) in working space.

`Renderer::new(project, RenderSettings)` validates output geometry, each destination rectangle and the project-owned source crop. `render_frame(t, sources, hooks)` returns one `Image`; `preview_frame` is the separate placeholder path. Track order is bottom-to-top. Hidden visual tracks and half-open inactive items produce no pixels. Position uses the project-owned retime mapping. Cover center-crops, Contain letterboxes, Stretch maps the selected crop. Background blur renders behind every timeline layer. Selected caption languages admit up to two; the default is the source language or the first available text.

`Placement.crop` is project's `Crop` and `Rect` is destination geometry only; nothing else owns a normalized source rectangle. An item's `VideoParams.crop` is composed into the placement's source window before the first sample; it therefore precedes transform, mask, effect and blend stages and the cropped region keeps its aspect through the placement's `Fit`. Absent or full-frame crop samples the whole source, and the shorts batch path sets the same placement crop.

An item's `opacity` animation overrides its static `Placement.opacity`. Evaluation calls project's `Animations::value_at` with item-local ticks, the evaluator the engine uses. Values clamp to 0..=1. Missing opacity data keeps the static placement.

The five transform channels evaluate through project's `TransformField` at item-local time. Position is in project-canvas pixels. Scale and clockwise rotation pivot around the item's untransformed destination centre. A nonidentity transform reserves an extra canvas layer, inverse-samples with transparent borders after masks and effects, then applies opacity and blending against the original backdrop. Cancellation is checked per row. Identity transforms take a fast path. Content is clipped to the canvas before transformation; the off-canvas raster extent is not expanded.

## Preview viewport geometry

`render::viewport::Viewport` owns screen and canvas conversion for player input, snapping and mask tools. Its constructor takes a logical-screen-pixel rectangle, project canvas dimensions, a positive zoom about the fitted canvas centre and a screen-pixel pan. Aspect-fit letterboxing precedes zoom and pan. Conversion stays defined beyond the canvas for captured drags. Invalid, non-finite or unrepresentable geometry returns `render::Error::Invalid`. `canvas_distance` converts screen-pixel tolerances without translation.

`Renderer::item_geometry(id, output_tick)` evaluates the same placement, opacity and transform as pixels. `hit_test(viewport, screen_point, output_tick)` walks bottom-to-top render order and returns the topmost selectable `ItemId`, which the caller passes to `Session::set_selection`. The viewport must name this project's canvas. Hidden or locked tracks, audio, fully transparent solid overlays, empty text, zero evaluated opacity and inactive caption cues are excluded. Time and screen/canvas edges are half-open. Selection uses the destination rectangle, including transparent pixels inside a text or media box; letterboxes, masks and effect transparency do not cut holes in it. Selection does not decode or rasterize media. Active transitions use project's resolved source-handle contributors, excluding zero-weight sides and both clips at the opaque dip midpoint.

`ItemGeometry::bounds` is the untransformed destination rectangle in project canvas pixels; `point` maps a canvas point inside it through the same placement transform to screen space and `inverse` maps a screen point back. `ItemGeometry::handle_position` applies scale, clockwise rotation and translation around the placement centre before canvas-to-screen conversion. Eight `Handle` positions run clockwise from top-left. `hit_handle` picks the nearest within an explicit logical-screen-pixel radius. `scale_at` takes an absolute canvas pointer: centre-anchored edge handles scale their own axis, corners scale both independently, and crossing the centre clamps to the project-owned minimum instead of flipping. `rotation_handle_position` places the grip beyond the rotated bottom midpoint at a caller-chosen constant screen-pixel gap, independent of zoom.

Edits use the [engine preview transaction](../../../engine/src/session/README.md#transactions-gestures-and-changes). The caller converts the pointer relative to its gesture origin and submits absolute `timeline.set_item_param` patches for `transform.positionX/Y`, `transform.scaleX/Y`, `transform.rotate` or `text_content` inside `timeline.preview`. Every update replaces the candidate from the committed origin. End with `timeline.commit_preview` (one undo step) or `timeline.discard_preview`. Geometry reads `preview_project()` while a candidate exists. Widgets, pointer capture and inline text editing belong to the player.

## Color and text

`color` owns hex and HSV entry conversion over project's straight sRGB RGBA8 value: six or eight hex digits, hue 0..360 degrees, saturation, value and alpha 0..1. Finite HSV entries clamp and non-finite input refuses. Alpha stays straight and independent of RGB. Linear composite colour channels use the compositor's transfer conversion.

Every working pixel is RGBA, premultiplied alpha, linear-light f32, finite 0..1 with RGB <= alpha. Compositing applies the item's blend function, then Porter-Duff source-over. No gamma-space blending. Bilinear sampling has transparent borders, and repeated box half-filters precede large downscales, including odd-sized edges. The separable sampler matches the scalar reference in arithmetic and border behavior.

BT.709, sRGB and linear transfers convert at ingress. Text's premultiplied RGBA8 is unpremultiplied before transfer conversion, then premultiplied once in linear light. Strided decoder RGBA rows convert directly into working pixels; padding is ignored, row bounds are validated before allocation and conversion checks cancellation per row. Opaque video packing and alpha-aware packing use separate loops with the same transfer tables and rounding. `rgba8` produces straight-alpha boundary pixels. `video_rgba8` composites residual transparency over black in linear light before producing opaque BT.709 pixels for MP4.

File-backed pixels admit 8-bit BT.709 primaries (unknown SDR tags use media's BT.709 default), the transfers above, square pixels and identity display matrices. Other primaries, HDR, high depth, non-square pixels and rotation return `Unsupported`; there is no implicit gamut change, tone mapping or display transform. Unchanged passthrough keeps the source parameters. Output geometry is positive and at most 2^26 pixels; encoded H.264 dimensions are even.

Text uses the item's bundled font family, centered in its project-owned destination. Positions are canvas percentages of the box centre (default 50/84.5), clipping at canvas edges instead of clamping the box. Text background boxes use project's `TextParams.background` (padding, offset, radius, colour, each overridden by its keyed `background.*` channel at item-local time; text colour by `color`). The box paints before the glyphs. The raster grows by the box margin on every side, so padding and offset never move the text, and preview and export draw the same cached raster. Pixel size and box padding scale with output/project width. Generated solid overlays use `params.strokeColor` as a two-project-pixel inside stroke.

Immutable raster reuse is local to a Renderer, keyed by NFC text, font and fallback identity, size, color, background and pixel geometry. Default caps are 8 MiB and four entries; settings cap at 256 MiB and 64 entries; zero bytes disables reuse. LRU entries hold byte leases, and pressure clears idle entries before one retry. `text_cache_stats` reports hits, misses, bytes and entries; `clear_text_cache` releases them.

## Subtitle styling and burn-in

`RenderSettings::caption_style` controls bundled font family and style, RGBA fill, outer outline, rounded background box, top/middle/bottom position, safe area and maximum total lines. Size, outline and box dimensions are pixels at 1080 output lines and scale with output height in every aspect ratio. The safe area is a 0 to 40 percent inset on each canvas edge, default 5. An explicit item placement overrides the default destination. Caption geometry uses a bounded line-height band inside the safe rectangle, not a full-frame raster. Settings are transient and outside the project schema.

Selected language payloads join in display order with a mandatory line break. UAX #14 wrapping and bundled CJK fallback handle both Han scripts without inserted spaces; the text owner normalizes NFC and keeps graphemes intact. Empty payloads paint no box. Captions wrap at the requested size first. When they exceed the line count or destination, a bounded ten-step search shrinks the font to fit, down to one output pixel (or the requested size if smaller). Fitting captions keep their exact pixels, and outline and background padding stay fixed. A missing requested language, mandatory breaks exceeding the line count, or text that cannot fit at the minimum size returns a typed refusal; the second language is never silently dropped. Cue activity uses half-open source ranges, so gaps have no caption pixels. `preview_frame` and `render_frame` share the overlay path. Outline, box, font, size, position, line limit, text and pixel geometry key the same bounded raster cache; there is no second caption cache or colour conversion.

## Effects and desktop adapter

`effects::EffectCompositor` receives a working item image, the evaluated ordered pass list, optional item and mask coverage, and the already-composited track-below image. `CpuEffects` is the reference and the CLI/export default; `RenderSettings.effects` optionally injects the desktop adapter. Unsupported GPU capability falls back to CPU, while cancellation, budget and invalid-data failures propagate. The renderer paints an effected item into a transparent output-sized layer, filters it, then composites it over the unchanged below image. Effects disable video stream copy.

Colour grade (`EffectParams::ColorGrade`, one `grade` pass of four weights) runs in premultiplied linear working space on the straight colour in this order: brightness is linear gain 2^(v/50); contrast is power 1+v/100 about linear mid grey 0.18; saturation is a Rec. 709 luma blend 1+v/50; hue rotates v/50*180 degrees about the grey axis; then clamp to 0..1 and re-premultiply. All-zero emits no pass. `grade_pixel` is the reference the app's `grade` shader mirrors. Masks apply after the grade.

Chroma key (`EffectParams::ChromaKey`, one `chroma_key` pass of five weights) keys in sRGB-encoded BT.709 YCbCr, encoded from the premultiplied linear pixel. Keyness is the chroma projection onto the key's chroma, clamped 0..1.5, faded by the perpendicular chroma distance over a tolerance of `0.08 + 0.5*intensity`; the matte is `1 - keyness*(1 + 2*intensity)` plus `(key luma - pixel luma) * shadow * 2 * keyness` where the pixel is darker than the key, so shadows survive. Intensity 0 emits no pass. Intensity and shadow evaluate from their scalar keyframes at `effects.<instance-id>.params.intensity`/`shadow` at item-local time, clamped to 0..100; the key colour stays static. Spill suppression is inherent because the reference exposes no spill control: the key's dominant channel loses its excess over the larger of the other two, then the straight colour re-premultiplies by the new alpha. Stages and constants follow the implementation reference named in the ticket; the 0..100 ranges and defaults (green key colour, intensity 40, shadow 0) are measured from the observed reference UI. `chroma_key_pixel` is the reference the app's `chroma_key` shader mirrors. Masks and blend run after the key, so keyed alpha composes with them.

Blur intensity uses the static project value unless its scalar `Param` override exists, evaluated at item-local time and clamped to 0..100. Sigma is intensity/5 times min(output width/1920, output height/1080). The normalized separable Gaussian clamps edge taps and processes all RGBA channels in premultiplied linear f32. Sigma <= 0.01 output pixels is an identity. Above sigma 8, n=ceil((sigma/8)^2) iterations use sigma/sqrt(n) per axis, capped at 256 iterations and 49 taps per pass. The finite kernels approximate the untruncated Gaussian without source bit identity.

The desktop WGSL adapter consumes the same pass weights and entry names, with maximum absolute CPU/GPU working-channel error 2e-5. A missing native GPU is a labelled skip in offline CI and a failure in required-oracle mode.

## Mask coverage primitives

`mask::FlatMask` rasterizes an evaluated closed contour with nonzero winding. `prepare_builtin` supplies rectangle, cinematic band, ellipse, heart, diamond and star contours with cubic chord tolerance 0.05 output pixels. `SplitMask` evaluates an unbounded half-plane so frame edges add no artificial feather or stroke. Feather is a symmetric signed-distance ramp: one-pixel linear at zero feather, smoothstep over the feather width at >=1 pixel. Expansion precedes invert, then opacity scales coverage. Stroke uses distances on the retained and removed sides for inside, centre and outside alignment with independent one-output-pixel antialiasing. Mask feather affects body coverage only.

`RasterMask::from_image` consumes ingressed glyph alpha and reuses text's Euclidean distance transform; one prepared distance raster serves coverage and stroke. No glyph renderer, cache or transfer conversion lives in the mask engine. Coverage images use the compositor's premultiplied linear space, and `scale_by` scales every channel together. Effects consume working pixels before masking. Geometry mismatches refuse before mutation. Contour, scratch, distance and image allocations hold compositor reservations, and cancellation is checked per row, including text's distance-transform rows and columns.

`Renderer` applies project mask vectors to every visual layer in this order: item effects, combined coverage in vector order, enabled strokes in vector order, then item opacity, blend and source-over. None modes are skipped. Strokes stay visible outside the combined coverage. Only the current mask's raster data is retained; accumulated coverage and strokes are bounded working images. Active masks disable video stream copy for Auto and forced-copy admission.

Mask geometry uses the placed item's output rectangle. Project-pixel appearance widths scale by the smaller output/item axis ratio. Text masks use the bundled font, cache and ingress with height scaling, then a working-space affine sample. Contour geometry has a 65,536-point admission bound, and extreme output geometry refuses before uncontrolled allocation.

## Blend formulas in working space

`blend::composite` is the CPU reference for preview and export, including CLI. It accepts equal-size working images and finite opacity in 0..=1. Effects and mask or stroke coverage precede blend, and opacity scales every source channel once after them. Zero opacity leaves the backdrop bit-identical and the renderer skips that item, including its background-blur contribution.

Let `b`, `s` be straight linear-light channel values recovered from premultiplied backdrop and source RGB, with backdrop alpha `ab` and effective source alpha `as = source_alpha * opacity`. Working images stay premultiplied with no per-mode transfer choice.

`out_rgb = source_rgb * opacity * (1-ab) + backdrop_rgb * (1-as) + as*ab*B`

`out_alpha = as + ab*(1-as)`.

Zero source alpha bypasses division. Zero backdrop alpha uses ordinary over. Roundoff clamps to `0..out_alpha`. Nonseparable functions operate on all three components together.

| Mode | B(b,s) |
|---|---|
| normal | s |
| darken | min(b,s) |
| multiply | b*s |
| color-burn | b=1: 1; s=0: 0; otherwise 1-min(1,(1-b)/s) |
| lighten | max(b,s) |
| screen | b+s-b*s |
| plus-lighter | min(1,b+s) |
| color-dodge | b=0: 0; s=1: 1; otherwise min(1,b/(1-s)) |
| overlay | b<=0.5: 2*b*s; otherwise 1-2*(1-b)*(1-s) |
| soft-light | s<=0.5: b-(1-2*s)*b*(1-b); otherwise b+(2*s-1)*(D(b)-b) |
| hard-light | s<=0.5: 2*b*s; otherwise 1-2*(1-b)*(1-s) |
| difference | abs(b-s) |
| exclusion | b+s-2*b*s |
| hue | SetLum(SetSat(s,Sat(b)),Lum(b)) |
| saturation | SetLum(SetSat(b,Sat(s)),Lum(b)) |
| color | SetLum(s,Lum(b)) |
| luminosity | SetLum(b,Lum(s)) |

`D(b)=((16*b-12)*b+4)*b` for b<=0.25, otherwise sqrt(b). `Lum(c)=0.3*r+0.59*g+0.11*b` is the blend convention, not BT.709 luminance. `Sat(c)=max(c)-min(c)`. `SetSat(c,t)=(c-min(c))*t/(max(c)-min(c))`, or zero for range <=1e-6. `SetLum(c,t)` adds `t-Lum(c)` to every component, then clips around its luminosity: for original minimum n<0 use `l+(c-l)*l/max(l-n,1e-6)`; for original maximum x>1 use `l+(c-l)*(1-l)/max(x-l,1e-6)`. Plus-lighter is the saturated additive blend function with source-over alpha, not the Porter-Duff additive operator. These are the [W3C blend formulas](https://www.w3.org/TR/compositing-1/#blending) evaluated in linear space.

CPU tolerance against the f64 fixtures in `tests/fixtures/blend-linear.json` is 3e-6 per working channel. The desktop WGSL path allows 2e-5.

## Video transitions

The active transition replaces its two clips in track order over its half-open span. Project resolves one piece on each complete transition side at that output tick, including the first and last piece's handles beyond its side extent. Progress uses the unchanged timeline span, even after repeated splits. Both contributors use the retained source mapping and pass through the item compositor (effects, masks, transform, opacity, blend) over the same backdrop. Premultiplied linear-light frames then interpolate, directly or through the definition's opaque dip colour. Hidden tracks contribute no transition pixels; video mute is an audio control. The exact end resumes the incoming clip with no extra frame or gap. Preview, CLI snapshots and export share this path. Audio crossfades are excluded.

Export's stream-copy plan, HDR detection and source admission select clips with `Timeline::rendered_items`, so a range inside one clip that overlaps a transition span re-encodes through the compositor and checks the other clip's source. Admitted definitions are owned by [project](../../../project/src/visual/README.md#video-transitions).

Backdrop and outgoing images hold normal compositor byte leases. Cancellation during copying, item processing and each interpolation row releases every partial image.

Horizontal interpolation keeps two destination-width rows per active pixel task and reuses them when adjacent output rows share source neighbors. Vertical interpolation and source-over keep their floating-point order, and fully opaque samples replace the destination after the same clamp. Transfer packing uses exact float-bit transition buckets derived from the 16-bit transfer table, preserving its quantization.
