# Keyword emphasis

`select_lexicon(&Cue, Option<&EmphasisLexicon>)` returns up to three word ids for one cue, deterministically and locally: no provider, model weight or network. An id is an index into the cue's verified `CueWordTiming::Words` records, so a selection is always a subset of timed words. `validate_selection` and `resolve_selection` check a stored selection against the cue's current words; render reads emphasis only through `resolve_selection`, so ids that outlived their words paint nothing.

## Input contract

A cue is eligible only when its `word_timing` is `Words`, has records, and its `text` equals the cue's current source text (`Cue::source_text`). Anything else refuses with `Error::NoWordTiming`; nothing is estimated from text. Translations are never read.

## Selection

A cue of `n` timed words may emphasise `(n + 2) / 4` words, at most `MAX_EMPHASIS_WORDS` (3): none for one word, one for two to five, two for six to nine, three from ten. Each word is normalized (NFC, lowercase, edge punctuation removed) and skipped when it is a stopword. The rest score by length in characters (capped at 12), plus 20 for a digit, plus 15 for an all-capital word of two or more letters, or plus 10 for a leading capital that is not the first word. The highest scores win, ties keep the earlier word, and ids come back ascending. A language without data uses no stopwords.

## Lexicon data

`EmphasisLexicon::builtin(&LanguageTag)` selects `data/emphasis/{en,vi,zh}.json` by primary subtag. Each file is `{language, source, stopwords}`; `source` must be non-empty. The shipped lists are the single-token entries of the stopwords-iso collections for each language (MIT licence): https://github.com/stopwords-iso/stopwords-en, `-vi` and `-zh`. Vietnamese entries that span several syllables never match because cue words are single tokens. `parse` accepts caller data for any language up to 512 KiB.
