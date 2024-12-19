use crate::{
    mapper::TorrentMetaData,
    peer::{Peer, PeerInfo},
    tracker::{generate_peer_id, request_tracker},
};
use data_encoding::BASE32;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{collections::HashMap, time::Duration};
use url::Url;
const METADATA_PIECE_SIZE: usize = 16384; //metadata is chunked into 16kb pieces!
const EXTENSION_HANDSHAKE_ID: usize = 0;
const METADATA_EXTENSION_ID: usize = 1;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PIECE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
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
    pub async fn fetch_metadata_from_peers(
        &self,
        peers: &[PeerInfo],
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        todo!()
    }
    pub async fn fetch_metadata_from_peer(
        &self,
        peer_info: &PeerInfo,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        //    let mut peer = Peer::new(peer_info.ip.clone(), peer_info.port);
        //    peer.connect().await.unwrap();
        //
        //    let mut extension_bits = [0u8; 8];
        //    extension_bits[5] |= 0x10;
        //    let peer_id = generate_peer_id();
        //
        //    peer.handshake(self.info_hash, peer_id).await.unwrap();
        //
        //    let mut extension_handshake = HashMap::new();
        //    extension_handshake
        //        .insert("ut_metadata".to_string(), METADATA_EXTENSION_ID as i64)
        //        .unwrap();
        //
        //    let mut handshake_msg = ExtensionHandshake {
        //        m: extension_handshake,
        //        metadata_size: None,
        //        v: Some("RU0001".to_string()),
        //    };
        //
        //    let handshake_bytes = serde_bencode::to_bytes(&mut handshake_msg).unwrap();
        //    self.send_extension_message(&mut peer, EXTENSION_HANDSHAKE_ID, &handshake_bytes)
        //        .await?;
        todo!()
    }
    async fn send_extension_message(
        &self,
        peer: &mut Peer,
        extension_id: u8,
        payload: &[u8],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let msg_len: usize = 2 + payload.len();
        let mut message: Vec<u8> = Vec::with_capacity(4 + msg_len);

        // Message length prefix
        message.extend_from_slice(&(msg_len as u32).to_be_bytes());
        // Extension message ID
        message.push(20);
        // Extension-specific ID
        message.push(extension_id);
        // Payload
        message.extend_from_slice(payload);

        peer.send_msg(&mut message).await?;
        Ok(())
    }
    pub fn parse(magnet_url: &str) -> Result<MagnetInfo, Box<dyn std::error::Error>> {
        let url = Url::parse(magnet_url).unwrap();
        let params: HashMap<_, _> = url.query_pairs().collect();

        let info_hash = if let Some(xt) = params.get("xt") {
            if let Some(hash) = xt.strip_prefix("urn:btih") {
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
}
