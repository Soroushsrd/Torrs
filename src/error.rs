use std::fmt;

#[derive(Debug)]
#[allow(dead_code)]
pub enum TorrentError {
    // Network errors
    ConnectionTimedOut(String),
    ConnectionFailed(String),
    TrackerError(String),
    PeerError(String),

    // protocol related errors
    InvalidHandshake(String),
    InvalidMessage(String),
    ProtocolViolation(String),

    // file or io related errors
    FileNotFound(String),
    PermissionDenied(String),
    DiskFull,
    IoError(std::io::Error),

    //parsing related errors
    InvalidTorrentFile(String),
    InvalidMagnetLink(String),
    BencodeError(serde_bencode::Error),

    //downloading errors
    PieceVerificationFailed(u32),
    NoAvailablePeers(u32),
    DownloadTimedout,
    InsufficientSeeds,

    //config errs
    InvalidConfigs(String),
}

impl fmt::Display for TorrentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TorrentError::ConnectionTimedOut(msg) => write!(f, "Connection Timed Out: {}", msg),
            TorrentError::ConnectionFailed(msg) => write!(f, "Connection failed: {}", msg),
            TorrentError::TrackerError(msg) => write!(f, "Tracker error: {}", msg),
            TorrentError::PeerError(msg) => write!(f, "Peer error: {}", msg),
            TorrentError::InvalidHandshake(msg) => write!(f, "Invalid Handshake: {}", msg),
            TorrentError::InvalidMessage(msg) => write!(f, "Invalid message: {}", msg),
            TorrentError::ProtocolViolation(msg) => write!(f, "Protocol violation: {}", msg),
            TorrentError::FileNotFound(path) => write!(f, "File not found: {}", path),
            TorrentError::PermissionDenied(path) => write!(f, "Permission denied: {}", path),
            TorrentError::DiskFull => write!(f, "Disk is full."),
            TorrentError::IoError(e) => write!(f, "IO error: {}", e),
            TorrentError::InvalidTorrentFile(msg) => write!(f, "Invalid torrent file: {}", msg),
            TorrentError::InvalidMagnetLink(msg) => write!(f, "Invalid magnet link: {}", msg),
            TorrentError::BencodeError(e) => write!(f, "Bencode error: {}", e),
            TorrentError::PieceVerificationFailed(piece) => {
                write!(f, "Piece {} verification failed", piece)
            }
            TorrentError::NoAvailablePeers(piece) => {
                write!(f, "No available peers for piece {}", piece)
            }
            TorrentError::DownloadTimedout => write!(f, "Download timed out"),
            TorrentError::InsufficientSeeds => write!(f, "Insufficient seeds"),
            TorrentError::InvalidConfigs(msg) => write!(f, "Invalid configurations: {}", msg),
        }
    }
}

impl std::error::Error for TorrentError {}

pub type Result<T> = std::result::Result<T, TorrentError>;

impl From<std::io::Error> for TorrentError {
    fn from(value: std::io::Error) -> Self {
        match value.kind() {
            std::io::ErrorKind::NotFound => TorrentError::FileNotFound(value.to_string()),
            std::io::ErrorKind::PermissionDenied => {
                TorrentError::PermissionDenied(value.to_string())
            }
            std::io::ErrorKind::TimedOut => TorrentError::ConnectionTimedOut(value.to_string()),
            _ => TorrentError::IoError(value),
        }
    }
}

impl From<serde_bencode::Error> for TorrentError {
    fn from(value: serde_bencode::Error) -> Self {
        TorrentError::BencodeError(value)
    }
}

impl From<reqwest::Error> for TorrentError {
    fn from(value: reqwest::Error) -> Self {
        if value.is_timeout() {
            TorrentError::ConnectionTimedOut(value.to_string())
        } else if value.is_connect() {
            TorrentError::ConnectionFailed(value.to_string())
        } else {
            TorrentError::TrackerError(value.to_string())
        }
    }
}
