use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::time::Duration;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

//TODO: each peer has some of the data(not all) so we should first see which files does the peer
//offer and then request those or keep a journal of pieces that each peer has. This can reduce the
//number or retry/error results and also allow me to handle the downloading on multiple threads

//TODO: have to implement better error handling and retry mechanisms

//TODO: have to modify the mapper to optionally look for some fields but otherwise ignore them if
//they dont exist!

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PeerInfo {
    pub ip: String,
    pub port: u16,
}

#[derive(Debug)]
pub struct Peer {
    pub peer_info: PeerInfo,
    stream: Option<TcpStream>,
    pub bitfields: Option<Vec<u8>>,
    pub is_choked: bool,
    pub is_interested: bool,
    pub piece_availability: HashSet<u32>,
}

#[derive(Debug)]
pub struct PieceDownloader {
    pub peers: Vec<Peer>,
    pub current_peer_idx: usize,
    pub hash_info: [u8; 20],
    pub peer_id: [u8; 20],
}

impl PieceDownloader {
    pub fn new(peers: Vec<PeerInfo>, info_hash: [u8; 20], peer_id: [u8; 20]) -> Self {
        let peers = peers
            .into_iter()
            .map(|peer_info| Peer {
                peer_info,
                stream: None,
                bitfields: None,
                is_choked: true,
                is_interested: false,
                piece_availability: HashSet::new(),
            })
            .collect();

        PieceDownloader {
            peers,
            current_peer_idx: 0,
            hash_info: info_hash,
            peer_id,
        }
    }
    /// First used at the beginning to get the available pieces for each peer,
    /// then used to try other peers when a peer fails
    async fn try_next_peer(&mut self) -> Result<(), Box<dyn Error>> {
        let mut attempts = 0;
        let max_attempts = self.peers.len();

        while attempts < max_attempts {
            self.current_peer_idx = (self.current_peer_idx + 1) % self.peers.len();
            attempts += 1;

            let peer = &mut self.peers[self.current_peer_idx];
            match peer.connect().await {
                Ok(()) => {
                    println!(
                        "Connected to new peer {}:{}",
                        peer.peer_info.ip, peer.peer_info.port
                    );
                    match peer.handshake(self.hash_info, self.peer_id).await {
                        Ok(()) => match peer.receive_init_msg().await {
                            Ok(()) => return Ok(()),
                            Err(e) => println!("Failed to receive initial messages: {}", e),
                        },
                        Err(e) => println!("Failed handshake: {}", e),
                    }
                }
                Err(e) => println!("Failed to connect to peer: {}", e),
            }
        }

        Err("No more peers available".into())
    }
    /// Finds peers that have a particular piece
    fn find_peer_with_piece(&self, piece: u32) -> Option<usize> {
        let output = self
            .peers
            .iter()
            .enumerate()
            .find(|(_, peer)| peer.piece_availability.contains(&piece))
            .map(|(index, _)| index);
        return output;
    }
    pub async fn download_piece(
        &mut self,
        index: u32,
        piece_length: u32,
        file_path: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut offset = 0;
        let mut retries_with_same_peer = 0;
        const MAX_RETRIES_PER_PEER: i32 = 2;

        while offset < piece_length {
            if let Some(peer_idx) = self.find_peer_with_piece(index) {
                self.current_peer_idx = peer_idx;

                let peer = &mut self.peers[self.current_peer_idx];
                println!(
                    "Attempting download of piece {} from peer {}:{}",
                    index, peer.peer_info.ip, peer.peer_info.port
                );

                match peer
                    .request_piece(index, piece_length, offset, file_path)
                    .await
                {
                    Ok(bytes_downloaded) => {
                        offset += bytes_downloaded;
                        retries_with_same_peer = 0;
                        if offset >= piece_length {
                            return Ok(());
                        }
                    }
                    Err(_e) => {
                        println!(
                            "Error downloading from peer: {}:{}",
                            peer.peer_info.ip, peer.peer_info.port
                        );
                        retries_with_same_peer += 1;
                        if retries_with_same_peer >= MAX_RETRIES_PER_PEER {
                            println!(
                                "Switching to the next peer after {} retries",
                                retries_with_same_peer
                            );
                            match self.try_next_peer().await {
                                Ok(()) => retries_with_same_peer = 0,
                                Err(e) => return Err(e),
                            }
                        } else {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

impl Peer {
    pub fn new(ip: String, port: u16) -> Self {
        let peer_info = PeerInfo { ip, port };
        Peer {
            peer_info,
            stream: None,
            bitfields: None,
            is_choked: true,
            is_interested: false,
            piece_availability: HashSet::new(),
        }
    }

    pub async fn connect(&mut self) -> Result<(), Box<dyn Error>> {
        let address = format!("{}:{}", self.peer_info.ip, self.peer_info.port);
        //self.stream = Some(TcpStream::connect(&address).await.unwrap());
        let connect_future = TcpStream::connect(&address);
        match timeout(Duration::from_secs(10), connect_future).await {
            Ok(Ok(stream)) => {
                self.stream = Some(stream);
                println!("Successfully connected to stream: {}", address);
                Ok(())
            }
            Ok(Err(e)) => Err(format!("Connection error to peer {}: {}", address, e).into()),
            Err(_) => Err(format!("Connection timeout to peer {}", address).into()),
        }
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
            return Err(format!("Invalid Handshake Response! Got: {:?}", &response[..20]).into());
        }

        let peer_info_hash = &response[28..48];
        if peer_info_hash != info_hash {
            return Err(format!(
                "Info hash mismatch.\nExpected: {:02x?}\nReceived: {:02x?}",
                info_hash, peer_info_hash
            )
            .into());
        }

        Ok(())
    }

    pub async fn receive_init_msg(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut got_bitfield = false;
        let start_time = std::time::Instant::now();
        let timeout_duration = std::time::Duration::from_secs(30);

        while !got_bitfield && start_time.elapsed() < timeout_duration {
            let mut msg_len = [0u8; 4];
            let stream = self.stream.as_mut().ok_or("No stream available")?;
            stream.read_exact(&mut msg_len).await?;
            let msg_len = u32::from_be_bytes(msg_len) as usize;

            // Handle keep-alive message
            if msg_len == 0 {
                println!("Received keep-alive");
                continue;
            }

            let mut msg_id = [0u8; 1];
            stream.read_exact(&mut msg_id).await?;

            println!("Received message type: {}", msg_id[0]);

            match msg_id[0] {
                0 => println!("Peer sent choke"),
                1 => println!("Peer sent unchoke"),
                2 => println!("Peer sent interested"),
                3 => println!("Peer sent not interested"),
                4 => {
                    // Have message
                    let mut have = [0u8; 4];
                    stream.read_exact(&mut have).await?;
                    let pieces = u32::from_be_bytes(have);
                    self.piece_availability.insert(pieces);
                    println!(
                        "Received have message for piece {}",
                        u32::from_be_bytes(have)
                    );
                }
                5 => {
                    // Bitfield
                    let mut bitfield = vec![0u8; msg_len - 1];
                    stream.read_exact(&mut bitfield).await?;
                    println!("Received bitfield of length {}", bitfield.len());
                    self.parse_bitfield(&mut bitfield);
                    self.bitfields = Some(bitfield);
                    got_bitfield = true;
                }
                _ => {
                    // Skip unknown message
                    let mut payload = vec![0u8; msg_len - 1];
                    stream.read_exact(&mut payload).await?;
                    println!("Skipping unknown message type: {}", msg_id[0]);
                }
            }
        }

        if !got_bitfield {
            return Err("Timeout waiting for bitfield".into());
        }
        Ok(())
    }
    pub async fn request_piece(
        &mut self,
        index: u32,
        piece_length: u32,
        start_offset: u32,
        file_path: &str,
    ) -> Result<u32, Box<dyn Error>> {
        if self.stream.is_none() {
            return Err("Not connected to peer".into());
        }

        //////////////////////////// Waiting for bitfield //////////////////////////////
        if self.bitfields.is_none() {
            self.receive_init_msg().await?;
        }
        println!("Got initial messages, sending interested message");
        //////////////////////////// Send interested message //////////////////////////////
        let stream = self.stream.as_mut().unwrap();
        if !self.is_interested {
            let interested_msg = [0u8, 0, 0, 1, 2];
            stream.write_all(&interested_msg).await?;
            self.is_interested = true;
            println!("interested message sent!");
        }
        //////////////////////////// Wait for unchoke message //////////////////////////////
        let start_time = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(30);
        while self.is_choked && start_time.elapsed() < timeout {
            let mut msg_len = [0u8; 4];
            stream.read_exact(&mut msg_len).await?;
            let msg_len = u32::from_be_bytes(msg_len);
            if msg_len == 0 {
                println!("recevied keep-alive msg");
                continue;
            }

            let mut msg_id = [0u8; 1];
            stream.read_exact(&mut msg_id).await?;
            match msg_id[0] {
                0 => println!("Received choke message"),
                1 => {
                    println!("Received unchoke message");
                    self.is_choked = false;
                }
                2 => println!("Received interested message"),
                3 => println!("Received not interested message"),
                4 => {
                    let mut have_payload = vec![0u8; msg_len as usize - 1];
                    stream.read_exact(&mut have_payload).await?;
                    println!(
                        "Received have message for piece {}",
                        u32::from_be_bytes(have_payload[..4].try_into()?)
                    );
                }
                _ => {
                    let mut payload = vec![0u8; msg_len as usize - 1];
                    stream.read_exact(&mut payload).await?;
                    println!("Received unknown message type: {}", msg_id[0]);
                }
            }
        }
        if self.is_choked {
            return Err("Timeout waiting for unchoke message".into());
        }

        //////////////////////////// Requesting Piece //////////////////////////////
        println!("Requesting piece: {}", index);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)
            .await?;

        let block_size = 16 * 1024;
        let mut offset = start_offset;
        let mut bytes_downloaded = 0;

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

            // Wait for piece data with timeout
            let mut got_piece = false;
            let start_time = std::time::Instant::now();
            let timeout_duration = std::time::Duration::from_secs(30);

            while !got_piece && start_time.elapsed() < timeout_duration {
                let mut msg_len = [0u8; 4];
                stream.read_exact(&mut msg_len).await?;
                let msg_len = u32::from_be_bytes(msg_len) as usize;

                if msg_len == 0 {
                    println!("Received keep-alive");
                    continue;
                }

                let mut msg_id = [0u8; 1];
                stream.read_exact(&mut msg_id).await?;

                println!("Received message type: {}", msg_id[0]);

                match msg_id[0] {
                    0 => return Err("Peer sent choke message during download".into()),
                    4 => {
                        // Have message
                        let mut have = [0u8; 4];
                        stream.read_exact(&mut have).await?;
                        println!(
                            "Received 'have' message for piece {}",
                            u32::from_be_bytes(have)
                        );
                    }
                    7 => {
                        // Piece message
                        let mut piece_index = [0u8; 4];
                        stream.read_exact(&mut piece_index).await?;
                        let mut piece_offset = [0u8; 4];
                        stream.read_exact(&mut piece_offset).await?;

                        let block_size = msg_len - 9; // subtract message type and index/offset
                        let mut block = vec![0u8; block_size];
                        stream.read_exact(&mut block).await?;

                        file.write_all(&block).await?;
                        offset += block_length;
                        bytes_downloaded += block_length;
                        got_piece = true;
                        println!("Received piece block at offset {}", offset);
                    }
                    _ => {
                        // Skip unknown message types
                        let mut payload = vec![0u8; msg_len - 1];
                        stream.read_exact(&mut payload).await?;
                        println!("Skipping unknown message type: {}", msg_id[0]);
                    }
                }
            }

            if !got_piece {
                return Err("Timeout waiting for piece data".into());
            }
        }

        Ok(bytes_downloaded)
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
    fn parse_bitfield(&mut self, bitfield: &[u8]) {
        for (byte_idx, &byte) in bitfield.iter().enumerate() {
            // For each bit in the byte
            for bit_idx in 0..8 {
                // Check if the bit is set (1)
                if (byte & (1 << (7 - bit_idx))) != 0 {
                    // Calculate piece index from byte_idx and bit_idx
                    let piece_idx = (byte_idx * 8 + bit_idx) as u32;
                    self.piece_availability.insert(piece_idx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::mapper::TorrentMetaData;
    use crate::tracker::{generate_peer_id, request_peers};
    use tokio::test;

    #[test]
    async fn test_get_peers() {
        let path = r"/home/rusty/Rs/Torrs/Gym Manager [FitGirl Repack].torrent";
        let torrent_meta_data = TorrentMetaData::from_trnt_file(path).unwrap();
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
            }
        }
    }
    #[tokio::test]
    async fn test_download() {
        let path = r"/home/rusty/Rs/Torrs/Gym Manager [FitGirl Repack].torrent";
        let torrent_meta_data = TorrentMetaData::from_trnt_file(path).unwrap();
        println!("Got the torrent meta data");

        // Get peers from trackers
        let peers = request_peers(&torrent_meta_data).await.unwrap();
        println!("Got {} peers", peers.len());
        assert!(!peers.is_empty(), "No peers found");

        let peer_id = generate_peer_id();
        let info_hash = torrent_meta_data.calculate_info_hash().unwrap();

        // Create downloader with our peer list
        let mut downloader = PieceDownloader::new(peers, info_hash, peer_id);

        // Initialize all peer connections first
        match downloader.try_next_peer().await {
            Ok(()) => println!("Successfully initialized peer connections"),
            Err(e) => {
                println!("Failed to initialize peer connections: {}", e);
                return;
            }
        }

        // Download the files
        let file_struct = torrent_meta_data.get_file_structure();
        let torrent_path = Path::new(path);
        let parent_dir = torrent_path.parent().unwrap().to_string_lossy().to_string();

        for (file_index, (file, _)) in file_struct.iter().enumerate() {
            println!("Downloading file: {:?}", file);
            let piece_length = torrent_meta_data.get_pieces_length();
            let file_path = format!("{}/{}", parent_dir, file);

            match downloader
                .download_piece(file_index as u32, piece_length as u32, &file_path)
                .await
            {
                Ok(()) => println!("Successfully downloaded file: {:?}", file),
                Err(e) => {
                    println!("Failed to download file {:?}: {}", file, e);
                    break;
                }
            }
        }
    }
    //#[tokio::test]
    //async fn test_download() {
    //    let path = r"/home/rusty/Rs/Torrs/Gym Manager [FitGirl Repack].torrent";
    //    let torrent_meta_data = TorrentMetaData::from_trnt_file(path).unwrap();
    //    println!("Got the torrent meta data");
    //
    //    // Get peers from trackers
    //    let peers = request_peers(&torrent_meta_data).await.unwrap();
    //    println!("Got {} peers", peers.len());
    //    assert!(!peers.is_empty(), "No peers found");
    //
    //    let peer_id = generate_peer_id();
    //    let info_hash = torrent_meta_data.calculate_info_hash().unwrap();
    //
    //    // Create downloader with our peer list
    //    let mut downloader = PieceDownloader::new(peers, info_hash, peer_id);
    //
    //    downloader.try_next_peer().await.unwrap();
    //    // Download the files
    //    let file_struct = torrent_meta_data.get_file_structure();
    //    let torrent_path = Path::new(path);
    //    let parent_dir = torrent_path.parent().unwrap().to_string_lossy().to_string();
    //
    //    for (file_index, (file, _)) in file_struct.iter().enumerate() {
    //        println!("Downloading file: {:?}", file);
    //        let piece_length = torrent_meta_data.get_pieces_length();
    //        let file_path = format!("{}/{}", parent_dir, file);
    //
    //        match downloader
    //            .download_piece(file_index as u32, piece_length as u32, &file_path)
    //            .await
    //        {
    //            Ok(()) => println!("Successfully downloaded file: {:?}", file),
    //            Err(e) => {
    //                println!("Failed to download file {:?}: {}", file, e);
    //                break;
    //            }
    //        }
    //    }
    //}
}
