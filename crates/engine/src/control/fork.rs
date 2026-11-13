//! Runs control commands on a fork through the entries a live session uses.
use super::{Caller, ControlResult, Request, Response, Result};
use crate::jobs::JobRunner;
use crate::{Command, Fork};

impl Fork {
    /// [`Session::control_request`](crate::Session::control_request) against the fork's
    /// session: the request names the fork's `session_id` and revision, never the parent's.
    /// Replays of one idempotency key return the stored response and record nothing.
    pub fn control_request(
        &mut self,
        request: Request,
        caller: Caller<'_>,
        runner: &mut JobRunner,
    ) -> Result<Response> {
        self.session_mut().control_request(request, caller, runner)
    }

    /// [`Session::control_command`](crate::Session::control_command) against the fork's
    /// session, for a caller that holds a typed command.
    pub fn control_command(
        &mut self,
        command: Command,
        caller: Caller<'_>,
        runner: &mut JobRunner,
    ) -> Result<ControlResult> {
        self.session_mut().control_command(command, caller, runner)
    }
}
