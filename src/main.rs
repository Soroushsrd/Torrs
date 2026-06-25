mod error;
mod magnet;
mod mapper;
mod peer;
mod tracker;

use clap::Parser;
use std::path::PathBuf;

use error::TorrentError;
use mapper::TorrentMetaData;
use peer::PieceDownloader;
use tracker::{generate_peer_id, request_peers};

use crate::mapper::calculate_info_hash;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Absolute input path of the torrent file
    #[arg(short, long, value_parser = validate_input)]
    input_path: PathBuf,

    /// Absolute output path
    #[arg(short, long, value_parser = validate_output)]
    output_path: PathBuf,
}

fn validate_input(s: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(s);
    if !p.is_absolute() {
        return Err(format!("path must be absolute: {s}"));
    }
    if !p.exists() {
        return Err(format!("file doesnt exist: {s}"));
    }
    Ok(p)
}

fn validate_output(s: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(s);
    if !p.is_absolute() {
        return Err(format!("path must be absolute: {s}"));
    }
    // create_dir_all is idempotent!
    std::fs::create_dir_all(&p).map_err(|e| format!("failed to create {p:?}: {e}"))?;
    Ok(p)
}

#[tokio::main]
async fn main() -> Result<(), TorrentError> {
    let args = Args::parse();

    let bytes = std::fs::read(&args.input_path)?;
    let torrent_mta = TorrentMetaData::from_bytes(&bytes)?;
    let info_hash = calculate_info_hash(&bytes)?;

    let peers = request_peers(&torrent_mta, info_hash)
        .await
        .expect("request peers failed!");
    if peers.is_empty() {
        return Err(TorrentError::InsufficientSeeds);
    }
    let peer_id = generate_peer_id();
    let mut downloader = PieceDownloader::new(peers, info_hash, peer_id);
    downloader
        .download_torrent(&torrent_mta, &args.output_path)
        .await?;
    Ok(())
}
