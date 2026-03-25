use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

use super::super::{MusicProvider, ProviderId, SharedProvider};
use super::config::ProviderManifest;
use super::provider::{ConfiguredProvider, SmokeTestStatus};
use crate::library::{Library, ProviderRuntimeState};

const PROVIDER_LINK_PREFIX: &str = "oryx-provider://v1/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConfiguredProviderImportStatus {
    Activated,
    PendingAuth,
    RevertedToLastValidated,
}

#[derive(Clone, Debug)]
pub(crate) struct ConfiguredProviderImport {
    pub provider_id: ProviderId,
    pub status: ConfiguredProviderImportStatus,
}

pub(super) fn load_configured_providers(library: Option<&Library>) -> Result<Vec<SharedProvider>> {
    let provider_dir = provider_directory()?;
    let bundled_dirs = bundled_provider_directories()?;
    load_configured_providers_from_sources(&provider_dir, &bundled_dirs, library)
}

pub(super) fn load_configured_providers_from_dir(
    provider_dir: &Path,
    library: Option<&Library>,
) -> Result<Vec<SharedProvider>> {
    load_configured_providers_from_sources(provider_dir, &[], library)
}

pub(super) fn load_configured_providers_from_sources(
    provider_dir: &Path,
    bundled_dirs: &[PathBuf],
    library: Option<&Library>,
) -> Result<Vec<SharedProvider>> {
    let mut providers: Vec<SharedProvider> = Vec::new();
    let mut seen_ids = HashSet::from([ProviderId::Local.as_str().to_string()]);

    fs::create_dir_all(provider_dir).with_context(|| {
        format!(
            "failed to create provider directory {}",
            provider_dir.display()
        )
    })?;

    load_directory(provider_dir, library, &mut seen_ids, &mut providers)?;

    for bundled_dir in bundled_dirs {
        if !bundled_dir.exists() {
            continue;
        }
        load_directory(bundled_dir, library, &mut seen_ids, &mut providers)?;
    }

    Ok(providers)
}

pub(super) fn provider_directory() -> Result<PathBuf> {
    if let Ok(directory) = env::var("ORYX_PROVIDER_DIR") {
        return Ok(PathBuf::from(directory));
    }

    let root = dirs::home_dir()
        .map(|home| home.join(".config"))
        .or_else(dirs::config_dir)
        .ok_or_else(|| anyhow!("failed to resolve Oryx config directory"))?;

    Ok(root.join("oryx").join("providers"))
}

fn bundled_provider_directories() -> Result<Vec<PathBuf>> {
    let mut directories = Vec::new();

    if let Ok(directory) = env::var("ORYX_BUNDLED_PROVIDER_DIR") {
        directories.push(PathBuf::from(directory));
    }

    directories.push(PathBuf::from("bundled").join("providers"));

    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            directories.push(parent.join("bundled").join("providers"));
        }
    }

    let mut unique = Vec::new();
    let mut seen = HashSet::new();
    for directory in directories {
        let key = directory.to_string_lossy().into_owned();
        if seen.insert(key) {
            unique.push(directory);
        }
    }

    Ok(unique)
}

fn load_provider_manifest(path: &Path) -> Result<ProviderManifest> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    load_provider_manifest_from_str(&contents, path)
}

fn load_provider_manifest_from_str(contents: &str, path: &Path) -> Result<ProviderManifest> {
    let manifest: ProviderManifest =
        toml::from_str(contents).with_context(|| format!("invalid TOML in {}", path.display()))?;
    manifest.validate(path)?;
    Ok(manifest)
}

fn file_name_matches_id(path: &Path, id: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name == format!("{id}.toml"))
        .unwrap_or(false)
}

