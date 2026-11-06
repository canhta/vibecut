//! Integration tests share one binary and one copy of their helpers.

mod anthropic;
mod candle_alignment;
mod clips;
mod conformance;
mod elevenlabs;
mod elevenlabs_speech;
mod http_method;
mod local_gipformer;
mod local_whisper;
mod model_transport;
mod models;
mod openai;
mod openai_speech;
mod speech;
mod translation;
mod vision;

#[path = "../../../media/tests/it/support.rs"]
mod media_oracle;

#[test]
#[allow(clippy::expect_used)]
fn external_speech_fixture_conversion_uses_its_own_runtime() {
    if !media_oracle::require_oracles("external speech fixture conversion") {
        return;
    }
    let output = media_oracle::command("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.01",
            "-ar",
            "16000",
            "-ac",
            "1",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .output()
        .expect("run external speech fixture conversion");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), 160 * 4);
}
