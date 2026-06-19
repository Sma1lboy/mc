//! Mod-loader installation. A loader install boils down to "obtain the loader's
//! version json (which `inheritsFrom` the vanilla version) and drop it on disk";
//! after that it is just another component the version system merges. See
//! `docs/modules/version-system.md`.

pub mod fabric;
pub mod forge;
pub mod installer;
pub mod neoforge;
pub mod quilt;

pub use fabric::install_fabric;
pub use forge::install_forge;
pub use neoforge::install_neoforge;
pub use quilt::install_quilt;
