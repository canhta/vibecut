# Properties view

Interface: `show`.

Renders the properties tab strip and its collapsible rows: appearance, background (a video
clip's kind, colour and blur), canvas, colour and font dialogs, effects, numeric fields,
playback (audio volume and fades, speed), remove background, selection and text style. The
effects folder adds the Adjust tab's colour wheels and tone-curve editor; the
[basic](basic/README.md) folder holds the Basic group's background, remove background and
mask sub-tabs. The [text style](text_style/README.md) folder holds a caption item's
Keywords section. It reads the
area's state and returns `Action` values; it writes no project and keeps no state of its
own.
