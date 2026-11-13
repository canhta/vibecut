//! Composes a layout into per-slot crop, fit and transform params.
use super::choice::{self, Evidence};
use super::overrides::Layout;
use engine::SpeakerRef;
use project::{Crop, FitMode};

/// Canvas and source frame sizes in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    pub canvas: [f64; 2],
    pub source: [f64; 2],
}

/// One video item of a layout: the existing item params that place a piece of
/// the source. Slot 0 carries the audio; every other slot is silent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Slot {
    pub crop: Option<Crop>,
    pub fit: FitMode,
    /// Static `transform.scale{X,Y}`; `[1, 1]` for Fill and Fit.
    pub scale: [f64; 2],
    /// Static `transform.position{X,Y}` in canvas pixels, y down.
    pub offset: [f64; 2],
    pub audio: bool,
}

/// Fraction of the canvas height the speaker takes in Gameplay, the one
/// proportion the cited source documents (30 % speaker, 70 % gameplay).
/// Every other layout divides the canvas into equal cells: the source gives no
/// other proportion, so none is invented here.
const GAMEPLAY_SPEAKER: f64 = 0.30;

#[derive(Clone, Copy)]
enum Role {
    Person,
    Content,
}

/// A slot's rectangle as fractions of the canvas.
#[derive(Clone, Copy)]
struct Cell {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    role: Role,
}

const fn cell(x: f64, y: f64, width: f64, height: f64, role: Role) -> Cell {
    Cell {
        x,
        y,
        width,
        height,
        role,
    }
}

/// Slots in order; the first is the top (or top-left) slot.
fn cells(layout: Layout) -> &'static [Cell] {
    const SPLIT: [Cell; 2] = [
        cell(0.0, 0.0, 1.0, 0.5, Role::Person),
        cell(0.0, 0.5, 1.0, 0.5, Role::Person),
    ];
    const THREE: [Cell; 3] = [
        cell(0.0, 0.0, 1.0, 0.5, Role::Person),
        cell(0.0, 0.5, 0.5, 0.5, Role::Person),
        cell(0.5, 0.5, 0.5, 0.5, Role::Person),
    ];
    const FOUR: [Cell; 4] = [
        cell(0.0, 0.0, 0.5, 0.5, Role::Person),
        cell(0.5, 0.0, 0.5, 0.5, Role::Person),
        cell(0.0, 0.5, 0.5, 0.5, Role::Person),
        cell(0.5, 0.5, 0.5, 0.5, Role::Person),
    ];
    const SCREENSHARE: [Cell; 2] = [
        cell(0.0, 0.0, 1.0, 0.5, Role::Content),
        cell(0.0, 0.5, 1.0, 0.5, Role::Person),
    ];
    const GAMEPLAY: [Cell; 2] = [
        cell(0.0, 0.0, 1.0, GAMEPLAY_SPEAKER, Role::Person),
        cell(
            0.0,
            GAMEPLAY_SPEAKER,
            1.0,
            1.0 - GAMEPLAY_SPEAKER,
            Role::Content,
        ),
    ];
    match layout {
        Layout::Split => &SPLIT,
        Layout::Three => &THREE,
        Layout::Four => &FOUR,
        Layout::Screenshare => &SCREENSHARE,
        Layout::Gameplay => &GAMEPLAY,
        Layout::Fill | Layout::Fit => &[],
    }
}

/// Left edge of a window of fractional width `size` centred on `center`,
/// clamped into the frame so `start + size` never exceeds 1.
fn window_start(center: f64, size: f64) -> f64 {
    let mut start = (center - size / 2.0).clamp(0.0, (1.0 - size).max(0.0));
    while start > 0.0 && start + size > 1.0 {
        start = f64::from_bits(start.to_bits() - 1);
    }
    start
}

