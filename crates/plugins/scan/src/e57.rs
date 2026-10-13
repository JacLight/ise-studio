//! A reader for ASTM E57 laser-scan files (the vendor-neutral format
//! terrestrial scanners export), from the published standard (E2807): the
//! paged file (1024-byte pages, each ending in a checksum), its XML tree,
//! and each scan's compressed vector of points, bit-packed field by field.
//! Cartesian and spherical coordinates, float, integer and scaled-integer
//! fields, colours, and each scan's pose are read; images and intensity are
//! not needed here and skipped.

use crate::V3;
use crate::linalg::*;
use crate::points::Cloud;
use std::collections::HashMap;

/// An XML element: name, attributes, text, children.
#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: HashMap<String, String>,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(|s| s.as_str())
    }
    fn num(&self, name: &str) -> Option<f64> {
        self.attr(name).and_then(|v| v.trim().parse().ok())
    }
    fn value(&self) -> Option<f64> {
        self.text.trim().parse().ok()
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// A small XML parser: enough for E57's tree (no DTDs or entities beyond the five).
fn parse_xml(text: &str) -> Result<Node, String> {
    let mut stack: Vec<Node> = vec![Node::default()];
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            if text[i..].starts_with("<?") {
                i = text[i..].find("?>").map(|e| i + e + 2).ok_or("bad XML")?;
                continue;
            }
            if text[i..].starts_with("<!--") {
                i = text[i..].find("-->").map(|e| i + e + 3).ok_or("bad XML")?;
                continue;
            }
            if text[i..].starts_with("<![CDATA[") {
                let e = text[i..].find("]]>").ok_or("bad XML")?;
                stack.last_mut().unwrap().text.push_str(&text[i + 9..i + e]);
                i += e + 3;
                continue;
            }
            let end = text[i..].find('>').map(|e| i + e).ok_or("bad XML")?;
            let tag = &text[i + 1..end];
            if let Some(name) = tag.strip_prefix('/') {
                let node = stack.pop().ok_or("bad XML")?;
                if node.name != name.trim() {
                    return Err(format!("XML: </{}> closes <{}>", name.trim(), node.name));
                }
                stack.last_mut().ok_or("bad XML")?.children.push(node);
            } else {
                let empty = tag.ends_with('/');
                let tag = tag.trim_end_matches('/');
                let name_end = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
                let mut node = Node { name: tag[..name_end].to_string(), ..Default::default() };
                let mut rest = &tag[name_end..];
                while let Some(eq) = rest.find('=') {
                    let key = rest[..eq].trim().to_string();
                    let after = rest[eq + 1..].trim_start();
                    let q = after.chars().next().ok_or("bad XML attribute")?;
                    let close = after[1..].find(q).ok_or("bad XML attribute")? + 1;
                    node.attrs.insert(key, unescape(&after[1..close]));
                    rest = &after[close + 1..];
                }
                if empty {
                    stack.last_mut().unwrap().children.push(node);
                } else {
                    stack.push(node);
                }
            }
            i = end + 1;
        } else {
            let end = text[i..].find('<').map(|e| i + e).unwrap_or(b.len());
            stack.last_mut().unwrap().text.push_str(&unescape(&text[i..end]));
            i = end;
        }
    }
    let mut root = stack.pop().ok_or("bad XML")?;
    root.children.pop().ok_or_else(|| "empty XML".to_string())
}

/// One field of a scan's records.
#[derive(Debug)]
enum Field {
    Float { double: bool },
    Integer { min: i64, bits: u32 },
    Scaled { min: i64, bits: u32, scale: f64, offset: f64 },
}

fn bits_for(min: i64, max: i64) -> u32 {
    let range = (max as i128 - min as i128 + 1).max(1) as u128;
    if range <= 1 { 0 } else { 128 - (range - 1).leading_zeros() }
}

fn field_of(n: &Node) -> Result<Field, String> {
    match n.attr("type") {
        Some("Float") => Ok(Field::Float { double: n.attr("precision") != Some("single") }),
        Some("Integer") => {
            let (min, max) = (n.num("minimum").unwrap_or(i64::MIN as f64) as i64, n.num("maximum").unwrap_or(i64::MAX as f64) as i64);
            Ok(Field::Integer { min, bits: bits_for(min, max) })
        }
        Some("ScaledInteger") => {
            let (min, max) = (n.num("minimum").unwrap_or(i64::MIN as f64) as i64, n.num("maximum").unwrap_or(i64::MAX as f64) as i64);
            Ok(Field::Scaled { min, bits: bits_for(min, max), scale: n.num("scale").unwrap_or(1.0), offset: n.num("offset").unwrap_or(0.0) })
        }
        other => Err(format!("E57: unsupported field type {other:?} for {}", n.name)),
    }
}

