use crate::mapper::*;
use crate::peer::PeerInfo;
use rand::Rng;
use serde::Deserialize;
use serde_bytes::ByteBuf;
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::task;
use url::Url;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TrackerResponse {
    interval: Option<i64>,
    #[serde(default)]
    peer: Vec<PeerInfo>,
    #[serde(rename = "peers", default)]
    peers_binary: Option<ByteBuf>,
}

/// Used to generate Peer Id that is later used to request trackers
pub fn generate_peer_id() -> [u8; 20] {
    let mut rng = rand::thread_rng();
    let mut peer_id = [0u8; 20];

    rng.fill(&mut peer_id);

    peer_id[0] = b'-';
    peer_id[1..6].copy_from_slice(b"RU001");

    peer_id
}

/// Request Peers in order to get the Peer Info that is needed to establish connections.
pub async fn request_peers(
    torrent: &TorrentMetaData,
) -> Result<Vec<PeerInfo>, Box<dyn std::error::Error>> {
    let info_hash = torrent.calculate_info_hash()?;
    let trackers = torrent.get_tracker_url();
    let total_length = torrent.get_total_size();

    let mut handles = vec![];

    for tracker in trackers {
        if tracker.starts_with("udp://") {
            println!("Skipping UDP tracker: {}", tracker);
            continue;
        }

        let tracker = tracker.to_string();
        let info_hash = info_hash.clone();

        let handle = task::spawn(async move {
            match tokio::time::timeout(
                Duration::from_secs(10),
                request_tracker(&tracker, &info_hash, total_length),
            )
            .await
            {
                Ok(Ok(peers)) => Ok((tracker, peers)),
                Ok(Err(e)) => Err(format!("Failed to connect to tracker {}: {:?}", tracker, e)),
                Err(_) => Err(format!("Timeout connecting to tracker {}", tracker)),
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        match handle.await {
            Ok(Ok((tracker, peers))) => {
                println!(
                    "Successfully retrieved {} peers from tracker: {}",
                    peers.len(),
                    tracker
                );
                return Ok(peers);
            }
            Ok(Err(e)) => eprintln!("{}", e),
            Err(e) => eprintln!("Task failed: {:?}", e),
        }
    }

    Err("Failed to retrieve peers from any tracker".into())
}

/// Request Trackers based on the info that has been parsed from torrent file.
pub async fn request_tracker(
    announce: &str,
    info_hash: &[u8; 20],
    total_length: i64,
) -> Result<Vec<PeerInfo>, Box<dyn std::error::Error>> {
    let mut url = Url::parse(announce)?;
    let peer_id = generate_peer_id();

    url.query_pairs_mut()
        .append_pair("info_hash", urlencode(info_hash).as_str())
        .append_pair("peer_id", &String::from_utf8_lossy(&peer_id))
        .append_pair("port", "6881")
        .append_pair("uploaded", "0")
        .append_pair("downloaded", "0")
        .append_pair("left", total_length.to_string().as_str())
        .append_pair("compact", "1")
        .append_pair("event", "started");

    let response = reqwest::get(url).await.unwrap().bytes().await?;

    let tracker_response: TrackerResponse = serde_bencode::de::from_bytes(&response)?;

    let peers = if !tracker_response.peer.is_empty() {
        tracker_response.peer
    } else if let Some(binary_peer) = tracker_response.peers_binary {
        parse_binary_peers(&binary_peer)
    } else {
        return Err("No peers found in response".into());
    };

    if peers.is_empty() {
        Err("Tracker returned no peers".into())
    } else {
        Ok(peers)
    }
}

pub fn parse_binary_peers(binary: &[u8]) -> Vec<PeerInfo> {
    binary
        .chunks(6)
        .filter_map(|chunk| {
            if chunk.len() == 6 {
                let ip = Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]).to_string();
                let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                Some(PeerInfo { ip, port })
            } else {
                None
            }
        })
        .collect()
}
/// Encodes url to a String
fn urlencode(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| format!("%{:02X}", b)).collect()
}
