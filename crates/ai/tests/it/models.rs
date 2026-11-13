use ai::{Error, ExecutionHooks, Stage, foundation::models::*};
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
fn language_neutral_metadata_never_matches_a_speech_language() {
    let catalog = catalog().unwrap();
    let neutral: Vec<_> = catalog
        .iter()
        .filter(|model| model.language_neutral())
        .collect();
    assert_eq!(
        neutral
            .iter()
            .map(|model| model.id.as_str())
            .collect::<Vec<_>>(),
        [
            "sface-2021dec",
            "yunet-2026may",
            "pphumanseg-2023mar",
            "clip-vit-base-patch32"
        ]
    );
    for model in neutral {
        assert!(model.languages.is_empty());
        assert!(model.default_eligible());
        DownloadConsent::grant(model).unwrap();
        for language in ["vi", "en", "zh", "zh-Hans", "vi-VN"] {
            let language = captions::LanguageTag::parse(language).unwrap();
            assert!(!model.matches_language(&language));
            let suggested = suggested_transcriber(&language).unwrap();
            assert!(!suggested.language_neutral());
            assert!(suggested.matches_language(&language));
        }
    }
}

#[test]
fn existing_language_bound_catalog_entries_are_unchanged() {
    let catalog = catalog().unwrap();
    let languages = |id: &str| {
        catalog
            .iter()
            .find(|model| model.id == id)
            .unwrap()
            .languages
            .iter()
            .map(|tag| tag.as_str().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(languages("whisper-large-v3-turbo"), ["vi", "en", "zh"]);
    assert_eq!(languages("gipformer-68m-rnnt-int8"), ["vi"]);
    assert_eq!(
        languages("whisper-small-dtw"),
        ["vi", "en", "zh", "zh-Hans", "zh-Hant"]
    );
    assert!(
        suggested_transcriber(&captions::LanguageTag::parse("vi-VN").unwrap())
            .unwrap()
            .matches_language(&captions::LanguageTag::parse("vi-VN").unwrap())
    );
}

#[test]
fn vision_catalog_entries_pin_exact_upstream_files_and_refuse_bad_bytes() {
    let catalog = catalog().unwrap();
    let model = |id: &str| catalog.iter().find(|model| model.id == id).unwrap();
    let yunet = model("yunet-2026may");
    assert_eq!(yunet.files.len(), 1);
    assert_eq!(yunet.files[0].size, 229_738);
    assert_eq!(
        yunet.files[0].sha256,
        "ebafce4e3c118d6554634be5c27ab333b4c047a9a8c3faf1d7cf93101c22f0f0"
    );
    let clip = model("clip-vit-base-patch32");
    assert_eq!(clip.files.len(), 5);
    assert_eq!(
        clip.files.iter().map(|file| file.size).sum::<u64>(),
        607_973_425
    );
    let person = model("pphumanseg-2023mar");
    assert_eq!(person.license, "Apache-2.0");
    assert!(
        person
            .license_url
            .ends_with("/human_segmentation_pphumanseg/LICENSE")
    );
    assert_eq!(person.files.len(), 1);
    assert_eq!(
        person.files[0].name,
        "human_segmentation_pphumanseg_2023mar.onnx"
    );
    assert_eq!(person.files[0].size, 6_163_938);
    assert_eq!(
        person.files[0].sha256,
        "552d8a984054e59b5d773d24b9b12022b22046ceb2bbc4c9aaeacec36a9ddf24"
    );
    for model in [yunet, clip, person] {
        assert!(model.language_neutral());
        assert!(model.default_eligible());
        for file in &model.files {
            assert!(matches!(
                verify_file(&mut Cursor::new(Vec::new()), file, &Hooks(false)),
                Err(Error::ModelIntegrity)
            ));
            let mut mismatched = file.clone();
            mismatched.size = 3;
            mismatched.sha256 =
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
            assert!(matches!(
                verify_file(&mut Cursor::new(b"abd"), &mismatched, &Hooks(false)),
                Err(Error::ModelIntegrity)
            ));
        }
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
