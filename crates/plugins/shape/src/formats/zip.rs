//! Reading zip archives (3MF files are zips): stored and deflated entries.
//! A small inflater (RFC 1951) rather than a dependency: slicers deflate
//! their 3MF files, and nothing else here needs compression.

/// Every file in a zip archive, by path, uncompressed.
pub fn read(data: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let u16_at = |i: usize| data.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize).ok_or("the zip archive is cut short");
    let u32_at = |i: usize| data.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize).ok_or("the zip archive is cut short");
    // The end-of-central-directory record, searched for from the end (a comment may follow it).
    let end = (0..data.len().saturating_sub(21)).rev().take(65_557).find(|&i| data[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06])).ok_or("not a zip archive")?;
    let count = u16_at(end + 10)?;
    let mut at = u32_at(end + 16)?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if u32_at(at)? != 0x0201_4b50 {
            return Err("the zip archive's directory is damaged".into());
        }
        let (method, flags) = (u16_at(at + 10)?, u16_at(at + 8)?);
        let (packed, size) = (u32_at(at + 20)?, u32_at(at + 24)?);
        let (name_len, extra_len, comment_len) = (u16_at(at + 28)?, u16_at(at + 30)?, u16_at(at + 32)?);
        let local = u32_at(at + 42)?;
        let name = String::from_utf8_lossy(data.get(at + 46..at + 46 + name_len).ok_or("the zip archive is cut short")?).into_owned();
        at += 46 + name_len + extra_len + comment_len;
        if flags & 1 != 0 {
            return Err(format!("{name} is encrypted"));
        }
        if packed == 0xFFFF_FFFF || size == 0xFFFF_FFFF {
            return Err("zip64 archives (over 4 GB) are not read".into());
        }
        let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
        let raw = data.get(start..start + packed).ok_or("the zip archive is cut short")?;
        let bytes = match method {
            0 => raw.to_vec(),
            8 => inflate(raw, size)?,
            m => return Err(format!("{name}: compression method {m} is not supported")),
        };
        if !name.ends_with('/') {
            out.push((name, bytes));
        }
    }
    Ok(out)
}

/// Bits read least significant first, as deflate packs them.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    bit: u32,
    held: u32,
}

impl Bits<'_> {
    fn need(&mut self, n: u32) -> Result<u32, String> {
        while self.bit < n {
            let b = *self.data.get(self.at).ok_or("the compressed data is cut short")?;
            self.at += 1;
            self.held |= (b as u32) << self.bit;
            self.bit += 8;
        }
        let v = self.held & ((1u64 << n) - 1) as u32;
        self.held >>= n;
        self.bit -= n;
        Ok(v)
    }
}

