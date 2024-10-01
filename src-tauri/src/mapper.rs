use serde::{Deserialize,Serialize};
use std::collections::HashMap;
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