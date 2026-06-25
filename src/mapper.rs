use crate::error::*;
use serde::{Deserialize, Serialize};
use serde_bencode::value::Value;
use serde_bytes::ByteBuf;
use sha1::Digest;
use sha1::Sha1;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

const PIECE_HASH_LEN: usize = 20;

/// Stores length and path parameters in a torrent file
#[derive(Serialize, Deserialize, Debug)]
pub struct TorrentFile {
    pub length: i64,
    pub path: Vec<String>,
}

/// Stores the torrent info present in a torrent file
#[derive(Debug, Serialize, Deserialize)]
pub struct TorrentInfo {
    pub name: String,
    #[serde(rename = "piece length")]
    pub piece_length: i64,
    #[serde(with = "serde_bytes")]
    pub pieces: ByteBuf,
    pub files: Option<Vec<TorrentFile>>,
    pub length: Option<i64>,
    #[serde(default)]
    pub private: i64,
}

/// Stores the Actual MetaData of a torrent file
#[derive(Debug, Serialize, Deserialize)]
pub struct TorrentMetaData {
    #[serde(default)]
    announce: Option<String>,
    #[serde(rename = "announce-list", default)]
    announce_list: Option<Vec<Vec<String>>>,
    azureus_properties: Option<HashMap<String, i64>>,
    #[serde(rename = "created by", default)]
    created_by: Option<String>,
    #[serde(rename = "creation date", default)]
    creation_date: Option<i64>,
    #[serde(default)]
    encoding: Option<String>,
    pub info: TorrentInfo,
    publisher: Option<String>,
    #[serde(rename = "publisher-url", default)]
    publisher_url: Option<String>,
}

impl TorrentMetaData {
    pub fn from_bytes(raw: &[u8]) -> Result<TorrentMetaData> {
        Ok(serde_bencode::from_bytes(&raw)?)
    }

    /// Reads a torrent file and maps ints data to a TorrentMetaData format
    pub fn from_trnt_file(path: impl AsRef<Path>) -> Result<TorrentMetaData> {
        let bytes = std::fs::read(path)?;
        Ok(serde_bencode::from_bytes(&bytes)?)
    }
    pub fn calculate_total_pieces(&self) -> u32 {
        (self.info.pieces.len() / PIECE_HASH_LEN) as u32
    }

    /// gets tracker urls
    pub fn get_tracker_url(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        self.announce
            .iter()
            .chain(self.announce_list.iter().flatten().flatten())
            .filter(|url| seen.insert(url.as_str()))
            .cloned()
            .collect()
    }

    /// Gets the pieces length for each info
    pub fn get_pieces_length(&self) -> i64 {
        self.info.piece_length
    }

    /// Gets pieces hashes stored in Info
    #[allow(dead_code)]
    pub fn get_pieces_hashes(&self) -> Result<Vec<[u8; 20]>> {
        let pieces_bytes = self.info.pieces.as_ref();

        if pieces_bytes.len() % 20 != 0 {
            return Err(TorrentError::InvalidTorrentFile(
                "piece length is not a multiple of 20".to_string(),
            ));
        }

        Ok(pieces_bytes
            .chunks(20)
            .map(|chunk| chunk.try_into().expect("chunk should be exactly 20 bytes"))
            .collect())
    }

    /// Gets the total size of files in a torrent file
    pub fn get_total_size(&self) -> i64 {
        self.info
            .files
            .as_ref()
            .map(|s| s.iter().map(|f| f.length).sum())
            .unwrap_or_else(|| self.info.length.unwrap_or(0))
    }

    /// Gets the file structure for later use
    pub fn get_file_structure(&self) -> Vec<(String, i64)> {
        if let Some(files) = &self.info.files {
            files
                .iter()
                .map(|file| (file.path.join("/"), file.length))
                .collect()
        } else {
            vec![(self.info.name.clone(), self.info.length.unwrap_or(0))]
        }
    }
}

/// Calculates the complete info hash
/// Info bytes is the same byte data that is read through std::fs::read(path)
pub fn calculate_info_hash(raw: &[u8]) -> Result<[u8; 20]> {
    let info_bytes = extract_info_bytes(raw)?;
    let mut hasher = Sha1::new();
    hasher.update(&info_bytes);
    Ok(hasher.finalize().into())
}
fn extract_info_bytes(raw: &[u8]) -> Result<Vec<u8>> {
    let value: Value = serde_bencode::from_bytes(raw)?;
    let Value::Dict(dict) = value else {
        return Err(TorrentError::InvalidTorrentFile("not a dict".into()));
    };
    let info = dict
        .get(b"info".as_slice())
        .ok_or_else(|| TorrentError::InvalidTorrentFile("no info key".into()))?;
    Ok(serde_bencode::to_bytes(info)?)
}

#[cfg(test)]
mod tests {

    use super::*;

    fn load_test_torrent() -> TorrentMetaData {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/ubuntu.iso.torrent");
        TorrentMetaData::from_trnt_file(path).unwrap()
    }

    #[test]
    fn metadata_announce() {
        let result = load_test_torrent();
        assert_eq!(
            &result.announce.unwrap(),
            "https://torrent.ubuntu.com/announce"
        );
    }
    #[test]
    fn metadata_total_pieces_count() {
        let result = load_test_torrent();
        let total = result.calculate_total_pieces();
        assert_eq!(total, 24868);
    }
    #[test]
    fn metadata_info_hash() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/ubuntu.iso.torrent");
        let bytes = std::fs::read(path).unwrap();
        let hash = calculate_info_hash(&bytes).unwrap();
        assert_eq!(
            hex::encode(hash),
            "dafc8c076ca2f3ed376eeae7c76a0d6be2415c45",
        );
    }

    #[test]
    fn metadata_piece_length() {
        assert_eq!(load_test_torrent().get_pieces_length(), 256 * 1024);
    }

    #[test]
    fn metadata_total_size() {
        let size = load_test_torrent().get_total_size();
        assert_eq!(size, 6518974464);
    }

    #[test]
    fn metadata_name() {
        assert_eq!(
            load_test_torrent().info.name,
            "ubuntu-26.04-desktop-amd64.iso"
        );
    }

    #[test]
    fn metadata_tracker_url() {
        assert_eq!(
            load_test_torrent().get_tracker_url(),
            vec![
                "https://torrent.ubuntu.com/announce",
                "https://ipv6.torrent.ubuntu.com/announce"
            ]
        );
    }

    #[test]
    fn metadata_single_file_struct() {
        let files = load_test_torrent().get_file_structure();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "ubuntu-26.04-desktop-amd64.iso");
        assert_eq!(files[0].1, 6518974464);
    }

    #[test]
    fn metadata_pieces_hashes() {
        let hashes = load_test_torrent().get_pieces_hashes().unwrap();
        assert_eq!(hashes.len(), 24868);
    }

    #[test]
    fn metadata_file_error() {
        assert!(TorrentMetaData::from_trnt_file("/doesntexist.torrent").is_err());
    }
}
