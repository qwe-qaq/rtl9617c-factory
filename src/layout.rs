use crate::crc::{register, zlib, SEED};

pub const IMAGE_SIZE: usize = 0x0080_0000;
pub const MAGIC: &[u8; 8] = b"RTLFACT1";
pub const VERSION: u32 = 1;
pub const MODEL_TYFZA1: u32 = 1;
pub const MODEL_PT1500_H10: u32 = 2;

const DIR_OFFSET: usize = 0x0200;
const DIR_COUNT: usize = 32;
const ENTRY_SIZE: usize = 16;
const IMAGE_CRC_OFFSET: usize = 0x14;
const TABLE_CRC_OFFSET: usize = 0x20;
const HEADER_CRC_OFFSET: usize = 0x24;

pub struct FieldSpec {
    pub tag: u16,
    pub name: &'static str,
    pub offset: u32,
    pub capacity: u32,
}

pub const FIELDS: &[FieldSpec] = &[
    FieldSpec { tag: 0x0001, name: "BASE_MAC", offset: 0x1000, capacity: 0x10 },
    FieldSpec { tag: 0x0002, name: "EPON_MAC", offset: 0x1010, capacity: 0x06 },
    FieldSpec { tag: 0x0010, name: "PON_SN", offset: 0x1020, capacity: 0x10 },
    FieldSpec { tag: 0x0011, name: "DEVICE_SN", offset: 0x1030, capacity: 0x20 },
    FieldSpec { tag: 0x0100, name: "WIFI_CAL_RADIO0", offset: 0x2000, capacity: 0x1000 },
    FieldSpec { tag: 0x0101, name: "WIFI_CAL_RADIO1", offset: 0x3000, capacity: 0x1000 },
    FieldSpec { tag: 0x0110, name: "BOSA_CALIBRATION", offset: 0x4000, capacity: 0x0800 },
    FieldSpec { tag: 0x0111, name: "BOSA_OLT_CALIBRATION", offset: 0x4800, capacity: 0x0400 },
    FieldSpec { tag: 0x0112, name: "XTEMP_CALIBRATION", offset: 0x5000, capacity: 0x1000 },
    FieldSpec { tag: 0x0200, name: "FPGA_IMAGE", offset: 0x10000, capacity: 0x7E0000 },
];

pub fn spec(tag: u16) -> Result<&'static FieldSpec, String> {
    FIELDS
        .iter()
        .find(|f| f.tag == tag)
        .ok_or_else(|| format!("unknown factory tag 0x{tag:04x}"))
}

fn header_crc(image: &[u8]) -> u32 {
    let seed = register(&image[0x28..0x7C], SEED);
    let seed = register(&image[0x18..0x24], seed);
    register(&image[0x00..0x14], seed)
}

pub fn encode_string(value: &str, capacity: usize) -> Result<Vec<u8>, String> {
    let mut out = value.as_bytes().to_vec();
    out.push(0);
    if out.len() > capacity {
        return Err(format!(
            "string field does not fit: {} bytes > {} capacity",
            out.len(),
            capacity
        ));
    }
    Ok(out)
}

pub struct Section {
    pub tag: u16,
    pub data: Vec<u8>,
}

pub fn build(model: u32, generation: u64, sections: &[Section]) -> Result<Vec<u8>, String> {
    let mut image = vec![0xffu8; IMAGE_SIZE];
    image[0..8].copy_from_slice(MAGIC);
    image[8..12].copy_from_slice(&VERSION.to_le_bytes());
    image[12..16].copy_from_slice(&(IMAGE_SIZE as u32).to_le_bytes());
    image[16..20].copy_from_slice(&model.to_le_bytes());
    image[24..32].copy_from_slice(&generation.to_le_bytes());
    image[IMAGE_CRC_OFFSET..IMAGE_CRC_OFFSET + 4].copy_from_slice(&0u32.to_le_bytes());
    let mut written: Vec<(u16, u32, u32, u32)> = Vec::new();
    for section in sections {
        let spec = spec(section.tag)?;
        if section.data.len() > spec.capacity as usize {
            return Err(format!(
                "section {} exceeds capacity: {} > {}",
                spec.name,
                section.data.len(),
                spec.capacity
            ));
        }
        let offset = spec.offset as usize;
        let existing = written.iter().any(|(t, ..)| *t == section.tag);
        if existing {
            return Err(format!("duplicate section {}", spec.name));
        }
        image[offset..offset + section.data.len()].copy_from_slice(&section.data);
        written.push((
            section.tag,
            spec.offset,
            section.data.len() as u32,
            zlib(&section.data),
        ));
    }
    written.sort_by_key(|(t, ..)| *t);
    if written.len() > DIR_COUNT {
        return Err("too many sections for the directory".into());
    }
    for (slot, (tag, offset, size, crc)) in written.iter().enumerate() {
        let base = DIR_OFFSET + slot * ENTRY_SIZE;
        image[base..base + 2].copy_from_slice(&tag.to_le_bytes());
        image[base + 2..base + 4].copy_from_slice(&1u16.to_le_bytes());
        image[base + 4..base + 8].copy_from_slice(&offset.to_le_bytes());
        image[base + 8..base + 12].copy_from_slice(&size.to_le_bytes());
        image[base + 12..base + 16].copy_from_slice(&crc.to_le_bytes());
    }
    let table_crc = register(&image[DIR_OFFSET..DIR_OFFSET + DIR_COUNT * ENTRY_SIZE], SEED);
    image[TABLE_CRC_OFFSET..TABLE_CRC_OFFSET + 4].copy_from_slice(&table_crc.to_le_bytes());
    let header_crc = header_crc(&image);
    image[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + 4].copy_from_slice(&header_crc.to_le_bytes());
    image[IMAGE_CRC_OFFSET..IMAGE_CRC_OFFSET + 4].copy_from_slice(&0u32.to_le_bytes());
    let image_crc = register(&image, SEED);
    image[IMAGE_CRC_OFFSET..IMAGE_CRC_OFFSET + 4].copy_from_slice(&image_crc.to_le_bytes());
    Ok(image)
}

