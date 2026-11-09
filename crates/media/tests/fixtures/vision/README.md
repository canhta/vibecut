# Vision preparation fixtures

`face_crop_reference.bin` is a 112x112 raw grayscale crop from the reference recipe on the synthetic 128x96 RGBA pattern of the `face_crop_matches_reference_recipe_with_padding` test. Face box `(84, 34, 32, 32)`, expansion `0.40`, missing pixels padded with gray 110, BGR-to-grayscale, `cv2.resize(..., (224, 224), INTER_LINEAR)`, centre crop `[56..168, 56..168]`. The test allows two gray levels. No private excerpt pixels are stored.
