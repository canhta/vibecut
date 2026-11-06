# Language profiles

`LanguageRegistry::standard` provides editable profiles for vi, en, bare zh, zh-Hans and
zh-Hant. Bare zh uses the Han-character policy as well; no locale fallback to
English word timing is implied. Lookup tries progressively shorter tag prefixes then the generic profile.
The profile's `timing_unit` selects preferred source grouping: English/generic
words, Vietnamese syllables, and Han characters. UAX #29 keeps mixed Latin and
full-width letter runs together while separating Han; dictionary-protected spans
still constrain cue breaks. Profiles select policy without language branches.
Limits are grapheme counts/rates, not byte counts; policies and font preferences
are initial defaults, not measured reading-speed or font-coverage guarantees.
Defaults use adult interlingual guidelines: [Vietnamese](https://partnerhelp.netflixstudios.com/hc/en-us/articles/220447048-Vietnamese-Timed-Text-Style-Guide)
42 graphemes/line and 17 CPS; [English](https://partnerhelp.netflixstudios.com/hc/en-us/articles/217350977-English-USA-Timed-Text-Style-Guide)
42/20; [Simplified Chinese](https://partnerhelp.netflixstudios.com/hc/en-us/articles/215986007-Chinese-Simplified-Timed-Text-Style-Guide)
and [Traditional Chinese](https://partnerhelp.netflixstudios.com/hc/en-us/articles/215994807-Chinese-Traditional-Timed-Text-Style-Guide)
16/9. [General requirements](https://partnerhelp.netflixstudios.com/hc/en-us/articles/215758617-Timed-Text-Style-Guide-General-Requirements)
supply two lines and 5/6..7 seconds; [timing guidelines](https://partnerhelp.netflixstudios.com/hc/en-us/articles/360051554394-Timed-Text-Style-Guide-Subtitle-Timing-Guidelines)
supply the two-frame gap. These are editable defaults, not a certification of
Netflix delivery compliance: our counts use graphemes, and no shot analysis,
linguistic reduction or children's/SDH preset is implied.
