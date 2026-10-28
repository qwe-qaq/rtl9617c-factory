use crate::crc::register;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

pub type Files = BTreeMap<String, Vec<u8>>;

const MAX_FILE: usize = 16 * 1024 * 1024;
const UBIFS_MAGIC: u32 = 0x0610_1831;
const SEED: u32 = 0xFFFF_FFFF;
const NODE_HEADER: usize = 24;
const PAGE_SIZE: usize = 4096;

fn le16(d: &[u8], offset: usize) -> Result<u16, String> {
    let raw = d
        .get(offset..offset + 2)
        .ok_or("image truncated in UBIFS node")?;
    Ok(u16::from_le_bytes(raw.try_into().unwrap()))
}

fn le32(d: &[u8], offset: usize) -> Result<u32, String> {
    let raw = d
        .get(offset..offset + 4)
        .ok_or("image truncated in UBIFS node")?;
    Ok(u32::from_le_bytes(raw.try_into().unwrap()))
}

fn le64(d: &[u8], offset: usize) -> Result<u64, String> {
    let raw = d
        .get(offset..offset + 8)
        .ok_or("image truncated in UBIFS node")?;
    Ok(u64::from_le_bytes(raw.try_into().unwrap()))
}

fn node(d: &[u8], offset: usize) -> Result<&[u8], String> {
    let header = d
        .get(offset..offset + NODE_HEADER)
        .ok_or("UBIFS node truncated")?;
    if le32(header, 0)? != UBIFS_MAGIC {
        return Err("invalid UBIFS node magic".into());
    }
    let len = le32(header, 16)? as usize;
    let full = d.get(offset..offset + len).ok_or("UBIFS node truncated")?;
    if full.len() < NODE_HEADER {
        return Err("invalid UBIFS node length".into());
    }
    let expected = le32(full, 4)?;
    let actual = register(&full[8..], SEED);
    if actual != expected {
        return Err(format!(
            "UBIFS node CRC mismatch: stored 0x{expected:08x}, calculated 0x{actual:08x}"
        ));
    }
    Ok(full)
}

fn decompress(d: &[u8], size: usize, kind: u16) -> Result<Vec<u8>, String> {
    if size > MAX_FILE {
        return Err("UBIFS data block too large".into());
    }
    let out = match kind {
        0 => d.to_vec(),
        1 => lzokay_native::decompress(&mut Cursor::new(d), Some(size)).map_err(|e| e.to_string())?,
        2 => {
            let mut out = Vec::new();
            flate2::read::DeflateDecoder::new(d)
                .take(size as u64)
                .read_to_end(&mut out)
                .map_err(|e| e.to_string())?;
            out
        }
        3 => return Err("UBIFS zstd compression is not supported".into()),
        other => return Err(format!("unsupported UBIFS compression type {other}")),
    };
    if out.len() != size {
        return Err("UBIFS decompressed length mismatch".into());
    }
    Ok(out)
}

fn valid_name(d: &[u8]) -> Result<String, String> {
    let s = std::str::from_utf8(d).map_err(|e| e.to_string())?;
    if s.is_empty() || s.contains(['/', '\0']) || s == "." || s == ".." {
        return Err("invalid UBIFS directory entry name".into());
    }
    Ok(s.to_string())
}

