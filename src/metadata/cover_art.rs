use anyhow::Result;

#[derive(Clone, Debug, Default)]
pub struct CoverArtClient;

impl CoverArtClient {
    pub fn new() -> Self {
        Self
    }

    pub fn fetch_release_artwork_url(&self, release_id: &str) -> Result<Option<String>> {
        Ok(Some(format!(
            "https://coverartarchive.org/release/{release_id}/front-250"
        )))
    }
}
