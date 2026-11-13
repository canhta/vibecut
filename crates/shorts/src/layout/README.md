# Short layouts

A short shows each shot in one of seven layouts (source: the [layout and reframing](https://help.opus.pro/docs/article/layout-and-reframing) help article of the reference clip editor). A layout is not a render path: it is a list of video items on the short's derived project, each made of params that already exist, `VideoParams.crop`, `VideoParams.fit` and the static `transform.scale{X,Y}` / `transform.position{X,Y}` channels. Preview and export render that project through the one scene evaluation, so they agree by construction.

| Layout | Condition (shot evidence from [shot analysis](../../../engine/src/analysis/shot_analysis/README.md)) | Items |
|---|---|---|
| Fill | always | one item, `fit` Fill, crop = the largest window of the canvas aspect centred on the active speaker's track; no track, no crop (centred cover) |
| Fit | always | one item, `fit` Fit, no crop (letterbox) |
| Split | at least 2 people | two equal rows, one person each |
| Three | at least 3 people | a full-width top row, two half-width bottom cells |
| Four | at least 4 people | a 2x2 grid |
| Screenshare | `Scene::Screen` and at least 1 person | whole frame (Fit box) on top, the speaker below |
| Gameplay | `Scene::Screen` and at least 1 person | speaker top 30 %, whole frame (Fit box) below 70 % |

People are `min(people_count, tracks)`. Slot 0 is the main-track item and carries the source audio; every further slot is a silent item on its own track (ids 4 and up, below the captions). A person slot is `fit` Manual (Stretch base) with a crop window of the slot's aspect (`FitMode::Fill.extent` over the cell, the one cover computation), scaled into its cell by `scale = cell / canvas` and moved by `position = cell centre - canvas centre`. A content slot has no crop; its Fit box (`FitMode::Fit.place` over the cell) gives the scale, so the picture is letterboxed inside the cell. The derived project's canvas is the 1080x1920 output, so `project::fit_box` and the transforms work in output pixels.

## Choice

Per shot, with the override first (segment beats short): a forced layout whose condition holds is used; one whose condition fails falls back (`ShotLayout.fallback`). Without an override: `Scene::Screen` with a person is Screenshare, `Screen` without one Fit; otherwise 0 people Fit, 1 Fill, 2 Split, 3 Three, 4 or more Four. The fallback is Fit when the shot has analysis but no person to crop around, else Fill. A stretch outside the analysis, or a request with no analysis, is a centred Fill. Gameplay is never automatic: no detector separates gameplay from other screen content (open).

Switching happens only at shot ends: a segment is cut into one piece per shot it overlaps, each piece takes one layout, a piece shorter than one canvas frame joins its neighbour, and neighbours with equal layout and slots are one item. Segment boundaries stay where the short puts them.

## Pin first speaker

`LayoutOverrides.pin_first_speaker` keeps the short's first visible speaker in slot 0 of Split, Three and Four. The speaker is the first piece in play order whose `SpeakerState` is Visible; its anchor is the track's cross-shot `identity`, else its shot-local reference. Later shots match tracks by `identity == anchor`. A shot without that person falls back to the unpinned order: the most prominent people, left to right.

## Overrides, persistence, undo

`LayoutOverrides` (`short`, `segments` by play-order index, `pin_first_speaker`) rides on `ClipShort`; it serializes with `deny_unknown_fields` and no defaults. `LayoutOverrides::apply(LayoutEdit)` returns the inverse edit, so a history of inverses undoes and redoes an override exactly. Changing an override changes the derived project, hence the export cache key.

## Open values

The cited source gives one proportion, Gameplay 30 % / 70 %. Equal cells for Split, Screenshare and Four, the Three arrangement, the Gameplay and Screenshare slot order, the left-to-right order of unpinned people and the use of the largest window (no zoom around the face) are not sourced and stay open until a source or the maintainer fixes them. All of them live in `composition.rs`.
