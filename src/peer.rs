use serde::{Deserialize, Serialize};
use std::error::Error;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PeerInfo {
    pub ip: String,
    pub port: u16,
}

#[derive(Debug)]
pub struct Peer {
    peer_info: PeerInfo,
    stream: Option<TcpStream>,
}

pub const PROTOCOL: &str = "BitTorrent protocol";

impl Peer {
    pub fn new(ip: String, port: u16) -> Self {
        let peer_info = PeerInfo { ip, port };
        Peer {
            peer_info,
            stream: None,
        }
    }

    pub async fn connect(&mut self) -> Result<(), Box<dyn Error>> {
        let address = format!("{}:{}", self.peer_info.ip, self.peer_info.port);
        self.stream = Some(TcpStream::connect(address).await.unwrap());
        Ok(())
    }

    pub async fn handshake(
        &mut self,
        info_hash: [u8; 20],
        peer_id: [u8; 20],
    ) -> Result<(), Box<dyn Error>> {
        if self.stream.is_none() {
            return Err("Stream not established yet".into());
        }

        let mut handshake_msg = Vec::with_capacity(68);
        let stream = self.stream.as_mut().unwrap();

        handshake_msg.push(19);
        handshake_msg.extend_from_slice(b"BitTorrent protocol");
        handshake_msg.extend_from_slice(&[0u8; 8]);
        handshake_msg.extend_from_slice(&info_hash);
        handshake_msg.extend_from_slice(&peer_id);

        println!("Sending handshake, length: {}", handshake_msg.len());

        // Send handshake with timeout
        match timeout(Duration::from_secs(4), stream.write_all(&handshake_msg)).await {
            Ok(result) => result?,
            Err(_) => return Err("Handshake send timeout".into()),
        };

        println!("Handshake sent, waiting for response...");

        // Read peer's handshake with timeout
        let mut response = vec![0u8; 68];
        match timeout(Duration::from_secs(10), stream.read_exact(&mut response)).await {
            Ok(result) => result?,
            Err(_) => return Err("Handshake receive timeout".into()),
        };

        println!("Handshake response received!");
        if response[0] != 19 || &response[1..20] != b"BitTorrent protocol" {
            return Err("Invalid Handshake Response!".into());
        }

        let peer_info_hash = &response[20..48];
        if peer_info_hash != info_hash {
            return Err("Info-Hash Mismatch!".into());
        }

        Ok(())
    }

