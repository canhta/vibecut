use ai::{Error, ExecutionHooks, Stage, models::*};
use std::io::{self, Cursor, Read};
struct Hooks(bool);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0
    }
    fn stage(&self, _: Stage) {}
}
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "small offline catalog fixture"
)]
fn fixture() -> Model {
    let mut model = catalog().unwrap().remove(0);
    model.files[0].size = 3;
    model.files[0].sha256 =
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
    model
}
#[test]
fn consent_binds_every_displayed_model_field() {
    let model = fixture();
    let consent = DownloadConsent::grant(&model).unwrap();
    consent.authorize(&model).unwrap();
    let mutations: [fn(&mut Model); 7] = [
        |m| m.id.push('x'),
        |m| m.license.push('x'),
        |m| m.source.push('x'),
        |m| m.files[0].url.push('x'),
        |m| m.files[0].size += 1,
        |m| m.files[0].sha256.replace_range(..1, "c"),
        |m| m.languages.clear(),
    ];
    for mutate in mutations {
        let mut changed = model.clone();
        mutate(&mut changed);
        assert!(consent.authorize(&changed).is_err());
    }
    assert!(!format!("{consent:?}").contains(&model.id));
    assert!(model.default_eligible());
    let mut custom = model;
    custom.license = "non-commercial".into();
    assert!(!custom.default_eligible());
    DownloadConsent::grant(&custom).unwrap(); // Still opt-in, never silently defaulted.
}
#[test]
fn verification_rejects_truncated_oversized_corrupted_and_cancelled_data() {
    let file = &fixture().files[0];
    verify_file(&mut Cursor::new(b"abc"), file, &Hooks(false)).unwrap();
    for data in [b"".as_slice(), b"ab", b"abcd", b"abd"] {
        assert!(matches!(
            verify_file(&mut Cursor::new(data), file, &Hooks(false)),
            Err(Error::ModelIntegrity)
        ));
    }
    assert!(matches!(
        verify_file(&mut Cursor::new(b"abc"), file, &Hooks(true)),
        Err(Error::Cancelled)
    ));
    struct Endless(usize);
    impl Read for Endless {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0 += b.len();
            b.fill(b'a');
            Ok(b.len())
        }
    }
    let mut endless = Endless(0);
    assert!(matches!(
        verify_file(&mut endless, file, &Hooks(false)),
        Err(Error::ModelIntegrity)
    ));
    assert_eq!(endless.0, 4);
}
#[test]
fn metadata_is_hostile_and_storage_errors_redact_paths() {
    for name in ["../model", "..", "a/b", "a\\b", "", "x:y"] {
        let mut model = fixture();
        model.files[0].name = name.into();
        assert!(DownloadConsent::grant(&model).is_err());
    }
    for url in [
        "http://example.com/a",
        "https://secret@example.com/a",
        "https://example.com/a?key=secret",
    ] {
        let mut model = fixture();
        model.files[0].url = url.into();
        assert!(model.validate().is_err());
    }
    let mut model = fixture();
    model.files.push(model.files[0].clone());
    assert!(model.validate().is_err());
    let err = storage_error(io::Error::new(io::ErrorKind::StorageFull, "/secret/path"));
    assert!(matches!(err, Error::StorageFull));
    assert!(!format!("{err:?} {err}").contains("secret"));
    let err = storage_error(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "/secret/path",
    ));
    assert!(matches!(
        err,
        Error::ModelStorage(io::ErrorKind::PermissionDenied)
    ));
}
#[test]
fn io_failure_and_mid_read_cancellation_never_verify() {
    struct Failed;
    impl Read for Failed {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::StorageFull, "sensitive"))
        }
    }
    assert!(matches!(
        verify_file(&mut Failed, &fixture().files[0], &Hooks(false)),
        Err(Error::StorageFull)
    ));
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Flag(AtomicBool);
    impl ExecutionHooks for Flag {
        fn cancelled(&self) -> bool {
            self.0.load(Ordering::Relaxed)
        }
        fn stage(&self, _: Stage) {}
    }
    struct Cancels<'a>(&'a Flag);
    impl Read for Cancels<'_> {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            self.0.0.store(true, Ordering::Relaxed);
            b[0] = b'a';
            Ok(1)
        }
    }
    let flag = Flag(AtomicBool::new(false));
    assert!(matches!(
        verify_file(&mut Cancels(&flag), &fixture().files[0], &flag),
        Err(Error::Cancelled)
    ));
}

#[test]
fn invalid_hash_size_and_missing_display_metadata_refuse_approval() {
    for hash in [
        "",
        "abcd",
        "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD",
        "zz7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
    ] {
        let mut model = fixture();
        model.files[0].sha256 = hash.into();
        assert!(DownloadConsent::grant(&model).is_err());
    }
    let mutations: [fn(&mut Model); 5] = [
        |m| m.files[0].size = 0,
        |m| m.id.clear(),
        |m| m.license.clear(),
        |m| m.files.clear(),
        |m| m.license_url = "file:///private/licence".into(),
    ];
    for mutate in mutations {
        let mut model = fixture();
        mutate(&mut model);
        assert!(DownloadConsent::grant(&model).is_err());
    }
}

#[test]
fn local_catalog_suggestions_keep_download_consent_and_provider_choice_explicit() {
    let vi = suggested_transcriber(&captions::LanguageTag::parse("vi").unwrap()).unwrap();
    assert_eq!(vi.id, "gipformer-68m-rnnt-int8");
    assert_eq!(vi.license, "MIT");
    assert_eq!(vi.files.len(), 4);
    assert!(vi.default_eligible());
    let consent = DownloadConsent::grant(&vi).unwrap();
    consent.authorize(&vi).unwrap();
    for language in ["en", "zh", "vi-VN"] {
        let model =
            suggested_transcriber(&captions::LanguageTag::parse(language).unwrap()).unwrap();
        assert_eq!(model.id, "whisper-large-v3-turbo");
        assert!(consent.authorize(&model).is_err());
    }
    for model in catalog().unwrap() {
        model.validate().unwrap();
    }
}
