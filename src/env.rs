use crate::crc::zlib;
use std::collections::BTreeMap;

pub struct EnvSet {
    pub variables: BTreeMap<String, String>,
}

impl EnvSet {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.variables.get(key).map(|v| v.as_str())
    }
}

pub fn load(partitions: &[&[u8]]) -> Result<EnvSet, String> {
    let mut best: Option<(u8, usize, BTreeMap<String, String>)> = None;
    for (index, partition) in partitions.iter().enumerate() {
        if let Some((flags, variables)) = parse(partition) {
            if best.as_ref().map_or(true, |(f, ..)| flags > *f) {
                best = Some((flags, index, variables));
            }
        }
    }
    match best {
        Some((_, _, variables)) => Ok(EnvSet { variables }),
        None => Err("no valid U-Boot environment partition".into()),
    }
}

fn parse(data: &[u8]) -> Option<(u8, BTreeMap<String, String>)> {
    if data.len() < 8 || data[0..4] == [0xff; 4] {
        return None;
    }
    let stored = u32::from_le_bytes(data[0..4].try_into().ok()?);
    let body = &data[5..];
    if zlib(body) != stored {
        return None;
    }
    let mut map = BTreeMap::new();
    for entry in body.split(|&b| b == 0) {
        if entry.is_empty() {
            continue;
        }
        let text = std::str::from_utf8(entry).ok()?;
        let (key, value) = text.split_once('=')?;
        map.insert(key.to_string(), value.to_string());
    }
    if map.is_empty() {
        return None;
    }
    Some((data[4], map))
}

pub fn parse_mac(value: &str) -> Result<[u8; 6], String> {
    let hex: String = value.chars().filter(|c| *c != ':').collect();
    if hex.len() != 12 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid MAC value: {value}"));
    }
    let mut mac = [0u8; 6];
    for (i, byte) in mac.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(mac)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_formats() {
        assert_eq!(
            parse_mac("d0:5b:cb:9c:9b:f0").unwrap(),
            [0xd0, 0x5b, 0xcb, 0x9c, 0x9b, 0xf0]
        );
        assert_eq!(
            parse_mac("d05bcb9c9bf0").unwrap(),
            [0xd0, 0x5b, 0xcb, 0x9c, 0x9b, 0xf0]
        );
        assert!(parse_mac("d05bcb9c9b").is_err());
        assert!(parse_mac("zz5bcb9c9bf0").is_err());
    }
}