/// Reads `count` values of a field from its whole bytestream.
fn unpack(stream: &[u8], f: &Field, count: usize) -> Vec<f64> {
    let mut out = Vec::with_capacity(count);
    match f {
        Field::Float { double } => {
            let w = if *double { 8 } else { 4 };
            for c in stream.chunks_exact(w).take(count) {
                out.push(if *double { f64::from_le_bytes(c.try_into().unwrap()) } else { f32::from_le_bytes(c.try_into().unwrap()) as f64 });
            }
        }
        Field::Integer { min, bits } | Field::Scaled { min, bits, .. } => {
            let (bits, min) = (*bits as usize, *min);
            let mut bit = 0usize;
            for _ in 0..count {
                let mut raw: u64 = 0;
                for k in 0..bits {
                    let byte = stream.get((bit + k) / 8).copied().unwrap_or(0);
                    raw |= (((byte >> ((bit + k) % 8)) & 1) as u64) << k;
                }
                bit += bits;
                let v = raw as i64 + min;
                out.push(match f {
                    Field::Scaled { scale, offset, .. } => v as f64 * scale + offset,
                    _ => v as f64,
                });
            }
        }
    }
    out
}

/// Reads every scan in the file, posed into one cloud.
pub fn read(data: &[u8]) -> Result<Cloud, String> {
    if data.len() < 48 || !data.starts_with(b"ASTM-E57") {
        return Err("not an E57 file".into());
    }
    let u64_at = |o: usize| u64::from_le_bytes(data[o..o + 8].try_into().unwrap());
    let xml_offset = u64_at(24) as usize;
    let xml_length = u64_at(32) as usize;
    let page = (u64_at(40) as usize).max(64);
    let payload = page - 4;
    // The logical file: every page without its checksum.
    let logical: Vec<u8> = data.chunks(page).flat_map(|p| &p[..p.len().min(payload)]).copied().collect();
    let to_logical = |phys: usize| (phys / page) * payload + phys % page;
    let xml_start = to_logical(xml_offset);
    let xml = logical.get(xml_start..xml_start + xml_length).ok_or("E57: the XML section is cut short")?;
    let root = parse_xml(&String::from_utf8_lossy(xml))?;
    let data3d = root.child("data3D").ok_or("E57: no 3D data")?;
    let mut cloud = Cloud::default();
    let mut colours_everywhere = true;
    for scan in &data3d.children {
        let points = scan.child("points").ok_or("E57: a scan has no points")?;
        let count = points.num("recordCount").unwrap_or(0.0) as usize;
        let offset = points.num("fileOffset").ok_or("E57: points have no file offset")? as usize;
        let proto = points.child("prototype").ok_or("E57: points have no prototype")?;
        let fields: Vec<(&str, Field)> = proto.children.iter().map(|c| field_of(c).map(|f| (c.name.as_str(), f))).collect::<Result<_, _>>()?;
        // The binary section: header, then data packets.
        let sec = to_logical(offset);
        let head = logical.get(sec..sec + 32).ok_or("E57: binary section cut short")?;
        let data_phys = u64::from_le_bytes(head[16..24].try_into().unwrap()) as usize;
        let section_len = u64::from_le_bytes(head[8..16].try_into().unwrap()) as usize;
        let mut at = to_logical(data_phys);
        let end = sec + section_len;
        let mut streams: Vec<Vec<u8>> = vec![Vec::new(); fields.len()];
        while at + 4 <= end.min(logical.len()) {
            let kind = logical[at];
            let len = u16::from_le_bytes([logical[at + 2], logical[at + 3]]) as usize + 1;
            if kind == 1 {
                let n = u16::from_le_bytes([logical[at + 4], logical[at + 5]]) as usize;
                let mut p = at + 6 + 2 * n;
                for (s, stream) in streams.iter_mut().enumerate().take(n) {
                    let bl = u16::from_le_bytes([logical[at + 6 + 2 * s], logical[at + 7 + 2 * s]]) as usize;
                    stream.extend_from_slice(logical.get(p..p + bl).ok_or("E57: packet cut short")?);
                    p += bl;
                }
            }
            at += len;
        }
        let mut cols: HashMap<&str, Vec<f64>> = HashMap::new();
        for (i, (name, f)) in fields.iter().enumerate() {
            cols.insert(name, unpack(&streams[i], f, count));
        }
        let get = |n: &str| cols.get(n).filter(|v| v.len() == count);
        let mut pts: Vec<V3> = Vec::with_capacity(count);
        if let (Some(x), Some(y), Some(z)) = (get("cartesianX"), get("cartesianY"), get("cartesianZ")) {
            for i in 0..count {
                pts.push([x[i], y[i], z[i]]);
            }
        } else if let (Some(r), Some(az), Some(el)) = (get("sphericalRange"), get("sphericalAzimuth"), get("sphericalElevation")) {
            for i in 0..count {
                let (ce, se) = (el[i].cos(), el[i].sin());
                pts.push([r[i] * ce * az[i].cos(), r[i] * ce * az[i].sin(), r[i] * se]);
            }
        } else {
            return Err("E57: a scan has neither cartesian nor spherical coordinates".into());
        }
        // Invalid points are flagged.
        let invalid = get("cartesianInvalidState").or(get("sphericalInvalidState"));
        // Pose: rotation quaternion then translation.
        let (rot, tr) = match scan.child("pose") {
            Some(pose) => {
                let q = pose.child("rotation").map(|r| ["w", "x", "y", "z"].map(|k| r.child(k).and_then(Node::value).unwrap_or(if k == "w" { 1.0 } else { 0.0 }))).unwrap_or([1.0, 0.0, 0.0, 0.0]);
                let t = pose.child("translation").map(|t| ["x", "y", "z"].map(|k| t.child(k).and_then(Node::value).unwrap_or(0.0))).unwrap_or([0.0; 3]);
                (quat_matrix(q), t)
            }
            None => ([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], [0.0; 3]),
        };
        let colour = match (get("colorRed"), get("colorGreen"), get("colorBlue")) {
            (Some(r), Some(g), Some(b)) => {
                let lim = scan.child("colorLimits");
                let range = |k: &str, dmin: f64, dmax: f64| (lim.and_then(|l| l.child(&format!("{k}Minimum"))).and_then(Node::value).unwrap_or(dmin), lim.and_then(|l| l.child(&format!("{k}Maximum"))).and_then(Node::value).unwrap_or(dmax));
                Some((r, g, b, range("colorRed", 0.0, 255.0), range("colorGreen", 0.0, 255.0), range("colorBlue", 0.0, 255.0)))
            }
            _ => None,
        };
        colours_everywhere &= colour.is_some();
        for (i, p) in pts.into_iter().enumerate() {
            if invalid.is_some_and(|v| v[i] != 0.0) {
                continue;
            }
            // Scanners write metres: Forma works in millimetres.
            cloud.positions.push(scale(add(mat_vec(&rot, p), tr), 1000.0));
            if let Some((r, g, b, lr, lg, lb)) = colour {
                let c = |v: f64, (lo, hi): (f64, f64)| (((v - lo) / (hi - lo).max(1e-12)) * 255.0).round().clamp(0.0, 255.0) as u8;
                cloud.colors.push([c(r[i], lr), c(g[i], lg), c(b[i], lb)]);
            }
        }
    }
    if !colours_everywhere || cloud.colors.len() != cloud.positions.len() {
        cloud.colors.clear();
    }
    Ok(cloud)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal E57 writer for the test: one scan, float coordinates,
    /// 8-bit colours, posed, paged with checksums (left zero).
    fn write(points: &[[f64; 3]], colours: &[[u8; 3]]) -> Vec<u8> {
        let n = points.len();
        let mut streams: Vec<Vec<u8>> = vec![Vec::new(); 6];
        for (i, p) in points.iter().enumerate() {
            for k in 0..3 {
                streams[k].extend_from_slice(&(p[k] as f32).to_le_bytes());
            }
            for k in 0..3 {
                streams[3 + k].push(colours[i][k]);
            }
        }
        let mut packet = vec![1u8, 0, 0, 0, 6, 0];
        for s in &streams {
            packet.extend_from_slice(&(s.len() as u16).to_le_bytes());
        }
        for s in &streams {
            packet.extend_from_slice(s);
        }
        while packet.len() % 4 != 0 {
            packet.push(0);
        }
        let plen = (packet.len() - 1) as u16;
        packet[2..4].copy_from_slice(&plen.to_le_bytes());
        // Logical layout: header (48), binary section (32 + packet), XML.
        let sec_logical = 48usize;
        let data_logical = sec_logical + 32;
        let xml_logical = data_logical + packet.len();
        let phys = |l: usize| (l / 1020) * 1024 + l % 1020;
        let xml = format!(
            r#"<?xml version="1.0"?><e57Root type="Structure"><data3D type="Vector"><vectorChild type="Structure"><pose type="Structure"><rotation type="Structure"><w type="Float">0.7071067811865476</w><x type="Float">0</x><y type="Float">0</y><z type="Float">0.7071067811865476</z></rotation><translation type="Structure"><x type="Float">1</x><y type="Float">0</y><z type="Float">0</z></translation></pose><points type="CompressedVector" fileOffset="{}" recordCount="{n}"><prototype type="Structure"><cartesianX type="Float" precision="single"/><cartesianY type="Float" precision="single"/><cartesianZ type="Float" precision="single"/><colorRed type="Integer" minimum="0" maximum="255"/><colorGreen type="Integer" minimum="0" maximum="255"/><colorBlue type="Integer" minimum="0" maximum="255"/></prototype></points></vectorChild></data3D></e57Root>"#,
            phys(sec_logical)
        );
        let mut logical = vec![0u8; 48];
        let mut sec = vec![1u8, 0, 0, 0, 0, 0, 0, 0];
        sec.extend_from_slice(&((32 + packet.len()) as u64).to_le_bytes());
        sec.extend_from_slice(&(phys(data_logical) as u64).to_le_bytes());
        sec.extend_from_slice(&0u64.to_le_bytes());
        logical.extend_from_slice(&sec);
        logical.extend_from_slice(&packet);
        logical.extend_from_slice(xml.as_bytes());
        logical[0..8].copy_from_slice(b"ASTM-E57");
        logical[8..12].copy_from_slice(&1u32.to_le_bytes());
        logical[24..32].copy_from_slice(&(phys(xml_logical) as u64).to_le_bytes());
        logical[32..40].copy_from_slice(&(xml.len() as u64).to_le_bytes());
        logical[40..48].copy_from_slice(&1024u64.to_le_bytes());
        let mut out = Vec::new();
        for chunk in logical.chunks(1020) {
            out.extend_from_slice(chunk);
            out.resize(out.len() + (1020 - chunk.len()), 0);
            out.extend_from_slice(&[0, 0, 0, 0]);
        }
        let len = out.len() as u64;
        out[16..24].copy_from_slice(&len.to_le_bytes());
        out
    }

    #[test]
    fn a_posed_scan_across_pages_reads_back_in_millimetres() {
        // 600 points: the data spans several 1024-byte pages.
        let pts: Vec<[f64; 3]> = (0..600).map(|i| [i as f64 * 0.001, 0.5, -0.25]).collect();
        let cols: Vec<[u8; 3]> = (0..600).map(|i| [(i % 256) as u8, 10, 250]).collect();
        let file = write(&pts, &cols);
        assert!(file.len() > 4 * 1024);
        let c = read(&file).unwrap();
        assert_eq!(c.len(), 600);
        // Turned 90° about z, moved 1 m along x, in mm: (x, y) → (1 − y, x).
        let p = c.positions[100];
        assert!((p[0] - 500.0).abs() < 1e-3 && (p[1] - 100.0).abs() < 1e-3 && (p[2] + 250.0).abs() < 1e-3, "{p:?}");
        assert_eq!(c.colors[300], [44, 10, 250]);
    }

    #[test]
    fn bit_packed_integers_unpack() {
        // Three values of 3 bits each (0…7 range), least significant bit first.
        let f = Field::Integer { min: 0, bits: 3 };
        // 5 = 101, 3 = 011, 6 = 110 → bits 101 110 011 (LSB first) → byte0 = 0b10_011_101, byte1 = 0b1
        let v = unpack(&[0b1001_1101, 0b0000_0001], &f, 3);
        assert_eq!(v, vec![5.0, 3.0, 6.0]);
        assert_eq!(bits_for(0, 255), 8);
        assert_eq!(bits_for(-1, 1), 2);
    }
}
