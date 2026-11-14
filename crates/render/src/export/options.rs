//! Resolves portable output choices against immutable scene snapshots.
use crate::{Error, ExportSettings, Result};
use project::{Project, SceneId, SceneScope};
use serde::{Deserialize, Serialize};
use time::{FrameRate, Tick, TimeRange};

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RangeChoice {
    #[default]
    Full,
    InOut {
        start: Tick,
        end: Tick,
    },
    Selected {
        start: Tick,
        end: Tick,
    },
}
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SceneChoice {
    #[default]
    Active,
    All,
    Selected {
        scene: SceneId,
    },
}
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    #[default]
    Automatic,
    H264,
    Vp9,
}
impl VideoCodec {
    /// Whether this codec can be written into `container`. WebM carries VP9
    /// only, MP4 and MOV H.264 only and GIF neither; `Automatic` fits all of them.
    pub fn fits(self, container: media::Container) -> bool {
        match self {
            Self::Automatic => true,
            Self::H264 => matches!(container, media::Container::Mp4 | media::Container::Mov),
            Self::Vp9 => matches!(container, media::Container::WebM),
        }
    }
}
/// Versioned portable extension of the existing export settings. Runtime render
/// caches, effect adapters, overwrite consent and destinations never serialize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OutputOptions {
    pub version: u32,
    /// None matches the project canvas; explicit dimensions change output only.
    #[serde(deserialize_with = "required_option")]
    #[schemars(with = "RequiredNullable<[u32; 2]>")]
    pub resolution: Option<[u32; 2]>,
    #[serde(deserialize_with = "required_option")]
    #[schemars(with = "RequiredNullable<FrameRate>")]
    pub fps: Option<FrameRate>,
    #[serde(deserialize_with = "required_option")]
    #[schemars(with = "RequiredNullable<usize>")]
    pub bit_rate: Option<usize>,
    pub codec: VideoCodec,
    pub profile: media::H264Profile,
    /// Palette choices; used only when the container is GIF.
    pub gif: media::GifOptions,
    pub range: RangeChoice,
    pub scene: SceneChoice,
    /// Audio-only output (`m4a` AAC or `webm` Opus); refuses every video field.
    pub audio_only: bool,
}

