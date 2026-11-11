//! Defines native argument interpretation at the CLI boundary.
mod errors;
mod parsing;
pub use errors::CliError;
pub(crate) use parsing::{
    HELP, number_arg, one_path, one_utf8, parse_f64, parse_i64, parse_selection, parse_u32,
    parse_u64, seconds_arg, seconds_ticks, translation_required, utf8,
};
#[cfg(test)]
mod exit_codes {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use std::io;
    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(CliError::Usage("".into()).exit_code(), 2);
        assert_eq!(CliError::Project(project::Error::Busy).exit_code(), 3);
        assert_eq!(CliError::Project(project::Error::Conflict).exit_code(), 3);
        assert_eq!(
            CliError::Render(render::Error::Unavailable(Vec::new())).exit_code(),
            4
        );
        assert_eq!(CliError::Rejected("").exit_code(), 5);
        assert_eq!(CliError::Render(render::Error::Cancelled).exit_code(), 6);
        assert_eq!(
            CliError::Media(media::MediaError::Invalid("")).exit_code(),
            6
        );
        assert_eq!(CliError::Io(io::Error::other("")).exit_code(), 7);
    }
    #[test]
    fn engine_and_sound_diagnostics_keep_exact_labels_and_exits() {
        use engine::{EditError as E, SoundError as S, providers::Error as P};
        use std::sync::Arc;
        type Case = (fn() -> E, (&'static str, u8));
        macro_rules! row {
            ($e:expr, $l:literal, $x:literal) => {
                (|| $e, ($l, $x))
            };
        }
        let cases: &[Case] = &[
            row!(E::Runtime(engine::RuntimeError::NetworkTransient), "job", 7),
            row!(E::Runtime(engine::RuntimeError::CacheCorrupt), "job", 7),
            row!(E::SchemaMismatch, "rejected", 5),
            row!(E::SessionGone, "rejected", 5),
            row!(E::StaleRevision, "rejected", 5),
            row!(E::Permission, "rejected", 5),
            row!(E::IdempotencyConflict, "rejected", 5),
            row!(E::ReceiptCapacity, "rejected", 5),
            row!(E::ResyncRequired, "rejected", 5),
            row!(E::JobNotRetained, "rejected", 5),
            row!(E::ControlIdentity, "rejected", 5),
            row!(E::Runtime(engine::RuntimeError::QueueFull), "job", 7),
            row!(E::Runtime(engine::RuntimeError::BudgetFull), "job", 7),
            row!(E::Runtime(engine::RuntimeError::ArtifactTooLarge), "job", 7),
            row!(
                E::Runtime(engine::RuntimeError::Io(std::io::Error::other("eviction"))),
                "io",
                7
            ),
            row!(E::Runtime(engine::RuntimeError::Limit), "usage", 2),
            row!(
                E::Runtime(engine::RuntimeError::Invalid("model accounting owner")),
                "rejected",
                5
            ),
            row!(E::Cancelled, "job", 7),
            row!(E::Runtime(engine::RuntimeError::WorkerPanicked), "job", 7),
            row!(E::Runtime(engine::RuntimeError::Poisoned), "job", 7),
            row!(E::StaleResult, "job", 7),
            row!(
                E::Runtime(engine::RuntimeError::Failed(Arc::new(E::Limit))),
                "usage",
                2
            ),
            row!(E::Render(render::Error::Cancelled), "render", 6),
            row!(E::Media(media::MediaError::Cancelled), "media", 6),
            row!(E::Ai(ai::Error::Auth), "ai", 4),
            row!(E::Sounds(S::NoCredentials), "setup", 4),
            row!(E::Io(io::Error::other("io")), "io", 7),
            row!(E::UnknownCommand, "usage", 2),
            row!(E::InvalidParams, "usage", 2),
            row!(E::EmptyProject, "rejected", 5),
            row!(E::Panicked, "rejected", 5),
            row!(E::TransactionActive, "rejected", 5),
            row!(E::Project(project::Error::Busy), "busy", 3),
            row!(E::Sounds(S::Auth), "auth", 4),
            row!(E::Sounds(S::Quota), "quota", 4),
            row!(
                E::Sounds(S::RateLimit {
                    retry_after_seconds: Some(3)
                }),
                "rate_limit",
                4
            ),
            row!(E::Sounds(S::Network), "network", 7),
            row!(E::Sounds(S::InvalidResponse), "sounds", 5),
            row!(E::Sounds(S::Limit), "limit", 2),
            row!(E::Sounds(S::NoAudioDevice), "audio_device", 4),
            row!(E::Sounds(S::Cancelled), "cancelled", 7),
            row!(E::Sounds(S::Io(io::Error::other("io"))), "io", 7),
            row!(
                E::Render(render::Error::Unavailable(Vec::new())),
                "unavailable",
                4
            ),
            row!(E::Project(project::Error::Conflict), "conflict", 3),
        ];
        for (make, expected) in cases {
            for error in [
                CliError::Engine(make()),
                CliError::Job(Arc::new(make())),
                CliError::Shorts(shorts::Error::Engine(make())),
            ] {
                assert_eq!(error.code(), *expected, "{error:?}");
                let value = error.value();
                assert_eq!(value["code"], expected.0);
                assert_eq!(value["exit_code"], expected.1);
            }
        }
        let sounds = [
            (S::NoCredentials, ("setup", 4)),
            (S::Auth, ("auth", 4)),
            (S::Quota, ("quota", 4)),
            (
                S::RateLimit {
                    retry_after_seconds: Some(3),
                },
                ("rate_limit", 4),
            ),
            (S::Network, ("network", 7)),
            (S::Server, ("network", 7)),
            (S::InvalidResponse, ("sounds", 5)),
            (S::Limit, ("limit", 2)),
            (S::NoAudioDevice, ("audio_device", 4)),
            (S::NotCached, ("sounds", 5)),
            (S::LibraryCorrupt, ("sounds", 5)),
            (S::Cancelled, ("cancelled", 7)),
            (S::Io(io::Error::other("io")), ("io", 7)),
            (S::Media(media::MediaError::Cancelled), ("media", 6)),
            (S::Project(project::Error::Busy), ("io", 7)),
            (S::Invalid("input"), ("sounds", 5)),
        ];
        for (error, expected) in sounds {
            assert_eq!(CliError::Sounds(error).code(), expected);
        }
        for (make, expected) in cases {
            let inner = E::Runtime(engine::RuntimeError::Failed(Arc::new(E::Runtime(
                engine::RuntimeError::Failed(Arc::new(make())),
            ))));
            assert_eq!(CliError::Job(Arc::new(inner)).code(), *expected);
        }
        let messages: [(fn() -> E, &str); 4] = [
            (
                || E::Runtime(engine::RuntimeError::NetworkTransient),
                "transient network failure",
            ),
            (
                || E::Runtime(engine::RuntimeError::CacheCorrupt),
                "corrupt cache entry skipped",
            ),
            (
                || E::Runtime(engine::RuntimeError::QueueFull),
                "job queue is full",
            ),
            (
                || E::Runtime(engine::RuntimeError::WorkerPanicked),
                "job worker unwound",
            ),
        ];
        for (make, message) in messages {
            for depth in 0..=2 {
                let mut cause = make();
                for _ in 0..depth {
                    cause = E::Runtime(engine::RuntimeError::Failed(Arc::new(cause)));
                }
                let error = CliError::Job(Arc::new(cause));
                assert_eq!(error.value()["message"], message);
                assert_eq!(error.code(), ("job", 7));
            }
        }
        let model = E::Runtime(engine::RuntimeError::Failed(Arc::new(E::Providers(
            P::ModelMissing("test-model"),
        ))));
        assert_eq!(
            CliError::Engine(model).value()["message"],
            "local model test-model is not installed; run `vibecut-cli model download test-model`"
        );
        for depth in 0..=2 {
            let mut model = E::Providers(P::ModelMissing("test-model"));
            for _ in 0..depth {
                model = E::Runtime(engine::RuntimeError::Failed(Arc::new(model)));
            }
            let error = CliError::Job(Arc::new(model));
            assert_eq!(error.code(), ("setup", 4));
            assert_eq!(
                error.value()["message"],
                "local model test-model is not installed; run `vibecut-cli model download test-model`"
            );
        }
        let concrete = CliError::Job(Arc::new(engine::RuntimeError::Failed(Arc::new(
            E::Providers(P::ModelMissing("test-model")),
        ))));
        let network = CliError::Job(Arc::new(engine::RuntimeError::NetworkTransient));
        assert_eq!(network.code(), ("job", 7));
        assert_eq!(network.value()["message"], "transient network failure");
        let wrapped = CliError::Job(Arc::new(engine::RuntimeError::Failed(Arc::new(
            engine::RuntimeError::NetworkTransient,
        ))));
        assert_eq!(wrapped.code(), ("job", 7));
        assert_eq!(wrapped.value()["message"], "transient network failure");
        let cache = CliError::Job(Arc::new(engine::RuntimeError::CacheCorrupt));
        assert_eq!(cache.code(), ("job", 7));
        assert_eq!(cache.value()["message"], "corrupt cache entry skipped");
        assert_eq!(
            concrete.value()["message"],
            "local model test-model is not installed; run `vibecut-cli model download test-model`"
        );
        let persistence = CliError::Engine(
            engine::recovery::Error::PersistenceFailed(Arc::new(E::Io(io::Error::other("disk"))))
                .into(),
        );
        assert_eq!(persistence.code(), ("io", 7));
        assert_eq!(persistence.value()["message"], "session persistence failed");
    }
}

#[cfg(test)]
mod translation {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use engine::subtitles::LanguageTag;
    #[test]
    fn translation_is_needed_only_for_different_languages_or_bilingual() {
        let vi = LanguageTag::parse("vi").unwrap();
        let en = LanguageTag::parse("en").unwrap();
        assert!(!translation_required(&vi, &vi, false));
        assert!(translation_required(&vi, &en, false));
        assert!(translation_required(&vi, &vi, true));
    }
}
