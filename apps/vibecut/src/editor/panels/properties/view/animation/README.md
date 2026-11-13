# Animation tile previews

`previews.rs` runs the Animation tab's tile preview jobs on one panel-owned runner and
keeps each slot and look's strip as one texture. Strips come from the engine's
[animation-preview job](../../../../../../../../crates/engine/src/artifacts/README.md#animation-previews);
the view never renders a frame itself, so a draw only picks a strip frame.

The runner holds a 16 MiB budget and a 48-entry queue, enough for every look of every slot
(12 In, 12 Out, 13 Combo) at once. A strip is requested the first time its tab is drawn;
a failed job leaves that tile without a preview and is not retried while the panel lives.
