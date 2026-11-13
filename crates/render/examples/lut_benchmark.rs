//! Measures the CPU imported-look path against an ungraded 1080p frame.
use render::{ExecutionHooks, MemoryKind, Reservation};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
#[derive(Default)]
struct Hooks {
    used: Arc<AtomicU64>,
    peak: AtomicU64,
}
struct Lease(Arc<AtomicU64>, u64);
impl Reservation for Lease {}
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.fetch_sub(self.1, Ordering::SeqCst);
    }
}
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(&self, _: MemoryKind, bytes: u64) -> render::Result<Box<dyn Reservation>> {
        let used = self.used.fetch_add(bytes, Ordering::SeqCst) + bytes;
        if used > 512 << 20 {
            self.used.fetch_sub(bytes, Ordering::SeqCst);
            return Err(render::Error::Budget {
                requested: bytes,
                limit: 512 << 20,
            });
        }
        self.peak.fetch_max(used, Ordering::SeqCst);
        Ok(Box::new(Lease(self.used.clone(), bytes)))
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "pid={} os={} arch={} profile={} canvas=1920x1080 repetitions=2 filesystem_cache=uncontrolled",
        std::process::id(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    let directory = tempfile::tempdir()?;
    for (label, size, identity) in [
        ("no_lut", 0, false),
        ("identity", 17, true),
        ("cube17", 17, false),
        ("cube33", 33, false),
        ("cube65", 65, false),
    ] {
        let hooks = Hooks::default();
        let mut data =
            project::Project::empty(project::ProjectId(1), "CPU measurement".into())?.into_data();
        data.canvas.width = 1920;
        data.canvas.height = 1080;
        let mut effect = project::effect_definition("color_grade")?.instance("grade");
        if size != 0 {
            let mut text = format!("LUT_3D_SIZE {size}\n");
            for b in 0..size {
                for g in 0..size {
                    for r in 0..size {
                        let [r, g, b] = [r, g, b].map(|v| v as f64 / (size - 1) as f64);
                        let [r, g, b] = if identity {
                            [r, g, b]
                        } else {
                            [r * g, g * b, b * r]
                        };
                        text.push_str(&format!("{r:.9} {g:.9} {b:.9}\n"));
                    }
                }
            }
            let cube = render::lut::Cube3d::parse(text.as_bytes(), &hooks)?;
            let path = directory.path().join(label);
            std::fs::create_dir(&path)?;
            let canonical = path.join("canonical.cube");
            cube.write_cube(&mut std::fs::File::create(&canonical)?, &hooks)?;
            drop(cube);
            drop(text);
            let hash = media::ContentHash::read(&canonical)?;
            let id = project::LutId::new(hash);
            data.luts.push(project::LutRecord {
                id,
                name: label.into(),
                licence: None,
                source_name: "synthetic.cube".into(),
                source_hash: hash,
                directory: path,
            });
            effect = effect.with("lut", project::EffectValue::Lut(Some(id)))?;
        }
        data.scene_mut().timeline.main.items.push(
            testkit::item::item(
                project::ItemId(1),
                project::ItemParams::Overlay(project::OverlayParams {
                    color: project::Color([51, 117, 193, 255]),
                }),
            )
            .source(time::TimeRange::new(
                time::Tick::ZERO,
                time::Tick(time::TICKS_PER_SECOND),
            )?)
            .effects(vec![effect])
            .build(),
        );
        let project = project::Project::new(data)?;
        let settings = render::RenderSettings::from_project(&project);
        let mut renderer = render::Renderer::new(&project, settings)?;
        let mut sources = render::FileSources::default();
        hooks.peak.store(0, Ordering::SeqCst);
        for pass in ["first", "filesystem_warm"] {
            let start = Instant::now();
            let frame = renderer.render_frame(time::Tick::ZERO, &mut sources, &hooks)?;
            println!(
                "case={label} pass={pass} elapsed_ms={:.3} charged_peak_bytes={} output_pixels={}",
                start.elapsed().as_secs_f64() * 1000.,
                hooks.peak.load(Ordering::SeqCst),
                frame.pixels().len()
            );
            drop(frame);
        }
        drop(renderer);
        if hooks.used.load(Ordering::SeqCst) != 0 {
            return Err("retained frame memory".into());
        }
    }
    #[cfg(windows)]
    {
        let output=std::process::Command::new("powershell").args(["-NoProfile","-Command",&format!("Get-Process -Id {} | Select-Object PeakWorkingSet64,PeakPagedMemorySize64 | ConvertTo-Json -Compress",std::process::id())]).output()?;
        if !output.status.success() {
            return Err("process memory measurement failed".into());
        }
        println!(
            "process_peak={}",
            std::str::from_utf8(&output.stdout)?.trim()
        );
    }
    Ok(())
}
