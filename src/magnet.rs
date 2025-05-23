use crate::{
    mapper::TorrentMetaData,
    peer::{Peer, PeerInfo},
    tracker::{generate_peer_id, request_tracker},
};
use data_encoding::BASE32;
use futures::future::join_all;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::time::timeout;
use url::Url;

const METADATA_PIECE_SIZE: usize = 16384; //metadata is chunked into 16kb pieces!
const EXTENSION_HANDSHAKE_ID: usize = 0;
const METADATA_EXTENSION_ID: usize = 1;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PIECE_TIMEOUT: Duration = Duration::from_secs(30);

//TODO: Add TorrentError to this module
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
        let mut all_peers = Vec::new();

        println!(
            "Attempting to get peers from {} trackers",
            self.trackers.len()
        );
        for tracker in &self.trackers {
            println!("Trying tracker: {}", tracker);
            match request_tracker(tracker, &self.info_hash, 0).await {
                Ok(peers) => {
                    println!("Got {} peers from tracker {}", peers.len(), tracker);
                    all_peers.extend(peers);
                }
                Err(e) => {
                    println!("Failed to get peers from tracker {}: {}", tracker, e);
                    continue;
                }
            }
        }

        if all_peers.is_empty() {
            return Err("Could not get any peers from any tracker".into());
        }

        println!("Total peers collected: {}", all_peers.len());
        match self.fetch_metadata_from_peers(&all_peers).await {
            Ok(metadata_bytes) => {
                let torrent_metadata: TorrentMetaData = serde_bencode::from_bytes(&metadata_bytes)
                    .map_err(|e| format!("Failed to decode metadata: {}", e))?;
                Ok(torrent_metadata)
            }
            Err(e) => Err(format!(
                "Failed to get metadata from {} peers: {}",
                all_peers.len(),
                e
            )
            .into()),
        }
    }
    pub async fn fetch_metadata_from_peer(
        &self,
        peer_info: &PeerInfo,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut peer = Peer::new(peer_info.ip.clone(), peer_info.port);
        match peer.connect().await {
            Ok(_) => println!(
                "Connected to peer: {}:{}",
                peer.peer_info.ip.clone(),
                peer.peer_info.port
            ),
            Err(e) => println!(
                "Failed to connect to peer {}:{} - {}",
                peer.peer_info.ip.clone(),
                peer_info.port,
                e
            ),
        }

        let mut extension_bits = [0u8; 8];
        extension_bits[5] |= 0x10;
        let peer_id = generate_peer_id();

        match peer.handshake(self.info_hash, peer_id).await {
            Ok(_) => println!("Handshake successful!"),
            Err(e) => println!("Handshake failed: {}", e),
        }

        let mut extension_handshake = HashMap::new();
        extension_handshake.insert("ut_metadata".to_string(), METADATA_EXTENSION_ID as i64);

        let handshake_msg = ExtensionHandshake {
            m: extension_handshake,
            metadata_size: None,
            v: Some("RU0001".to_string()),
        };

        let handshake_bytes = serde_bencode::to_bytes(&handshake_msg).unwrap();
        println!("Handshake Bytes: {:?}", handshake_bytes.clone());
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
            let self_clone = self.clone();

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
        if let Some((most_common_hash, _)) = metadata_count.iter().max_by_key(|&(_, count)| count) {
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
    /// parses the link to extract info-hash, display-name(optiona), tracker urls and peer info!
    pub fn parse(magnet_url: &str) -> Result<MagnetInfo, Box<dyn std::error::Error>> {
        let url = Url::parse(magnet_url)?;

        // Get all query parameters, including duplicates
        let params: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();

        let info_hash = if let Some((_, xt)) = params.iter().find(|(k, _)| k == "xt") {
            if let Some(hash) = xt.strip_prefix("urn:btih:") {
                if hash.len() == 40 {
                    let mut result = [0u8; 20];
                    hex::decode_to_slice(hash, &mut result)?;
                    result
                } else if hash.len() == 32 {
                    let mut result = [0u8; 20];
                    let decoded = BASE32.decode(hash.as_bytes())?;
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
            .iter()
            .find(|(k, _)| k == "dn")
            .map(|(_, v)| percent_decode_str(v).decode_utf8_lossy().into_owned());

        // Collect all trackers (tr parameters)
        let trackers = params
            .iter()
            .filter(|(k, _)| k == "tr")
            .map(|(_, v)| percent_decode_str(v).decode_utf8_lossy().into_owned())
            .collect::<Vec<String>>();

        println!("Found {} trackers in magnet link:", trackers.len());
        for tracker in &trackers {
            println!("  - {}", tracker);
        }

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
    //TODO:: Modify the magnet structure to handle offsets

    //pub async fn download(&self, output_dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    //    println!("Fetching metadata from the magnet link!");
    //
    //    let metadata = self
    //        .to_torrent_metadata()
    //        .await
    //        .expect("Failed to convert the link to metadata!");
    //
    //    println!("Getting the peer list...!");
    //
    //    let peers = request_peers(&metadata)
    //        .await
    //        .expect("Failed to request peers from magnet link metadata!");
    //    if peers.is_empty() {
    //        return Err("No peer is available!".into());
    //    }
    //
    //    tokio::fs::create_dir_all(output_dir)
    //        .await
    //        .expect("Failed to create a directory!");
    //
    //    let pieces_length = metadata.get_pieces_length();
    //    let pieces_hashes = metadata.get_pieces_hashes();
    //    let file_structure = metadata.get_file_structure();
    //    let total_pieces = pieces_hashes.len();
    //
    //    println!("Starting the download of {} pieces...!", total_pieces);
    //
    //    let mut good_peer = None;
    //    for peer in peers {
    //        let mut tmp_peer = Peer::new(peer.ip.clone(), peer.port);
    //
    //        match tmp_peer.connect().await {
    //            Ok(_) => {
    //                println!("Connected to {}:{}", peer.ip.clone(), peer.port);
    //                let info_hash = metadata
    //                    .calculate_info_hash()
    //                    .expect("Failed to calculate the info hash!");
    //                match tmp_peer.handshake(info_hash, generate_peer_id()).await {
    //                    Ok(_) => {
    //                        println!("Handshake successful!");
    //                        good_peer = Some(tmp_peer);
    //                        break;
    //                    }
    //                    Err(e) => {
    //                        println!("Handshake Failed! {}", e);
    //                        continue;
    //                    }
    //                }
    //            }
    //            Err(_) => {
    //                println!("Failed to connect to the peer!");
    //                continue;
    //            }
    //        }
    //    }
    //    let mut peer = good_peer.ok_or("Could not find a peer!").unwrap();
    //
    //    for (piece_index, piece_hash) in pieces_hashes.iter().enumerate() {
    //        println!("Downloading {}/{} ... ", piece_index + 1, total_pieces);
    //        let file_path = format!("{}/piece_{}", output_dir, piece_index);
    //        match peer
    //            .request_piece(piece_index as u32, pieces_length as u32, &file_path)
    //            .await
    //        {
    //            Ok(_) => {
    //                let piece_data = tokio::fs::read(file_path.as_str())
    //                    .await
    //                    .expect("Failed to read the file");
    //                let mut hasher = Sha1::new();
    //                Digest::update(&mut hasher, &piece_data);
    //                let downloaded_data: [u8; 20] = hasher.finalize().into();
    //                if &downloaded_data != piece_hash {
    //                    return Err(
    //                        format!("Piece {} hash verification failed!", piece_index).into()
    //                    );
    //                }
    //                println!("Piece {} verified sucessfully!", piece_index);
    //            }
    //            Err(e) => {
    //                return Err(format!("Failed to receive piece {}: {}", piece_index, e).into());
    //            }
    //        }
    //    }
    //    println!("Reconstructing the file tree..");
    //    for (file_path, file_length) in file_structure {
    //        let output_path = format!("{}/{}", output_dir, file_path);
    //        if let Some(parent) = Path::new(&output_path).parent() {
    //            tokio::fs::create_dir_all(parent).await?;
    //        }
    //        let mut output_file = tokio::fs::File::create(&output_path)
    //            .await
    //            .expect("Failed to create output file!");
    //        let mut bytes_written = 0i64;
    //
    //        while bytes_written < file_length {
    //            let piece_index = (bytes_written / pieces_length as i64) as usize;
    //            let piece_path = format!("{}/piece_{}", output_dir, piece_index);
    //            let mut piece_data = tokio::fs::File::open(&piece_path).await?;
    //
    //            let offset = bytes_written % pieces_length as i64;
    //            piece_data
    //                .seek(std::io::SeekFrom::Start(offset as u64))
    //                .await
    //                .unwrap();
    //
    //            let bytes_to_write =
    //                std::cmp::min(pieces_length as i64 - offset, file_length - bytes_written)
    //                    as usize;
    //            let mut buffer = vec![0u8; bytes_to_write];
    //            piece_data.read_exact(&mut buffer).await?;
    //            output_file.write_all(&buffer).await?;
    //
    //            bytes_written += bytes_to_write as i64;
    //        }
    //    }
    //    for piece_index in 0..total_pieces {
    //        let piece_path = format!("{}/piece_{}", output_dir, piece_index);
    //        tokio::fs::remove_file(piece_path).await?;
    //    }
    //    println!("Download Completed Successfully!");
    //    Ok(())
    //}
}

#[cfg(test)]
mod tests {

    use super::*;

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
        let magnet = "magnet:?xt=urn:btih:678BC6AC22A5BEFAC6BBC50834E91D4F9755DEE4&dn=Dragon%26%23039%3Bs+Dogma+2+%28Dev+Build+v1.0.0.1%2C+MULTi14%29+%5BFitGirl+Repack%2C+Selective+Download+-+from+38.4+GB%5D%5D&tr=udp%3A%2F%2Fopentor.net%3A6969&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Ftracker.theoks.net%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.ccp.ovh%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=http%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce&tr=https%3A%2F%2Ftracker.tamersunion.org%3A443%2Fannounce&tr=udp%3A%2F%2Fexplodie.org%3A6969%2Fannounce&tr=http%3A%2F%2Ftracker.bt4g.com%3A2095%2Fannounce&tr=udp%3A%2F%2Fbt2.archive.org%3A6969%2Fannounce&tr=udp%3A%2F%2Fbt1.archive.org%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.filemail.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker1.bt.moack.co.kr%3A80%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=http%3A%2F%2Ftracker.openbittorrent.com%3A80%2Fannounce&tr=udp%3A%2F%2Fopentracker.i2p.rocks%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.internetwarriors.net%3A1337%2Fannounce&tr=udp%3A%2F%2Ftracker.leechers-paradise.org%3A6969%2Fannounce&tr=udp%3A%2F%2Fcoppersurfer.tk%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.zer0day.to%3A1337%2Fannounce";

        let magnet_info = MagnetInfo::parse(magnet).unwrap();
        println!("Getting the peers from trackers...");

        let mut peers = Vec::new();
        for tracker in &magnet_info.trackers {
            match request_tracker(tracker, &magnet_info.info_hash, 0).await {
                Ok(peer_vec) => {
                    println!("Got {} number of peers from {}", peer_vec.len(), tracker);
                    peers.extend(peer_vec);
                }
                Err(e) => {
                    println!("Failed to get peers from tracker {}, {}", tracker, e);
                    continue;
                }
            }
        }
        peers.sort_by_key(|p| (p.ip.clone(), p.port));
        peers.dedup_by_key(|p| (p.ip.clone(), p.port));

        if peers.is_empty() {
            panic!("could not find any peers");
        }
        println!("Testing with {} unique peers", peers.len());

        match magnet_info.fetch_metadata_from_peers(&peers).await {
            Ok(metadata) => {
                println!("Fetched metadata {} bytes", metadata.len());
                assert!(!metadata.is_empty(), "Metadata should not be empty");
            }
            Err(e) => {
                panic!("Failed to fetch metadata {}", e);
            }
        }
    }

    //#[tokio::test]
    //async fn test_magnet_download() {
    //    let magnet = "magnet:?xt=urn:btih:c0084178f6df4d5d11f87e61f0f84c6de0c72993&dn=MomWantsToBreed%2024%2012%2020%20Alexis%20Malone%20Stepmom%20Only%20Says%20Yes%20XXX%20480p%20MP4-XXX%20[XC]&tr=udp%3A%2F%2Fopen.stealth.si%3A80%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.cyberia.is%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Fexplodie.org%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.birkenwald.de%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.moeking.me%3A6969%2Fannounce&tr=udp%3A%2F%2Fipv4.tracker.harry.lu%3A80%2Fannounce&tr=udp%3A%2F%2Ftracker.tiny-vps.com%3A6969%2Fannounce";
    //
    //    let magnet_info = MagnetInfo::parse(magnet).expect("Failed to parse magnet link");
    //    println!("Successfully parsed magnet link");
    //    println!("Display name: {:?}", magnet_info.display_name);
    //    println!("Number of trackers: {}", magnet_info.trackers.len());
    //
    //    let output_dir = "./test_downloads";
    //
    //    match magnet_info.download(output_dir).await {
    //        Ok(_) => println!("Download completed successfully"),
    //        Err(e) => {
    //            println!("Download failed: {}", e);
    //            panic!("Download test failed");
    //        }
    //    }
    //}
}
