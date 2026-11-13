//! The one timeline-item builder for engine, render, shorts and app tests.
//!
//! A built item is a plain clip: not hidden, normal blend, no masks, effects or
//! animations, default retime, no source asset, a one-second source window and
//! start at zero. Setters change only what a test is about.
use media::AssetId;
use project::{
    Animations, AudioParams, BlendMode, EffectInstance, Item, ItemId, ItemParams, Mask, Retime,
    VideoParams,
};
use time::{SourceRange, Speed, TICKS_PER_SECOND, Tick};

/// An item under construction.
#[derive(Clone, Debug)]
pub struct ItemBuilder {
    item: Item,
}

/// Starts an item with `content`.
pub fn item(id: ItemId, content: ItemParams) -> ItemBuilder {
    ItemBuilder {
        item: Item {
            blend_mode: BlendMode::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Animations::default(),
            id,
            hidden: false,
            source_asset: None,
            source_range: SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND))
                .expect("valid fixture source range"),
            start: Tick::ZERO,
            retime: Retime::default(),
            content,
        },
    }
}

/// Starts a plain video item ([`VideoParams::plain`]).
pub fn video(id: ItemId) -> ItemBuilder {
    item(id, ItemParams::Video(VideoParams::plain()))
}

/// Starts a plain audio item ([`AudioParams::default`]).
pub fn audio(id: ItemId) -> ItemBuilder {
    item(id, ItemParams::Audio(AudioParams::default()))
}

impl ItemBuilder {
    pub fn asset(mut self, asset: AssetId) -> Self {
        self.item.source_asset = Some(asset);
        self
    }

    /// Sets the asset reference as given, for a test whose item may have none.
    pub fn source_asset(mut self, asset: Option<AssetId>) -> Self {
        self.item.source_asset = asset;
        self
    }

    pub fn source(mut self, range: SourceRange) -> Self {
        self.item.source_range = range;
        self
    }

    pub fn start(mut self, start: Tick) -> Self {
        self.item.start = start;
        self
    }

    pub fn retime(mut self, retime: Retime) -> Self {
        self.item.retime = retime;
        self
    }

    /// Plays at `speed` (1.0 is normal), keeping the other retime defaults.
    pub fn speed(mut self, speed: f64) -> Self {
        self.item.retime = Retime {
            speed: Speed::from_f64(speed).expect("valid fixture speed"),
            ..Retime::default()
        };
        self
    }

    pub fn hidden(mut self, hidden: bool) -> Self {
        self.item.hidden = hidden;
        self
    }

    pub fn blend_mode(mut self, blend_mode: BlendMode) -> Self {
        self.item.blend_mode = blend_mode;
        self
    }

    pub fn masks(mut self, masks: Vec<Mask>) -> Self {
        self.item.masks = masks;
        self
    }

    pub fn effects(mut self, effects: Vec<EffectInstance>) -> Self {
        self.item.effects = effects;
        self
    }

    pub fn animations(mut self, animations: Animations) -> Self {
        self.item.animations = animations;
        self
    }

    pub fn content(mut self, content: ItemParams) -> Self {
        self.item.content = content;
        self
    }

    pub fn build(self) -> Item {
        self.item
    }
}
