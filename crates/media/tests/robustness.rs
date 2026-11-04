#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod common;
use media::{AssetId, MediaError, PacketReader, SourceAsset, probe};
use std::io::{Seek, SeekFrom, Write};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn source() -> Option<std::path::PathBuf> {
    static SOURCE: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    SOURCE
        .get_or_init(|| {
            common::generate(
                "hostile input seed",
                "ffmpeg_hostile_seed.mp4",
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=64x48:rate=25:duration=0.2",
                    "-c:v",
                    "libx264",
                    "-movflags",
                    "+faststart",
                    "-an",
                ],
            )
        })
        .clone()
}
#[test]
fn native_unicode_paths_cancellation_and_sparse_streaming() {
    let Some(source) = source() else {
        return;
    };
    let dir = common::fixtures_dir().join("thư mục 输出");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("clip có dấu.mp4");
    std::fs::copy(&source, &path).unwrap();
    let cancel = Arc::new(AtomicBool::new(true));
    assert!(matches!(
        PacketReader::open_with_cancel(&path, cancel.clone()),
        Err(MediaError::Cancelled)
    ));
    cancel.store(false, Ordering::Relaxed);
    let mut reader = PacketReader::open_with_cancel(&path, cancel.clone()).unwrap();
    assert!(reader.next_packet().unwrap().is_some());
    cancel.store(true, Ordering::Relaxed);
    assert!(matches!(reader.next_packet(), Err(MediaError::Cancelled)));
    let sparse = dir.join("sparse.mp4");
    std::fs::copy(source, &sparse).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&sparse)
        .unwrap();
    let original = file.seek(SeekFrom::End(0)).unwrap();
    let padding = 2_500_000_000u32;
    file.write_all(&padding.to_be_bytes()).unwrap();
    file.write_all(b"free").unwrap();
    file.set_len(original + u64::from(padding)).unwrap();
    drop(file);
    let mut reader = PacketReader::open(&sparse).unwrap();
    assert!(reader.next_packet().unwrap().is_some());
    assert!(
        reader.bytes_read() < 1 << 20,
        "probe read {} bytes",
        reader.bytes_read()
    );
    println!(
        "sparse file {} bytes; read {} bytes",
        original + u64::from(padding),
        reader.bytes_read()
    );
    drop(reader);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn truncated_and_mutated_inputs_return_without_rust_panics() {
    let Some(source) = source() else {
        return;
    };
    let original = std::fs::read(source).unwrap();
    let path = common::fixtures_dir().join("ffmpeg_hostile.bin");
    for n in (0..original.len()).step_by((original.len() / 80).max(1)) {
        std::fs::write(&path, &original[..n]).unwrap();
        assert!(
            std::panic::catch_unwind(|| {
                let _ = probe(&path);
            })
            .is_ok()
        );
    }
    let mut state = 0x5eed_u64;
    for _ in 0..120 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut bytes = original.clone();
        let i = state as usize % bytes.len();
        bytes[i] ^= 1 << ((state >> 32) % 8);
        std::fs::write(&path, bytes).unwrap();
        assert!(
            std::panic::catch_unwind(|| {
                if let Ok(mut reader) = PacketReader::open(&path) {
                    for _ in 0..100 {
                        if !matches!(reader.next_packet(), Ok(Some(_))) {
                            break;
                        }
                    }
                }
            })
            .is_ok()
        );
    }
    std::fs::write(&path, []).unwrap();
    assert!(probe(&path).is_err());
    assert!(matches!(
        probe("missing-file-00b.mp4"),
        Err(MediaError::Io(_))
    ));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn hostile_pssh_is_bounded_on_small_worker_stack() {
    // Run native parsing in a subprocess: a stack abort must fail this test,
    // rather than take down the rest of the regression suite.
    const CHILD: &str = "VIBECUT_PSSH_CHILD";
    if let Some(path) = std::env::var_os(CHILD) {
        let thread = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || {
                assert!(probe(path).is_ok());
            })
            .unwrap();
        thread.join().unwrap();
        let path = std::env::var_os(CHILD).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let token = cancel.clone();
        let trigger = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1));
            token.store(true, Ordering::Relaxed);
        });
        let result = PacketReader::open_with_cancel(path, cancel.clone());
        trigger.join().unwrap();
        match result {
            Err(MediaError::Cancelled) => {}
            Ok(mut reader) => assert!(matches!(reader.next_packet(), Err(MediaError::Cancelled))),
            Err(error) => panic!("unexpected cancellation failure: {error}"),
        }
        return;
    }
    let Some(seed) = source() else {
        return;
    };
    let mut bytes = std::fs::read(seed).unwrap();
    let mut index = 0;
    while &bytes[index + 4..index + 8] != b"moov" {
        index += u32::from_be_bytes(bytes[index..index + 4].try_into().unwrap()) as usize;
    }
    let size = u32::from_be_bytes(bytes[index..index + 4].try_into().unwrap()) as usize;
    let mut box_bytes = 32u32.to_be_bytes().to_vec();
    box_bytes.extend_from_slice(b"pssh");
    box_bytes.extend_from_slice(&[0; 24]);
    let addition = box_bytes.repeat(20_000);
    bytes[index..index + 4].copy_from_slice(&((size + addition.len()) as u32).to_be_bytes());
    bytes.splice(index + size..index + size, addition);
    let path = common::fixtures_dir().join("hostile-20000-pssh.mp4");
    std::fs::write(&path, bytes).unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "hostile_pssh_is_bounded_on_small_worker_stack",
            "--nocapture",
        ])
        .env(CHILD, &path)
        .spawn()
        .unwrap();
    let start = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "hostile PSSH child: {status}");
            println!(
                "20,000 PSSH boxes, 512 KiB stack and active cancellation: {:?}",
                start.elapsed()
            );
            break;
        }
        if start.elapsed() > std::time::Duration::from_secs(5) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("hostile PSSH probe exceeded five seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let cancel = Arc::new(AtomicBool::new(true));
    assert!(matches!(
        PacketReader::open_with_cancel(&path, cancel),
        Err(MediaError::Cancelled)
    ));
}

#[test]
fn source_import_honors_pre_cancelled_token() {
    let Some(source) = source() else {
        return;
    };
    let cancel = Arc::new(AtomicBool::new(true));
    assert!(matches!(
        SourceAsset::import_with_cancel(AssetId(1), source.clone(), cancel.clone()),
        Err(MediaError::Cancelled)
    ));
    cancel.store(false, Ordering::Relaxed);
    let asset = SourceAsset::import_with_cancel(AssetId(1), source, cancel).unwrap();
    assert_eq!(asset.id(), AssetId(1));
    assert!(!asset.probe().streams.is_empty());
}
