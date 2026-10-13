//! Containers other EDA tools wrap their files in: zip archives (EasyEDA Pro
//! projects, KiCad project bundles) read with their deflated entries
//! inflated, and OLE compound files (Microsoft's Compound File Binary format,
//! [MS-CFB], which Altium's binary PcbDoc/SchDoc use) read stream by stream.

use std::collections::BTreeMap;

fn u16le(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).unwrap_or(0)
}
fn u32le(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).unwrap_or(0)
}

pub fn is_zip(b: &[u8]) -> bool {
    b.starts_with(b"PK\x03\x04") || b.starts_with(b"PK\x05\x06")
}

/// Every file in a zip archive by its path (stored and deflated entries).
pub fn unzip(b: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    // End of central directory: the last "PK\5\6" within the final 64 KiB + 22 bytes.
    let start = b.len().saturating_sub(65557);
    let eocd = (start..b.len().saturating_sub(21)).rev().find(|&i| &b[i..i + 4] == b"PK\x05\x06").ok_or("not a zip archive (no central directory)")?;
    let count = u16le(b, eocd + 10) as usize;
    let mut p = u32le(b, eocd + 16) as usize;
    let mut out = BTreeMap::new();
    for _ in 0..count {
        if u32le(b, p) != 0x0201_4b50 {
            return Err("zip central directory is damaged".into());
        }
        let method = u16le(b, p + 10);
        let csize = u32le(b, p + 20) as usize;
        let usize_ = u32le(b, p + 24) as usize;
        let (nl, xl, cl) = (u16le(b, p + 28) as usize, u16le(b, p + 30) as usize, u16le(b, p + 32) as usize);
        let local = u32le(b, p + 42) as usize;
        let name = String::from_utf8_lossy(b.get(p + 46..p + 46 + nl).ok_or("zip entry name is cut off")?).replace('\\', "/");
        p += 46 + nl + xl + cl;
        if name.ends_with('/') {
            continue;
        }
        let data_at = local + 30 + u16le(b, local + 26) as usize + u16le(b, local + 28) as usize;
        let raw = b.get(data_at..data_at + csize).ok_or_else(|| format!("zip entry {name} is cut off"))?;
        let data = match method {
            0 => raw.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(raw, usize_.max(1) * 2 + 1024).map_err(|e| format!("zip entry {name}: {e:?}"))?,
            m => return Err(format!("zip entry {name} uses compression method {m}")),
        };
        out.insert(name, data);
    }
    Ok(out)
}

/// A zlib stream inflated (Altium keeps some blobs this way).
pub fn inflate_zlib(b: &[u8]) -> Option<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib(b).ok()
}

// ---- OLE compound files ---------------------------------------------------------------

const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FREESECT: u32 = 0xFFFF_FFFF;

pub fn is_cfb(b: &[u8]) -> bool {
    b.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1])
}

