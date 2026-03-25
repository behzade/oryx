use std::path::Path;

use anyhow::Result;

use super::model::MetadataResolution;

#[derive(Clone, Debug, Default)]
pub struct MetadataWriter;

impl MetadataWriter {
    pub fn new() -> Self {
        Self
    }

    pub fn apply_to_file(&self, _path: &Path, _resolution: &MetadataResolution) -> Result<()> {
        Ok(())
    }
}
