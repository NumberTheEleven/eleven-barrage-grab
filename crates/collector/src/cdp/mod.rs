//! Chrome DevTools Protocol client (native impl, no chromiumoxide)

pub mod error;
pub mod commands;
pub mod frame;
pub mod client;
pub mod mock;

pub use error::{CdpError, Result};
pub use mock::CdpTransport;
