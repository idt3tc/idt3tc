//! Helpers shared by all subcommands: errors, string utilities, file system
//! access.

pub mod fs;
pub mod util;

pub use util::{Error, Result};
