//! Decides which registry commands the agent caller may run.
use engine::control::{ControlError, Error as ControlFailure, Policy};
use engine::{Command, CommandKind, GrantClass};
use serde::Serialize;

/// Why the agent may not run a command. The model reads the text of the reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Writes a file or folder the model would have to name.
    Export,
    /// Saving belongs to the user.
    Save,
    /// Creates, copies, deletes or opens a project.
    Project,
    /// Reads a source file the model would have to name.
    Path,
    /// Undo and redo belong to the user; the fork has no history.
    History,
    /// Gestures and previews belong to the UI.
    Gesture,
    /// View state is not an edit.
    ViewState,
    /// A person's sound or music library.
    Library,
    /// Downloads models or fonts.
    Download,
    /// A dedicated tool already covers it.
    UseTool,
    /// Frames were not enabled for this conversation.
    Frames,
    /// A person or trusted application only.
    HumanOnly,
    /// The command has no row in the policy table.
    Unlisted,
}
impl Reason {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Export => "The agent cannot export or write files. Ask the user to export.",
            Self::Save => "The agent cannot save. The user saves after applying the changes.",
            Self::Project => "The agent cannot create, copy, delete, restore or open projects.",
            Self::Path => "The agent cannot read files or name a file path.",
            Self::History => "The agent cannot undo or redo. The user can discard the changes.",
            Self::Gesture => "Gestures and previews belong to the interface; use an edit command.",
            Self::ViewState => "View state is not an edit; the agent cannot change it.",
            Self::Library => "The agent cannot use the user's sound or music library.",
            Self::Download => "The agent cannot download models or fonts.",
            Self::UseTool => "Use project_summary, timeline_read or command_list instead.",
            Self::Frames => "Frames are not enabled for this conversation.",
            Self::HumanOnly => "Only the user can run this command.",
            Self::Unlisted => "This command has no agent policy and is refused.",
        }
    }
}

/// The decision for one command id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allow,
    /// Allowed only when the user enabled frames for the conversation.
    Frames,
    Refuse(Reason),
}

/// The agent's `Caller` policy. The table decides; the command's declared grant can only
/// narrow it, so a command a person alone may run stays refused whatever its row says.
#[derive(Clone, Copy, Debug, Default)]
pub struct AgentPolicy {
    frames: bool,
}
impl AgentPolicy {
    pub const fn new(frames: bool) -> Self {
        Self { frames }
    }

    /// Whether the agent may run the command `id`, and the reason when it may not.
    pub fn check(&self, id: &str) -> Result<(), Reason> {
        let kind = CommandKind::from_id(id).ok_or(Reason::Unlisted)?;
        let agent = kind.grant() == Some(GrantClass::Agent);
        match verdict(id) {
            None => Err(Reason::Unlisted),
            Some(Verdict::Refuse(reason)) => Err(reason),
            Some(_) if !agent => Err(Reason::HumanOnly),
            Some(Verdict::Allow) => Ok(()),
            Some(Verdict::Frames) if self.frames => Ok(()),
            Some(Verdict::Frames) => Err(Reason::Frames),
        }
    }
}
impl Policy for AgentPolicy {
    /// Engine calls this on the command and on every batch child before any effect.
    fn authorize(
        &self,
        command: &Command,
        _project: &project::Project,
    ) -> engine::control::Result<()> {
        self.check(command.id())
            .map_err(|_| ControlFailure::Control(ControlError::Permission))
    }
}

/// The row of a command id; `None` when the table has none.
pub fn verdict(id: &str) -> Option<Verdict> {
    TABLE
        .iter()
        .find(|(row, _)| *row == id)
        .map(|(_, verdict)| *verdict)
}