struct RequiredNullable<T>(std::marker::PhantomData<T>);
impl<T: schemars::JsonSchema> schemars::JsonSchema for RequiredNullable<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("RequiredNullable_{}", T::schema_name()).into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        generator.subschema_for::<Option<T>>()
    }
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}
impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            version: 1,
            resolution: None,
            fps: None,
            bit_rate: None,
            codec: VideoCodec::Automatic,
            profile: media::H264Profile::Automatic,
            gif: media::GifOptions::default(),
            range: RangeChoice::Full,
            scene: SceneChoice::Active,
            audio_only: false,
        }
    }
}
impl OutputOptions {
    /// Stable scene ids disambiguate equal names without accepting path fragments.
    pub fn paths(
        self,
        project: &Project,
        output: &std::path::Path,
    ) -> Result<Vec<std::path::PathBuf>> {
        let scenes = self.ranges(project)?;
        if !matches!(self.scene, SceneChoice::All) {
            return Ok(vec![output.to_owned()]);
        }
        let stem = output
            .file_stem()
            .ok_or(Error::Invalid("output filename"))?;
        let extension = output
            .extension()
            .ok_or(Error::Invalid("output extension"))?;
        Ok(scenes
            .iter()
            .map(|(scene, _)| {
                let mut name = stem.to_os_string();
                name.push(format!("-scene-{}.", scene.0));
                name.push(extension);
                output.with_file_name(name)
            })
            .collect())
    }
    pub fn validate(self) -> Result<()> {
        if self.version != 1 {
            return Err(Error::Unsupported("export options version"));
        }
        if self.audio_only
            && (self.resolution.is_some()
                || self.fps.is_some()
                || self.bit_rate.is_some()
                || self.codec != VideoCodec::Automatic
                || self.profile != media::H264Profile::Automatic)
        {
            return Err(Error::Invalid(
                "audio-only output carries no video settings",
            ));
        }
        if let Some([w, h]) = self.resolution
            && (w < 2
                || h < 2
                || !w.is_multiple_of(2)
                || !h.is_multiple_of(2)
                || u64::from(w) * u64::from(h) > 1 << 26)
        {
            return Err(Error::Invalid("output resolution"));
        }
        if self
            .bit_rate
            .is_some_and(|rate| !(1_000..=1_000_000_000).contains(&rate))
        {
            return Err(Error::Invalid("output bit rate"));
        }
        if let Some(fps) = self.fps
            && (!FrameRate::COMMON.contains(&fps) || !fps.has_exact_frame_ticks())
        {
            return Err(Error::Invalid("output frame rate"));
        }
        self.gif
            .validate()
            .map_err(|limit| Error::Media(limit.into()))
    }
    /// GIF admission for the exact scenes, geometry, rate and palette `settings`
    /// resolve to, including each scene's duration. Other containers pass.
    pub fn check_gif(self, project: &Project, settings: &ExportSettings) -> Result<()> {
        if settings.container != media::Container::Gif {
            return Ok(());
        }
        let mut configured = settings.clone();
        self.apply(&mut configured)?;
        self.ranges(project)?
            .into_iter()
            .try_for_each(|(_, range)| super::gif::admission(range, &configured))
    }
    /// Image-sequence admission for the exact scene, range and rate `settings` resolve
    /// to: at most `MAX_SEQUENCE_FRAMES` frames. Other outputs pass.
    pub fn check_sequence(self, project: &Project, settings: &ExportSettings) -> Result<()> {
        if !settings.image_sequence {
            return Ok(());
        }
        let mut configured = settings.clone();
        self.apply(&mut configured)?;
        self.ranges(project)?
            .into_iter()
            .try_for_each(|(_, range)| super::sequence::admission(range, &configured))
    }
    /// Admit every scene and range before starting any file.
    pub fn ranges(self, project: &Project) -> Result<Vec<(SceneId, TimeRange)>> {
        self.validate()?;
        let scope = match self.scene {
            SceneChoice::Active => SceneScope::Active,
            SceneChoice::All => SceneScope::All,
            SceneChoice::Selected { scene } => SceneScope::Selected(vec![scene]),
        };
        project
            .scene_ids(&scope)?
            .into_iter()
            .map(|id| {
                let scene = project
                    .data()
                    .scenes
                    .get(id)
                    .ok_or(Error::Invalid("scene not found"))?;
                let range = self.range_at(scene.duration(), project.data().canvas.fps)?;
                Ok((id, range))
            })
            .collect()
    }
    fn range_at(self, duration: Tick, fps: FrameRate) -> Result<TimeRange> {
        let (start, end) = match self.range {
            RangeChoice::Full => (Tick::ZERO, duration),
            RangeChoice::InOut { start, end } | RangeChoice::Selected { start, end } => {
                if start < Tick::ZERO || end > duration || start >= end {
                    return Err(Error::Invalid("export range"));
                }
                (fps.snap_nearest(start)?, fps.snap_nearest(end)?)
            }
        };
        if start < Tick::ZERO || start >= end || end > duration {
            return Err(Error::Invalid("empty or invalid export range"));
        }
        Ok(TimeRange::new(start, end)?)
    }
    /// Container an output path selects. Audio-only admits `m4a` (AAC in MP4) plus
    /// `mp4` and `webm`; video admits `mp4`, `mov` and `webm`. MOV is video-only.
    pub fn container_for_path(
        path: &std::path::Path,
        audio_only: bool,
    ) -> Result<media::Container> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .ok_or(Error::Invalid("output extension"))?;
        let container = if audio_only && extension.eq_ignore_ascii_case("m4a") {
            media::Container::Mp4
        } else {
            media::Container::from_extension(extension).ok_or(Error::Invalid("output extension"))?
        };
        if audio_only && matches!(container, media::Container::Mov | media::Container::Gif) {
            return Err(Error::Unsupported("audio-only container"));
        }
        Ok(container)
    }
    /// Whether `profile` can be requested for `container`: an explicit H.264
    /// profile has no meaning in WebM or GIF.
    pub fn profile_fits(profile: media::H264Profile, container: media::Container) -> bool {
        profile == media::H264Profile::Automatic
            || matches!(container, media::Container::Mp4 | media::Container::Mov)
    }
    /// Safe word-highlight preflight for the exact selected scenes and output geometry.
    /// Shares the immutable caption reader and shaped fit with preview/burn; no media IO.
    pub fn caption_word_warnings(
        self,
        project: &Project,
        settings: &ExportSettings,
        hooks: &dyn crate::ExecutionHooks,
    ) -> Result<project::CaptionWordWarnings> {
        let mut configured = settings.clone();
        self.apply(&mut configured)?;
        let mut result = project::CaptionWordWarnings::default();
        for (scene, _) in self.ranges(project)? {
            let snapshot = project.for_scene(scene)?;
            let warnings = crate::Renderer::new(&snapshot, configured.render.clone())?
                .caption_word_warnings(None, hooks)?;
            result.additional = result.additional.saturating_add(warnings.additional);
            for warning in warnings.warnings {
                if result.warnings.iter().any(|prior| {
                    prior.item == warning.item
                        && prior.language == warning.language
                        && prior.reason == warning.reason
                }) {
                    continue;
                }
                if result.warnings.len() < 64 {
                    result.warnings.push(warning);
                } else {
                    result.additional = result.additional.saturating_add(1);
                }
            }
        }
        Ok(result)
    }

    pub fn apply(self, settings: &mut ExportSettings) -> Result<()> {
        self.validate()?;
        if !self.codec.fits(settings.container)
            || !Self::profile_fits(self.profile, settings.container)
        {
            return Err(Error::Unsupported("codec/profile for container"));
        }
        if settings.image_sequence {
            // One directory of frames: no codec, profile, bitrate or audio apply, and
            // one directory cannot hold several scenes' frames.
            if self.audio_only
                || self.bit_rate.is_some()
                || self.codec != VideoCodec::Automatic
                || self.profile != media::H264Profile::Automatic
            {
                return Err(Error::Unsupported("video settings for image sequence"));
            }
            if self.scene == SceneChoice::All {
                return Err(Error::Unsupported("image sequence for all scenes"));
            }
        }
        if self.audio_only {
            if matches!(
                settings.container,
                media::Container::Mov | media::Container::Gif
            ) {
                return Err(Error::Unsupported("audio-only container"));
            }
            settings.audio_only = true;
        }
        if settings.container == media::Container::Gif {
            if self.bit_rate.is_some() {
                return Err(Error::Unsupported("bit rate for GIF"));
            }
            settings.gif = self.gif;
        }
        if let Some([width, height]) = self.resolution {
            settings.render.width = width;
            settings.render.height = height;
        }
        if let Some(fps) = self.fps {
            settings.fps = fps;
        }
        if let Some(rate) = self.bit_rate {
            settings.bit_rate = rate;
        }
        settings.profile = self.profile;
        if self.resolution.is_some()
            || self.fps.is_some()
            || self.bit_rate.is_some()
            || self.codec != VideoCodec::Automatic
            || self.profile != media::H264Profile::Automatic
        {
            settings.mode = crate::ExportMode::Encode;
        }
        if settings.container == media::Container::Gif {
            media::GifEncoderSettings {
                width: settings.render.width,
                height: settings.render.height,
                frame_rate: settings.fps,
                options: settings.gif,
            }
            .validate()
            .map_err(|limit| Error::Media(limit.into()))?;
        }
        Ok(())
    }
}
