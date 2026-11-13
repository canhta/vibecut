//! Serves the MCP tool surface to local clients over authenticated transports.
mod error;
mod grants;
mod handler;
mod inbox;
mod server;
mod socket;
#[cfg(test)]
mod tests;
mod transport;

pub use error::Error;
pub use grants::Grants;
pub use handler::Handler;
pub use inbox::{Inbox, StickerLend};
pub use server::Server;
pub use socket::proxy;
pub use transport::Framed;

use inbox::{DEADLINE, Pending};
use transport::frame;
