use crate::crc::register;
use std::collections::BTreeMap;

const SEED: u32 = 0xFFFF_FFFF;
const VTBL_VOL_ID: u32 = 0x7FFF_EFFF;
const VTBL_RECORD_SIZE: usize = 172;
const MAX_VOLUMES: usize = 128;

pub struct Volume {
    pub lebs: Vec<Vec<u8>>,
}

pub struct Image {
    pub peb_size: usize,
    pub leb_size: usize,
    pub volumes: BTreeMap<String, Volume>,
}

impl Image {
    pub fn volume(&self, name: &str) -> Result<&Volume, String> {
        self.volumes
            .get(name)
            .ok_or_else(|| format!("UBI volume {name} is missing"))
    }
}

fn be32(d: &[u8], offset: usize) -> Result<u32, String> {
    let raw = d
        .get(offset..offset + 4)
        .ok_or("image truncated in UBI header")?;
    Ok(u32::from_be_bytes(raw.try_into().unwrap()))
}

fn be64(d: &[u8], offset: usize) -> Result<u64, String> {
    let raw = d
        .get(offset..offset + 8)
        .ok_or("image truncated in UBI header")?;
    Ok(u64::from_be_bytes(raw.try_into().unwrap()))
}

fn check_crc(d: &[u8], expected: u32) -> Result<(), String> {
    let actual = register(d, SEED);
    if actual != expected {
        return Err(format!(
            "UBI CRC mismatch: stored 0x{expected:08x}, calculated 0x{actual:08x}"
        ));
    }
    Ok(())
}

pub fn parse(image: &[u8]) -> Result<Image, String> {
    let peb_size = (16..=20)
        .map(|n| 1usize << n)
        .find(|&s| image.len() % s == 0 && image.get(s..s + 4) == Some(b"UBI#"))
        .ok_or("cannot determine UBI erase block size")?;
    let mut blocks: BTreeMap<(u32, u32), (u64, Vec<u8>)> = BTreeMap::new();
    let mut leb_size = 0usize;
    for chunk in image.chunks_exact(peb_size) {
        if chunk.iter().all(|&b| b == 0xff) {
            continue;
        }
        if !chunk.starts_with(b"UBI#") {
            return Err("invalid UBI erase counter header".into());
        }
        check_crc(&chunk[..60], be32(chunk, 60)?)?;
        let vid_offset = be32(chunk, 16)? as usize;
        let data_offset = be32(chunk, 20)? as usize;
        if vid_offset >= peb_size || data_offset > peb_size || vid_offset + 64 > peb_size {
            return Err("UBI header offsets out of range".into());
        }
        let vid = &chunk[vid_offset..vid_offset + 64];
        if vid.iter().all(|&b| b == 0xff) {
            continue;
        }
        if !vid.starts_with(b"UBI!") {
            return Err("invalid UBI VID header".into());
        }
        check_crc(&vid[..60], be32(vid, 60)?)?;
        let vol_id = be32(vid, 8)?;
        let lnum = be32(vid, 12)?;
        let data_size = be32(vid, 20)? as usize;
        let payload = chunk[data_offset..].to_vec();
        if leb_size == 0 {
            leb_size = payload.len();
        }
        if vid[6] != 0 || vid[5] == 2 {
            let size = data_size.min(payload.len());
            let validity = check_crc(&payload[..size], be32(vid, 32)?);
            if validity.is_err() && vid[6] != 0 {
                continue;
            }
            validity?;
        }
        let sequence = be64(vid, 40)?;
        match blocks.get(&(vol_id, lnum)) {
            Some((old, _)) if *old > sequence => {}
            _ => {
                blocks.insert((vol_id, lnum), (sequence, payload));
            }
        }
    }
    if leb_size == 0 {
        return Err("UBI image contains no data blocks".into());
    }
    let table = blocks
        .get(&(VTBL_VOL_ID, 0))
        .or_else(|| blocks.get(&(VTBL_VOL_ID, 1)))
        .ok_or("UBI volume table is missing")?
        .1
        .clone();
    let mut volumes: BTreeMap<String, Volume> = BTreeMap::new();
    for (index, record) in table.chunks_exact(VTBL_RECORD_SIZE).take(MAX_VOLUMES).enumerate() {
        let reserved = i32::from_be_bytes(record[0..4].try_into().unwrap());
        if reserved == 0 {
            continue;
        }
        let name_len = u16::from_be_bytes(record[14..16].try_into().unwrap()) as usize;
        if name_len > 128 {
            return Err("invalid UBI volume name length".into());
        }
        check_crc(&record[..168], be32(record, 168)?)?;
        if record[13] != 0 {
            return Err("UBI volume update marker is set".into());
        }
        let name = std::str::from_utf8(&record[16..16 + name_len])
            .map_err(|_| "invalid UBI volume name".to_string())?
            .to_string();
        let reserved = reserved.max(0) as u32;
        let mut lebs = Vec::new();
        for lnum in 0..reserved {
            match blocks.get(&(index as u32, lnum)) {
                Some((_, payload)) => lebs.push(payload.clone()),
                None => {
                    if record[12] == 1 {
                        lebs.push(vec![0xff; leb_size]);
                    } else {
                        return Err(format!(
                            "static volume {name} is missing LEB {lnum}"
                        ));
                    }
                }
            }
        }
        volumes.insert(
            name.clone(),
            Volume { lebs },
        );
    }
    Ok(Image {
        peb_size,
        leb_size,
        volumes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_garbage() {
        assert!(parse(&[0u8; 4096]).is_err());
        assert!(parse(&[0xff; 1 << 18]).is_err());
    }
}