struct DirEntry {
    name: String,
    kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

/// Every stream of a compound file by its path ("Board6/Data", "FileHeader").
pub fn cfb_streams(b: &[u8]) -> Result<BTreeMap<String, Vec<u8>>, String> {
    if !is_cfb(b) || b.len() < 512 {
        return Err("not an OLE compound file".into());
    }
    let shift = u16le(b, 0x1E) as u32;
    let mini_shift = u16le(b, 0x20) as u32;
    if !(7..=16).contains(&shift) || mini_shift > shift {
        return Err("compound file header has a bad sector size".into());
    }
    let ss = 1usize << shift;
    let mss = 1usize << mini_shift;
    let n_fat = u32le(b, 0x2C) as usize;
    let dir_start = u32le(b, 0x30);
    let cutoff = u32le(b, 0x38) as u64;
    let minifat_start = u32le(b, 0x3C);
    let mut difat_sector = u32le(b, 0x44);
    let sector = |n: u32| -> Option<&[u8]> {
        let at = (n as usize + 1) * ss;
        b.get(at..at + ss)
    };
    // The FAT's sectors: 109 in the header, the rest in the DIFAT chain.
    let mut fat_sectors: Vec<u32> = (0..109).map(|i| u32le(b, 0x4C + i * 4)).filter(|&s| s != FREESECT).collect();
    let mut guard = 0;
    while difat_sector != ENDOFCHAIN && difat_sector != FREESECT && fat_sectors.len() < n_fat && guard < 1_000_000 {
        let s = sector(difat_sector).ok_or("DIFAT sector out of the file")?;
        let per = ss / 4 - 1;
        for i in 0..per {
            let v = u32le(s, i * 4);
            if v != FREESECT {
                fat_sectors.push(v);
            }
        }
        difat_sector = u32le(s, per * 4);
        guard += 1;
    }
    fat_sectors.truncate(n_fat.max(fat_sectors.len().min(n_fat)));
    let mut fat: Vec<u32> = Vec::new();
    for &fs in &fat_sectors {
        let s = sector(fs).ok_or("FAT sector out of the file")?;
        fat.extend((0..ss / 4).map(|i| u32le(s, i * 4)));
    }
    let chain = |start: u32| -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        let mut s = start;
        let mut n = 0usize;
        while s != ENDOFCHAIN && s != FREESECT {
            out.extend_from_slice(sector(s).ok_or("a stream runs out of the file")?);
            s = *fat.get(s as usize).ok_or("a stream's chain leaves the FAT")?;
            n += 1;
            if n > fat.len() + 1 {
                return Err("a sector chain loops".into());
            }
        }
        Ok(out)
    };
    let dir = chain(dir_start)?;
    let entries: Vec<DirEntry> = dir
        .chunks(128)
        .filter(|c| c.len() == 128)
        .map(|c| {
            let nl = (u16le(c, 0x40) as usize).min(64);
            let units: Vec<u16> = (0..nl.saturating_sub(2) / 2).map(|i| u16le(c, i * 2)).collect();
            DirEntry { name: String::from_utf16_lossy(&units), kind: c[0x42], left: u32le(c, 0x44), right: u32le(c, 0x48), child: u32le(c, 0x4C), start: u32le(c, 0x74), size: u32le(c, 0x78) as u64 | ((u32le(c, 0x7C) as u64) << 32) * (if ss == 512 { 0 } else { 1 }) }
        })
        .collect();
    let root = entries.first().ok_or("compound file has no root")?;
    let ministream = if root.start != ENDOFCHAIN { chain(root.start)? } else { vec![] };
    let minifat: Vec<u32> = if minifat_start != ENDOFCHAIN { chain(minifat_start)?.chunks(4).map(|c| u32le(c, 0)).collect() } else { vec![] };
    let read = |e: &DirEntry| -> Result<Vec<u8>, String> {
        let size = e.size as usize;
        if e.size < cutoff {
            let mut out = Vec::with_capacity(size);
            let mut s = e.start;
            let mut n = 0;
            while s != ENDOFCHAIN && s != FREESECT && out.len() < size {
                let at = s as usize * mss;
                out.extend_from_slice(ministream.get(at..at + mss).ok_or("a mini stream runs out")?);
                s = *minifat.get(s as usize).ok_or("a mini chain leaves the mini FAT")?;
                n += 1;
                if n > minifat.len() + 1 {
                    return Err("a mini chain loops".into());
                }
            }
            out.truncate(size);
            Ok(out)
        } else {
            let mut out = chain(e.start)?;
            out.truncate(size);
            Ok(out)
        }
    };
    // Walk the tree: each storage's children are a binary tree of siblings.
    let mut out = BTreeMap::new();
    let mut stack: Vec<(u32, String)> = vec![(root.child, String::new())];
    let mut seen = 0usize;
    while let Some((i, prefix)) = stack.pop() {
        if i == FREESECT || i as usize >= entries.len() {
            continue;
        }
        seen += 1;
        if seen > entries.len() * 2 {
            break;
        }
        let e = &entries[i as usize];
        stack.push((e.left, prefix.clone()));
        stack.push((e.right, prefix.clone()));
        let path = if prefix.is_empty() { e.name.clone() } else { format!("{prefix}/{}", e.name) };
        match e.kind {
            1 => stack.push((e.child, path)),
            2 => {
                out.insert(path, read(e)?);
            }
            _ => {}
        }
    }
    Ok(out)
}

