# Crop

Owns the transient source-normalized crop interaction; project owns the crop
contract and render consumes it.

The toolbar opens one dialog for the selected video or image clip. The dialog
edits a transient `project::Crop` through the control in this folder, locked to
the selected ratio (Custom resizes freely, Original keeps the source shape), and
writes one `timeline.set_item_param` patch on Apply, so closing or Escape never
mutates the project. `portrait` returns the largest centred 9:16 rectangle a
source can show.
