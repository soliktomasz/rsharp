//! A real .nupkg written by a test, for feeds and restores that never reach nuget.org.

use std::path::Path;

/// A real .nupkg: a zip with its .nuspec at the root.
pub fn nupkg(path: &Path, id: &str, version: &str) {
    use async_zip::base::write::ZipFileWriter;
    use async_zip::{Compression, ZipEntryBuilder};
    let nuspec = format!(
        r#"<?xml version="1.0" encoding="utf-8"?><package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd"><metadata><id>{id}</id><version>{version}</version><authors>Folk</authors><description>About {id} &amp; more</description></metadata></package>"#
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = Vec::new();
    futures::executor::block_on(async {
        let mut zip = ZipFileWriter::new(futures::io::Cursor::new(&mut bytes));
        zip.write_entry_whole(
            ZipEntryBuilder::new(format!("{id}.nuspec").into(), Compression::Deflate),
            nuspec.as_bytes(),
        )
        .await
        .unwrap();
        zip.close().await.unwrap();
    });
    std::fs::write(path, bytes).unwrap();
}
