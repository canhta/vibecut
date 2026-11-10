//! Repository policy gates: crate edges, assets, names, modules and documentation.
pub(crate) mod assets;
pub mod docs;
pub(crate) mod edges;
pub mod modules;
pub(crate) mod names;
pub(crate) mod scaffold;
#[cfg(test)]
mod tests;

use std::path::Path;

/// Every policy gate in the order `cargo xtask checks` reports them.
pub fn check(root: &Path) -> Result<(), String> {
    edges::layers(root)?;
    assets::check(root)?;
    names::check(root)?;
    modules::check(root)?;
    docs::check(root)
}
