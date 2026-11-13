# Dubbing

Interface: `Narrator`, `SpeechOpener`, `Request`, `Prepared`, `Disclosure`, `CancelFlag`, `DubTake`, `encode_take`, `decode_take`, `write_wav`, `wav_bytes`, `ticks_of`, `fit`, `Fit`, `FitInput`, `DurationFit`, `MAX_TAKE_BYTES`.

Narration is an ordinary audio take generated from text with the provider, model and voice the user chose. There is no cloning, no bundled weights, no automatic provider, model or voice switch and no separate dubbing mixer. Provider adapters, `SynthesisConsent` and cost live in [`ai::speech`](../../../ai/src/speech/README.md); this folder composes them.

## Request, consent and run

`Narrator::prepare(Request)` asks the lent `SpeechOpener` for the exact provider and model (an unknown pair is `ai::Error::Unsupported`, never a substitute), builds the `ai::speech::SpeechRequest` and checks the adapter's capabilities and quote with no IO. An unsupported language or voice (`UnsupportedLanguage`, `UnsupportedVoice`), an over-limit text or an unusable cue refuses here, before any consent. `Prepared::disclosure` is what the consent sheet shows: provider, model, voice, language and script, grapheme count, a text preview, the text SHA-256, the provider speed when set, the policy link, retention, training, the AI-generated-voice statement, the upper-bound cost and the adapter fingerprint. `Prepared::upload` is the same data as the shared `providers::UploadRequest` (operation `SynthesizeSpeech`), so the existing consent prompt and its secret path are the only ones.

`Prepared::approve` asks the prompt (`approve_unknown_price` when the quote is unknown) and binds the answer into `SynthesisConsent`; any change to text, voice, speed or quote needs a new approval. `Prepared::synthesize` is one attempt through `ai::speech::synthesize`: no retry (the job kind is not in the network retry list, so a token is never reused), no fallback, no partial audio. Cancellation, a provider failure, a rejected transcript or a cost above the approved bound discards the whole result and leaves the project untouched. The result is a `DubTake`: mono PCM16 plus a `project::Narration` carrying provider, model, voice, language, fingerprint, text hash, consent descriptor and cue duration.

`control::DubWorkflow` is how `dub.generate` reaches a `Narrator`: it is lent through `CaptionAccess::with_dubbing`, asks the prompt of that access and submits one `JobKind::Synthesize` job whose output is the encoded take. `dub.cancel` cancels that job. A desktop dialog that runs the same `prepare`/`approve`/`synthesize` on its own thread uses `CancelFlag` instead of a job.

## Fit

`fit` is a pure integer decision from the take's natural duration, the cue duration, the silent time after the cue, the character count, whether the provider offers a speed setting and whether a rephrase was already requested. The speed the cue asks for is `natural / cue`:

| Needed speed | Result |
|---|---|
| exactly 1.000 | `Exact` |
| below 0.900 | `Gap`: the take plays at 1.0 and leaves a gap |
| 0.900 to 1.100 | `Stretch`: the take is stretched through `timeline.set_speed` (the pitch-preserving retime path; there is no second stretch path) |
| above 1.100, provider speed unused and reachable (at most 1.2 with the remaining stretch inside 1.1) | `ProviderSpeed`: regenerate with the shown, fingerprinted speed, then fit again |
| above 1.100, no rephrase yet | `Rephrase`: one shorter text of at most `characters * cue * 1.1 / natural` characters |
| above 1.100 after a rephrase | `Overflow` at 1.100 when the result is at most 1.5 times the cue and the time after the cue is silent; otherwise `DurationFit` |

Audio is never cut and the limits are never exceeded silently.

## Placement and ducking

`dub.place` places the take as an ordinary audio item (a `project::AudioSource::Narration` record) with its asset in one undo step: asset, item, stretch and ducking commit together. Repeating the same cue places further items; the cue stays a source cue and EditMapping keeps owning where it lands. `dub.set_fit` changes the stretch of a placed take and refuses outside 0.900 to 1.100 or past the overflow rule with `EditError::DurationFit`. `dub.set_ducking` stores the ducking record and rewrites the keys it owns. The rules live in [the dubbing edit](../session/edit/README.md#dubbing).
