mod error;
mod magnet;
mod mapper;
mod parser;
mod peer;
mod tracker;

use std::io;

use error::TorrentError;
use mapper::TorrentMetaData;
use peer::PieceDownloader;
use tracker::{generate_peer_id, request_peers};

#[tokio::main]
async fn main() -> Result<(), TorrentError> {
    let mut input_path = String::new();
    print!("Enter your torrent file path: ");
    io::stdin()
        .read_line(&mut input_path)
        .expect("failed to read from stdin");

    print!("Now enter your output path: ");

    let mut output_path = String::new();
    io::stdin()
        .read_line(&mut output_path)
        .expect("failed to read from stdin");

    let torrent_mta = TorrentMetaData::from_trnt_file(&input_path)?;
    let info_hash = torrent_mta.calculate_info_hash()?;

    let peers = request_peers(&torrent_mta)
        .await
        .expect("request peers failed!");
    if peers.is_empty() {
        return Err(TorrentError::InsufficientSeeds);
    }
    let peer_id = generate_peer_id();
    let mut downloader = PieceDownloader::new(peers, info_hash, peer_id);
    downloader
        .download_torrent(&torrent_mta, &output_path)
        .await?;
    Ok(())
}
