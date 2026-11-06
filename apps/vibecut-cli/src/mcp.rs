//! Adapts authenticated MCP transports to engine control admission.
mod command;
mod grants;
mod handler;
mod inbox;
mod server;
mod socket;
#[cfg(test)]
mod tests;
mod transport;

pub(super) use command::run;
pub use grants::Grants;
pub use inbox::Inbox;
pub use server::Server;

use handler::Handler;
use inbox::{DEADLINE, Pending};
use transport::{Framed, frame};