fn load_directory(
    directory: &Path,
    library: Option<&Library>,
    seen_ids: &mut HashSet<String>,
    providers: &mut Vec<SharedProvider>,
) -> Result<()> {
    let mut manifest_paths = fs::read_dir(directory)
        .with_context(|| format!("failed to read provider directory {}", directory.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| extension.eq_ignore_ascii_case("toml"))
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    manifest_paths.sort();

    for manifest_path in manifest_paths {
        let Some(provider_id) = provider_id_from_path(&manifest_path) else {
            eprintln!(
                "skipping configured provider {} because the filename is not a valid provider id",
                manifest_path.display()
            );
            continue;
        };
        if !seen_ids.insert(provider_id.as_str().to_string()) {
            eprintln!(
                "skipping configured provider '{}' from {} because that id is already registered",
                provider_id,
                manifest_path.display()
            );
            continue;
        }

        match load_provider_from_candidate_path(library, &manifest_path, provider_id)? {
            Some(provider) => providers.push(provider),
            None => {}
        }
    }

    Ok(())
}

fn provider_id_from_path(path: &Path) -> Option<ProviderId> {
    path.file_stem()
        .and_then(|name| name.to_str())
        .and_then(ProviderId::parse)
}

fn manifest_hash(contents: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(contents.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn export_provider_link(
    library: Option<&Library>,
    provider_id: ProviderId,
) -> Result<String> {
    let manifest_toml = load_runtime_state(library, provider_id)?
        .and_then(|state| state.active_manifest_toml)
        .map(Ok)
        .unwrap_or_else(|| read_manifest_toml_from_dir(&provider_directory()?, provider_id))?;
    export_provider_link_from_toml(&manifest_toml)
}

pub(crate) fn import_provider_link(
    library: &Library,
    encoded: &str,
) -> Result<ConfiguredProviderImport> {
    import_provider_link_into_dir(library, encoded, &provider_directory()?)
}

pub(super) fn export_provider_link_from_toml(manifest_toml: &str) -> Result<String> {
    let compressed = zstd::stream::encode_all(Cursor::new(manifest_toml.as_bytes()), 3)
        .context("failed to compress provider config")?;
    let encoded = URL_SAFE_NO_PAD.encode(compressed);
    Ok(format!("{PROVIDER_LINK_PREFIX}{encoded}"))
}

pub(super) fn import_provider_link_into_dir(
    library: &Library,
    encoded: &str,
    provider_dir: &Path,
) -> Result<ConfiguredProviderImport> {
    let manifest_toml = decode_provider_link(encoded)?;
    let manifest = load_provider_manifest_from_str(&manifest_toml, Path::new("<provider-link>"))?;
    let provider_id = ProviderId::parse(&manifest.id)
        .ok_or_else(|| anyhow!("provider id {:?} is invalid", manifest.id))?;
    fs::create_dir_all(&provider_dir).with_context(|| {
        format!(
            "failed to create provider directory {}",
            provider_dir.display()
        )
    })?;
    let provider_path = provider_dir.join(format!("{}.toml", manifest.id));
    fs::write(&provider_path, manifest_toml.as_bytes()).with_context(|| {
        format!(
            "failed to write imported provider config to {}",
            provider_path.display()
        )
    })?;

    let candidate_hash = manifest_hash(&manifest_toml);
    let runtime_before = library.load_provider_runtime_state(provider_id)?;
    let loaded = load_provider_from_candidate_path(Some(library), &provider_path, provider_id)?;
    if loaded.is_none() && runtime_before.is_none() {
        anyhow::bail!(
            "provider '{}' could not be activated from the imported config",
            provider_id
        );
    }

    let runtime_after = library
        .load_provider_runtime_state(provider_id)?
        .ok_or_else(|| {
            anyhow!(
                "provider '{}' did not record runtime state after import",
                provider_id
            )
        })?;
    let status = if runtime_after.active_manifest_hash.as_deref() == Some(candidate_hash.as_str()) {
        ConfiguredProviderImportStatus::Activated
    } else if runtime_after.last_validation_status == "pending_auth" {
        ConfiguredProviderImportStatus::PendingAuth
    } else {
        ConfiguredProviderImportStatus::RevertedToLastValidated
    };

    Ok(ConfiguredProviderImport {
        provider_id,
        status,
    })
}

fn decode_provider_link(encoded: &str) -> Result<String> {
    let trimmed = encoded.trim();
    if trimmed.starts_with(PROVIDER_LINK_PREFIX) {
        let payload = &trimmed[PROVIDER_LINK_PREFIX.len()..];
        let compressed = URL_SAFE_NO_PAD
            .decode(payload.as_bytes())
            .context("provider link is not valid base64url data")?;
        let decompressed = zstd::stream::decode_all(Cursor::new(compressed))
            .context("provider link payload could not be decompressed")?;
        return String::from_utf8(decompressed).context("provider link payload is not valid UTF-8");
    }

    Ok(trimmed.to_string())
}

fn read_manifest_toml_from_dir(provider_dir: &Path, provider_id: ProviderId) -> Result<String> {
    let path = provider_dir.join(format!("{}.toml", provider_id.as_str()));
    fs::read_to_string(&path)
        .with_context(|| format!("failed to read provider config {}", path.display()))
}

fn load_provider_from_candidate_path(
    library: Option<&Library>,
    path: &Path,
    provider_id: ProviderId,
) -> Result<Option<SharedProvider>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) => {
            eprintln!(
                "failed to read provider manifest {}: {error}",
                path.display()
            );
            return load_active_provider_fallback(library, provider_id, path, None, None);
        }
    };
    let candidate_hash = manifest_hash(&contents);
    let runtime_state = load_runtime_state(library, provider_id)?;

    let manifest = match load_provider_manifest_from_str(&contents, path) {
        Ok(manifest) => manifest,
        Err(error) => {
            let message = format!(
                "failed to parse provider manifest {}: {error}",
                path.display()
            );
            eprintln!("{message}");
            return load_active_provider_fallback(
                library,
                provider_id,
                path,
                Some(&candidate_hash),
                Some(message.as_str()),
            );
        }
    };

    if !file_name_matches_id(path, &manifest.id) {
        let message = format!(
            "skipping configured provider {} because filename must match id '{}.toml'",
            path.display(),
            manifest.id
        );
        eprintln!("{message}");
        return load_active_provider_fallback(
            library,
            provider_id,
            path,
            Some(&candidate_hash),
            Some(message.as_str()),
        );
    }

    if runtime_state
        .as_ref()
        .and_then(|state| state.active_manifest_hash.as_deref())
        == Some(candidate_hash.as_str())
    {
        return instantiate_provider(library, manifest).map(Some);
    }

    let provider = instantiate_configured_provider(library, manifest.clone())?;
    match provider.validate_configuration() {
        Ok(SmokeTestStatus::Passed | SmokeTestStatus::Skipped) => {
            if let Some(library) = library {
                library.save_validated_provider_manifest(
                    provider.id(),
                    &candidate_hash,
                    &contents,
                    &manifest.display_name,
                )?;
            }
            Ok(Some(Arc::new(provider) as Arc<dyn MusicProvider>))
        }
        Ok(SmokeTestStatus::PendingAuth) => {
            if let Some(library) = library {
                library.record_provider_validation_pending_auth(provider.id(), &candidate_hash)?;
            }
            if runtime_state
                .as_ref()
                .and_then(|state| state.active_manifest_toml.as_deref())
                .is_some()
            {
                eprintln!(
                    "provider '{}' has an updated manifest at {} but validation is pending authentication; using the last validated config",
                    provider_id,
                    path.display()
                );
                load_active_provider_fallback(
                    library,
                    provider_id,
                    path,
                    Some(&candidate_hash),
                    None,
                )
            } else {
                Ok(Some(Arc::new(provider) as Arc<dyn MusicProvider>))
            }
        }
        Err(error) => {
            let message = format!(
                "provider '{}' failed validation from {}: {error}",
                provider_id,
                path.display()
            );
            eprintln!("{message}");
            load_active_provider_fallback(
                library,
                provider_id,
                path,
                Some(&candidate_hash),
                Some(message.as_str()),
            )
        }
    }
}

