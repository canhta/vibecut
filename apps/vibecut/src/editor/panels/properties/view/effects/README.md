# Effects view

The Adjust tab's colour wheels and tone-curve editor.

`wheel.rs` owns the UI-only polar mapping between a puck position and the stored lift, gamma and gain percentages. `curve.rs` owns the pointer-edit proposal that produces a validated monotone `ToneCurve`. The parent `effects.rs` draws both, records their automation ids and turns each gesture into effect commands; it keeps no project copy.

A wheel drag writes all three channel percentages of one family per pointer move inside one engine gesture; the release is one undo step. A wheel's reset writes its three neutral values as one batch. The curve plot adds a point on a click near the polyline, moves an interior point on a drag, removes an interior point on a double-click and keeps endpoints fixed; every publication replaces the whole curve through `effects.update_param`. Reset restores the two-point identity. The shared plot geometry, wheel diameter and curve height are a new laydown under the measured 460 px Properties panel; no reference capture measures them.
