//! Presents headless command outcomes to the process caller.
mod consent;
mod progress;
mod result;
pub(crate) use consent::{CliConsent, cost_json, cost_label};
pub(crate) use progress::{CliProgress, progress_percent, stage_name, wait_for_job};
pub use result::Output;
pub(crate) use result::{hex, json_path, tick_json};
#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    #[test]
    fn hex_is_lowercase_and_fixed_width() {
        assert_eq!(hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }
}
