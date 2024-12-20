use crate::{
    mapper::TorrentMetaData,
    peer::{Peer, PeerInfo},
    tracker::{generate_peer_id, request_peers, request_tracker},
};
use data_encoding::BASE32;
use futures::future::join_all;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use sha1::{
    digest::{crypto_common::KeyInit, Update},
    Digest, Sha1,
};
use std::path::Path;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::time::timeout;
use url::Url;

const METADATA_PIECE_SIZE: usize = 16384; //metadata is chunked into 16kb pieces!
const EXTENSION_HANDSHAKE_ID: usize = 0;
const METADATA_EXTENSION_ID: usize = 1;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PIECE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MagnetInfo {
    pub info_hash: [u8; 20],
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
    pub peers: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExtensionHandshake {
    m: HashMap<String, i64>,
    metadata_size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    v: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetaDataMessage {
    msg_type: i64,
    piece: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_size: Option<i64>,
}

#[allow(dead_code)]
impl MagnetInfo {
    pub async fn to_torrent_metadata(&self) -> Result<TorrentMetaData, Box<dyn std::error::Error>> {
        let mut metadata = None;
        for tracker in &self.trackers {
            match request_tracker(tracker, &self.info_hash, 0).await {
                Ok(peer) => {
                    if let Ok(data) = self.fetch_metadata_from_peers(&peer).await {
                        metadata = Some(data);
                        break;
                    }
                }
                Err(e) => {
                    println!("FAiled to get peers from tracker {}: {}", tracker, e);
                    continue;
                }
            }
        }
        let metadata_bytes = metadata.ok_or("Failed to get metadata bytes from trackers")?;
        let torrent_metadata: TorrentMetaData =
            serde_bencode::from_bytes(&metadata_bytes).expect("Failed to extract torrent metadata");
        Ok(torrent_metadata)
    }
    pub async fn fetch_metadata_from_peer(
        &self,
        peer_info: &PeerInfo,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut peer = Peer::new(peer_info.ip.clone(), peer_info.port);
        peer.connect().await.unwrap();

        let mut extension_bits = [0u8; 8];
        extension_bits[5] |= 0x10;
        let peer_id = generate_peer_id();

        peer.handshake(self.info_hash, peer_id).await.unwrap();

        let mut extension_handshake = HashMap::new();
        extension_handshake
            .insert("ut_metadata".to_string(), METADATA_EXTENSION_ID as i64)
            .unwrap();

        let mut handshake_msg = ExtensionHandshake {
            m: extension_handshake,
            metadata_size: None,
            v: Some("RU0001".to_string()),
        };

        let handshake_bytes = serde_bencode::to_bytes(&mut handshake_msg).unwrap();
        self.send_extension_message(&mut peer, EXTENSION_HANDSHAKE_ID as u8, &handshake_bytes)
            .await?;

        let response = timeout(HANDSHAKE_TIMEOUT, peer.receive_msg())
            .await
            .unwrap()?;

        if response[0] != 20 {
            return Err("Invalid Extension message".into());
        }

        let handshake_resp: ExtensionHandshake = serde_bencode::from_bytes(&response[2..])
            .map_err(|e| format!("Failed handshake mapping :{}", e))
            .unwrap();

        let metadata_size = handshake_resp
            .metadata_size
            .ok_or("No metadata received!")
            .unwrap();
        let num_pieces =
            (metadata_size + METADATA_PIECE_SIZE as i64 - 1) / (METADATA_PIECE_SIZE as i64);

        // Request all metadata pieces
        let mut metadata = vec![0u8; metadata_size as usize];

        for piece in 0..num_pieces {
            let msg = MetaDataMessage {
                msg_type: 0, // request
                piece,
                total_size: None,
            };

            let msg_bytes = serde_bencode::to_bytes(&msg)?;
            self.send_extension_message(&mut peer, METADATA_EXTENSION_ID as u8, &msg_bytes)
                .await?;

            // Wait for piece response
            let piece_data = timeout(PIECE_TIMEOUT, peer.receive_msg()).await??;

            if piece_data[0] != 20 {
                return Err("Invalid metadata piece response".into());
            }

            // Extract and validate piece data
            let start = piece as usize * METADATA_PIECE_SIZE;
            let end = std::cmp::min(start + METADATA_PIECE_SIZE, metadata_size as usize);
            metadata[start..end].copy_from_slice(&piece_data[2..end - start + 2]);
        }

        // Verify metadata hash matches info_hash
        let mut hasher = Sha1::new();
        Digest::update(&mut hasher, &metadata);
        //hasher.update(&metadata);
        let hash: [u8; 20] = hasher.finalize().into();

        if hash != self.info_hash {
            return Err("Metadata hash mismatch".into());
        }

        Ok(metadata)
    }
    pub async fn fetch_metadata_from_peers(
        &self,
        peers: &[PeerInfo],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let max_concurrent = 10;

        let mut handles = Vec::with_capacity(std::cmp::min(peers.len(), max_concurrent));

        for peer in peers.iter().take(max_concurrent) {
            let peer_info = peer.clone();
            let self_clone = self.clone(); // Implement Clone for MagnetInfo if needed

            let handle = tokio::spawn(async move {
                match self_clone.fetch_metadata_from_peer(&peer_info).await {
                    Ok(metadata) => Some((peer_info.clone(), metadata)),
                    Err(_) => None,
                }
            });
            handles.push(handle);
        }

        let results = join_all(handles).await;

        let mut valid_metadata = HashSet::new();
        let mut metadata_count = HashMap::new();

        for result in results {
            if let Ok(Some((_peer_info, metadata))) = result {
                let metadata_hash = {
                    let mut hasher = Sha1::new();
                    Digest::update(&mut hasher, &metadata);
                    hasher.finalize().to_vec()
                };
                valid_metadata.insert((metadata_hash.clone(), metadata.clone()));
                *metadata_count.entry(metadata_hash).or_insert(0) += 1
            }
        }
        if let Some((most_common_hash, _)) = metadata_count.iter().max_by_key(|(_, &count)| count) {
            if let Some((_, metadata)) = valid_metadata
                .iter()
                .find(|(hash, _)| hash == most_common_hash)
            {
                return Ok(metadata.clone());
            }
        }
        Err("failed to get consistent emtadata from peers".into())
    }

    async fn send_extension_message(
        &self,
        peer: &mut Peer,
        extension_id: u8,
        payload: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let msg_len = 2 + payload.len();
        let mut message = Vec::with_capacity(4 + msg_len);

        message.extend_from_slice(&(msg_len as u32).to_be_bytes());
        message.push(20); // Extension message ID
        message.push(extension_id);
        message.extend_from_slice(payload);

        peer.send_msg(&message).await?;
        Ok(())
    }
    pub fn parse(magnet_url: &str) -> Result<MagnetInfo, Box<dyn std::error::Error>> {
        let url = Url::parse(magnet_url).unwrap();
        let params: HashMap<_, _> = url.query_pairs().collect();

        let info_hash = if let Some(xt) = params.get("xt") {
            if let Some(hash) = xt.strip_prefix("urn:btih:") {
                if hash.len() == 40 {
                    let mut result = [0u8; 20];
                    hex::decode_to_slice(hash, &mut result).unwrap();
                    result
                } else if hash.len() == 32 {
                    let mut result = [0u8; 20];
                    let decoded = BASE32.decode(hash.as_bytes()).unwrap();
                    result.copy_from_slice(&decoded);
                    result
                } else {
                    return Err("Invalid info hash".into());
                }
            } else {
                return Err("Invalid xt parameter".into());
            }
        } else {
            return Err("Missing xt parameter".into());
        };

        let display_name = params
            .get("dn")
            .map(|dn| percent_decode_str(dn).decode_utf8_lossy().into_owned());

        let trackers = params
            .iter()
            .filter(|(k, _)| *k == "tr")
            .map(|(_, tracker)| percent_decode_str(tracker).decode_utf8_lossy().into_owned())
            .collect();

        let peers = Some(
            params
                .iter()
                .filter(|(k, _)| k.starts_with("x.pe"))
                .map(|(_, v)| percent_decode_str(v).decode_utf8_lossy().into_owned())
                .collect(),
        );

        Ok(MagnetInfo {
            info_hash,
            display_name,
            trackers,
            peers,
        })
    }
    pub async fn download(&self, output_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
        println!("Fetching metadata from the magnet link!");

        let metadata = self
            .to_torrent_metadata()
            .await
            .expect("Failed to convert the link to metadata!");

        println!("Getting the peer list...!");

        let peers = request_peers(&metadata)
            .await
            .expect("Failed to request peers from magnet link metadata!");
        if peers.is_empty() {
            return Err("No peer is available!".into());
        }

        tokio::fs::create_dir_all(output_dir)
            .await
            .expect("Failed to create a directory!");

        let pieces_length = metadata.get_pieces_length();
        let pieces_hashes = metadata.get_pieces_hashes();
        let file_structure = metadata.get_file_structure();
        let total_pieces = pieces_hashes.len();

        println!("Starting the download of {} pieces...!", total_pieces);

        let mut good_peer = None;
        for peer in peers {
            let mut tmp_peer = Peer::new(peer.ip.clone(), peer.port);

            match tmp_peer.connect().await {
                Ok(_) => {
                    println!("Connected to {}:{}", peer.ip.clone(), peer.port);
                    let info_hash = metadata
                        .calculate_info_hash()
                        .expect("Failed to calculate the info hash!");
                    match tmp_peer.handshake(info_hash, generate_peer_id()).await {
                        Ok(_) => {
                            println!("Handshake successful!");
                            good_peer = Some(tmp_peer);
                            break;
                        }
                        Err(e) => {
                            println!("Handshake Failed! {}", e);
                            continue;
                        }
                    }
                }
                Err(_) => {
                    println!("Failed to connect to the peer!");
                    continue;
                }
            }
        }
        let mut peer = good_peer.ok_or("Could not find a peer!").unwrap();

        for (piece_index, piece_hash) in pieces_hashes.iter().enumerate() {
            println!("Downloading {}/{} ... ", piece_index + 1, total_pieces);
            let file_path = format!("{}/piece_{}", output_dir, piece_index);
            match peer
                .request_piece(piece_index as u32, pieces_length as u32, &file_path)
                .await
            {
                Ok(_) => {
                    let piece_data = tokio::fs::read(file_path.as_str())
                        .await
                        .expect("Failed to read the file");
                    let mut hasher = Sha1::new();
                    Digest::update(&mut hasher, &piece_data);
                    let downloaded_data: [u8; 20] = hasher.finalize().into();
                    if &downloaded_data != piece_hash {
                        return Err(
                            format!("Piece {} hash verification failed!", piece_index).into()
                        );
                    }
                    println!("Piece {} verified sucessfully!", piece_index);
                }
                Err(e) => {
                    return Err(format!("Failed to receive piece {}: {}", piece_index, e).into());
                }
            }
        }
        println!("Reconstructing the file tree..");
        for (file_path, file_length) in file_structure {
            let output_path = format!("{}/{}", output_dir, file_path);
            if let Some(parent) = Path::new(&output_path).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let mut output_file = tokio::fs::File::create(&output_path)
                .await
                .expect("Failed to create output file!");
            let mut bytes_written = 0i64;

            while bytes_written < file_length {
                let piece_index = (bytes_written / pieces_length as i64) as usize;
                let piece_path = format!("{}/piece_{}", output_dir, piece_index);
                let mut piece_data = tokio::fs::File::open(&piece_path).await?;

                let offset = bytes_written % pieces_length as i64;
                piece_data
                    .seek(std::io::SeekFrom::Start(offset as u64))
                    .await
                    .unwrap();

                let bytes_to_write =
                    std::cmp::min(pieces_length as i64 - offset, file_length - bytes_written)
                        as usize;
                let mut buffer = vec![0u8; bytes_to_write];
                piece_data.read_exact(&mut buffer).await?;
                output_file.write_all(&buffer).await?;

                bytes_written += bytes_to_write as i64;
            }
        }
        for piece_index in 0..total_pieces {
            let piece_path = format!("{}/piece_{}", output_dir, piece_index);
            tokio::fs::remove_file(piece_path).await?;
        }
        println!("Download Completed Successfully!");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio;

    #[test]
    fn test_magnet_parser() {
        let magnet = "magnet:?xt=urn:btih:12451f81a977a2d8bb402f21cd643422c5d4c50a&dn=The.Agency.2024.S01E05.WEB.x264-TORRENTGALAXY&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.cyberia.is%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Fexplodie.org%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.birkenwald.de%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.moeking.me%3A6969%2Fannounce&tr=udp%3A%2F%2Fipv4.tracker.harry.lu%3A80%2Fannounce&tr=udp%3A%2F%2Ftracker.tiny-vps.com%3A6969%2Fannounce";

        let result = MagnetInfo::parse(magnet).unwrap();

        assert_eq!(
            result.display_name.unwrap().as_str(),
            "The.Agency.2024.S01E05.WEB.x264-TORRENTGALAXY"
        );
    }
    #[test]
    fn test_base32_magnet() {
        let magnet = "magnet:?xt=urn:btih:c9e15763f722f23e98a29decdfae341b98d53056&dn=Test&tr=udp%3A%2F%2Ftracker.example.org%3A6969";
        let magnet_info = MagnetInfo::parse(magnet).unwrap();
        assert!(magnet_info.info_hash.len() == 20);
    }

    #[tokio::test]
    async fn test_fetch_metadata_from_peers() {
        let magnet = "magnet:?xt=urn:btih:12451f81a977a2d8bb402f21cd643422c5d4c50a&dn=The.Agency.2024.S01E05.WEB.x264-TORRENTGALAXY&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.cyberia.is%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Fexplodie.org%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.birkenwald.de%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.moeking.me%3A6969%2Fannounce&tr=udp%3A%2F%2Fipv4.tracker.harry.lu%3A80%2Fannounce&tr=udp%3A%2F%2Ftracker.tiny-vps.com%3A6969%2Fannounce";

        let magnet_info = MagnetInfo::parse(magnet).unwrap();
        let peer_test = vec![PeerInfo {
            ip: "127.0.0.1".to_string(),
            port: 6881,
        }];

        let results = magnet_info
            .fetch_metadata_from_peers(&peer_test)
            .await
            .unwrap();
        println!("Metadata fetched: {:?}", results);
    }
}