    pub async fn request_piece(
        &mut self,
        index: u32,
        piece_length: u32,
        file_path: &str,
    ) -> Result<(), Box<dyn Error>> {
        if self.stream.is_none() {
            return Err("Not connected to peer".into());
        }

        let stream = self.stream.as_mut().unwrap();
        //////////////////////////// Waiting for bitfield //////////////////////////////
        let mut msg_len = [0u8; 4];
        stream.read_exact(&mut msg_len).await?;
        let msg_len = u32::from_be_bytes(msg_len) as usize;

        let mut msg_id = [0u8; 1];
        stream.read_exact(&mut msg_id).await?;
        if msg_id[0] != 5 {
            return Err("Expected bitfield message".into());
        }

        let mut bitfield_payload = vec![0u8; msg_len - 1];
        stream.read_exact(&mut bitfield_payload).await?;
        println!("bitfield received!");
        //////////////////////////// Send interested message //////////////////////////////
        let interested_msg = [0u8, 0, 0, 1, 2];
        stream.write_all(&interested_msg).await?;
        println!("interested message sent!");
        //////////////////////////// Wait for unchoke message //////////////////////////////

        let mut msg_len = [0u8; 4];
        stream.read_exact(&mut msg_len).await?;
        let msg_len = u32::from_be_bytes(msg_len) as usize;

        let mut msg_id = [0u8; 1];
        stream.read_exact(&mut msg_id).await?;
        if msg_id[0] != 1 {
            return Err("Expected unchoke message".into());
        }

        println!("unchoke message received!");

        //////////////////////////// Requesting Piece //////////////////////////////
        println!("Requesting piece: {}", index);
        let mut file = File::create(file_path).await?;
        let block_size = 16 * 1024;
        let mut offset = 0;

        while offset < piece_length {
            let block_length = if piece_length - offset < block_size {
                piece_length - offset
            } else {
                block_size
            };
            let mut request: Vec<u8> = Vec::new();
            request.extend_from_slice(&(13u32).to_be_bytes());
            request.push(6);
            request.extend_from_slice(&index.to_be_bytes());
            request.extend_from_slice(&offset.to_be_bytes());
            request.extend_from_slice(&block_length.to_be_bytes());
            stream.write_all(&request).await?;

            let mut msg_len = [0u8; 4];
            stream.read_exact(&mut msg_len).await?;
            let msg_len = u32::from_be_bytes(msg_len) as usize;

            let mut msg_id = [0u8; 1];
            stream.read_exact(&mut msg_id).await?;

            if msg_id[0] != 7 {
                return Err("Invalid msg Id!".into());
            }

            let mut piece_index = [0u8; 4];
            stream.read_exact(&mut piece_index).await?;
            let mut piece_offset = [0u8; 4];
            stream.read_exact(&mut piece_offset).await?;

            let mut block = vec![0u8; msg_len - 9];
            stream.read_exact(&mut block).await?;

            file.write_all(&block).await?;

            offset += block_length;
        }

        Ok(())
    }
    pub async fn send_msg(&mut self, message: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(stream) = self.stream.as_mut() {
            stream
                .write_all(message)
                .await
                .expect("Failed to write the message in TCP stream!");
            stream.flush().await.expect("Failed to flush the stream");
            Ok(())
        } else {
            Err("No stream was found!".into())
        }
    }
    pub async fn receive_msg(&mut self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        if let Some(stream) = self.stream.as_mut() {
            let mut length_bytes = [0u8; 4];
            stream
                .read_exact(&mut length_bytes)
                .await
                .expect("Failed to read the length bytes");
            let length = u32::from_be_bytes(length_bytes);
            let mut msg = vec![0u8; length as usize];
            stream
                .read_exact(&mut msg)
                .await
                .expect("Failed to read the message!");
            Ok(msg)
        } else {
            Err("No stream was found!".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapper::TorrentMetaData;
    use crate::tracker::{generate_peer_id, request_peers};
    use tokio::test;

    #[test]
    async fn test_get_peers() {
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let torrent_meta_data =
            TorrentMetaData::from_file(path).expect("Failed to read torrent file");
        println!("Got the torrent meta data");

        match request_peers(&torrent_meta_data).await {
            Ok(peers) => {
                println!("Successfully retrieved {} peers", peers.len());
                for (i, peer) in peers.iter().enumerate() {
                    println!("Peer {}: {:?}", i + 1, peer);
                }
                assert!(!peers.is_empty(), "Peer list should not be empty");
            }
            Err(e) => {
                eprintln!("Failed to retrieve peers: {:?}", e);
                panic!("Failed to retrieve peers");
            }
        }
    }

    #[test]
    async fn test_download() {
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let torrent_meta_data = TorrentMetaData::from_file(path).unwrap();
        println!("got the torrent meta data");

        let peers = request_peers(&torrent_meta_data).await.unwrap();
        assert!(!peers.is_empty(), "Peer list should not be empty");
        println!("got the peers: {:?}", peers.clone());
        let file_struct: Vec<(String, i64)> = torrent_meta_data.get_file_structure();
        // .iter()
        // .map(|file| file.0.clone()).collect();
        println!("file structure extracted");

        let peer_id = generate_peer_id();
        let info_hash = torrent_meta_data.calculate_info_hash().unwrap();
        println!("info hash calculated");

        for peer_info in peers {
            let mut peer = Peer::new(peer_info.clone().ip, peer_info.clone().port);
            if peer.connect().await.is_ok() {
                println!("connection to peer successful");

                peer.handshake(info_hash, peer_id)
                    .await
                    .map_err(|e| {
                        eprintln!("Failed to handshake with peer: {:?}", e);
                    })
                    .unwrap();
                println!("handshake with peer done!");
                let total_bytes = 0;
                for (file_index, (file, file_length)) in file_struct.iter().enumerate() {
                    println!("downloading file: {:?}", file.clone());

                    let piece_length = torrent_meta_data.get_pieces_length();
                    let file_path = format!("C:\\Users\\Lenovo\\Downloads\\{}", file);
                    // let start_piece = total_bytes / piece_length;
                    // let end_piece = (total_bytes + file_length - 1) / piece_length;

                    peer.request_piece(file_index as u32, piece_length as u32, &file_path)
                        .await
                        .map_err(|e| {
                            eprintln!("Failed to download file: {:?}", e);
                        });
                    println!("downloading file: {:?} done!", file.clone());
                }
            } else {
                eprintln!("failed to connect to peer: {:?}", peer_info);
                continue;
            }
        }
    }
}
