//! Defines native argument interpretation at the CLI boundary.
mod errors;
mod parsing;
pub use errors::CliError;
pub(crate) use parsing::{
    HELP, number_arg, one_path, one_utf8, parse_i64, parse_selection, parse_u32, parse_u64,
    seconds_arg, seconds_ticks, translation_required, utf8,
};
#[cfg(test)]
mod exit_codes {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use std::io;
    #[test]
    fn exit_codes_are_stable() {
        assert_eq!(CliError::Usage("".into()).exit_code(), 2);
        assert_eq!(CliError::Project(project::Error::Busy).exit_code(), 3);
        assert_eq!(CliError::Project(project::Error::Conflict).exit_code(), 3);
        assert_eq!(
            CliError::Render(render::Error::Unavailable(Vec::new())).exit_code(),
            4
        );
        assert_eq!(CliError::Rejected("").exit_code(), 5);
        assert_eq!(CliError::Render(render::Error::Cancelled).exit_code(), 6);
        assert_eq!(
            CliError::Media(media::MediaError::Invalid("")).exit_code(),
            6
        );
        assert_eq!(CliError::Io(io::Error::other("")).exit_code(), 7);
    }
}

#[cfg(test)]
mod translation {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use engine::subtitles::LanguageTag;
    #[test]
    fn translation_is_needed_only_for_different_languages_or_bilingual() {
        let vi = LanguageTag::parse("vi").unwrap();
        let en = LanguageTag::parse("en").unwrap();
        assert!(!translation_required(&vi, &vi, false));
        assert!(translation_required(&vi, &en, false));
        assert!(translation_required(&vi, &vi, true));
    }
}
