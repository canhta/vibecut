# Built-in looks

Closed catalogues of built-in text looks (`TextLook`) and caption looks (`CaptionLook`), each with its shelf, our own display name and the complete style value it copies. They are data only: `engine` applies a look as one undo step, and project JSON stores only the style value a look writes, never the look itself.

Wire ids are stable snake_case. Looks use only the curated OFL families that cover Vietnamese. A caption look materializes into a cue style through `CaptionLook::cue_style`; cues carry no max-lines or word-highlight colour, so those parts of a look are not kept on the cue.