pub struct SummaryEntry {
    pub name: &'static str,
    pub offset: u32,
    pub size: u32,
    pub crc: u32,
}

pub fn summary(image: &[u8]) -> Result<Vec<SummaryEntry>, String> {
    if image.len() != IMAGE_SIZE || image[0..8] != *MAGIC {
        return Err("not an RTLFACT1 image".into());
    }
    let stored = u32::from_le_bytes(
        image[IMAGE_CRC_OFFSET..IMAGE_CRC_OFFSET + 4].try_into().unwrap(),
    );
    let mut probe = image.to_vec();
    probe[IMAGE_CRC_OFFSET..IMAGE_CRC_OFFSET + 4].copy_from_slice(&0u32.to_le_bytes());
    if register(&probe, SEED) != stored {
        return Err("image CRC mismatch".into());
    }
    let stored_header_crc = u32::from_le_bytes(
        image[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + 4].try_into().unwrap(),
    );
    if header_crc(&image) != stored_header_crc {
        return Err("header CRC mismatch".into());
    }
    let mut entries = Vec::new();
    for slot in 0..DIR_COUNT {
        let base = DIR_OFFSET + slot * ENTRY_SIZE;
        let tag = u16::from_le_bytes(image[base..base + 2].try_into().unwrap());
        if tag == 0xffff {
            continue;
        }
        let present = u16::from_le_bytes(image[base + 2..base + 4].try_into().unwrap());
        if present & 1 == 0 {
            continue;
        }
        let offset = u32::from_le_bytes(image[base + 4..base + 8].try_into().unwrap());
        let size = u32::from_le_bytes(image[base + 8..base + 12].try_into().unwrap());
        let crc = u32::from_le_bytes(image[base + 12..base + 16].try_into().unwrap());
        let spec = spec(tag)?;
        if zlib(&image[offset as usize..offset as usize + size as usize]) != crc {
            return Err(format!("section {} CRC mismatch", spec.name));
        }
        entries.push(SummaryEntry {
            name: spec.name,
            offset,
            size,
            crc,
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_and_verify_roundtrip() {
        let image = build(
            MODEL_TYFZA1,
            0,
            &[
                Section { tag: 0x0002, data: vec![1, 2, 3, 4, 5, 6] },
                Section { tag: 0x0010, data: b"TYZDD0046AB9\0".to_vec() },
            ],
        )
        .unwrap();
        assert_eq!(image.len(), IMAGE_SIZE);
        let entries = summary(&image).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "EPON_MAC");
        assert_eq!(entries[0].size, 6);
        assert_eq!(entries[1].name, "PON_SN");
        assert_eq!(&image[0x1020..0x1020 + 13], b"TYZDD0046AB9\0");
        assert_eq!(image[0x1010..0x1010 + 6], [1, 2, 3, 4, 5, 6]);
        assert_eq!(image[0x102D], 0xff);
    }

    #[test]
    fn rejects_oversize() {
        assert!(build(
            MODEL_TYFZA1,
            0,
            &[Section { tag: 0x0010, data: vec![0x41; 32] }]
        )
        .is_err());
    }

    #[test]
    fn rejects_unknown_tag() {
        assert!(build(
            MODEL_TYFZA1,
            0,
            &[Section { tag: 0x0012, data: vec![0x41; 4] }]
        )
        .is_err());
    }
}
