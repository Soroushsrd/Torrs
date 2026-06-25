use crate::error::*;
use serde::{Deserialize, Serialize};
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
    pub fn calculate_total_pieces(&self) -> u32 {
        (self.info.pieces.len() / PIECE_HASH_LEN) as u32
    }
    /// Reads a torrent file and maps ints data to a TorrentMetaData format
    pub fn from_trnt_file(path: impl AsRef<Path>) -> Result<TorrentMetaData> {
        let bytes = std::fs::read(path)?;
        Ok(serde_bencode::from_bytes(&bytes)?)
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

    /// Calculates the complete info hash
    pub fn calculate_info_hash(&self) -> Result<[u8; 20]> {
        let info_bencoded = serde_bencode::to_bytes(&self.info)?;
        let mut hasher = Sha1::new();
        hasher.update(&info_bencoded);
        Ok(hasher.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use crate::tracker::urlencode;

    use super::*;

    macro_rules! test_torrent {
        ($name:ident,$method:ident) => {
            #[test]
            fn $name() {
                let path = "/home/rusty/Codes/Fun/Torrs/Violet [FitGirl Repack].torrent";
                let result = TorrentMetaData::from_trnt_file(path).unwrap();
                let output = result.$method();

                println!("{}: {:?}", stringify!($method), output);
            }
        };
    }

    #[test]
    fn test_from_file() {
        let path = "/home/rusty/Codes/Fun/Torrs/Violet [FitGirl Repack].torrent";
        let result = TorrentMetaData::from_trnt_file(path).unwrap();
        println!("torrent file publishe: {:?}", result.publisher)
    }

    test_torrent!(test_get_pieces_length, get_pieces_length);
    test_torrent!(test_get_file_structure, get_file_structure);
    test_torrent!(test_get_tracker_url, get_tracker_url);
    test_torrent!(test_get_total_size, get_total_size);
    test_torrent!(test_get_pieces_hashes, get_pieces_hashes);
    test_torrent!(test_hash_info, calculate_info_hash);
    #[tokio::test]
    async fn debug_info_hash() {
        let path = r"C:\Users\Lenovo\Downloads\ubuntu-24.10-desktop-amd64.iso.torrent";
        let torrent_meta_data = TorrentMetaData::from_trnt_file(path).unwrap();

        let info_hash = torrent_meta_data.calculate_info_hash().unwrap();
        // println!("Raw info hash bytes: {:?}", info_hash);
        println!("URL encoded info hash: {}", urlencode(&info_hash));
        println!(
            "Url encoded info hash using earlier funct: {}",
            urlencode(&info_hash)
        );
    }
}