/// A canonical Huffman code: symbol counts per length and symbols in order.
struct Huffman {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Huffman {
        let mut counts = [0u16; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offs = [0u16; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Huffman { counts, symbols }
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.need(1)? as i32;
            let count = self.counts[len] as i32;
            if code - count < first {
                return self.symbols.get((index + code - first) as usize).copied().ok_or_else(|| "bad compressed data".to_string());
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("bad compressed data".into())
    }
}

const LENGTH_BASE: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DIST_BASE: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// Raw deflate data (no zlib header) to the bytes it holds.
pub fn inflate(data: &[u8], expected: usize) -> Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    let mut bits = Bits { data, at: 0, bit: 0, held: 0 };
    loop {
        let last = bits.need(1)?;
        match bits.need(2)? {
            0 => {
                // Stored: byte aligned, length and its complement, then the bytes.
                bits.held = 0;
                bits.bit = 0;
                let b = data.get(bits.at..bits.at + 4).ok_or("the compressed data is cut short")?;
                let len = u16::from_le_bytes([b[0], b[1]]) as usize;
                bits.at += 4;
                out.extend_from_slice(data.get(bits.at..bits.at + len).ok_or("the compressed data is cut short")?);
                bits.at += len;
            }
            kind @ (1 | 2) => {
                let (lit, dist) = if kind == 1 {
                    let mut l = [8u8; 288];
                    l[144..256].fill(9);
                    l[256..280].fill(7);
                    (Huffman::new(&l), Huffman::new(&[5u8; 30]))
                } else {
                    let hlit = bits.need(5)? as usize + 257;
                    let hdist = bits.need(5)? as usize + 1;
                    let hclen = bits.need(4)? as usize + 4;
                    const ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
                    let mut cl = [0u8; 19];
                    for &i in &ORDER[..hclen] {
                        cl[i] = bits.need(3)? as u8;
                    }
                    let code = Huffman::new(&cl);
                    let mut lengths = Vec::with_capacity(hlit + hdist);
                    while lengths.len() < hlit + hdist {
                        let (value, times) = match code.decode(&mut bits)? {
                            s @ 0..=15 => (s as u8, 1),
                            16 => (*lengths.last().ok_or("bad compressed data")?, 3 + bits.need(2)? as usize),
                            17 => (0, 3 + bits.need(3)? as usize),
                            _ => (0, 11 + bits.need(7)? as usize),
                        };
                        lengths.extend(std::iter::repeat_n(value, times));
                    }
                    if lengths.len() > hlit + hdist {
                        return Err("bad compressed data".into());
                    }
                    (Huffman::new(&lengths[..hlit]), Huffman::new(&lengths[hlit..]))
                };
                loop {
                    let s = lit.decode(&mut bits)? as usize;
                    if s < 256 {
                        out.push(s as u8);
                    } else if s == 256 {
                        break;
                    } else {
                        let i = s - 257;
                        let len = *LENGTH_BASE.get(i).ok_or("bad compressed data")? as usize + bits.need(LENGTH_EXTRA[i] as u32)? as usize;
                        let d = dist.decode(&mut bits)? as usize;
                        let back = *DIST_BASE.get(d).ok_or("bad compressed data")? as usize + bits.need(DIST_EXTRA[d] as u32)? as usize;
                        if back > out.len() {
                            return Err("bad compressed data".into());
                        }
                        let from = out.len() - back;
                        for k in 0..len {
                            out.push(out[from + k]);
                        }
                    }
                }
            }
            _ => return Err("bad compressed data".into()),
        }
        if last == 1 {
            return Ok(out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflates_fixed_and_dynamic_blocks() {
        // Raw deflate from zlib (level 9): a fixed-code block…
        let fixed = [0xcb, 0x48, 0xcd, 0xc9, 0xc9, 0x57, 0xc8, 0x40, 0x27, 0xb9, 0x00];
        assert_eq!(inflate(&fixed, 0).unwrap(), b"hello hello hello hello\n");
        // …and a dynamic-code block (40 lines of 3MF-like vertices).
        let dynamic = [
            0x6d, 0xd3, 0xcb, 0x0d, 0xc2, 0x30, 0x10, 0x45, 0xd1, 0x3d, 0x55, 0x44, 0x54, 0x80, 0x67, 0xfc, 0x5d, 0x50, 0x4e, 0x1a, 0x40, 0x28, 0x0a, 0x54, 0x8f, 0x98, 0x67,
            0x65, 0xe3, 0xbb, 0xc9, 0xe6, 0xca, 0xbf, 0x63, 0xe7, 0xd8, 0x5f, 0xef, 0xfd, 0xdc, 0xce, 0xe7, 0xfd, 0x71, 0xdf, 0x3e, 0xf1, 0xfd, 0xfe, 0xbf, 0xb7, 0xe3, 0x0a,
            0x29, 0x42, 0x5a, 0x83, 0x45, 0xc8, 0x6b, 0xf0, 0x08, 0x63, 0x0d, 0x59, 0x53, 0xd5, 0xb5, 0x94, 0x28, 0x56, 0xd6, 0x52, 0xa3, 0x38, 0x8c, 0x69, 0x5a, 0x1f, 0xd6,
            0xe9, 0x51, 0x2a, 0x6c, 0x6d, 0x44, 0xe9, 0x70, 0x9a, 0x24, 0x00, 0x87, 0x22, 0x01, 0x83, 0xe9, 0xd2, 0x34, 0x68, 0x90, 0xa4, 0xd0, 0x0c, 0x92, 0x1c, 0xa8, 0xc8,
            0xc1, 0x69, 0x7f, 0x82, 0xa8, 0x34, 0x4a, 0x12, 0x03, 0xf4, 0x92, 0x28, 0x9c, 0x8e, 0x25, 0x8b, 0x06, 0x57, 0x6e, 0xb2, 0x48, 0xb0, 0x96, 0x09, 0xa3, 0xc0, 0x84,
            0x26, 0x8c, 0x01, 0x57, 0x65, 0xc2, 0xc8, 0x40, 0x68, 0xc2, 0x18, 0xf4, 0xc0, 0xa4, 0x91, 0x69, 0x2d, 0x69, 0x0c, 0x9a, 0x50, 0x1a, 0x85, 0xce, 0x25, 0x8d, 0x0e,
            0x45, 0x18, 0x15, 0x08, 0x5d, 0x18, 0x06, 0x77, 0xec, 0xc2, 0xe8, 0x30, 0xa1, 0x0b, 0xa3, 0xd0, 0xef, 0x21, 0x0c, 0x03, 0x5d, 0x17, 0x46, 0x87, 0x37, 0xed, 0xc2,
            0xa8, 0xe0, 0xe4, 0xf3, 0x1f, 0xa1, 0xcd, 0x0b, 0x23, 0xd1, 0xa8, 0x89, 0x01, 0xd7, 0xe5, 0x53, 0xe3, 0x4a, 0x3f,
        ];
        let want: String = (0..40).map(|i| format!("vertex x=\"{i}\" y=\"{}\" z=\"0\"\n", i * i % 97)).collect();
        assert_eq!(String::from_utf8(inflate(&dynamic, 0).unwrap()).unwrap(), want);
        // A stored block.
        let stored = [0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        assert_eq!(inflate(&stored, 3).unwrap(), b"abc");
    }

    #[test]
    fn reads_the_stored_archives_ise_writes() {
        let z = crate::fabricate::zip(&[("a.txt".into(), b"one".to_vec()), ("dir/b.txt".into(), b"two".to_vec())]);
        let files = read(&z).unwrap();
        assert_eq!(files, vec![("a.txt".to_string(), b"one".to_vec()), ("dir/b.txt".to_string(), b"two".to_vec())]);
    }
}
