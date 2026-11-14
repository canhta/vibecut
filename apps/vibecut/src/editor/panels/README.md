# Editor panels

Interface: `agent`, `assets`, `player`, `properties`.

The editor's docked areas, each a folder interface that owns its `State` and returns
requests for the frame to route. `player` holds the preview, playback and crop area;
`properties` owns the inspector, its colour and font popovers and its workflow; `assets`
owns the media, import, sounds, music, effects, transitions and captions tabs; `agent`
holds the chat docked beside the timeline ([README](agent/README.md)). The
captions and timeline areas sit beside this folder under `editor/`.
