use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::error::{Result, TorrentError};
use crate::mapper::TorrentMetaData;

// TODO: have to modify the mapper to optionally look for some fields but otherwise ignore them if
//they dont exist!
// TODO: Go through the downloading process once again and implement multi threading
// TODO: Should start the downloading process once it has found a peer with the right bitfields
//then run the rest of the process in the background
// TODO: Add rarest first algo to download the pieces that fewer peers have first
// TODO: Refactor getting piece availability to be dynamically called instead of once at the
//beginning
// TODO: Skip pieces that fail to be downloaded- must have them saved somewhere to download them
//later on
// TODO: Add piece verification

#[derive(Debug)]
pub enum PeerMessage {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have {
        piece_idx: u32,
    },
    BitField {
        bitfield: Vec<u8>,
    },
    #[allow(dead_code)]
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    #[allow(dead_code)]
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
}

#[derive(Debug, Default, Deserialize, Serialize, Clone, Hash, PartialEq, Eq)]
pub struct PeerInfo {
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Default)]
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
                is_choked: true,
                is_interested: false,
                ..Default::default()
            })
            .collect();

        PieceDownloader {
            peers,
            current_peer_idx: 0,
            hash_info: info_hash,
            peer_id,
        }
    }
    // a method to initialize all peer connections and gather piece availability
    // should be called first so that we could run get_piece_availability on peers
    // TODO: redundant?
    pub async fn initialize_peers(&mut self) -> Result<()> {
        let mut successful_connections = 0;

        // connect to all peers
        for peer_idx in 0..self.peers.len() {
            self.current_peer_idx = peer_idx;
            let peer = &mut self.peers[peer_idx];

            peer.connect().await?;
            peer.handshake(self.hash_info, self.peer_id).await?;

            if let Ok(()) = peer.receive_init_msg().await {
                successful_connections += 1;
                println!(
                    "Successfully initialized peer {}:{}",
                    peer.peer_info.ip, peer.peer_info.port
                );
            }
        }
        if successful_connections == 0 {
            return Err(TorrentError::PeerError(
                "couldnt connect to any peer".to_string(),
            ));
        }

        Ok(())
    }

    /// Enumerates through the peers and map the piece index to peers that offer that piece!
    pub fn get_piece_availability(&self, total_pieces: u32) -> HashMap<u32, Vec<usize>> {
        let mut pices_with_peers: HashMap<u32, Vec<usize>> = HashMap::new();
        for piece in 0..total_pieces {
            let peers_with_piece: Vec<usize> = self
                .peers
                .iter()
                .enumerate()
                .filter(|(_, peer)| peer.piece_availability.contains(&piece))
                .map(|(idx, _)| idx)
                .collect();
            if !peers_with_piece.is_empty() {
                pices_with_peers.insert(piece, peers_with_piece);
            }
        }
        pices_with_peers
    }
    async fn download_piece(
        &mut self,
        index: u32,
        piece_length: u32,
        file_path: &PathBuf,
        total_pieces: u32,
    ) -> Result<()> {
        let mut offset = 0;
        let mut retries_with_same_peer = 0;
        const MAX_RETRIES_PER_PEER: i32 = 2;

        while offset < piece_length {
            let peers_per_piece = self.get_piece_availability(total_pieces);
            if let Some(peers) = peers_per_piece.get(&index) {
                let mut peer_idx = 0;
                while peer_idx < peers.len() {
                    self.current_peer_idx = peers[peer_idx];

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
                                retries_with_same_peer = 0;
                                peer_idx += 1;
                            } else {
                                tokio::time::sleep(Duration::from_secs(1)).await;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn download_torrent(
        &mut self,
        torrent: &TorrentMetaData,
        output_dir: &Path,
    ) -> Result<()> {
        self.initialize_peers().await?;

        let piece_length = torrent.get_pieces_length() as u32;
        let total_pieces = torrent.calculate_total_pieces();

        let temp_dir = output_dir.join("temp_pieces");
        tokio::fs::create_dir_all(&temp_dir).await?;

        let mut downloaded_pieces = HashSet::new();

        let piece_availability = self.get_piece_availability(total_pieces);

        for piece_index in 0..total_pieces {
            if downloaded_pieces.contains(&piece_index) {
                continue;
            }
            let actual_piece_length = if piece_index == total_pieces - 1 {
                let total_size = torrent.get_total_size();
                let remainder = total_size % torrent.get_pieces_length();
                if remainder == 0 {
                    piece_length
                } else {
                    remainder as u32
                }
            } else {
                piece_length
            };

            let temp_piece_path = temp_dir.join(format!("piece_{}", piece_index));

            if let Some(peer_indices) = piece_availability.get(&piece_index) {
                let mut success = false;

                for &peer_idx in peer_indices {
                    self.current_peer_idx = peer_idx;
                    match self
                        .download_piece(
                            piece_index,
                            actual_piece_length,
                            &temp_piece_path,
                            total_pieces,
                        )
                        .await
                    {
                        Ok(_) => {
                            downloaded_pieces.insert(piece_index);
                            success = true;
                            println!("Successfully downloaded and verified piece {}", piece_index);
                            break;
                        }
                        Err(e) => {
                            println!("failed to download the piece {}: {}", piece_index, e);
                        }
                    }
                }
                if !success {
                    return Err(TorrentError::DownloadTimedout);
                }
            } else {
                return Err(TorrentError::NoAvailablePeers(piece_index));
            }
        }
        println!("All pieces downloaded! Assembling files...");
        PieceDownloader::assemble_files(torrent, &temp_dir, output_dir).await?;
        tokio::fs::remove_dir_all(&temp_dir).await?;
        Ok(())
    }
    pub async fn assemble_files(
        torrent: &TorrentMetaData,
        temp_dir: &Path,
        output_dir: &Path,
    ) -> Result<()> {
        let piece_length = torrent.get_pieces_length() as u64;
        let file_structure = torrent.get_file_structure();
        let mut absolute_offset = 0u64;

        for (file_path, file_length) in file_structure {
            let full_path = output_dir.join(file_path);

            if let Some(parent) = std::path::Path::new(&full_path).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let mut outputfile = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&full_path)
                .await?;

            let file_length = file_length as u64;
            let mut bytes_written = 0u64;

            while bytes_written < file_length {
                let current_piece = (absolute_offset / piece_length) as u32;
                let offset_in_piece = absolute_offset % piece_length;

                let piece_path = temp_dir.join(format!("piece_{current_piece}"));
                let piece_data = tokio::fs::read(&piece_path).await?;

                let bytes_remaining_in_piece = piece_data.len() as u64 - offset_in_piece;
                let bytes_remaining_in_file = file_length - bytes_written;

                let bytes_to_write =
                    std::cmp::min(bytes_remaining_in_piece, bytes_remaining_in_file) as usize;
                outputfile
                    .write_all(
                        &piece_data
                            [offset_in_piece as usize..(offset_in_piece as usize + bytes_to_write)],
                    )
                    .await?;
                bytes_written += bytes_to_write as u64;
                absolute_offset += bytes_to_write as u64;
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

    async fn read_message(&mut self) -> Result<PeerMessage> {
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| TorrentError::PeerError("No active stream".to_string()))?;

        let mut len_buf = [0u8; 4];
        stream.read_exact(&mut len_buf).await?;
        let msg_len = u32::from_be_bytes(len_buf) as usize;
        if msg_len == 0 {
            return Ok(PeerMessage::KeepAlive);
        }

        let mut msg_id = [0u8; 1];
        stream.read_exact(&mut msg_id).await?;

        match msg_id[0] {
            0 => Ok(PeerMessage::Choke),
            1 => Ok(PeerMessage::Unchoke),
            2 => Ok(PeerMessage::Interested),
            3 => Ok(PeerMessage::NotInterested),
            4 => {
                let mut piece_idx = [0u8; 4];
                stream.read_exact(&mut piece_idx).await?;
                Ok(PeerMessage::Have {
                    piece_idx: u32::from_be_bytes(piece_idx),
                })
            }
            5 => {
                let payload_len = msg_len - 1;
                let mut bitfield = vec![0u8; payload_len];
                stream.read_exact(&mut bitfield).await?;
                Ok(PeerMessage::BitField { bitfield })
            }

            6 => {
                let mut buff = [0u8; 12];
                stream.read_exact(&mut buff).await?;
                Ok(PeerMessage::Request {
                    index: u32::from_be_bytes(buff[0..4].try_into().unwrap()),
                    begin: u32::from_be_bytes(buff[4..8].try_into().unwrap()),
                    length: u32::from_be_bytes(buff[8..12].try_into().unwrap()),
                })
            }
            7 => {
                // Piece message
                let mut index = [0u8; 4];
                let mut begin = [0u8; 4];
                stream.read_exact(&mut index).await?;
                stream.read_exact(&mut begin).await?;

                let block_len = msg_len - 9; // msg_len - (1 + 4 + 4)
                let mut block = vec![0u8; block_len];
                stream.read_exact(&mut block).await?;

                Ok(PeerMessage::Piece {
                    index: u32::from_be_bytes(index),
                    begin: u32::from_be_bytes(begin),
                    block,
                })
            }

            8 => {
                // Cancel message
                let mut buf = [0u8; 12];
                stream.read_exact(&mut buf).await?;
                Ok(PeerMessage::Cancel {
                    index: u32::from_be_bytes(buf[0..4].try_into().unwrap()),
                    begin: u32::from_be_bytes(buf[4..8].try_into().unwrap()),
                    length: u32::from_be_bytes(buf[8..12].try_into().unwrap()),
                })
            }

            id => {
                // Unknown message - skip payload
                let payload_len = msg_len - 1;
                let mut payload = vec![0u8; payload_len];
                stream.read_exact(&mut payload).await?;
                Err(TorrentError::InvalidMessage(format!(
                    "Unknown message ID: {id:?}",
                )))
            }
        }
    }
    pub async fn connect(&mut self) -> Result<()> {
        let address = format!("{}:{}", self.peer_info.ip, self.peer_info.port);
        let connect_future = TcpStream::connect(&address);
        match timeout(Duration::from_secs(5), connect_future).await {
            Ok(Ok(stream)) => {
                self.stream = Some(stream);
                println!("Successfully connected to stream: {address}");
                Ok(())
            }
            Ok(Err(e)) => Err(TorrentError::ConnectionFailed(format!(
                "Connection error to peer {address}: {e}"
            ))),
            Err(_) => Err(TorrentError::ConnectionTimedOut(format!(
                "Connection timeout to {address}"
            ))),
        }
    }

    pub async fn handshake(&mut self, info_hash: [u8; 20], peer_id: [u8; 20]) -> Result<()> {
        if self.stream.is_none() {
            return Err(TorrentError::PeerError(
                "Stream not established yet".to_string(),
            ));
        }

        let mut handshake_msg = Vec::with_capacity(68);
        let stream = self.stream.as_mut().unwrap();

        handshake_msg.push(19);
        handshake_msg.extend_from_slice(b"BitTorrent protocol");
        handshake_msg.extend_from_slice(&[0u8; 8]);
        handshake_msg.extend_from_slice(&info_hash);
        handshake_msg.extend_from_slice(&peer_id);

        match timeout(Duration::from_secs(4), stream.write_all(&handshake_msg)).await {
            Ok(result) => result?,
            Err(_) => {
                return Err(TorrentError::ConnectionTimedOut(
                    "Handshake send timeout".to_string(),
                ));
            }
        };

        println!("Handshake sent, waiting for response...");

        let mut response = vec![0u8; 68];
        timeout(Duration::from_secs(10), stream.read_exact(&mut response))
            .await
            .map_err(|_| TorrentError::ConnectionTimedOut("Handshake receive timeout".to_string()))?
            .map_err(|e| TorrentError::PeerError(format!("Failed to read handshake: {}", e)))?;

        println!("Handshake response received!");
        if response[0] != 19 || &response[1..20] != b"BitTorrent protocol" {
            return Err(TorrentError::InvalidHandshake(format!(
                "Invalid protocol string. Got: {:?}",
                &response[..20]
            )));
        }

        let peer_info_hash = &response[28..48];
        if peer_info_hash != info_hash {
            return Err(TorrentError::InvalidHandshake(format!(
                "Info hash mismatch.\nExpected: {:02x?}\nReceived: {:02x?}",
                info_hash, peer_info_hash
            )));
        }

        Ok(())
    }

    pub async fn receive_init_msg(&mut self) -> Result<()> {
        match self.read_message().await? {
            PeerMessage::BitField { bitfield } => {
                self.bitfields = Some(bitfield.clone());
                self.parse_bitfield(&bitfield);
                Ok(())
            }
            PeerMessage::Unchoke => {
                self.is_choked = false;
                Ok(())
            }
            msg => Err(TorrentError::InvalidMessage(format!(
                "expected either bitfield or unchoke msgs, got {msg:?}"
            ))),
        }
    }

    pub async fn request_piece(
        &mut self,
        index: u32,
        piece_length: u32,
        start_offset: u32,
        file_path: &PathBuf,
    ) -> Result<u32> {
        if !self.is_interested {
            self.send_interested().await?;
            self.is_interested = true;
            println!("interested message sent!");
        }
        self.wait_for_unchoke().await?;

        println!("Requesting piece: {}", index);
        let mut file = if start_offset == 0 {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(file_path)
                .await
                .map_err(|e| match e.kind() {
                    std::io::ErrorKind::PermissionDenied => {
                        TorrentError::PermissionDenied(file_path.to_string_lossy().to_string())
                    }
                    _ => TorrentError::IoError(e),
                })?
        } else {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(file_path)
                .await
                .map_err(TorrentError::from)?
        };

        const BLOCK_SIZE: u32 = 16 * 1024;
        let mut offset = start_offset;
        let mut bytes_downloaded = 0;

        while offset < piece_length {
            let block_length = std::cmp::min(BLOCK_SIZE, piece_length - offset);
            self.send_piece_req(index, offset, block_length).await?;
            // Wait for piece data with timeout
            let time_dur = Duration::from_secs(15);
            let msg = timeout(time_dur, self.wait_for_piece_block(index, offset))
                .await
                .map_err(|_| {
                    TorrentError::PeerError("Timeout waiting for piece data".to_string())
                })??;

            if let PeerMessage::Piece { block, .. } = msg {
                file.write_all(&block).await?;
                offset += block_length;
                bytes_downloaded += block_length;
                println!("Received piece block at offset {}", offset);
            }
        }

        Ok(bytes_downloaded)
    }

    pub async fn send_msg(&mut self, message: &[u8]) -> Result<()> {
        if let Some(stream) = self.stream.as_mut() {
            stream
                .write_all(message)
                .await
                .expect("Failed to write the message in TCP stream!");
            stream.flush().await.expect("Failed to flush the stream");
            Ok(())
        } else {
            Err(TorrentError::PeerError("No stream was found!".to_string()))
        }
    }

    pub async fn receive_msg(&mut self) -> Result<Vec<u8>> {
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
            Err(TorrentError::PeerError("No stream was found!".to_string()))
        }
    }

    async fn send_interested(&mut self) -> Result<()> {
        let interested_msg = [0u8, 0, 0, 1, 2];
        let stream = self
            .stream
            .as_mut()
            .ok_or_else(|| TorrentError::PeerError("Not connected to peer".to_string()))?;
        stream
            .write_all(&interested_msg)
            .await
            .map_err(|e| TorrentError::PeerError(format!("Failed to send interested: {}", e)))?;
        Ok(())
    }

    async fn wait_for_unchoke(&mut self) -> Result<()> {
        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(20);
        while self.is_choked && start_time.elapsed() < timeout {
            match self.read_message().await? {
                PeerMessage::Unchoke => {
                    self.is_choked = true;
                    return Ok(());
                }
                PeerMessage::Have { piece_idx } => {
                    self.piece_availability.insert(piece_idx);
                }
                _ => {}
            }
        }
        if self.is_choked {
            Err(TorrentError::DownloadTimedout)
        } else {
            Ok(())
        }
    }

    async fn send_piece_req(&mut self, index: u32, begin: u32, length: u32) -> Result<()> {
        let mut request = Vec::with_capacity(17);
        request.extend_from_slice(&13u32.to_be_bytes()); // msg length
        request.push(6);
        request.extend_from_slice(&index.to_be_bytes());
        request.extend_from_slice(&begin.to_be_bytes());
        request.extend_from_slice(&length.to_be_bytes());

        self.stream
            .as_mut()
            .ok_or_else(|| TorrentError::PeerError("No active stream".to_string()))?
            .write_all(&request)
            .await?;
        Ok(())
    }
    async fn wait_for_piece_block(
        &mut self,
        expected_index: u32,
        expected_offset: u32,
    ) -> Result<PeerMessage> {
        loop {
            match self.read_message().await? {
                msg @ PeerMessage::Piece { index, begin, .. } => {
                    if index == expected_index && begin == expected_offset {
                        return Ok(msg);
                    } else {
                        println!(
                            "Received piece for wrong index/offset: expected {}/{}, got {}/{}",
                            expected_index, expected_offset, index, begin
                        );
                    }
                }
                PeerMessage::Choke => {
                    return Err(TorrentError::PeerError(
                        "Peer choked during download".to_string(),
                    ));
                }
                PeerMessage::Have { piece_idx } => {
                    println!("Received have message for piece {}", piece_idx);
                    self.piece_availability.insert(piece_idx);
                }
                PeerMessage::KeepAlive => {
                    println!("Received keep-alive");
                }
                msg => {
                    println!(
                        "Skipping unexpected message during piece download: {:?}",
                        msg
                    );
                }
            }
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
        let path = "/home/rusty/Codes/Fun/Torrs/Violet [FitGirl Repack].torrent";
        let torrent_meta_data = TorrentMetaData::from_trnt_file(path).unwrap();
        let peers = request_peers(&torrent_meta_data).await.unwrap();
        let peer_id = generate_peer_id();
        let info_hash = torrent_meta_data.calculate_info_hash().unwrap();

        let mut downloader = PieceDownloader::new(peers, info_hash, peer_id);

        let torrent_path = std::path::Path::new(path);
        let output_dir = torrent_path.parent().unwrap().join("downloads");

        match downloader
            .download_torrent(&torrent_meta_data, &output_dir)
            .await
        {
            Ok(()) => println!("Successfully downloaded torrent"),
            Err(e) => println!("Failed to download: {}", e),
        }
    }
}
