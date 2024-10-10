use crate::mapper::*;
use url::Url;
use rand::Rng;
use reqwest;
use serde::Deserialize;
use serde_bencode;
use crate::peer::{PeerInfo};

#[derive(Debug,Deserialize)]
pub struct TrackerResponse{
    interval:i64,
    peer: Vec<PeerInfo>
}

/// Used to generate Peer Id that is later used to request trackers
pub fn generate_peer_id() ->[u8;20]{
    let mut rng = rand::thread_rng();
    let mut peer_id =[0u8;20];

    rng.fill(&mut peer_id);

    peer_id[0] = b'-';
    peer_id[1..7].copy_from_slice(b"RU001");

    peer_id
}

/// Request Peers in order to get the Peer Info that is needed to establish connections.
pub async fn request_peers(torrent: &TorrentMetaData)->Result<Vec<PeerInfo>, Box<dyn std::error::Error>>{
    let info_hash = torrent.calculate_info_hash()?;
    let trackers = torrent.get_tracker_url();
    let total_length = torrent.get_total_size();

    for tracker in trackers{
        if let Ok(peers) = request_tracker(&tracker,&info_hash,total_length).await {
            return Ok(peers)
        }
    }
    Err("Failed to connect to any tracker".into())

}

/// Request Trackers based on the info that has been parsed from torrent file.
pub async fn request_tracker(announce: &str, info_hash:&[u8;20], total_length:i64)
    ->Result<Vec<PeerInfo>, Box<dyn std::error::Error>> {
    let mut url = Url::parse(announce)?;
    let peer_id = generate_peer_id();

    url.query_pairs_mut()
        .append_pair("info_hash",urlencode(info_hash).as_str())
        .append_pair("peer_id",&String::from_utf8_lossy(&peer_id))
        .append_pair("port","6881")
        .append_pair("uploaded","0")
        .append_pair("downloaded","0")
        .append_pair("left",total_length.to_string().as_str())
        .append_pair("compact","1")
        .append_pair("event","started");

    let response = reqwest::get(url).await.unwrap().bytes().await?;
    let tracker_response: TrackerResponse = serde_bencode::from_bytes(&response)?;

    Ok(tracker_response.peer)
}



/// Encodes url to a String
fn urlencode(bytes:&[u8])->String{
    bytes.iter()
        .map(|&b| format!("%{:02X}", b))
        .collect()
}
