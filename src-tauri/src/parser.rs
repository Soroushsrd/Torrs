use bencode_encoder::{Decoder,BencodeError,BencodeResult};
use std::path::Path;
use std::fs::File;
use std::io::Write;


pub fn decode_json(bpath: &str, opath: &str) -> Result<(), Box<dyn std::error::Error>> {
    let input_path = Path::new(bpath);
    let output_path = Path::new(opath);

    if !input_path.exists() {
        return Err(format!("Input file does not exist: {}", bpath).into());
    }

    match Decoder::decode_from(bpath) {
        Err(e) => Err(format!("Error opening or decoding file: {}", e).into()),
        Ok(t) => {
            println!("Successfully decoded torrent file");
            let json_content = t.to_json()?;
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
        decode_json("C:\\Users\\Lenovo\\Downloads\\DreadHaunt [FitGirl Repack].torrent","C:\\Users\\Lenovo\\Downloads\\example.json").unwrap();
    }
}