# Transition kinds

Implementation reference: the structure reference's CPU transition kernels, adapted to the working-pixel contract (premultiplied linear RGBA f32, finite 0..=1, RGB at most alpha). `project` owns the ids, parameters and shelves; the parent [registry](../README.md#transition-kinds) owns the dispatcher and the endpoint contract.

`pixels` provides the shared maths: clamped bilinear sampling, integer hash and fractal noise, light addition, motion/disc sampling kernels, and the three-box-pass Gaussian blur. Gaussian radii are capped independently by each image axis and computed in f64. Frames and column scratch are reserved before allocation; rows read the immutable source without a scratch copy. Initialization and long rows check cancellation in bounded groups. Pixel extents that cannot be represented return a typed invalid-input error.

| Child | Kinds |
|---|---|
| `dissolve` | Light Fade, Film Fade, Soft Focus Fade, Burn Through, Brightness Fade, Pixel Blocks |
| `wipe` | Straight, Soft Edge, Clock, Corner Sweep, Star, Panel, Stair, Stretch, Neon Edge, Circle/Box/Diamond/Cross Reveal, Double Doors, Corner Inset |
| `lights` | Zoom Cross, Zoom Smear, Spin Smear, Streak Smear, Lens Defocus, Bloom, Flash, Lens Flare, Light Leak, Colour Leak, Colour Burn, Light Sweep, Light Rays, Phosphor Trail, Solarise, Stripes |
| `grunge` | Scramble, Digital Glitch, Tape Damage, Flicker Cut, Quake, Glass Shards, Rough Edge, Kaleidoscope, Liquid Warp, Screen Off |

Every kind is a pure function of the outgoing frame, incoming frame, progress strictly inside (0, 1), the pixel scale and the persisted parameters: progress 0 is exactly the outgoing frame, 1 exactly the incoming one, and the output is finite, deterministic and continuous towards both ends. Pixel-denominated parameters multiply by `scale`; points are frame fractions with the origin at the top left; angles are clockwise degrees.
