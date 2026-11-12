//! Edit-time canvas envelopes for item alignment or even spacing.
use super::Rect;
use super::transform::Transform;
use super::viewport::ItemGeometry;
use crate::{Error, Result};
use project::{Item, ProjectData};
use time::Tick;

/// The saved geometric box of one item at output `at`, as an axis-aligned
/// envelope in project canvas pixels.
///
/// The transform is sampled at `clamp(at - start, 0, duration)` so a selection
/// of items that do not overlap in time still shares one clock. The box is the
/// canonical placement saved in the project: full canvas for video, image and
/// overlay items, the text box around its saved position, and the centred
/// graphic author square. Transient render placements are not consulted, and
/// alpha, glyph, effect and mask coverage never tighten the box.
///
/// # Errors
///
/// Refuses a negative time, audio or caption items, a non-finite or empty
/// destination, an unrepresentable transform and a non-finite envelope.
pub fn bounds(data: &ProjectData, item: &Item, at: Tick) -> Result<Rect> {
    if at < Tick::ZERO {
        return Err(Error::Invalid("negative arrangement time"));
    }
    let local = at.saturating_sub(item.start).min(
        item.duration()
            .map_err(|_| Error::Invalid("item duration"))?,
    );
    let placement = super::evaluation::saved_placement(&data.canvas, item)
        .ok_or(Error::Unsupported("arrangement on unsupported item"))?;
    let destination = placement.destination;
    if [
        destination.x,
        destination.y,
        destination.width,
        destination.height,
    ]
    .iter()
    .any(|value| !value.is_finite())
        || destination.width <= 0.0
        || destination.height <= 0.0
    {
        return Err(Error::Invalid("item destination"));
    }
    let bounds = Rect {
        x: destination.x * f64::from(data.canvas.width),
        y: destination.y * f64::from(data.canvas.height),
        width: destination.width * f64::from(data.canvas.width),
        height: destination.height * f64::from(data.canvas.height),
    };
    let envelope =
        ItemGeometry::from_parts(bounds, Transform::at(item, local)?).transformed_bounds();
    if [envelope.x, envelope.y, envelope.width, envelope.height]
        .iter()
        .any(|value| !value.is_finite())
        || envelope.width <= 0.0
        || envelope.height <= 0.0
    {
        return Err(Error::Invalid("item arrangement geometry"));
    }
    Ok(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::{
        Animations, BuiltinPath, Color, ItemId, ItemParams, OverlayParams, Param, ParamValue,
        TextParams,
    };
    use time::SourceRange;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    fn item(content: ItemParams, animations: Animations) -> Item {
        Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations,
            id: ItemId(1),
            source_asset: None,
            source_range: SourceRange::new(Tick::ZERO, Tick(1)).unwrap(),
            start: Tick::ZERO,
            content,
        }
    }

    fn transformed(animations: &[(BuiltinPath, f64)]) -> Animations {
        let mut params = Animations::default();
        for (path, value) in animations {
            let value = if path.is_flip() {
                ParamValue::Bool(true)
            } else {
                ParamValue::Scalar(*value)
            };
            params
                .set((*path).into(), Param::new(value).unwrap())
                .unwrap();
        }
        params
    }

    fn close(actual: Rect, expected: Rect) -> bool {
        (actual.x - expected.x).abs() < 1e-9
            && (actual.y - expected.y).abs() < 1e-9
            && (actual.width - expected.width).abs() < 1e-9
            && (actual.height - expected.height).abs() < 1e-9
    }

    #[test]
    fn envelope_matches_the_placed_box_after_scale_reflection_and_quarter_turn() {
        // Reference example: destination (40,30,80,20), translate (10,-5),
        // scale (.5,2), 90 degrees gives the envelope (70,15,40,40).
        let item = item(
            ItemParams::Text(TextParams::new("Box")),
            transformed(&[
                (BuiltinPath::TransformPositionX, 10.0),
                (BuiltinPath::TransformPositionY, -5.0),
                (BuiltinPath::TransformScaleX, 0.5),
                (BuiltinPath::TransformScaleY, 2.0),
                (BuiltinPath::TransformRotate, 90.0),
            ]),
        );
        let envelope = Transform::at(&item, Tick::ZERO)
            .unwrap()
            .bounds(rect(40.0, 30.0, 80.0, 20.0));
        assert!(
            close(envelope, rect(70.0, 15.0, 40.0, 40.0)),
            "{envelope:?}"
        );

        let reflected = item(
            ItemParams::Overlay(OverlayParams {
                color: Color([255; 4]),
            }),
            transformed(&[
                (BuiltinPath::TransformFlipX, 1.0),
                (BuiltinPath::TransformFlipY, 1.0),
            ]),
        );
        let plain = rect(10.0, 20.0, 40.0, 30.0);
        assert!(
            close(
                Transform::at(&reflected, Tick::ZERO).unwrap().bounds(plain),
                plain
            ),
            "a centre reflection permutes corners without changing the envelope"
        );
    }

    #[test]
    fn bounds_use_the_saved_placement_and_clamp_the_clock() {
        let mut data = project::Project::empty(project::ProjectId(1), "Arrange".into())
            .unwrap()
            .into_data();
        data.canvas.width = 200;
        data.canvas.height = 100;
        let mut text = item(
            ItemParams::Text(TextParams::new("Box")),
            transformed(&[
                (BuiltinPath::TransformPositionX, 10.0),
                (BuiltinPath::TransformPositionY, -5.0),
                (BuiltinPath::TransformScaleX, 0.5),
                (BuiltinPath::TransformScaleY, 2.0),
                (BuiltinPath::TransformRotate, 90.0),
            ]),
        );
        text.start = Tick(100);
        text.content = ItemParams::Text({
            let mut params = TextParams::new("Box");
            params.position = project::TextPosition { x: 60.0, y: 40.0 };
            params
        });
        assert!(
            close(
                bounds(&data, &text, Tick::ZERO).unwrap(),
                rect(105.0, -5.0, 50.0, 80.0)
            ),
            "before the item starts, its first transform sample is used"
        );
        let audio = item(
            ItemParams::Audio(project::AudioParams {
                volume: 0.0,
                muted: false,
                source: Default::default(),
                fades: Default::default(),
            }),
            Animations::default(),
        );
        assert!(matches!(
            bounds(&data, &audio, Tick::ZERO),
            Err(Error::Unsupported("arrangement on unsupported item"))
        ));
    }
}
