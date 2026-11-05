//! Prepare local RGBA frames and five landmarks through the production media
//! adapter. Input rows: key width height x0 y0 ... x4 y4 (source pixels).
//! No media paths, pixels or descriptors are logged.
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(args.next().ok_or("input directory required")?);
    let dest = PathBuf::from(args.next().ok_or("output directory required")?);
    std::fs::create_dir_all(&dest)?;
    let inputs = std::fs::read_to_string(root.join("inputs.txt"))?;
    let (mut prepared, mut rejected) = (0, 0);
    for line in inputs.lines() {
        let mut fields = line.split_whitespace();
        let key = fields.next().ok_or("input key")?;
        if !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err("invalid input key".into());
        }
        let width = fields.next().ok_or("width")?.parse()?;
        let height = fields.next().ok_or("height")?.parse()?;
        let mut points = [[0.; 2]; 5];
        for point in &mut points {
            for value in point {
                *value = fields.next().ok_or("landmark")?.parse()?;
            }
        }
        if fields.next().is_some() {
            return Err("trailing input".into());
        }
        let pixels = std::fs::read(root.join(format!("{key}.rgba")))?;
        let frame = media::VisionFrame::from_rgba8(time::Tick::ZERO, width, height, pixels)?;
        if let Some(tensor) = frame.identity_tensor(points)? {
            let bytes: Vec<u8> = tensor.iter().flat_map(|v| v.to_le_bytes()).collect();
            std::fs::write(dest.join(format!("{key}.bin")), bytes)?;
            prepared += 1;
        } else {
            rejected += 1;
        }
    }
    println!("prepared={prepared} rejected={rejected}");
    Ok(())
}