pub fn extract(lebs: &BTreeMap<u32, Vec<u8>>) -> Result<Files, String> {
    let superblock = node(lebs.get(&0).ok_or("UBIFS superblock is missing")?, 0)?;
    if superblock[NODE_HEADER + 3] != 0 {
        return Err("unsupported UBIFS key format".into());
    }
    let mut master: Option<&[u8]> = None;
    for index in [1u32, 2u32] {
        let leb = match lebs.get(&index) {
            Some(l) => l,
            None => continue,
        };
        for offset in (0..leb.len().saturating_sub(NODE_HEADER)).step_by(8) {
            if leb.get(offset..offset + 4) != Some(&UBIFS_MAGIC.to_le_bytes()) {
                continue;
            }
            if let Ok(candidate) = node(leb, offset) {
                if candidate[NODE_HEADER - 4] == 7 {
                    let newer = match master {
                        Some(old) => le64(candidate, 8).unwrap_or(0) > le64(old, 8).unwrap_or(0),
                        None => true,
                    };
                    if newer {
                        master = Some(candidate);
                    }
                }
            }
        }
    }
    let master = master.ok_or("UBIFS master node is missing")?;
    let mut todo = vec![(
        le32(master, 48)?,
        le32(master, 52)? as usize,
        le32(master, 56)? as usize,
    )];
    let mut seen: BTreeSet<(u32, usize)> = BTreeSet::new();
    let mut dents: BTreeMap<(u64, String), u64> = BTreeMap::new();
    let mut sizes: BTreeMap<u64, usize> = BTreeMap::new();
    let mut chunks: BTreeMap<(u64, u32), &[u8]> = BTreeMap::new();
    while let Some((lnum, offset, len)) = todo.pop() {
        if !seen.insert((lnum, offset)) {
            return Err("UBIFS index contains duplicate references".into());
        }
        let leb = lebs
            .get(&lnum)
            .ok_or_else(|| {
                format!(
                    "UBIFS index references missing LEB {lnum}; volume has {} LEBs: {:?}",
                    lebs.len(),
                    lebs.keys().take(8).collect::<Vec<_>>()
                )
            })?;
        let n = node(leb, offset)?;
        if n.len() != len {
            return Err("UBIFS index node length mismatch".into());
        }
        match n[NODE_HEADER - 4] {
            9 => {
                let count = le16(n, 24)? as usize;
                if n.len() != 28 + count * 20 {
                    return Err("unsupported UBIFS index branch layout".into());
                }
                for branch in n[28..].chunks_exact(20) {
                    todo.push((
                        le32(branch, 0)?,
                        le32(branch, 4)? as usize,
                        le32(branch, 8)? as usize,
                    ));
                }
            }
            0 => {
                if le32(n, 104)? & 0xf000 == 0x8000 {
                    sizes.insert(le32(n, 24)? as u64, le64(n, 48)? as usize);
                }
            }
            2 => {
                let parent = le32(n, 24)? as u64;
                let nlen = le16(n, 50)? as usize;
                let name = valid_name(n.get(56..56 + nlen).ok_or("UBIFS dent name truncated")?)?;
                let ino = le64(n, 40)?;
                dents.insert((parent, name), ino);
            }
            1 => {
                if le32(n, 40)? > PAGE_SIZE as u32 {
                    return Err("UBIFS data block exceeds page size".into());
                }
                let ino = le32(n, 24)? as u64;
                let block = le32(n, 28)? & 0x1fff_ffff;
                chunks.insert((ino, block), n);
            }
            3 => {}
            other => return Err(format!("unexpected UBIFS node type {other} in index")),
        }
    }
    build_files(&dents, &sizes, &chunks)
}

fn build_files(
    dents: &BTreeMap<(u64, String), u64>,
    sizes: &BTreeMap<u64, usize>,
    chunks: &BTreeMap<(u64, u32), &[u8]>,
) -> Result<Files, String> {
    let mut files = Files::new();
    let mut todo = vec![(1u64, String::new())];
    let mut visited: BTreeSet<u64> = BTreeSet::new();
    while let Some((parent, prefix)) = todo.pop() {
        if !visited.insert(parent) {
            return Err("UBIFS directory tree contains a cycle".into());
        }
        for ((p, name), ino) in dents {
            if *p != parent || *ino == 0 {
                continue;
            }
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            match sizes.get(ino) {
                Some(&size) => {
                    if size > MAX_FILE {
                        return Err(format!("{path} exceeds the size limit"));
                    }
                    let mut out = vec![0u8; size];
                    for ((i, block), n) in chunks {
                        if *i != *ino {
                            continue;
                        }
                        let start = (*block as usize)
                            .checked_mul(PAGE_SIZE)
                            .ok_or("UBIFS block offset overflow")?;
                        if start >= size {
                            return Err(format!("{path} block exceeds file size"));
                        }
                        let decoded = decompress(
                            &n[48..],
                            le32(n, 40)? as usize,
                            le16(n, 44)?,
                        )?;
                        let count = decoded.len().min(size - start);
                        out[start..start + count].copy_from_slice(&decoded[..count]);
                    }
                    files.insert(path, out);
                }
                None => todo.push((*ino, path)),
            }
        }
    }
    Ok(files)
}
