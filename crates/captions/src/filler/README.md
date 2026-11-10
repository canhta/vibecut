# Filler detection

`detect_fillers(&CueTrack, &FillerLexicon, include_markers)` returns `FillerCandidate { id, range, cue_id, edited_cue, kind }` values in source order. It is deterministic and local: no provider, model weight or audio decode.

## Input contract

The detector accepts a cue only when its source language and the lexicon share a primary subtag, then reads the cue's source text (`Cue::source_text`) and tokenizes it with UAX #29 word boundaries, keeping segments that contain a letter or digit. Word ranges are recognition timing, so translated text is never paired with them. Tokens are NFC and lowercase; punctuation and whitespace are not words. `Cue::word_ranges` supplies one measured source range per token in source order, so a cue is used only when its token count equals its `word_ranges` length. Imported, estimated or edited cues whose counts disagree have no measured word timing and contribute no candidates; measured ranges are all-or-nothing per cue because segmentation writes them only when every contributing unit is timed.

Lexicon phrases are tokenized by the same rule at load, so `"you know"` is two tokens and `"那个"` is two Han tokens. Every phrase declares its `kind`: `filled_pause` for hesitation sounds (`um`, `ừm`, `嗯`) and `discourse_marker` for ordinary words that can also mark discourse (`like`, `you know`, `那个`). Matching is whole-word only, and the longest eligible phrase at a token wins. By default only filled pauses match; `discourse_markers` match when the caller passes `include_markers`, because a context-free marker match is usually an ordinary word ("I like it", "那个人"). Context-aware marker classification belongs to the LLM slice, not this detector.

Contiguous filled-pause words inside one cue merge into one candidate; each discourse marker stays a separate candidate because it is its own review decision, even when markers are adjacent. A candidate never crosses a cue or a word boundary. `range` starts at the first matched word's start and ends at the last matched word's end, and `kind` says which lexicon class produced it so a review UI can show markers unchecked. `id` is `filler:<cue id>:<start tick>`, stable across runs while that cue and span exist; `edited_cue` is the cue source text's `CueText::edited` flag.

## Lexicon data

`FillerLexicon::builtin(&LanguageTag)` selects a shipped file by primary subtag (`vi`, `en`, `zh`). `FillerLexicon::parse(json)` accepts caller data for any BCP 47 language, so adding a language needs no detector code change; shipping a built-in also needs one row in `BUILTIN`. Every file declares `language`, a `references` header map and entries whose `references` name keys from that map. Debug output of candidates keeps text redacted through `NfcText`.

The shipped Vietnamese file holds only hesitation evidence. `ừm`, `ờ` and `ờm` ship as filled pauses; `à` is also a question particle ("Anh đi à?"), so it ships as an opt-in discourse marker. Sentence-final particles such as `đấy`, `nhé` and `nhỉ` are meaning-bearing and are deliberately excluded; `ừ` means "yes" and is excluded too. The file header cites the corpus per entry.

A file is refused with `Error::MalformedLexicon` when JSON, the language tag, a kind, a reference, an entry reference, a duplicate phrase or a phrase token is invalid, and above 512 KiB; `Error::NoFillerLexicon` refuses a language with no shipped lexicon. Limits: 2,048 entries, 8 tokens per phrase, 128 bytes per token, 1,000,000 tokenized words per track and 100,000 candidates.

## Sources

Each lexicon file header cites its sources per entry, so translations and updates stay inside the data file.