fn load_active_provider_fallback(
    library: Option<&Library>,
    provider_id: ProviderId,
    path: &Path,
    candidate_hash: Option<&str>,
    error_message: Option<&str>,
) -> Result<Option<SharedProvider>> {
    if let Some(library) = library {
        if let Some(candidate_hash) = candidate_hash {
            let message = error_message.unwrap_or("provider validation failed");
            library.record_provider_validation_failure(provider_id, candidate_hash, message)?;
        }

        if let Some(runtime_state) = library.load_provider_runtime_state(provider_id)? {
            if let Some(active_manifest_toml) = runtime_state.active_manifest_toml {
                let manifest = load_provider_manifest_from_str(&active_manifest_toml, path)
                    .with_context(|| {
                        format!(
                            "failed to restore last validated provider manifest for '{}'",
                            provider_id
                        )
                    })?;
                eprintln!(
                    "using last validated provider config for '{}' after rejecting {}",
                    provider_id,
                    path.display()
                );
                return instantiate_provider(Some(library), manifest).map(Some);
            }
        }
    }

    Ok(None)
}

fn load_runtime_state(
    library: Option<&Library>,
    provider_id: ProviderId,
) -> Result<Option<ProviderRuntimeState>> {
    match library {
        Some(library) => library.load_provider_runtime_state(provider_id),
        None => Ok(None),
    }
}

fn instantiate_provider(
    library: Option<&Library>,
    manifest: ProviderManifest,
) -> Result<SharedProvider> {
    Ok(Arc::new(instantiate_configured_provider(library, manifest)?) as Arc<dyn MusicProvider>)
}

fn instantiate_configured_provider(
    library: Option<&Library>,
    manifest: ProviderManifest,
) -> Result<ConfiguredProvider> {
    let provider = ConfiguredProvider::from_manifest(manifest)?;
    if let Some(library) = library {
        if let Ok(Some(serialized)) = library.load_provider_auth(provider.id()) {
            let _ = provider.restore_credentials(&serialized);
        }
    }
    Ok(provider)
}
