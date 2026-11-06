# Transform

`TransformField` owns the editor's units, defaults and finite clamps for the five
existing transform channels: position X/Y in project-canvas pixels
(−100,000..=100,000, default 0), independent X/Y scale ratios
(0.01..=10, default 1), and clockwise rotation in degrees
(−360..=360, default 0). The command adapter refuses out-of-bounds input;
animated evaluation clamps overshooting curves to these bounds. These channels
reuse the existing persisted animation representation without a schema change.