/// A compound file holding these streams (paths with one storage level at most), 512-byte
/// sectors, every stream in regular sectors. Written for test fixtures and round trips.
pub fn cfb_write(streams: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let ss = 512usize;
    // Directory: root, storages, streams. Siblings chained through `right` (a degenerate tree).
    struct E {
        name: String,
        kind: u8,
        child: u32,
        right: u32,
        start: u32,
        size: u32,
    }
    let mut ents: Vec<E> = vec![E { name: "Root Entry".into(), kind: 5, child: FREESECT, right: FREESECT, start: ENDOFCHAIN, size: 0 }];
    let mut storages: BTreeMap<String, usize> = BTreeMap::new();
    let mut data: Vec<u8> = Vec::new();
    let mut fat: Vec<u32> = Vec::new();
    let place = |bytes: &[u8], fat: &mut Vec<u32>, data: &mut Vec<u8>| -> u32 {
        if bytes.is_empty() {
            return ENDOFCHAIN;
        }
        let first = fat.len() as u32;
        let n = bytes.len().div_ceil(ss);
        for k in 0..n {
            fat.push(if k + 1 == n { ENDOFCHAIN } else { first + k as u32 + 1 });
        }
        data.extend_from_slice(bytes);
        data.resize(fat.len() * ss, 0);
        first
    };
    let link = |ents: &mut Vec<E>, parent: usize, idx: usize| {
        let mut c = ents[parent].child;
        if c == FREESECT {
            ents[parent].child = idx as u32;
            return;
        }
        while ents[c as usize].right != FREESECT {
            c = ents[c as usize].right;
        }
        ents[c as usize].right = idx as u32;
    };
    for (path, bytes) in streams {
        let (parent, name) = match path.split_once('/') {
            Some((st, name)) => {
                let p = match storages.get(st) {
                    Some(&p) => p,
                    None => {
                        ents.push(E { name: st.to_string(), kind: 1, child: FREESECT, right: FREESECT, start: 0, size: 0 });
                        let i = ents.len() - 1;
                        link(&mut ents, 0, i);
                        storages.insert(st.to_string(), i);
                        i
                    }
                };
                (p, name.to_string())
            }
            None => (0, path.to_string()),
        };
        let start = place(bytes, &mut fat, &mut data);
        ents.push(E { name, kind: 2, child: FREESECT, right: FREESECT, start, size: bytes.len() as u32 });
        let i = ents.len() - 1;
        link(&mut ents, parent, i);
    }
    let mut dir = Vec::new();
    for e in &ents {
        let mut rec = vec![0u8; 128];
        let units: Vec<u16> = e.name.encode_utf16().take(31).collect();
        for (k, u) in units.iter().enumerate() {
            rec[k * 2..k * 2 + 2].copy_from_slice(&u.to_le_bytes());
        }
        rec[0x40..0x42].copy_from_slice(&(((units.len() + 1) * 2) as u16).to_le_bytes());
        rec[0x42] = e.kind;
        rec[0x43] = 1;
        rec[0x44..0x48].copy_from_slice(&FREESECT.to_le_bytes());
        rec[0x48..0x4C].copy_from_slice(&e.right.to_le_bytes());
        rec[0x4C..0x50].copy_from_slice(&e.child.to_le_bytes());
        rec[0x74..0x78].copy_from_slice(&e.start.to_le_bytes());
        rec[0x78..0x7C].copy_from_slice(&e.size.to_le_bytes());
        dir.extend_from_slice(&rec);
    }
    let dir_start = place(&dir, &mut fat, &mut data);
    // FAT sectors themselves (marked 0xFFFFFFFD), enough to cover everything including them.
    let mut n_fat = 1;
    while (fat.len() + n_fat) > n_fat * (ss / 4) {
        n_fat += 1;
    }
    let fat_first = fat.len() as u32;
    for _ in 0..n_fat {
        fat.push(0xFFFF_FFFD);
    }
    let mut fat_bytes: Vec<u8> = fat.iter().flat_map(|v| v.to_le_bytes()).collect();
    fat_bytes.resize(n_fat * ss, 0xFF);
    data.extend_from_slice(&fat_bytes);
    let mut h = vec![0u8; 512];
    h[..8].copy_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    h[0x18..0x1A].copy_from_slice(&0x3Eu16.to_le_bytes());
    h[0x1A..0x1C].copy_from_slice(&3u16.to_le_bytes());
    h[0x1C..0x1E].copy_from_slice(&0xFFFEu16.to_le_bytes());
    h[0x1E..0x20].copy_from_slice(&9u16.to_le_bytes());
    h[0x20..0x22].copy_from_slice(&6u16.to_le_bytes());
    h[0x2C..0x30].copy_from_slice(&(n_fat as u32).to_le_bytes());
    h[0x30..0x34].copy_from_slice(&dir_start.to_le_bytes());
    // Cutoff 0: no mini stream, every stream lives in regular sectors.
    h[0x38..0x3C].copy_from_slice(&0u32.to_le_bytes());
    h[0x3C..0x40].copy_from_slice(&ENDOFCHAIN.to_le_bytes());
    h[0x44..0x48].copy_from_slice(&ENDOFCHAIN.to_le_bytes());
    for i in 0..109 {
        let v = if i < n_fat { fat_first + i as u32 } else { FREESECT };
        h[0x4C + i * 4..0x50 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    let mut out = h;
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compound_files_round_trip() {
        let big: Vec<u8> = (0..5000u32).map(|i| (i % 251) as u8).collect();
        let f = cfb_write(&[("FileHeader", b"hello".to_vec()), ("Board6/Data", big.clone()), ("Board6/Header", vec![1, 0, 0, 0])]);
        let s = cfb_streams(&f).unwrap();
        assert_eq!(s["FileHeader"], b"hello");
        assert_eq!(s["Board6/Data"], big);
        assert_eq!(s["Board6/Header"], vec![1, 0, 0, 0]);
    }

    #[test]
    fn zips_read_back() {
        let z = de_shape::fabricate::zip(&[("a/b.txt".into(), b"one".to_vec()), ("c.json".into(), b"{}".to_vec())]);
        let m = unzip(&z).unwrap();
        assert_eq!(m["a/b.txt"], b"one");
        assert_eq!(m["c.json"], b"{}");
    }
}
