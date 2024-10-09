use tokio::net::TcpStream;
use std::error::Error;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::quirks::protocol;

#[derive(Debug, Deserialize,Serialize,Clone)]
pub struct PeerInfo{
    ip:String,
    port: u16
}

#[derive(Debug)]
pub struct Peer{
    peer_info: PeerInfo,
    stream: Option<TcpStream>
}

pub const PROTOCOL:&str = "BitTorrent protocol";

impl Peer {
    pub fn new(ip:String,port:u16)->Self{
        let peer_info = PeerInfo{
            ip,
            port,
        };
        Peer {
            peer_info,
            stream: None
        }
    }

    pub async fn connect(&mut self)->Result<(),Box<dyn Error>>{
        let address= format!("{}:{}",self.peer_info.ip,self.peer_info.port);
        self.stream = Some(TcpStream::connect(address).await.unwrap());
        Ok(())
    }

    pub async fn handshake(&mut self, info_hash:[u8;20], peer_id:[u8;20])->Result<(),Box<dyn Error>>{
        if self.stream.is_none(){
            return Err("Stream not established yet".into())
        }

        let mut handshake_msg: Vec<u8> = Vec::new();
        let stream = self.stream.as_mut().unwrap();

        handshake_msg.push(PROTOCOL.len() as u8);
        handshake_msg.extend_from_slice(PROTOCOL.as_bytes());
        handshake_msg.extend_from_slice(&[0u8;8]);
        handshake_msg.extend_from_slice(&info_hash);
        handshake_msg.extend_from_slice(&peer_id);

        stream.write_all(&handshake_msg).await?;

        let mut response = vec![0u8; 68];
        stream.read_exact(&mut response).await?;

        if response[0] as usize != PROTOCOL.len() || &response[1..20] != PROTOCOL.as_bytes(){
            return Err("Invalid Handshake Response!".into())
        }

        let peer_info_hash = &response[20..48];
        if peer_info_hash != info_hash{
            return Err("Info-Hash Mismatch!".into())
        }

        Ok(())
    }

    pub async fn request_piece(&mut self, index: u32, begin: u32, length: u32) -> Result<Vec<u8>, Box<dyn Error>> {
        if self.stream.is_none() {
            return Err("Not connected to peer".into());
        }

        let stream = self.stream.as_mut().unwrap();

        let mut request :Vec<u8> = Vec::new();
        request.extend_from_slice(&(13u32).to_be_bytes());
        request.push(6);
        request.extend_from_slice(&index.to_be_bytes());
        request.extend_from_slice(&begin.to_be_bytes());
        request.extend_from_slice(&length.to_be_bytes());

        stream.write_all(&request).await?;

        let mut msg_len = [0u8;4];
        stream.read_exact(&mut msg_len).await?;
        let msg_len = u32::from_be_bytes(msg_len) as usize;

        let mut msg_id = [0u8;1];
        stream.read_exact(&mut msg_id).await?;

        if msg_id[0] != 7 { // 7 is the message id for piece
            return Err("Invalid msg Id!".into())
        }
        let mut piece_data = vec![0u8; msg_len-9];
        stream.read_exact(&mut piece_data).await?;

        Ok(piece_data)
    }
}