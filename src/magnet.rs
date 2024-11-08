use data_encoding::BASE32;
use percent_encoding::percent_decode_str;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use url::Url;

#[derive(Debug)]
#[allow(dead_code)]
pub struct MagnetInfo {
    pub info_hash: [u8; 20],
    pub display_name: Option<String>,
    pub trackers: Vec<String>,
    pub peers: Option<Vec<String>>,
}

impl MagnetInfo {
    pub fn parse(magnet_url: &str) -> Result<MagnetInfo, Box<dyn std::error::Error>> {
        let url = Url::parse(magnet_url).unwrap();
        let params: HashMap<_, _> = url.query_pairs().collect();

        let info_hash = if let Some(xt) = params.get("xt") {
            if let Some(hash) = xt.strip_prefix("urn:btih") {
                if hash.len() == 40 {
                    let mut result = [0u8; 20];
                    hex::decode_to_slice(hash, &mut result).unwrap();
                    result
                } else if hash.len() == 32 {
                    let mut result = [0u8; 20];
                    let decoded = BASE32.decode(hash.as_bytes()).unwrap();
                    result.copy_from_slice(&decoded);
                    result
                } else {
                    return Err("Invalid info hash".into());
                }
            } else {
                return Err("Invalid xt parameter".into());
            }
        } else {
            return Err("Missing xt parameter".into());
        };

        let display_name = params
            .get("dn")
            .map(|dn| percent_decode_str(dn).decode_utf8_lossy().into_owned());

        let trackers = params
            .iter()
            .filter(|(k, _)| *k == "tr")
            .map(|(_, tracker)| percent_decode_str(tracker).decode_utf8_lossy().into_owned())
            .collect();

        let peers = Some(
            params
                .iter()
                .filter(|(k, _)| k.starts_with("x.pe"))
                .map(|(_, v)| percent_decode_str(v).decode_utf8_lossy().into_owned())
                .collect(),
        );

        Ok(MagnetInfo {
            info_hash,
            display_name,
            trackers,
            peers,
        })
    }
}
