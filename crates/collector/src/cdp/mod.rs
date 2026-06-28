//! Chrome DevTools Protocol client (native impl, no chromiumoxide)

pub mod error;
// pub mod commands;  // added in T2
// pub mod frame;     // added in T3
// pub mod client;    // added in T4
// pub mod mock;      // added in T4

pub use error::{CdpError, Result};
