mod config;
mod listing;
mod loader;
mod provider;
mod search;
mod song;
#[cfg(test)]
mod tests;
mod util;

use anyhow::Result;

use crate::library::Library;

use super::{ProviderId, SharedProvider};

pub(crate) use loader::{ConfiguredProviderImport, ConfiguredProviderImportStatus};

pub(super) fn load_configured_providers(library: Option<&Library>) -> Result<Vec<SharedProvider>> {
    loader::load_configured_providers(library)
}

pub(crate) fn import_provider_link(
    library: &Library,
    encoded: &str,
) -> Result<ConfiguredProviderImport> {
    loader::import_provider_link(library, encoded)
}

pub(crate) fn export_provider_link(library: &Library, provider_id: ProviderId) -> Result<String> {
    loader::export_provider_link(Some(library), provider_id)
}
