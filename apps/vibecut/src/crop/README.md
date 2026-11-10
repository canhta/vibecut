# Crop

The source-normalized crop model and control; project owns the crop contract and render consumes it. The editor's crop dialog lives in the player panel and writes one `timeline.set_item_param` patch on Apply.

`portrait` returns the largest centred 9:16 rectangle a source can show and refuses zero or non-finite source dimensions. `control` drags the rectangle inside the normalized bounds: the pointer moves it from the inside or resizes it from an edge or corner, a ratio lock keeps the width/height shape in source pixels (corners anchor the opposite corner, edges grow the other axis around its centre) and `None` resizes freely. An edge never collapses below 5% of the source. Arrow keys move by 1% and Shift by 10%; the caller keeps the gesture-entry rectangle and commits one undo step on drag stop, and Escape is the caller's rollback. The control paints the source frame under the box when the caller has a texture and reports label, role and focus to assistive technology.
