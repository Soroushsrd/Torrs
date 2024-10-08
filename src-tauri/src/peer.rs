use tokio::net::TcpStream;
use std::error::Error;

pub struct Peer{
    ip: String,
    port: u16,
    stream: Option<TcpStream>
}


impl Peer {
    pub fn new(ip:String,port:u16)->Self{
        Peer {
            ip,
            port,
            stream: None
        }
    }

    pub async fn connect(&mut self)->Result<(),Box<dyn Error>>{
        let address= format!("{}:{}",self.ip,self.port);
        self.stream = Some(TcpStream::connect(address).await.unwrap());
        Ok(())
    }

    pub fn handshake(){
        todo!()
    }

    pub fn request_pieces(){
        todo!()
    }
}