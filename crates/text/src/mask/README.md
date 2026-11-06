# Mask coverage

`coverage_signed_distance(width, height, coverage, cancelled)` shares the outline
Euclidean distance transform with render's text masks. It accepts exact, finite
0..=1 glyph coverage, returns negative-inside distances in output pixels, and
preserves fractional edge coverage. Dimensions admit at most 64 Mi pixels;
invalid length/values, allocation failure and cancellation return typed errors.
Cancellation is checked per input row and transform row/column. A budgeted caller
reserves 16 bytes/pixel plus 20 bytes per longer axis plus 4 bytes for temporary
and returned distance buffers; the input working image is separately owned.
