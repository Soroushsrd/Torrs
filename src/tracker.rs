use crate::mapper::*;
use crate::peer::PeerInfo;
use rand::Rng;
use serde::Deserialize;
use serde_bytes::ByteBuf;
use std::net::Ipv4Addr;
use std::time::Duration;
use tokio::task;
use tokio::time::timeout;
use url::Url;

const UDP_TIMEOUT: Duration = Duration::from_secs(10);

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
    peer_id[1..7].copy_from_slice(b"TR2940");

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
            println!("Connecting to UDP tracker: {}", tracker);
            //continue;
            let tracker = tracker.to_string();
            let info_hash = info_hash.clone();

            let handle = task::spawn(async move {
                match tokio::time::timeout(
                    Duration::from_secs(20),
                    request_udp_tracker(&tracker, &info_hash, total_length),
                )
                .await
                {
                    Ok(Ok(peers)) => Ok((tracker, peers)),
                    Ok(Err(e)) => Err(format!("Failed to connect to tracker {}: {:?}", tracker, e)),
                    Err(_) => Err(format!("Timeout connecting to tracker {}", tracker)),
                }
            });
            handles.push(handle);
        } else {
            let tracker = tracker.to_string();
            let info_hash = info_hash.clone();

            let handle = task::spawn(async move {
                match tokio::time::timeout(
                    Duration::from_secs(20),
                    request_http_trackers(&tracker, &info_hash, total_length),
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

/// Sending connection request with a socker
pub async fn send_connection_request(
    socket: &tokio::net::UdpSocket,
) -> Result<u64, Box<dyn std::error::Error>> {
    let protocol_id: u64 = 0x0000041727101980; // Fixed protocol ID
    let action: u32 = 0; // connect
    let transaction_id: u32 = rand::random();

    let mut request = Vec::with_capacity(16);
    request.extend_from_slice(&protocol_id.to_be_bytes());
    request.extend_from_slice(&action.to_be_bytes());
    request.extend_from_slice(&transaction_id.to_be_bytes());

    socket.send(&request).await?;

    let mut response = vec![0u8; 16];
    let size = socket.recv(&mut response).await?;

    if size != 16 {
        return Err(format!("Invalid connection response size: {}", size).into());
    }

    let resp_action = u32::from_be_bytes(response[0..4].try_into()?);
    let resp_transaction_id = u32::from_be_bytes(response[4..8].try_into()?);

    if resp_action != 0 {
        return Err(format!("Invalid action in connection response: {}", resp_action).into());
    }
    if resp_transaction_id != transaction_id {
        return Err("Transaction ID mismatch in connection response".into());
    }

    Ok(u64::from_be_bytes(response[8..16].try_into()?))
}
/// Request Trakcers using udp links!
pub async fn request_udp_tracker(
    announce: &str,
    info_hash: &[u8; 20],
    total_length: i64,
) -> Result<Vec<PeerInfo>, Box<dyn std::error::Error>> {
    let url = Url::parse(announce)?;
    let host = url.host_str().ok_or("No host in tracker URL")?;
    let port = url.port().unwrap_or(80);

    // Bind to an IPv4 address specifically
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;

    let addr = match tokio::net::lookup_host((host, port)).await? {
        mut addrs => {
            let ipv4_addr = addrs
                .find(|addr| addr.is_ipv4())
                .ok_or("No IPv4 address found for tracker")?;
            ipv4_addr
        }
    };

    match timeout(UDP_TIMEOUT, socket.connect(addr)).await {
        Ok(result) => result?,
        Err(_) => return Err("UDP tracker connection timeout".into()),
    }

    let mut retries = 2;
    let mut connection_id = None;

    while retries > 0 && connection_id.is_none() {
        match timeout(UDP_TIMEOUT, send_connection_request(&socket)).await {
            Ok(Ok(id)) => {
                connection_id = Some(id);
                break;
            }
            Ok(Err(e)) => {
                println!("Connection request failed, retries left {}: {}", retries, e);
                retries -= 1;
            }
            Err(_) => {
                println!("Connection request timed out, retries left {}", retries);
                retries -= 1;
            }
        }
    }

    let connection_id = connection_id.ok_or("Failed to get connection ID after retries")?;

    let transaction_id: u32 = rand::random();
    let peer_id = generate_peer_id();

    let mut request = Vec::with_capacity(98);
    request.extend_from_slice(&connection_id.to_be_bytes()); // 8 bytes
    request.extend_from_slice(&1_u32.to_be_bytes()); // 4 bytes - action (1 for announce)
    request.extend_from_slice(&transaction_id.to_be_bytes()); // 4 bytes
    request.extend_from_slice(info_hash); // 20 bytes
    request.extend_from_slice(&peer_id); // 20 bytes
    request.extend_from_slice(&0_i64.to_be_bytes()); // 8 bytes - downloaded
    request.extend_from_slice(&total_length.to_be_bytes()); // 8 bytes - left
    request.extend_from_slice(&0_i64.to_be_bytes()); // 8 bytes - uploaded
    request.extend_from_slice(&0_i32.to_be_bytes()); // 4 bytes - event
    request.extend_from_slice(&0_u32.to_be_bytes()); // 4 bytes - IP address
    request.extend_from_slice(&0_u32.to_be_bytes()); // 4 bytes - key
    request.extend_from_slice(&(-1_i32).to_be_bytes()); // 4 bytes - num_want
    request.extend_from_slice(&6881_u16.to_be_bytes()); // 2 bytes - port

    retries = 2;
    while retries > 0 {
        // Send announce
        match timeout(UDP_TIMEOUT, socket.send(&request)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                println!("Failed to send announce, retries left {}: {}", retries, e);
                retries -= 1;
                continue;
            }
            Err(_) => {
                println!("Announce send timed out, retries left {}", retries);
                retries -= 1;
                continue;
            }
        }

        let mut response = vec![0u8; 1024];
        match timeout(UDP_TIMEOUT, socket.recv(&mut response)).await {
            Ok(Ok(size)) => {
                response.truncate(size);
                if size < 8 {
                    println!("Response too short: {} bytes", size);
                    retries -= 1;
                    continue;
                }

                let action = u32::from_be_bytes(response[0..4].try_into()?);
                let resp_transaction_id = u32::from_be_bytes(response[4..8].try_into()?);
                println!(
                    "Response action: {}, transaction_id: {}",
                    action, resp_transaction_id
                );
                println!("First 20 bytes: {:?}", &response[..20.min(size)]);

                if resp_transaction_id != transaction_id {
                    println!("Transaction ID mismatch");
                    retries -= 1;
                    continue;
                }

                match action {
                    1 => {
                        let interval = u32::from_be_bytes(response[8..12].try_into()?);
                        let leechers = u32::from_be_bytes(response[12..16].try_into()?);
                        let seeders = u32::from_be_bytes(response[16..20].try_into()?);

                        println!(
                            "Success! Interval: {}s, Leechers: {}, Seeders: {}",
                            interval, leechers, seeders
                        );

                        let mut peers = Vec::new();
                        for chunk in response[20..].chunks(6) {
                            if chunk.len() == 6 {
                                let ip =
                                    format!("{}.{}.{}.{}", chunk[0], chunk[1], chunk[2], chunk[3]);
                                let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                                peers.push(PeerInfo { ip, port });
                            }
                        }
                        return Ok(peers);
                    }
                    2 => {
                        // Scrape response
                        println!("Got scrape response, trying announce again...");

                        let mut announce_request = Vec::with_capacity(98);
                        announce_request.extend_from_slice(&connection_id.to_be_bytes());
                        announce_request.extend_from_slice(&1_u32.to_be_bytes()); // Action 1 for announce
                        announce_request.extend_from_slice(&transaction_id.to_be_bytes());

                        // Convert info_hash to proper network byte order
                        let mut formatted_hash = [0u8; 20];
                        for i in 0..20 {
                            formatted_hash[i] = info_hash[19 - i];
                        }
                        announce_request.extend_from_slice(&formatted_hash);

                        announce_request.extend_from_slice(&peer_id);
                        announce_request.extend_from_slice(&0_i64.to_be_bytes()); // downloaded
                        announce_request.extend_from_slice(&total_length.to_be_bytes()); // left
                        announce_request.extend_from_slice(&0_i64.to_be_bytes()); // uploaded
                        announce_request.extend_from_slice(&0_i32.to_be_bytes()); // event
                        announce_request.extend_from_slice(&0_u32.to_be_bytes()); // IP
                        announce_request.extend_from_slice(&0_u32.to_be_bytes()); // key
                        announce_request.extend_from_slice(&(-1_i32).to_be_bytes()); // num_want
                        announce_request.extend_from_slice(&6881_u16.to_be_bytes()); // port

                        socket.send(&announce_request).await?;

                        let mut retry_response = vec![0u8; 1024];
                        let retry_size = socket.recv(&mut retry_response).await?;
                        retry_response.truncate(retry_size);

                        if retry_size < 20 {
                            return Err("Retry size too short for announce".into());
                        }
                        let retry_action = u32::from_be_bytes(retry_response[0..4].try_into()?);
                        if retry_action == 1 {
                            let mut peers = Vec::new();
                            for chunk in retry_response[20..].chunks(6) {
                                if chunk.len() == 6 {
                                    let ip = format!(
                                        "{}.{}.{}.{}",
                                        chunk[0], chunk[1], chunk[2], chunk[3]
                                    );
                                    let port = u16::from_be_bytes([chunk[4], chunk[5]]);
                                    if &ip != "0.0.0.0" && port != 0 {
                                        peers.push(PeerInfo { ip, port });
                                    }
                                }
                            }
                            return Ok(peers);
                        } else {
                            return Err(
                                "Failed to get proper announce response after scrape".into()
                            );
                        };
                    }
                    3 => {
                        // Error
                        let error_msg = String::from_utf8_lossy(&response[8..]);
                        println!("Got error response: {}", error_msg);
                        retries -= 1;
                        continue;
                    }
                    _ => {
                        println!("Got unexpected action: {}", action);
                        retries -= 1;
                        continue;
                    }
                }
            }
            Ok(Err(e)) => {
                println!(
                    "Failed to receive announce response, retries left {}: {}",
                    retries, e
                );
                retries -= 1;
            }
            Err(_) => {
                println!("Announce receive timed out, retries left {}", retries);
                retries -= 1;
            }
        }
    }

    Err("Failed to get valid response after retries".into())
}
/// Request trackers from http and udp origins
pub async fn request_tracker(
    announce: &str,
    info_hash: &[u8; 20],
    total_length: i64,
) -> Result<Vec<PeerInfo>, Box<dyn std::error::Error>> {
    if announce.starts_with("udp://") {
        return request_udp_tracker(announce, info_hash, total_length).await;
    } else if announce.starts_with("http://") || announce.starts_with("https://") {
        return request_http_trackers(announce, info_hash, total_length).await;
    }
    Err("Unsupported tracker protocol".into())
}
/// Request Trackers based on the info that has been parsed from torrent file.
pub async fn request_http_trackers(
    announce: &str,
    info_hash: &[u8; 20],
    total_length: i64,
) -> Result<Vec<PeerInfo>, Box<dyn std::error::Error>> {
    let url = Url::parse(announce)?;
    let peer_id = generate_peer_id();

    let q = format!(
        "?info_hash={}&peer_id={}&port=6881&uploaded=0&downloaded=0&compact=1&left={}",
        urlencode(info_hash),
        urlencode(&peer_id),
        total_length
    );
    // let url = transform_tracker_url(url.as_str());
    let full_url = format!("{}{}", url.as_str().trim_end_matches('/'), q);

    let response = match reqwest::get(full_url.clone()).await {
        Ok(bytes) => match bytes.bytes().await {
            Ok(byte) => byte,
            Err(e) => return Err(format!("Failed to get a response: {}", e).into()),
        },
        Err(e) => return Err(format!("Failed to connect to {}:{}", url, e).into()),
    };

    if response.starts_with(&[b'<']) {
        return Err("Tracker returned HTML instead of bencoded data".into());
    }

    let tracker_response: TrackerResponse = serde_bencode::de::from_bytes(&response)
        .map_err(|e| format!("failed to decode the bytes {}", e))?;
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
pub fn urlencode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| format!("%{:02x}", b))
        .collect::<String>()
}
pub fn transform_tracker_url(announce: &str) -> String {
    let mut url = announce.to_string();

    url = url.replace("https://", "http://");
    if !url.contains(":6881") {
        url = url.replace("/announce", ":6969/announce");
    }
    println!("refined tracker url: {}", url);
    url
}

#[cfg(test)]
mod tests {

    use super::*;
    #[tokio::test]
    async fn test_request_http_tracker() {
        let path = r"/home/rusty/Rs/Torrs/The.Agency.2024.S01E09.WEB.x264-TORRENTGALAXY.torrent";

        let torrent_meta_data = TorrentMetaData::from_trnt_file(path).unwrap();
        println!("Got the torrent meta data");

        let info_hash = torrent_meta_data.calculate_info_hash().unwrap();
        let trackers = torrent_meta_data.get_tracker_url();
        let total_length = torrent_meta_data.get_total_size();

        let mut any_success = false;

        for tracker in trackers {
            println!("\nTrying tracker: {}", tracker);
            let result = if tracker.starts_with("udp") {
                match request_udp_tracker(&tracker, &info_hash, total_length).await {
                    Ok(peers) => {
                        println!("Successfully got {} peers from UDP tracker", peers.len());
                        any_success = true;
                        Ok(peers)
                    }
                    Err(e) => {
                        println!("UDP tracker failed: {}", e);
                        Err(e)
                    }
                }
            } else {
                match request_http_trackers(&tracker, &info_hash, total_length).await {
                    Ok(peers) => {
                        println!("Successfully got {} peers from HTTP tracker", peers.len());
                        any_success = true;
                        Ok(peers)
                    }
                    Err(e) => {
                        println!("HTTP tracker failed: {}", e);
                        Err(e)
                    }
                }
            };

            // Print peer info if successful
            if let Ok(peers) = result {
                println!("First 5 peers from tracker {}:", tracker);
                for (i, peer) in peers.iter().take(5).enumerate() {
                    println!("  Peer {}: {}:{}", i + 1, peer.ip, peer.port);
                }
                if peers.len() > 5 {
                    println!("  ... and {} more peers", peers.len() - 5);
                }
            }
        }

        // Test passes if at least one tracker worked
        assert!(any_success, "No trackers successfully returned peers");
    }
}
