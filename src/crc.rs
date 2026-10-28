const POLY: u32 = 0xEDB8_8320;

fn table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, entry) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { POLY ^ (c >> 1) } else { c >> 1 };
            }
            *entry = c;
        }
        t
    })
}

pub const SEED: u32 = 0xFFFF_FFFF;

pub fn register(data: &[u8], seed: u32) -> u32 {
    let t = table();
    let mut c = seed;
    for &b in data {
        c = t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8);
    }
    c
}

pub fn zlib(data: &[u8]) -> u32 {
    register(data, SEED) ^ SEED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(zlib(b"123456789"), 0xCBF4_3926);
        assert_eq!(register(b"123456789", SEED), 0xCBF4_3926 ^ SEED);
    }
}
