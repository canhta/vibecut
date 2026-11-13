//! Picks the layout of one shot from its people, speaker and screen evidence.
use super::overrides::Layout;
use engine::{FaceTrack, Scene, ShotAnalysis, SpeakerRef, SpeakerState};

/// One analysed shot with its position in the analysis, which is what a
/// speaker reference names.
#[derive(Clone, Copy)]
pub(super) struct Evidence<'a> {
    pub index: usize,
    pub shot: &'a ShotAnalysis,
}

/// The layout a shot gets and whether a layout's condition failed on the way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Choice {
    pub layout: Layout,
    pub fallback: bool,
}

/// Visible people with a track to crop around.
fn faces(evidence: Option<Evidence<'_>>) -> usize {
    evidence.map_or(0, |e| e.shot.people_count.min(e.shot.tracks.len()))
}

fn screen(evidence: Option<Evidence<'_>>) -> bool {
    evidence.is_some_and(|e| e.shot.scene == Scene::Screen)
}

/// Whether the evidence satisfies the layout's condition.
fn holds(layout: Layout, evidence: Option<Evidence<'_>>) -> bool {
    let people = faces(evidence);
    match layout {
        Layout::Fill | Layout::Fit => true,
        Layout::Split => people >= 2,
        Layout::Three => people >= 3,
        Layout::Four => people >= 4,
        Layout::Screenshare | Layout::Gameplay => screen(evidence) && people >= 1,
    }
}

/// Where a failed condition lands: Fit for a shot with analysis but no face
/// to crop around, Fill otherwise (a centred cover without any evidence).
fn fallback(evidence: Option<Evidence<'_>>) -> Layout {
    if evidence.is_some() && faces(evidence) == 0 {
        Layout::Fit
    } else {
        Layout::Fill
    }
}

/// The automatic choice. Gameplay is never chosen automatically: no detector
/// tells gameplay from other screen content.
fn automatic(evidence: Option<Evidence<'_>>) -> Choice {
    let people = faces(evidence);
    if screen(evidence) {
        return if people >= 1 {
            Choice {
                layout: Layout::Screenshare,
                fallback: false,
            }
        } else {
            Choice {
                layout: Layout::Fit,
                fallback: true,
            }
        };
    }
    let layout = match (evidence.is_some(), people) {
        (false, _) => Layout::Fill,
        (true, 0) => Layout::Fit,
        (true, 1) => Layout::Fill,
        (true, 2) => Layout::Split,
        (true, 3) => Layout::Three,
        (true, _) => Layout::Four,
    };
    Choice {
        layout,
        fallback: false,
    }
}

/// The layout of one shot: the forced one when its condition holds, else the
/// fallback; the automatic choice when nothing is forced.
pub(super) fn resolve(forced: Option<Layout>, evidence: Option<Evidence<'_>>) -> Choice {
    match forced {
        None => automatic(evidence),
        Some(layout) if holds(layout, evidence) => Choice {
            layout,
            fallback: false,
        },
        Some(_) => Choice {
            layout: fallback(evidence),
            fallback: true,
        },
    }
}

fn area(track: &FaceTrack) -> f64 {
    f64::from(track.bounds[2]) * f64::from(track.bounds[3])
}

fn center_x(track: &FaceTrack) -> f64 {
    f64::from(track.bounds[0]) + f64::from(track.bounds[2]) / 2.0
}

/// Source-pixel centre of a track's box (`bounds` is x, y, width, height).
pub(super) fn center(track: &FaceTrack) -> [f64; 2] {
    [
        center_x(track),
        f64::from(track.bounds[1]) + f64::from(track.bounds[3]) / 2.0,
    ]
}

/// The visible speaker's track, else the largest track (lowest id on a tie).
pub(super) fn speaker_track(evidence: Option<Evidence<'_>>) -> Option<&FaceTrack> {
    let e = evidence?;
    if let SpeakerState::Visible { local, .. } = e.shot.speaker
        && usize::try_from(local.shot).ok() == Some(e.index)
        && let Some(track) = e.shot.tracks.iter().find(|t| t.id == local.track)
    {
        return Some(track);
    }
    e.shot
        .tracks
        .iter()
        .max_by(|a, b| area(a).total_cmp(&area(b)).then(b.id.cmp(&a.id)))
}

/// Whether the track is the person the pin names in this shot.
fn is_pinned(index: usize, track: &FaceTrack, pin: SpeakerRef) -> bool {
    track.identity == Some(pin)
        || (usize::try_from(pin.shot).ok() == Some(index) && track.id == pin.track)
}

/// The short's first visible speaker, as the anchor later shots' identities
/// point at: the track's own cross-shot identity, else its local reference.
pub(super) fn first_speaker<'a>(
    mut shots: impl Iterator<Item = Evidence<'a>>,
) -> Option<SpeakerRef> {
    shots.find_map(|e| {
        let SpeakerState::Visible { local, .. } = e.shot.speaker else {
            return None;
        };
        if usize::try_from(local.shot).ok() != Some(e.index) {
            return None;
        }
        let track = e.shot.tracks.iter().find(|t| t.id == local.track)?;
        Some(track.identity.unwrap_or(local))
    })
}

/// `count` people for the slots of a multi-person layout: the pinned person
/// first when present, the rest the most prominent ones ordered left to right.
pub(super) fn people(
    evidence: Option<Evidence<'_>>,
    count: usize,
    pin: Option<SpeakerRef>,
) -> Vec<&FaceTrack> {
    let Some(e) = evidence else {
        return Vec::new();
    };
    let mut tracks: Vec<&FaceTrack> = e.shot.tracks.iter().collect();
    tracks.sort_by(|a, b| area(b).total_cmp(&area(a)).then(a.id.cmp(&b.id)));
    let pinned = pin
        .and_then(|pin| tracks.iter().position(|t| is_pinned(e.index, t, pin)))
        .map(|at| tracks.remove(at));
    tracks.truncate(count.saturating_sub(usize::from(pinned.is_some())));
    tracks.sort_by(|a, b| center_x(a).total_cmp(&center_x(b)).then(a.id.cmp(&b.id)));
    pinned.into_iter().chain(tracks).collect()
}
