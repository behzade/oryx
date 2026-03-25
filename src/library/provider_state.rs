use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use crate::library::Library;
use crate::provider::ProviderId;

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ProviderRuntimeState {
    pub active_manifest_hash: Option<String>,
    pub active_manifest_toml: Option<String>,
    pub active_display_name: Option<String>,
    pub last_seen_manifest_hash: Option<String>,
    pub last_validation_status: String,
    pub last_validation_error: Option<String>,
}

pub(super) fn initialize_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS provider_runtime_state (
            provider_id TEXT PRIMARY KEY,
            active_manifest_hash TEXT,
            active_manifest_toml TEXT,
            active_display_name TEXT,
            last_seen_manifest_hash TEXT,
            last_validation_status TEXT NOT NULL DEFAULT 'unknown',
            last_validation_error TEXT,
            updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
            validated_at INTEGER
        );
        "#,
    )?;

    Ok(())
}

pub(super) fn load_provider_runtime_state(
    library: &Library,
    provider: ProviderId,
) -> Result<Option<ProviderRuntimeState>> {
    let connection = library.open_connection()?;

    connection
        .query_row(
            r#"
            SELECT
                active_manifest_hash,
                active_manifest_toml,
                active_display_name,
                last_seen_manifest_hash,
                last_validation_status,
                last_validation_error
            FROM provider_runtime_state
            WHERE provider_id = ?1
            "#,
            params![provider.as_str()],
            |row| {
                Ok(ProviderRuntimeState {
                    active_manifest_hash: row.get(0)?,
                    active_manifest_toml: row.get(1)?,
                    active_display_name: row.get(2)?,
                    last_seen_manifest_hash: row.get(3)?,
                    last_validation_status: row.get(4)?,
                    last_validation_error: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

pub(super) fn save_validated_provider_manifest(
    library: &Library,
    provider: ProviderId,
    manifest_hash: &str,
    manifest_toml: &str,
    display_name: &str,
) -> Result<()> {
    let connection = library.open_connection()?;

    connection.execute(
        r#"
        INSERT INTO provider_runtime_state (
            provider_id,
            active_manifest_hash,
            active_manifest_toml,
            active_display_name,
            last_seen_manifest_hash,
            last_validation_status,
            last_validation_error,
            updated_at,
            validated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?2, 'ok', NULL, unixepoch(), unixepoch())
        ON CONFLICT(provider_id) DO UPDATE SET
            active_manifest_hash = excluded.active_manifest_hash,
            active_manifest_toml = excluded.active_manifest_toml,
            active_display_name = excluded.active_display_name,
            last_seen_manifest_hash = excluded.last_seen_manifest_hash,
            last_validation_status = excluded.last_validation_status,
            last_validation_error = excluded.last_validation_error,
            updated_at = unixepoch(),
            validated_at = unixepoch()
        "#,
        params![
            provider.as_str(),
            manifest_hash,
            manifest_toml,
            display_name
        ],
    )?;

    Ok(())
}

pub(super) fn record_provider_validation_failure(
    library: &Library,
    provider: ProviderId,
    candidate_hash: &str,
    error: &str,
) -> Result<()> {
    let connection = library.open_connection()?;

    connection.execute(
        r#"
        INSERT INTO provider_runtime_state (
            provider_id,
            last_seen_manifest_hash,
            last_validation_status,
            last_validation_error,
            updated_at
        )
        VALUES (?1, ?2, 'invalid', ?3, unixepoch())
        ON CONFLICT(provider_id) DO UPDATE SET
            last_seen_manifest_hash = excluded.last_seen_manifest_hash,
            last_validation_status = excluded.last_validation_status,
            last_validation_error = excluded.last_validation_error,
            updated_at = unixepoch()
        "#,
        params![provider.as_str(), candidate_hash, error],
    )?;

    Ok(())
}

pub(super) fn record_provider_validation_pending_auth(
    library: &Library,
    provider: ProviderId,
    candidate_hash: &str,
) -> Result<()> {
    let connection = library.open_connection()?;

    connection.execute(
        r#"
        INSERT INTO provider_runtime_state (
            provider_id,
            last_seen_manifest_hash,
            last_validation_status,
            last_validation_error,
            updated_at
        )
        VALUES (?1, ?2, 'pending_auth', NULL, unixepoch())
        ON CONFLICT(provider_id) DO UPDATE SET
            last_seen_manifest_hash = excluded.last_seen_manifest_hash,
            last_validation_status = excluded.last_validation_status,
            last_validation_error = excluded.last_validation_error,
            updated_at = unixepoch()
        "#,
        params![provider.as_str(), candidate_hash],
    )?;

    Ok(())
}