/// The largest window of the slot's aspect that fits the source, centred on
/// `center` (source pixels) or on the frame. `FitMode::Fill.extent` is the one
/// cover computation: its overflow is the part of the source the slot cuts.
/// `None` when the window is the whole frame or a size is unusable.
fn window(slot: [f64; 2], source: [f64; 2], center: Option<[f64; 2]>) -> Option<Crop> {
    let [over_x, over_y] = FitMode::Fill.extent(slot, source)?;
    let width = (1.0 / over_x).min(1.0);
    let height = (1.0 / over_y).min(1.0);
    if width >= 1.0 && height >= 1.0 {
        return None;
    }
    let [cx, cy] = center.map_or([0.5, 0.5], |c| [c[0] / source[0], c[1] / source[1]]);
    Some(Crop {
        x: window_start(cx, width),
        y: window_start(cy, height),
        width,
        height,
    })
}

fn slot_pixels(cell: &Cell, canvas: [f64; 2]) -> ([f64; 2], [f64; 2]) {
    let size = [cell.width * canvas[0], cell.height * canvas[1]];
    let offset = [
        cell.x * canvas[0] + size[0] / 2.0 - canvas[0] / 2.0,
        cell.y * canvas[1] + size[1] / 2.0 - canvas[1] / 2.0,
    ];
    (size, offset)
}

/// A person slot: the source window of the slot's aspect around the person,
/// stretched over the canvas and scaled into the cell, so the picture keeps its
/// aspect.
fn person_slot(
    cell: &Cell,
    geometry: Geometry,
    track: Option<&engine::FaceTrack>,
    audio: bool,
) -> Slot {
    let (size, offset) = slot_pixels(cell, geometry.canvas);
    Slot {
        crop: window(size, geometry.source, track.map(choice::center)),
        fit: FitMode::Manual,
        scale: [size[0] / geometry.canvas[0], size[1] / geometry.canvas[1]],
        offset,
        audio,
    }
}

/// A content slot: the whole frame, letterboxed inside the cell by the Fit box.
fn content_slot(cell: &Cell, geometry: Geometry, audio: bool) -> Slot {
    let (size, offset) = slot_pixels(cell, geometry.canvas);
    let boxed = FitMode::Fit
        .place(size, geometry.source)
        .map_or([1.0, 1.0], |b| [b.width, b.height]);
    Slot {
        crop: None,
        fit: FitMode::Manual,
        scale: [
            boxed[0] * size[0] / geometry.canvas[0],
            boxed[1] * size[1] / geometry.canvas[1],
        ],
        offset,
        audio,
    }
}

/// The slots of one shot under an already resolved `layout`. Fill is the item's
/// Fill fit around the active speaker's window and Fit its Fit fit, both through
/// `project::fit_box`; the other layouts stack Manual items on the canvas.
pub(super) fn compose(
    layout: Layout,
    evidence: Option<Evidence<'_>>,
    pin: Option<SpeakerRef>,
    geometry: Geometry,
) -> Vec<Slot> {
    let whole = |fit, crop| Slot {
        crop,
        fit,
        scale: [1.0, 1.0],
        offset: [0.0, 0.0],
        audio: true,
    };
    match layout {
        Layout::Fit => vec![whole(FitMode::Fit, None)],
        Layout::Fill => {
            let center = choice::speaker_track(evidence).map(choice::center);
            // Without a person the cover is already centred: no crop.
            let crop = center.and_then(|c| window(geometry.canvas, geometry.source, Some(c)));
            vec![whole(FitMode::Fill, crop)]
        }
        _ => {
            let cells = cells(layout);
            let persons = cells
                .iter()
                .filter(|c| matches!(c.role, Role::Person))
                .count();
            let tracks: Vec<&engine::FaceTrack> = match layout {
                Layout::Screenshare | Layout::Gameplay => {
                    choice::speaker_track(evidence).into_iter().collect()
                }
                _ => choice::people(evidence, persons, pin),
            };
            let mut tracks = tracks.into_iter();
            cells
                .iter()
                .enumerate()
                .map(|(at, c)| match c.role {
                    Role::Person => person_slot(c, geometry, tracks.next(), at == 0),
                    Role::Content => content_slot(c, geometry, at == 0),
                })
                .collect()
        }
    }
}
