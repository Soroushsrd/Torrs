use crate::error::*;
use crate::mapper::TorrentMetaData;
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Decodes the torrent file to a json
pub fn decode_json(bpath: &str, opath: &str) -> Result<()> {
    let input_path = Path::new(bpath);
    let output_path = Path::new(opath);

    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let torrent_bytes = std::fs::read(input_path)?;
    let torrent: TorrentMetaData = serde_bencode::from_bytes(&torrent_bytes)?;
    let json_content = serde_json::to_string_pretty(&torrent)?;
    let mut file = File::create(output_path)?;

    file.write_all(json_content.as_bytes())?;

    println!(
        "Successfully decoded torrent file to JSON at: {}",
        output_path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_json() {
        decode_json(
            "/home/rusty/Codes/Fun/Torrs/Violet [FitGirl Repack].torrent",
            "/home/rusty/Codes/Fun/Torrs/Violet [FitGirl Repack].json",
        )
        .unwrap();
    }
}
