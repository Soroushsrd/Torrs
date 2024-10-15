use tokio::net::TcpStream;
use std::error::Error;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::fs::File;

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
        let peer_info = PeerInfo {
            ip,
            port,
        };
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

    pub async fn handshake(&mut self, info_hash: [u8; 20], peer_id: [u8; 20]) -> Result<(), Box<dyn Error>> {
        if self.stream.is_none() {
            return Err("Stream not established yet".into());
        }

        let mut handshake_msg: Vec<u8> = Vec::new();
        let stream = self.stream.as_mut().unwrap();

        handshake_msg.push(PROTOCOL.len() as u8);
        handshake_msg.extend_from_slice(PROTOCOL.as_bytes());
        handshake_msg.extend_from_slice(&[0u8; 8]);
        handshake_msg.extend_from_slice(&info_hash);
        handshake_msg.extend_from_slice(&peer_id);

        stream.write_all(&handshake_msg).await?;

        let mut response = vec![0u8; 68];
        stream.read_exact(&mut response).await?;

        if response[0] as usize != PROTOCOL.len() || &response[1..20] != PROTOCOL.as_bytes() {
            return Err("Invalid Handshake Response!".into());
        }

        let peer_info_hash = &response[20..48];
        if peer_info_hash != info_hash {
            return Err("Info-Hash Mismatch!".into());
        }

        Ok(())
    }

    pub async fn request_piece(&mut self, index: u32, piece_length: u32, file_path: &str) -> Result<(), Box<dyn Error>> {
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

        //////////////////////////// Send interested message //////////////////////////////
        let interested_msg = [0u8, 0, 0, 1, 2];
        stream.write_all(&interested_msg).await?;

        //////////////////////////// Wait for unchoke message //////////////////////////////

        let mut msg_len = [0u8; 4];
        stream.read_exact(&mut msg_len).await?;
        let msg_len = u32::from_be_bytes(msg_len) as usize;

        let mut msg_id = [0u8; 1];
        stream.read_exact(&mut msg_id).await?;
        if msg_id[0] != 1 {
            return Err("Expected unchoke message".into());
        }


        //////////////////////////// Requesting Piece //////////////////////////////
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
}


#[cfg(test)]
mod tests {
    use crate::mapper::TorrentMetaData;
    use crate::tracker::{request_peers};
    use super::*;
    use tokio::test;

    #[test]
    async fn test_get_peers() {
        let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
        let torrent_meta_data = TorrentMetaData::from_file(path).expect("Failed to read torrent file");
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

    // #[test]
    // async fn test_download() {
    //     env_logger::init();
    //
    //     let path = "C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json";
    //     let torrent_meta_data = TorrentMetaData::from_file(path).unwrap();
    //     println!("got the torrent meta data");
    //
    //     let peers = request_peers(&torrent_meta_data).await.unwrap();
    //     let file_struct: Vec<String> = torrent_meta_data.get_file_structure()
    //         .iter()
    //         .map(|file| file.0.clone()).collect();
    //     println!("file structure extracted");
    //
    //
    //     let peer_id = generate_peer_id();
    //     let info_hash = torrent_meta_data.calculate_info_hash().unwrap();
    //     println!("info hash calculated");
    //
    //
    //     for peer_info in peers {
    //         let mut peer = Peer::new(peer_info.ip, peer_info.port);
    //         if peer.connect().await.is_ok() {
    //             println!("connection to peer successful");
    //
    //             peer.handshake(info_hash, peer_id).await.unwrap();
    //             println!("handshake with peer done!");
    //
    //             for (index, file) in file_struct.iter().enumerate() {
    //                 println!("downloading file: {:?}", file.clone());
    //
    //                 let piece_length = torrent_meta_data.get_pieces_length();
    //                 let file_path = format!("C:\\Users\\Lenovo\\Downloads\\{}", file);
    //                 peer.request_piece(index as u32, piece_length as u32, &file_path).await.unwrap();
    //                 println!("downloading file: {:?} done!", file.clone());
    //             }
    //         }
    //     }
    // }
}
