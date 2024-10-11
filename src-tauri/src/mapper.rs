use serde::{Deserialize,Serialize};
use std::collections::HashMap;
use linked_hash_set::LinkedHashSet;
use sha1::{Digest};
use sha1::Sha1;
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
    #[serde(rename = "announce-list")]
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


    pub fn get_pieces_length(&self) -> i64{
        self.info.piece_length
    }
    pub fn get_pieces_hashes(&self)->Vec<[u8;20]>{
        let pieces_bytes = self.info.pieces.as_bytes();
        if pieces_bytes.len() % 20 != 0 {
            panic!("The length of the pieces string is not a multiple of 20");
        }
        self.info.pieces.as_bytes()
            .chunks(20)
            .map(|chunk| {
                let mut hash = [0u8;20];
                hash.copy_from_slice(chunk);
                hash
            })
            .collect()
    }

    pub fn get_total_size(&self)->i64{
        if let Some(files) = &self.info.files {
            return files.iter().map(|file| file.length).sum()
        }
        return self.info.length.unwrap_or(0)
    }

    pub fn get_file_structure(&self)->Vec<(String,i64)>{
        if let Some(files) = &self.info.files{
            return files.iter().map(|file| (file.path.join("/"),file.length)).collect();
        }else{
            vec![(self.info.name.clone(),self.info.length.unwrap_or(0))]
        }
    }

    pub fn calculate_info_hash(&self) -> Result<[u8; 20], Box<dyn std::error::Error>> {
        let info_bencoded = serde_bencode::to_bytes(&self.info)?;
        let mut hasher = Sha1::new();
        hasher.update(&info_bencoded);
        Ok(hasher.finalize().into())
    }}


#[cfg(test)]
mod tests{
    use super::*;
    
    #[test]
    fn test_from_file(){
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let result = TorrentMetaData::from_file(path).unwrap();
        println!("torrent file publishe: {:?}", result.publisher)
    }

    #[test]
    fn test_get_pieces_lengthl(){
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let result = TorrentMetaData::from_file(path).unwrap();
        let pieces_length = result.get_pieces_length();
        println!("pieces length: {:?}",pieces_length);
    }

    #[test]
    fn test_get_file_structure(){
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let result = TorrentMetaData::from_file(path).unwrap();
        let file_structure = result.get_file_structure();
        println!("file structure: {:?}",file_structure);
        for file in file_structure{ 
            println!("files seperately:{:?}",file.0)
        }
    }
    #[test]
    fn test_get_tracker_url(){
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let result = TorrentMetaData::from_file(path).unwrap();
        let tracker_url = result.get_tracker_url();
        println!("Tracker url:{:?}",tracker_url);
    }

    #[test]
    fn test_get_total_size(){
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let result = TorrentMetaData::from_file(path).unwrap();
        let total_size = result.get_total_size();
        println!("total_size: {:?}",total_size);
    }

    #[test]
    fn test_get_pieces_hashes(){
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let result = TorrentMetaData::from_file(path).unwrap();
        let pieces_hashes = result.get_pieces_hashes();
        println!("pieces_hashes: {:?}",pieces_hashes);
    }
    #[test]
    fn test_hash_info(){
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let result = TorrentMetaData::from_file(path).unwrap();
        let info_hash = result.calculate_info_hash().unwrap();
        println!("info_hashes: {:?}",info_hash);
    }

}