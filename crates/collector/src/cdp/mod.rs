//! Chrome DevTools Protocol client (native impl, no chromiumoxide)

pub mod error;
pub mod commands;
pub mod frame;
// pub mod client;    // added in T4
// pub mod mock;      // added in T4

pub use error::{CdpError, Result};
