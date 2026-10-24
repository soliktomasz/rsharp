//! NuGet as restore sees it: the folder's package sources, their feeds, and changing a project's packages through
//! `dotnet add package`.

pub mod change;
pub mod feed;
pub mod sources;
pub mod version;
