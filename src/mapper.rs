use linked_hash_set::LinkedHashSet;
use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use serde_json;
use sha1::Digest;
use sha1::Sha1;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

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
    pub length: Option<i64>, //  optional
    #[serde(default)]
    pub private: i64,
}

/// Stores the Actual MetaData of a torrent file
#[derive(Debug, Serialize, Deserialize)]
pub struct TorrentMetaData {
    announce: String,
    #[serde(rename = "announce-list")]
    announce_list: Option<Vec<Vec<String>>>,
    azureus_properties: Option<HashMap<String, i64>>,
    #[serde(rename = "created by")]
    created_by: String,
    #[serde(rename = "creation date")]
    creation_date: i64,
    encoding: Option<String>,
    pub info: TorrentInfo,
    publisher: Option<String>,
    #[serde(rename = "publisher-url")]
    publisher_url: Option<String>,
}
#[allow(dead_code)]
impl TorrentMetaData {
    /// Reads a json file and maps its data to a TorrentMetaData format
    pub fn from_json_file(path: &str) -> Result<TorrentMetaData, serde_json::error::Error> {
        let file_path = Path::new(path);
        let file = File::open(file_path).expect("Could not open the file");
        let reader = BufReader::new(file);

        let torrent_meta_data =
            serde_json::from_reader(reader).expect("serde could not read the buffer");
        Ok(torrent_meta_data)
    }

    /// Reads a torrent file and maps ints data to a TorrentMetaData format
    pub fn from_trnt_file(path: &str) -> Result<TorrentMetaData, serde_bencode::error::Error> {
        let file_path = Path::new(path);
        // let file = File::open(file_path).expect("failed to open the file");
        let bytes = std::fs::read(file_path).expect("failed to read the file");
        let torrent: TorrentMetaData = serde_bencode::from_bytes(&bytes)
            .map_err(|e| {
                format!(
                    "Failed to decode bencode from {}: {}",
                    file_path.display(),
                    e
                )
            })
            .unwrap();
        Ok(torrent)
    }

    /// gets tracker urls
    pub fn get_tracker_url(&self) -> Vec<String> {
        let mut trackers = LinkedHashSet::new();
        let main_url = self.announce.clone();
        trackers.insert(main_url);

        if let Some(secondary_urls) = &self.announce_list {
            for sub_url in secondary_urls {
                for url in sub_url {
                    trackers.insert(url.clone());
                }
            }
        } else {
            println!("No secondary URLs found."); // Debug print
        }
        trackers.into_iter().collect()
    }

    /// Gets the pieces length for each info
    pub fn get_pieces_length(&self) -> i64 {
        self.info.piece_length
    }

    /// Gets pieces hashes stored in Info
    pub fn get_pieces_hashes(&self) -> Vec<[u8; 20]> {
        let pieces_bytes = self.info.pieces.as_ref();
        println!("length of pieces data: {}", pieces_bytes.len());
        if pieces_bytes.len() % 20 != 0 {
            panic!("The length of the pieces string is not a multiple of 20");
        }
        pieces_bytes
            .chunks(20)
            .map(|chunk| {
                let mut hash = [0u8; 20];
                hash.copy_from_slice(chunk);
                hash
            })
            .collect()
    }

    /// Gets the total size of files in a torrent file
    pub fn get_total_size(&self) -> i64 {
        if let Some(files) = &self.info.files {
            return files.iter().map(|file| file.length).sum();
        }
        return self.info.length.unwrap_or(0);
    }

    /// Gets the file structure for later use
    pub fn get_file_structure(&self) -> Vec<(String, i64)> {
        if let Some(files) = &self.info.files {
            return files
                .iter()
                .map(|file| (file.path.join("/"), file.length))
                .collect();
        } else {
            vec![(self.info.name.clone(), self.info.length.unwrap_or(0))]
        }
    }

    /// Calculates the complete info hash
    pub fn calculate_info_hash(&self) -> Result<[u8; 20], Box<dyn std::error::Error>> {
        let info_bencoded = serde_bencode::to_bytes(&self.info)?;
        let mut hasher = Sha1::new();
        hasher.update(&info_bencoded);
        Ok(hasher.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use crate::tracker::{bytes_to_url_string, urlencode};

    use super::*;

    macro_rules! test_torrent {
        ($name:ident,$method:ident) => {
            #[test]
            fn $name() {
                let path = r"C:\Users\Lenovo\Downloads\ubuntu-24.10-desktop-amd64.iso.torrent";
                let result = TorrentMetaData::from_trnt_file(path).unwrap();
                let output = result.$method();

                println!("{}: {:?}", stringify!($method), output);
            }
        };
    }

    #[test]
    fn test_from_file() {
        let path = r"C:\Users\Lenovo\Downloads\ubuntu-24.10-desktop-amd64.iso.torrent";
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
        println!("URL encoded info hash: {}", bytes_to_url_string(&info_hash));
        println!(
            "Url encoded info hash using earlier funct: {}",
            urlencode(&info_hash)
        );
    }
}
