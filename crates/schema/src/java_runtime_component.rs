use std::{path::Path, sync::Arc};

use indexmap::IndexMap;
use serde::Deserialize;
use ustr::Ustr;

#[derive(Deserialize, Clone, Debug)]
pub struct JavaRuntimeComponentManifest {
    pub files: IndexMap<Arc<Path>, JavaRuntimeComponentFile>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
pub enum JavaRuntimeComponentFile {
    Directory,
    File {
        executable: bool,
        downloads: JavaRuntimeComponentFileDownloads,
    },
    Link {
        target: Arc<Path>,
    },
}

#[derive(Deserialize, Clone, Debug)]
pub struct JavaRuntimeComponentFileDownloads {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lzma: Option<JavaRuntimeComponentFileDownload>,
    pub raw: JavaRuntimeComponentFileDownload,
}

#[derive(Deserialize, Clone, Debug)]
pub struct JavaRuntimeComponentFileDownload {
    pub sha1: Ustr,
    pub size: u32,
    pub url: Ustr,
}
