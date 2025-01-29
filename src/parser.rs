use sha1::{Digest, Sha1};
use std::fs::File;
use std::io::Write;
use std::path::Path;

use crate::mapper::TorrentMetaData;

/// Calculates infor hash
pub fn calculate_info_hash(torrent_path: &str) -> Result<[u8; 20], Box<dyn std::error::Error>> {
    let path = Path::new(torrent_path);
    if !path.exists() {
        return Err(format!("Input file does not exist: {}", torrent_path).into());
    }

    let torrent_bytes = std::fs::read(path)?;
    let torrent: TorrentMetaData = serde_bencode::from_bytes(&torrent_bytes)?;

    // Re-encode just the info dictionary to calculate its hash
    let info_bytes = serde_bencode::to_bytes(&torrent.info)?;
    let mut hasher = Sha1::new();
    hasher.update(&info_bytes);

    Ok(hasher.finalize().into())
}

// fix the pieces part!
/// Decodes the torrent file as a json
pub fn decode_json(bpath: &str, opath: &str) -> Result<(), Box<dyn std::error::Error>> {
    let input_path = Path::new(bpath);
    let output_path = Path::new(opath);

    // Validate input path
    if !input_path.exists() {
        return Err(format!("Input file does not exist: {}", bpath).into());
    }

    // Validate output directory exists
    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            return Err(format!("Output directory does not exist: {}", parent.display()).into());
        }
    }

    // Read torrent file
    let torrent_bytes = std::fs::read(input_path).map_err(|e| {
        format!(
            "Failed to read torrent file {}: {}",
            input_path.display(),
            e
        )
    })?;

    // Decode bencode to your existing TorrentMetaData struct
    let torrent: TorrentMetaData = serde_bencode::from_bytes(&torrent_bytes).map_err(|e| {
        format!(
            "Failed to decode bencode from {}: {}",
            input_path.display(),
            e
        )
    })?;

    // Convert to JSON
    let json_content = serde_json::to_string_pretty(&torrent)
        .map_err(|e| format!("Failed to serialize torrent data to JSON: {}", e))?;

    // Write to output file
    let mut file = File::create(output_path).map_err(|e| {
        format!(
            "Failed to create output file {}: {}",
            output_path.display(),
            e
        )
    })?;

    file.write_all(json_content.as_bytes()).map_err(|e| {
        format!(
            "Failed to write JSON content to {}: {}",
            output_path.display(),
            e
        )
    })?;

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
            r"C:\Users\Lenovo\Downloads\ubuntu-24.10-desktop-amd64.iso.torrent",
            r"C:\Users\Lenovo\Downloads\ubuntu-24.10-desktop-amd64.json",
        )
        .unwrap();
    }
}
