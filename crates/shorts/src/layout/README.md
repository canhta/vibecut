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

People are `min(people_count, tracks)`. Every slot is a silent video item: slot 0 on the main track, further slots on their own tracks (ids 4 to 6, below the captions). The short's audio is one continuous Audio item per segment on its own track (id 7), so the export's 10 ms ramps fall at segment edges only and never inside speech at a shot cut. A person slot is `fit` Manual (Stretch base) with a crop window of the slot's aspect (`FitMode::Fill.extent` over the cell, the one cover computation), scaled into its cell by `scale = cell / canvas` and moved by `position = cell centre - canvas centre`. A content slot has no crop; its Fit box (`FitMode::Fit.place` over the cell) gives the scale, so the picture is letterboxed inside the cell. The derived project's canvas is the 1080x1920 output, so `project::fit_box` and the transforms work in output pixels.

## Choice

Per shot, with the override first (segment beats short): a forced layout whose condition holds is used; one whose condition fails falls back (`ShotLayout.fallback`). Without an override: `Scene::Screen` with a person is Screenshare, `Screen` without one Fit; otherwise 0 people Fit, 1 Fill, 2 Split, 3 Three, 4 or more Four. The fallback is Fit when the shot has analysis but no person to crop around, else Fill. A stretch outside the analysis, or a request with no analysis, is a centred Fill. Gameplay is never automatic: no detector separates gameplay from other screen content (open).

## Decided and open

The cited source gives one proportion, Gameplay 30 % / 70 %. The maintainer decided the rest: equal cells for Split, Screenshare and Four; Three is one wide cell over two; the speaker is on top in Screenshare and Gameplay; unpinned people run left to right. All live in `composition.rs`.

Open: Gameplay is override-only until a detector separates gameplay from other screen content; no zoom around the face is sourced, so a person slot shows the largest window of its aspect.

## Where the analysis comes from

`clips.export` (engine control) resolves the shot analysis over the hull of the selected pieces and passes it as `ClipsExportRequest.shots`; `submit_export` hands it to `ClipBatchRequest.shots`. The engine reuses the cached `AnalysisResult` of that exact request, or starts the analysis as a job and exports with empty shots. The response's `shots` field is `None` when the analysis laid the shorts out, else a `ClipShotsDiagnostic`: `detector_unavailable`, `models_missing`, `pending { job }` or `failed`. An export after the pending job finishes reuses the result.

Switching happens only at shot ends: a segment is cut into one piece per shot it overlaps, each piece takes one layout, a piece shorter than one canvas frame joins its neighbour, and neighbours with equal layout and slots are one item. Segment boundaries stay where the short puts them.

## Pin first speaker

`LayoutOverrides.pin_first_speaker` keeps the short's first visible speaker in slot 0 of Split, Three and Four. The speaker is the first piece in play order whose `SpeakerState` is Visible; its anchor is the track's cross-shot `identity`, else its shot-local reference. Later shots match tracks by `identity == anchor`. A shot without that person falls back to the unpinned order: the most prominent people, left to right.

## Overrides, persistence, undo

`LayoutOverrides` (`short`, `segments` by play-order index, `pin_first_speaker`) rides on `ClipShort`; it serializes with `deny_unknown_fields` and no defaults. `LayoutOverrides::apply(LayoutEdit)` returns the inverse edit, so a history of inverses undoes and redoes an override exactly. Changing an override changes the derived project, hence the export cache key.

