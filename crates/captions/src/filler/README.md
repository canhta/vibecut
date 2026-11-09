# Filler detection

`detect_fillers(&CueTrack, &FillerLexicon)` returns `FillerCandidate { id, range, cue_id, edited_cue }` values in source order. It is deterministic and local: no provider, model weight or audio decode.

## Input contract

The detector reads the cue text for the lexicon's language (exact BCP 47 tag, else the first tag with the same primary subtag) and tokenizes it with UAX #29 word boundaries, keeping segments that contain a letter or digit. A cue contributes only when that text shares the primary subtag of `source_lang`: word ranges are recognition timing, so translated text is never paired with them. Tokens are NFC and lowercase; punctuation and whitespace are not words. `Cue::word_ranges` supplies one measured source range per token in source order, so a cue is used only when its token count equals its `word_ranges` length. Imported, estimated or edited cues whose counts disagree have no measured word timing and contribute no candidates; measured ranges are all-or-nothing per cue because segmentation writes them only when every contributing unit is timed.

Lexicon phrases are tokenized by the same rule at load, so `"you know"` is two tokens and `"那个"` is two Han tokens. The longest matching phrase at a token wins, phrases never overlap, and matching is whole-word only: `like` never matches inside `likely`. Contiguous matched words inside one cue merge into one candidate; a candidate never crosses a cue or a word boundary. `range` starts at the first matched word's start and ends at the last matched word's end. `id` is `filler:<cue id>:<start tick>`, stable across runs while that cue and span exist; `edited_cue` is the detected language's `CueText::edited` flag.

## Lexicon data

`FillerLexicon::builtin(&LanguageTag)` selects a shipped file by primary subtag (`vi`, `en`, `zh`) and keeps the full requested tag for cue lookup. `FillerLexicon::parse(json)` accepts caller data for any BCP 47 language, so adding a language needs no detector code change; shipping a built-in also needs one row in `BUILTIN`. Every file declares `language`, a `references` header map and entries whose `references` name keys from that map. Debug output of candidates keeps text redacted through `NfcText`.

A file is refused with `Error::MalformedLexicon` when JSON, the language tag, a reference, an entry reference, a duplicate phrase or a phrase token is invalid, and above 512 KiB; `Error::NoFillerLexicon` refuses a language with no shipped lexicon. Limits: 2,048 entries, 8 tokens per phrase, 128 bytes per token, 1,000,000 tokenized words per track and 100,000 candidates.

## Sources

Each lexicon file header cites its sources per entry, so translations and updates stay inside the data file.
