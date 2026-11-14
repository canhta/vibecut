# Built-in looks

Closed catalogues of built-in text looks (`TextLook`) and caption looks (`CaptionLook`), each with its shelf, our own display name and the complete style value it copies. They are data only: `engine` applies a look as one undo step, and project JSON stores only the style value a look writes, never the look itself.

A `TextTemplate` is a text look with its own sample words and vertical position, inserted as one new text item. Looks and templates carry the licence of their curated OFL families (`TEXT_LIBRARY_LICENSE`); `is_commercial_license` is the Commercial filter's test, and `search_text_looks` and `search_text_templates` filter by name, shelf and that licence. Their ids are the snake_case of the display name.

Wire ids are stable snake_case. Looks use only the curated OFL families that cover Vietnamese. A caption look materializes into a cue style through `CaptionLook::cue_style`, which keeps its max lines and word-highlight colour on the cue style so karaoke looks keep their highlight. An imported ASS cue style leaves both absent and follows the project caption setting.
