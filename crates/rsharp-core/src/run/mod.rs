//! Running what a folder's projects make: the run configurations its launch profiles give, what one of them
//! launches, the browser it opens and the links its output carries. Data and words only; a program started here is
//! started by the window, through `process::spawn`.

pub mod browser;
pub mod command_line;
pub mod configuration;
pub mod link;
pub mod plan;
pub mod profile;

pub use configuration::RunConfiguration;
pub use plan::{BuildStep, LaunchPlan, Properties};
