# Editor panels

Interface: `player`, `properties`.

The editor's docked areas, each a folder interface that owns its `State` and returns
requests for the frame to route. `player` holds the preview, playback and crop area;
`properties` owns the inspector, its colour and font popovers and its workflow. The
assets, captions and timeline areas sit beside this folder under `editor/`.
