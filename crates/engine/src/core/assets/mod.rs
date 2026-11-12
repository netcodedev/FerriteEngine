//! Assets: source files (models, textures, ...) registered in the project
//! under a stable id, with settings stored in a sidecar file next to them.
//! See `docs/asset-pipeline.md` for the overall design.

use std::sync::{Mutex, MutexGuard, OnceLock};

mod database;
mod id;
mod import;
mod meta;
#[cfg(test)]
pub(crate) mod test_models;

pub use database::{AssetDatabase, AssetEntry, AssetError};
pub use id::{AssetId, AssetRef};
pub use meta::{
    AnimationSettings, AssetKind, AssetMeta, ClipInfo, MaterialSettings, ModelSettings,
    TextureSettings, VariantInfo, ASSET_META_VERSION, META_SUFFIX,
};

/// Folder (relative to the working directory) that holds the project's
/// assets.
pub const ASSET_ROOT: &str = "assets";

static DATABASE: OnceLock<Mutex<AssetDatabase>> = OnceLock::new();

/// The project's asset database, rooted at `ASSET_ROOT`. Scanned on first
/// use. Keep the guard short-lived: loading code and the editor share it.
pub fn database() -> MutexGuard<'static, AssetDatabase> {
    DATABASE
        .get_or_init(|| {
            let mut database = AssetDatabase::new(ASSET_ROOT);
            for problem in database.scan() {
                log::warn!("assets: {problem}");
            }
            Mutex::new(database)
        })
        .lock()
        // A panic while holding the guard leaves the data intact enough to
        // keep reading; do not turn it into a second panic.
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
