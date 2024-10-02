use serde::{Deserialize,Serialize};
use std::collections::HashMap;
use linked_hash_set::LinkedHashSet;
use sha1::{Digest,Sha1};
use bencode_encoder::{Encoder};
use serde_json::Value;
use serde_json;
use std::path::Path;
use std::io::BufReader;
use std::fs::File;

#[derive(Serialize,Deserialize,Debug)]
pub struct TorrentFile{
    pub length:i64,
    pub path: Vec<String>
}

#[derive(Debug,Serialize,Deserialize)]
pub struct TorrentInfo {
    pub name: String,
    #[serde(rename = "piece length")]
    pub piece_length: i64,
    pub pieces: String,
    pub files: Option<Vec<TorrentFile>>,
    pub length: Option<i64>,  //  optional
    #[serde(default)]
    pub private: i64,
}
#[derive(Debug,Serialize,Deserialize)]
pub struct TorrentMetaData {
    announce:String,
    #[serde(rename = "announce list")]
    announce_list: Option<Vec<Vec<String>>>,
    azureus_properties: Option<HashMap<String, i64>>,
    #[serde(rename = "created by")]
    created_by: String,
    #[serde(rename="creation date")]
    creation_date: i64,
    encoding: String,
    info: TorrentInfo,
    publisher: String,
    #[serde(rename="publisher-url")]
    publisher_url: String
}

impl TorrentMetaData{
    pub fn from_file(path:&str)->Result<TorrentMetaData,serde_json::error::Error>{
        let file_path = Path::new(path);
        let file = File::open(file_path).expect("Could not open the file");
        let reader = BufReader::new(file);
        
        let torrent_meta_data = serde_json::from_reader(reader).expect("serde could not read the buffer");
        Ok(torrent_meta_data)
    }
    
    pub fn get_tracker_url(&self)-> Vec<String>{
        let mut trackers = LinkedHashSet::new();
        let main_url = self.announce.clone();
        trackers.insert(main_url);
        
        if let Some(secondary_urls) = self.announce_list.clone() {
            for sub_url in secondary_urls {
                for url in sub_url{
                    trackers.insert(url);
                }
            }
        }
        trackers.into_iter().collect()
    }

    pub fn get_pieces_length(&self) -> i64{
        self.info.piece_length
    }
    pub fn get_pieces_hashes(&self)->Vec<[u8;20]>{
        self.info.pieces.as_bytes()
            .chunks(20)
            .map(|chunk| {
                let mut hash = [0u8;20];
                hash.copy_from_slice(chunk);
                hash
            })
            .collect()
    }
    pub fn hash_info(&self) {
        todo!()
    }

    pub fn get_total_size(&self)->i64{
        todo!()
    }
    pub fn get_file_structure(&self)->Vec<(String,i64)>{
        todo!()
    }
}


#[cfg(test)]
mod tests{
    use super::*;
    
    #[test]
    fn test_from_file(){
        let path = "C:\\Users\\Lenovo\\Downloads\\example.json";
        let result = TorrentMetaData::from_file(path).unwrap();
        println!("torrent file publishe: {:?}", result.publisher)
    }
}