use crate::error::*;
use crate::mapper::TorrentMetaData;
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// Decodes the torrent file as a json
#[allow(dead_code)]
pub fn decode_json(bpath: &str, opath: &str) -> Result<()> {
    let input_path = Path::new(bpath);
    let output_path = Path::new(opath);

    if !input_path.exists() {
        return Err(TorrentError::FileNotFound(bpath.to_string()));
    }

    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| {
                TorrentError::PermissionDenied(format!(
                    "Cannot create output directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }
    }

    let torrent_bytes = std::fs::read(input_path).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => {
            TorrentError::PermissionDenied(format!("Cannot read file: {}", bpath))
        }
        std::io::ErrorKind::NotFound => TorrentError::FileNotFound(bpath.to_string()),
        _ => TorrentError::IoError(e),
    })?;

    let torrent: TorrentMetaData = serde_bencode::from_bytes(&torrent_bytes).map_err(|e| {
        TorrentError::InvalidTorrentFile(format!(
            "Bencode decode error in {}: {}",
            input_path.display(),
            e
        ))
    })?;
    let json_content = serde_json::to_string_pretty(&torrent).map_err(|e| {
        TorrentError::InvalidTorrentFile(format!("JSON serialization failed: {}", e))
    })?;

    let mut file = File::create(output_path).map_err(|e| match e.kind() {
        std::io::ErrorKind::PermissionDenied => {
            TorrentError::PermissionDenied(format!("Cannot create file: {}", output_path.display()))
        }
        std::io::ErrorKind::AlreadyExists => {
            TorrentError::InvalidConfigs(format!("File already exists: {}", output_path.display()))
        }
        _ => TorrentError::IoError(e),
    })?;

    file.write_all(json_content.as_bytes())
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::WriteZero => TorrentError::DiskFull,
            std::io::ErrorKind::PermissionDenied => TorrentError::PermissionDenied(format!(
                "Cannot write to file: {}",
                output_path.display()
            )),
            _ => TorrentError::IoError(e),
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
            "/home/rusty/Codes/Fun/Torrs/Violet [FitGirl Repack].torrent",
            "/home/rusty/Codes/Fun/Torrs/Violet [FitGirl Repack].json",
        )
        .unwrap();
    }
}
