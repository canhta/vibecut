# Speed curve view

The Speed tab's Curve sub-tab: the preset tiles and the speed plot. The parent `playback.rs` draws the Audio tab and the constant (Standard) speed rows and hands a curved or curve-opened item to `curve.rs`.

`curve.rs` draws eight tiles (None, Custom, Montage, Hero, Bullet, Jump cut, Flash in, Flash out) and, for a clip with a curve, the plot box: the Duration line, a log 0.1x-10x speed axis with the curve and its draggable points, Reset and an add/remove point button. It keeps no project copy; it plots `read::speed_shape`, the shape the clip plays (a split piece's slice of the curve). The drag in flight (the point and the shape it produces) and the selected point live in egui temporary data keyed by the item, so a drag shows live and commits once, on release, as one `timeline.set_speed_curve`; the add, remove, Reset and tile paths commit at once through the same command or `timeline.apply_speed_preset`. None sends `timeline.set_speed` at 1x and keeps the Curve view open.

Measured geometry (capture 100104, logical px) and the open values are in [the editor UI doc](../../../../../../../../docs/product/ui/editor.md#speed-tab-curve-view); the shape model is in the [animation README](../../../../../../../../crates/project/src/animation/README.md#speed-curves).