/// Every registry command id with its decision. A command added to the registry without a
/// row here fails the policy test and is refused at run time until it gets one.
pub const TABLE: &[(&str, Verdict)] = &[
    ("audio.add_cleanup_effect", Verdict::Allow),
    ("audio.apply_loudness", Verdict::Allow),
    ("audio.clear_loudness", Verdict::Allow),
    ("audio.measure_loudness", Verdict::Allow),
    ("audio.remove_cleanup_effect", Verdict::Allow),
    ("audio.set_fade", Verdict::Allow),
    ("audio.set_loudness", Verdict::Allow),
    ("audio.set_master", Verdict::Allow),
    ("audio.update_cleanup_effect", Verdict::Allow),
    ("batch", Verdict::Allow),
    ("begin_gesture", Verdict::Refuse(Reason::Gesture)),
    ("bookmark.add", Verdict::Allow),
    ("bookmark.move", Verdict::Allow),
    ("bookmark.remove", Verdict::Allow),
    ("bookmark.toggle", Verdict::Allow),
    ("bookmark.update", Verdict::Allow),
    ("brand.list", Verdict::Allow),
    ("burn_subtitles", Verdict::Refuse(Reason::Export)),
    ("cancel_gesture", Verdict::Refuse(Reason::Gesture)),
    ("captions.add_empty", Verdict::Refuse(Reason::HumanOnly)),
    ("captions.apply", Verdict::Allow),
    ("captions.apply_style", Verdict::Allow),
    ("captions.delete_preset", Verdict::Allow),
    ("captions.generate", Verdict::Allow),
    ("captions.import", Verdict::Refuse(Reason::Path)),
    ("captions.rename_preset", Verdict::Allow),
    ("captions.save_preset", Verdict::Allow),
    ("captions.select_emphasis", Verdict::Allow),
    ("captions.set_emphasis", Verdict::Allow),
    ("captions.set_word_highlight", Verdict::Allow),
    ("captions.suggest_emphasis", Verdict::Allow),
    ("captions.translate", Verdict::Allow),
    ("catalog", Verdict::Refuse(Reason::UseTool)),
    ("chapters.export_sidecars", Verdict::Refuse(Reason::Export)),
    ("chapters.generate", Verdict::Allow),
    ("clip.set_animation", Verdict::Allow),
    ("clipboard.copy", Verdict::Allow),
    ("clipboard.copy_keyframes", Verdict::Allow),
    ("clipboard.copy_mask_points", Verdict::Allow),
    ("clipboard.cut", Verdict::Allow),
    ("clipboard.paste", Verdict::Allow),
    ("clipboard.paste_keyframes", Verdict::Allow),
    ("clipboard.paste_mask_points", Verdict::Allow),
    ("clipboard.read", Verdict::Allow),
    ("clips.export", Verdict::Refuse(Reason::Export)),
    ("clips.suggest", Verdict::Allow),
    ("color.import_lut", Verdict::Refuse(Reason::Path)),
    ("color.register_lut", Verdict::Refuse(Reason::HumanOnly)),
    ("commit_gesture", Verdict::Refuse(Reason::Gesture)),
    ("delete", Verdict::Allow),
    ("dub.cancel", Verdict::Refuse(Reason::HumanOnly)),
    ("dub.generate", Verdict::Refuse(Reason::HumanOnly)),
    ("dub.place", Verdict::Refuse(Reason::Path)),
    ("dub.set_ducking", Verdict::Allow),
    ("dub.set_fit", Verdict::Allow),
    ("effects.add", Verdict::Allow),
    ("effects.remove", Verdict::Allow),
    ("effects.reorder", Verdict::Allow),
    ("effects.toggle", Verdict::Allow),
    ("effects.update_param", Verdict::Allow),
    ("export", Verdict::Refuse(Reason::Export)),
    ("export.clip", Verdict::Refuse(Reason::Export)),
    ("export.clips", Verdict::Refuse(Reason::Export)),
    ("export_subtitles", Verdict::Refuse(Reason::Export)),
    ("fillers.classify", Verdict::Allow),
    ("filters.insert", Verdict::Allow),
    ("fonts.download", Verdict::Refuse(Reason::Download)),
    ("fonts.import", Verdict::Refuse(Reason::Path)),
    ("fonts.list", Verdict::Allow),
    ("fonts.load", Verdict::Allow),
    ("footage.analyze", Verdict::Allow),
    ("import.add_asset", Verdict::Refuse(Reason::Path)),
    ("inspect", Verdict::Refuse(Reason::UseTool)),
    ("job_cancel", Verdict::Allow),
    ("job_status", Verdict::Allow),
    ("keyframes.add", Verdict::Allow),
    ("keyframes.apply_preset", Verdict::Allow),
    ("keyframes.delete_selection", Verdict::Allow),
    ("keyframes.remove", Verdict::Allow),
    ("keyframes.retime", Verdict::Allow),
    ("keyframes.scale_time", Verdict::Allow),
    ("keyframes.set_extrapolation", Verdict::Allow),
    ("keyframes.set_handle", Verdict::Allow),
    ("keyframes.set_handles", Verdict::Allow),
    ("keyframes.set_param", Verdict::Allow),
    ("keyframes.toggle_at", Verdict::Allow),
    ("masks.add", Verdict::Allow),
    ("masks.add_point", Verdict::Allow),
    ("masks.remove", Verdict::Allow),
    ("masks.remove_point", Verdict::Allow),
    ("masks.reorder", Verdict::Allow),
    ("masks.set_path", Verdict::Allow),
    ("masks.update", Verdict::Allow),
    ("media.detect_scenes", Verdict::Allow),
    ("media.generate_clip_thumbnails", Verdict::Allow),
    ("media.generate_waveform", Verdict::Allow),
    ("media.remove", Verdict::Allow),
    ("move_item", Verdict::Allow),
    ("move_items", Verdict::Allow),
    ("music.add_to_timeline", Verdict::Refuse(Reason::Path)),
    ("music.save", Verdict::Refuse(Reason::Library)),
    ("music.search", Verdict::Refuse(Reason::Library)),
    ("music.unsave", Verdict::Refuse(Reason::Library)),
    ("preview", Verdict::Frames),
    ("preview.snapshot", Verdict::Refuse(Reason::Export)),
    ("project.create", Verdict::Refuse(Reason::Project)),
    ("project.delete", Verdict::Refuse(Reason::Project)),
    ("project.duplicate", Verdict::Refuse(Reason::Project)),
    ("project.export_bundle", Verdict::Refuse(Reason::Export)),
    ("project.import_bundle", Verdict::Refuse(Reason::Path)),
    ("project.purge", Verdict::Refuse(Reason::Project)),
    ("project.record_recent_colour", Verdict::Allow),
    ("project.rename", Verdict::Allow),
    ("project.restore", Verdict::Refuse(Reason::Project)),
    ("project.save", Verdict::Refuse(Reason::Save)),
    ("project.save_as", Verdict::Refuse(Reason::Export)),
    ("project.set_background", Verdict::Allow),
    ("project.set_canvas", Verdict::Allow),
    ("project.set_fps", Verdict::Allow),
    ("project.set_view_state", Verdict::Refuse(Reason::ViewState)),
    ("project.thumbnail", Verdict::Refuse(Reason::Project)),
    ("provider_catalog", Verdict::Allow),
    ("redo", Verdict::Refuse(Reason::History)),
    ("reframe.apply", Verdict::Allow),
    ("reframe.clear", Verdict::Allow),
    ("reframe.generate", Verdict::Allow),
    ("reframe.set_manual", Verdict::Allow),
    ("retranslate", Verdict::Allow),
    ("ripple_delete", Verdict::Allow),
    ("scene.create", Verdict::Allow),
    ("scene.delete", Verdict::Allow),
    ("scene.rename", Verdict::Allow),
    ("scene.switch", Verdict::Allow),
    ("set_item_param", Verdict::Allow),
    ("sounds.add_to_timeline", Verdict::Refuse(Reason::Path)),
    ("sounds.save", Verdict::Refuse(Reason::Library)),
    ("sounds.search", Verdict::Refuse(Reason::Library)),
    ("sounds.unsave", Verdict::Refuse(Reason::Library)),
    ("split_at", Verdict::Allow),
    ("stickers.insert", Verdict::Allow),
    ("stickers.search", Verdict::Allow),
    ("text.apply_preset", Verdict::Allow),
    ("text.save_preset", Verdict::Allow),
    ("timeline.add_track", Verdict::Allow),
    ("timeline.align_items", Verdict::Allow),
    ("timeline.apply_speed_preset", Verdict::Allow),
    ("timeline.commit_preview", Verdict::Refuse(Reason::Gesture)),
    ("timeline.discard_preview", Verdict::Refuse(Reason::Gesture)),
    ("timeline.distribute_items", Verdict::Allow),
    ("timeline.duplicate", Verdict::Allow),
    ("timeline.edit", Verdict::Allow),
    ("timeline.extract_audio", Verdict::Allow),
    ("timeline.filler_candidates", Verdict::Allow),
    ("timeline.freeze_frame", Verdict::Refuse(Reason::Export)),
    ("timeline.insert", Verdict::Allow),
    ("timeline.move_items", Verdict::Allow),
    ("timeline.paste_attributes", Verdict::Allow),
    ("timeline.pause_candidates", Verdict::Allow),
    ("timeline.preview", Verdict::Refuse(Reason::Gesture)),
    ("timeline.recover_audio", Verdict::Allow),
    ("timeline.remove_fillers", Verdict::Allow),
    ("timeline.remove_pauses", Verdict::Allow),
    ("timeline.replace_media", Verdict::Allow),
    ("timeline.set_background", Verdict::Allow),
    ("timeline.set_flipped", Verdict::Allow),
    ("timeline.set_hidden", Verdict::Allow),
    ("timeline.set_item_param", Verdict::Allow),
    ("timeline.set_item_param_many", Verdict::Allow),
    ("timeline.set_muted", Verdict::Allow),
    ("timeline.set_reverse", Verdict::Allow),
    ("timeline.set_ripple", Verdict::Allow),
    ("timeline.set_speed", Verdict::Allow),
    ("timeline.set_speed_curve", Verdict::Allow),
    ("timeline.split_all", Verdict::Allow),
    ("timeline.split_left", Verdict::Allow),
    ("timeline.split_right", Verdict::Allow),
    ("timeline.trim", Verdict::Allow),
    ("track.remove", Verdict::Allow),
    ("track.rename", Verdict::Allow),
    ("track.reorder", Verdict::Allow),
    ("track.set_flags", Verdict::Allow),
    ("track.set_hidden", Verdict::Allow),
    ("track.set_locked", Verdict::Allow),
    ("track.set_magnet", Verdict::Allow),
    ("track.set_muted", Verdict::Allow),
    ("transcript.delete_range", Verdict::Allow),
    ("transcript.merge_cues", Verdict::Allow),
    ("transcript.search", Verdict::Allow),
    ("transition.add", Verdict::Allow),
    ("transition.apply_to_all", Verdict::Allow),
    ("transition.move", Verdict::Allow),
    ("transition.remove", Verdict::Allow),
    ("transition.set_duration", Verdict::Allow),
    ("transition.set_param", Verdict::Allow),
    ("trim_end", Verdict::Allow),
    ("trim_start", Verdict::Allow),
    ("undo", Verdict::Refuse(Reason::History)),
];
