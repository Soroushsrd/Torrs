use bencode_encoder::{Decoder, Encoder, Type};
use bencode_encoder::Type::Dictionary;
use std::path::Path;
use std::fs::{File};
use std::io::Write;
use sha1::{Digest, Sha1};
use base64::{Engine as _, engine::{self, general_purpose}, alphabet};

const CUSTOM_ENGINE: engine::GeneralPurpose =
    engine::GeneralPurpose::new(&alphabet::URL_SAFE, general_purpose::NO_PAD);
pub fn calculate_info_hash(torrent_path: &str) -> Result<[u8; 20], Box<dyn std::error::Error>> {
    let path = Path::new(torrent_path);
    if !path.exists(){
        return Err(format!("Input file does not exist: {}", torrent_path).into());
    }

    let decoded = Decoder::decode_from(path).unwrap();
    let info = decoded.get("info".to_string()).expect("Could not find the info part");
    let info_bencoded = Encoder::encode(info).expect("Could not bencode the info values");
    let mut hasher = Sha1::new();

    hasher.update(info_bencoded);
    Ok(hasher.finalize().into())

}

// fix the pieces part!
pub fn decode_json(bpath: &str, opath: &str) -> Result<(), Box<dyn std::error::Error>> {
    let input_path = Path::new(bpath);
    let output_path = Path::new(opath);

    if !input_path.exists() {
        return Err(format!("Input file does not exist: {}", bpath).into());
    }

    match Decoder::decode_from(bpath) {
        Err(e) => Err(format!("Error opening or decoding file: {}", e).into()),
        Ok(mut t) => {
            println!("Successfully decoded torrent file");
            println!("Type of decoded content: {:?}", t);

            if let Dictionary(ref mut dict) = t {
                if let Some(Dictionary(info_dict)) = dict.get_mut("info") {
                    if let Some(pieces) = info_dict.get_mut("pieces") {
                        if let Type::ByteString(ref pieces_bytes) = pieces {

                            let pieces_base64 = general_purpose::STANDARD.encode(pieces_bytes);
                            println!("Base64 encoded pieces (first 100 chars): {}", &pieces_base64[..100.min(pieces_base64.len())]);

                            *pieces = Type::ByteString(pieces_base64.into());

                        } else {
                            println!("Pieces entry is not a byte string");
                        }
                    } else {
                        println!("No pieces entry found in the dictionary");
                    }
                } else {
                    println!("Decoded content is not a dictionary");
                }
            }

            let json_content = serde_json::to_string_pretty(&t)?;
            match File::create(output_path) {
                Ok(mut file) => {
                    match file.write_all(json_content.as_bytes()) {
                        Ok(_) => {
                            println!("Decoded to json file at: {}", opath);
                            Ok(())
                        },
                        Err(e) => Err(format!("Error writing to file: {}", e).into())
                    }
                },
                Err(e) => Err(format!("Error creating output file: {}", e).into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_json(){
        decode_json("C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].torrent","C:\\Users\\Lenovo\\Downloads\\Anomalous [FitGirl Repack].json").unwrap();
    }
}