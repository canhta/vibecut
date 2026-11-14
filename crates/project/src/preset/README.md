# Built-in looks

Closed catalogues of built-in text looks (`TextLook`) and caption looks (`CaptionLook`), each with its shelf, our own display name and the complete style value it copies. They are data only: `engine` applies a look as one undo step, and project JSON stores only the style value a look writes, never the look itself.

Wire ids are stable snake_case. Looks use only the curated OFL families that cover Vietnamese. A caption look materializes into a cue style through `CaptionLook::cue_style`, which keeps its max lines and word-highlight colour on the cue style so karaoke looks keep their highlight. An imported ASS cue style leaves both absent and follows the project caption setting.
