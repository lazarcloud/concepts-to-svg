use crate::model::Drawing;
use crate::parse::parse_tree;
use anyhow::{Context, Result, bail};
use std::{fs::File, io::Read};
use zip::ZipArchive;

const THUMBNAIL_PATH: &str = "thumb.jpg";
const MAX_THUMBNAIL_BYTES: u64 = 64 * 1024 * 1024;
const TREE_PATH: &str = "tree.pack";
const METADATA_PATH: &str = "metadata.json";

pub(crate) fn read_thumbnail(archive: &mut ZipArchive<File>) -> Result<Vec<u8>> {
    let mut thumbnail = archive
        .by_name(THUMBNAIL_PATH)
        .context("archive does not contain thumb.jpg")?;
    if thumbnail.size() > MAX_THUMBNAIL_BYTES {
        bail!("thumbnail is larger than the 64 MiB safety limit");
    }

    let mut jpeg = Vec::with_capacity(thumbnail.size() as usize);
    thumbnail
        .by_ref()
        .take(MAX_THUMBNAIL_BYTES + 1)
        .read_to_end(&mut jpeg)
        .context("reading thumb.jpg")?;
    if jpeg.len() as u64 > MAX_THUMBNAIL_BYTES {
        bail!("thumbnail is larger than the 64 MiB safety limit");
    }
    Ok(jpeg)
}

pub(crate) fn parse_vector_archive(archive: &mut ZipArchive<File>) -> Result<Drawing> {
    let metadata: serde_json::Value = {
        let mut file = archive
            .by_name(METADATA_PATH)
            .context("archive does not contain metadata.json")?;
        let mut json = String::new();
        file.read_to_string(&mut json)
            .context("reading metadata.json")?;
        serde_json::from_str(&json).context("parsing metadata.json")?
    };
    let background = metadata
        .get("backgroundSolidColor")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("#ffffff")
        .to_owned();

    let tree = {
        let mut file = archive
            .by_name(TREE_PATH)
            .context("archive does not contain tree.pack")?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).context("reading tree.pack")?;
        rmpv::decode::read_value(&mut std::io::Cursor::new(bytes))
            .context("decoding tree.pack MessagePack")?
    };
    parse_tree(&tree, background, |path| read_resource(archive, path))
}

fn read_resource(archive: &mut ZipArchive<File>, path: &str) -> Result<Vec<u8>> {
    const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
    let mut resource = archive
        .by_name(path)
        .with_context(|| format!("reading embedded image resource {path}"))?;
    if resource.size() > MAX_IMAGE_BYTES {
        bail!("embedded image {path} is larger than the 64 MiB safety limit");
    }
    let mut bytes = Vec::with_capacity(resource.size() as usize);
    resource
        .by_ref()
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading embedded image resource {path}"))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        bail!("embedded image {path} is larger than the 64 MiB safety limit");
    }
    Ok(bytes)
}
