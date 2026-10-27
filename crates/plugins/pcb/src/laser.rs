//! Laser etching: the copper to burn off one layer of a board, for a fiber
//! (galvo) laser that removes copper directly, run from EZCAD or LightBurn.
//!
//! The copper comes from the layer's Gerber, the file a fab would get (and
//! that the Gerber check reads back), so what is burned is exactly what is not
//! copper there. The laser either clears every bit of unwanted copper, or
//! burns a ring of `isolate` mm around each piece (faster; the rest stays as
//! unconnected copper). Each hole gets a small dot burned at its centre for
//! the drill to start in.
//!
//! The art is centred on (0, 0), the middle of a galvo laser's field, and
//! mirrored for a bottom layer so it reads as seen from the copper side, which
//! faces the laser. Files: a DXF of closed outlines to hatch-fill (R12, the
//! form EZCAD imports), the same as SVG, and a 1-bit BMP at a set resolution
//! where black is burned.

use crate::board::BoardView;
use crate::geom::P;
use crate::{gerber, reader};
use manifold_csg::{CrossSection, FillRule, JoinType};
use std::collections::HashMap;
use std::fmt::Write as _;

/// Arcs are flattened to within this (mm): far below what a laser spot resolves.
const TOL: f64 = 0.005;

/// What to burn, and how the files are laid out.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The copper layer.
    pub layer: String,
    /// Width of the ring burned around the copper (mm); 0 clears all unwanted copper.
    pub isolate: f64,
    /// Diameter of the dot burned at each hole's centre (mm); 0 for none.
    pub pilot: f64,
    /// Left-right mirror; default: on the bottom layer, as seen from the copper side.
    pub mirror: Option<bool>,
    /// Bitmap resolution (dots per inch).
    pub dpi: f64,
}

impl Settings {
    /// From a request's query: `layer`, `isolate`, `pilot`, `mirror`, `dpi`.
    pub fn from_query(view: &BoardView, q: &HashMap<String, String>) -> Settings {
        let num = |k: &str, d: f64| q.get(k).and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(d);
        Settings {
            layer: q.get("layer").cloned().unwrap_or_else(|| default_layer(view)),
            isolate: num("isolate", 0.0),
            pilot: num("pilot", 0.5),
            mirror: q.get("mirror").map(|m| m == "1" || m == "true"),
            dpi: num("dpi", 1000.0).clamp(100.0, 4000.0),
        }
    }

    fn mirrored(&self, view: &BoardView) -> bool {
        self.mirror.unwrap_or_else(|| view.layers.len() > 1 && view.layers.last() == Some(&self.layer))
    }
}

/// The copper layer that has copper on it (the top one when both do).
pub fn default_layer(view: &BoardView) -> String {
    let used = |k: usize, name: &str| view.cu.iter().any(|c| c.layers & (1 << k) != 0) || view.zones.iter().any(|z| z.layer == name);
    view.layers.iter().enumerate().find(|(k, l)| used(*k, l)).map(|(_, l)| l.clone()).unwrap_or_else(|| view.layers.first().cloned().unwrap_or_else(|| "F.Cu".into()))
}

/// The art for one layer, in laser coordinates (mm, centred on the board).
#[derive(Clone, Debug)]
pub struct Art {
    pub layer: String,
    /// Closed outlines of the copper to burn off (counter-clockwise outside, clockwise holes).
    pub burn: Vec<Vec<P>>,
    /// The board's edge (and cutouts), for lining up and cutting out.
    pub edge: Vec<Vec<P>>,
    /// Board width and height.
    pub size: [f64; 2],
    pub mirrored: bool,
    pub burn_area: f64,
    pub copper_area: f64,
    pub pilots: usize,
    pub settings: Settings,
}

fn section(polys: &[Vec<P>]) -> CrossSection {
    CrossSection::from_polygons_with_fill_rule(polys, FillRule::NonZero)
}

/// The copper of `layer` as drawn in its Gerber: dark objects add, clear ones remove,
/// in file order (an aperture's own hole only shows what is under it).
pub fn copper(view: &BoardView, layer: &str) -> Result<CrossSection, String> {
    let g = reader::gerber(&gerber::copper(view, layer))?;
    let mut done = CrossSection::empty();
    let mut adding: Vec<CrossSection> = Vec::new();
    for o in &g.objects {
        let parts = g.outlines(o, TOL);
        let on: Vec<Vec<P>> = parts.iter().filter(|p| p.0).map(|p| p.1.clone()).collect();
        let off: Vec<Vec<P>> = parts.iter().filter(|p| !p.0).map(|p| p.1.clone()).collect();
        let mut shape = section(&on);
        if !off.is_empty() {
            shape = shape.difference(&section(&off));
        }
        if o.dark {
            adding.push(shape);
        } else {
            adding.push(done);
            done = CrossSection::batch_union(&adding).difference(&shape);
            adding.clear();
        }
    }
    adding.push(done);
    Ok(CrossSection::batch_union(&adding))
}

/// The board's area: its outline less its cutouts.
fn board_area(view: &BoardView) -> CrossSection {
    let mut area = section(&[view.outline.clone()]);
    if !view.cutouts.is_empty() {
        area = area.difference(&section(&view.cutouts));
    }
    area
}

fn circle(c: P, r: f64) -> Vec<P> {
    let n = ((std::f64::consts::PI / (1.0 - TOL / r).clamp(-1.0, 1.0).acos()).ceil() as usize).clamp(12, 256);
    (0..n).map(|i| {
        let a = std::f64::consts::TAU * i as f64 / n as f64;
        [c[0] + r * a.cos(), c[1] + r * a.sin()]
    }).collect()
}

pub fn art(view: &BoardView, settings: &Settings) -> Result<Art, String> {
    if !view.layers.contains(&settings.layer) {
        return Err(format!("{} is not a copper layer of this board ({})", settings.layer, view.layers.join(", ")));
    }
    if view.outline.len() < 3 {
        return Err("the board has no outline".into());
    }
    let board = board_area(view);
    let copper = copper(view, &settings.layer)?.intersection(&board);
    // Pilot dots: every hole through this layer, no bigger than the hole.
    let k = view.layers.iter().position(|l| *l == settings.layer).unwrap_or(0);
    let through = |span: &Option<[String; 2]>| match span {
        None => true,
        Some([a, b]) => {
            let (i, j) = (view.layers.iter().position(|l| l == a).unwrap_or(0), view.layers.iter().position(|l| l == b).unwrap_or(0));
            i.min(j) <= k && k <= i.max(j)
        }
    };
    let dots: Vec<Vec<P>> = if settings.pilot > 0.0 {
        view.holes.iter().filter(|h| through(&h.span)).map(|h| circle(h.at, settings.pilot.min(h.size[0].min(h.size[1])) / 2.0)).collect()
    } else {
        Vec::new()
    };
    let kept = if dots.is_empty() { copper.clone() } else { copper.difference(&section(&dots)) };
    let region = if settings.isolate > 0.0 {
        let n = ((std::f64::consts::PI / (1.0 - TOL / settings.isolate).clamp(-1.0, 1.0).acos()).ceil() as i32).clamp(16, 256);
        copper.offset(settings.isolate, JoinType::Round, 2.0, n).intersection(&board)
    } else {
        board.clone()
    };
    let burn = region.difference(&kept);

    // Laser coordinates: centred on the board, mirrored for the bottom.
    let xs = view.outline.iter().map(|p| p[0]);
    let ys = view.outline.iter().map(|p| p[1]);
    let (x0, x1) = (xs.clone().fold(f64::INFINITY, f64::min), xs.fold(f64::NEG_INFINITY, f64::max));
    let (y0, y1) = (ys.clone().fold(f64::INFINITY, f64::min), ys.fold(f64::NEG_INFINITY, f64::max));
    let mirrored = settings.mirrored(view);
    let place = |cs: &CrossSection| {
        let t = cs.translate(-(x0 + x1) / 2.0, -(y0 + y1) / 2.0);
        if mirrored { t.mirror(1.0, 0.0) } else { t }
    };
    let burn = place(&burn);
    Ok(Art {
        layer: settings.layer.clone(),
        burn_area: burn.area(),
        copper_area: kept.area(),
        burn: burn.to_polygons(),
        edge: place(&board).to_polygons(),
        size: [x1 - x0, y1 - y0],
        mirrored,
        pilots: dots.len(),
        settings: settings.clone(),
    })
}

fn f4(v: f64) -> String {
    let s = format!("{:.4}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

/// DXF (R12, millimetres): layer ETCH holds the closed outlines to hatch-fill,
/// layer EDGE the board's edge.
pub fn dxf(art: &Art) -> String {
    let mut s = String::new();
    let [w, h] = art.size;
    let _ = write!(s, "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n9\n$EXTMIN\n10\n{}\n20\n{}\n9\n$EXTMAX\n10\n{}\n20\n{}\n0\nENDSEC\n", f4(-w / 2.0), f4(-h / 2.0), f4(w / 2.0), f4(h / 2.0));
    s.push_str("0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n2\n");
    for (name, colour) in [("ETCH", 1), ("EDGE", 5)] {
        let _ = write!(s, "0\nLAYER\n2\n{name}\n70\n0\n62\n{colour}\n6\nCONTINUOUS\n");
    }
    s.push_str("0\nENDTAB\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n");
    for (layer, polys) in [("ETCH", &art.burn), ("EDGE", &art.edge)] {
        for p in polys {
            let _ = write!(s, "0\nPOLYLINE\n8\n{layer}\n66\n1\n10\n0\n20\n0\n30\n0\n70\n1\n");
            for q in p {
                let _ = write!(s, "0\nVERTEX\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n", f4(q[0]), f4(q[1]));
            }
            let _ = write!(s, "0\nSEQEND\n8\n{layer}\n");
        }
    }
    s.push_str("0\nENDSEC\n0\nEOF\n");
    s
}

/// SVG (millimetres): the copper to burn filled black, the board's edge as a thin line.
pub fn svg(art: &Art) -> String {
    let [w, h] = art.size;
    let path = |polys: &[Vec<P>]| {
        let mut d = String::new();
        for p in polys {
            for (i, q) in p.iter().enumerate() {
                let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { "L" }, f4(q[0]), f4(-q[1]));
            }
            d.push('Z');
        }
        d
    };
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}mm\" height=\"{h}mm\" viewBox=\"{} {} {} {}\">\n<path id=\"etch\" fill=\"#000\" fill-rule=\"evenodd\" d=\"{}\"/>\n<path id=\"edge\" fill=\"none\" stroke=\"#1e6fd9\" stroke-width=\"0.05\" d=\"{}\"/>\n</svg>\n",
        f4(-w / 2.0), f4(-h / 2.0), f4(w), f4(h), path(&art.burn), path(&art.edge), w = f4(w), h = f4(h)
    )
}

/// A 1-bit BMP over the board at `dpi`: black where the laser burns.
pub fn bmp(art: &Art) -> Vec<u8> {
    let px = 25.4 / art.settings.dpi;
    let [w, h] = art.size;
    // Whole pixels over the board (10 mm / 0.1 mm is 100, not 100.000…01 rounded up).
    let count = |mm: f64| ((mm / px) - 1e-6).ceil().max(1.0) as usize;
    let (cols, rows) = (count(w), count(h));
    let mut r = crate::gerber_check::Raster { x0: -w / 2.0, y0: -h / 2.0, px, w: cols, h: rows, on: vec![0; cols * rows] };
    r.fill(&art.burn, 1);
    let row = r.w.div_ceil(32) * 4;
    let image = row * r.h;
    let mut b = Vec::with_capacity(62 + image);
    let ppm = (art.settings.dpi / 0.0254).round() as u32;
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&(62 + image as u32).to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&62u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(r.w as i32).to_le_bytes());
    b.extend_from_slice(&(r.h as i32).to_le_bytes()); // positive: rows bottom to top, as the raster's
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&(image as u32).to_le_bytes());
    b.extend_from_slice(&ppm.to_le_bytes());
    b.extend_from_slice(&ppm.to_le_bytes());
    b.extend_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&2u32.to_le_bytes());
    b.extend_from_slice(&[255, 255, 255, 0, 0, 0, 0, 0]); // 0 white (kept), 1 black (burned)
    for j in 0..r.h {
        let mut line = vec![0u8; row];
        for i in 0..r.w {
            if r.on[j * r.w + i] != 0 {
                line[i / 8] |= 0x80 >> (i % 8);
            }
        }
        b.extend_from_slice(&line);
    }
    b
}

/// What the files are and how to run them.
pub fn readme(view: &BoardView, art: &Art) -> String {
    let s = &art.settings;
    let stem = gerber::file_stem(&view.name);
    let layer = gerber::file_stem(&art.layer);
    let mode = if s.isolate > 0.0 {
        format!("a {} mm ring around each piece of copper (the rest stays, unconnected)", f4(s.isolate))
    } else {
        "all copper that is not part of the circuit".to_string()
    };
    format!(
        // One line per paragraph and item: a text editor wraps them to its own width.
        "{name}: laser etching, layer {layer_name}\n\
         \n\
         The laser burns off {mode}, plus a {pilot} mm dot at the centre of each of the {pilots} holes for the drill to start in. Board {w} x {h} mm; {burn} mm² to burn, {cu} mm² of copper kept.\n\
         \n\
         Files\n\
         - {stem}-laser-{layer}.dxf: outlines to hatch-fill (layer ETCH) and the board edge (layer EDGE).\n\
         - {stem}-laser-{layer}.svg: the same, for LightBurn or to check by eye.\n\
         - {stem}-laser-{layer}.bmp: 1-bit, {dpi} dpi, black is burned.\n\
         \n\
         Placing it\n\
         - The art is centred on (0, 0), the middle of the laser's field: put the board's centre there.\n\
         - {mirror}\n\
         - Use the red preview (EZCAD: Red / F1) on layer EDGE to line the board up before burning.\n\
         \n\
         In EZCAD\n\
         1. File > Import Vector File, choose the .dxf. Delete or hide the EDGE outline before marking (or mark it once at low power to score where to cut the board out).\n\
         2. Select everything on ETCH and Hatch it: line spacing 0.02 to 0.03 mm, \"Enable contour\" on, and two or more hatch sets at different angles (0 and 90 degrees) so no lines are left. Copper inside burned areas (pads, traces, text) stays as islands, not filled.\n\
         3. Or import the .bmp instead (File > Import Bitmap), \"Fixed DPI\" {dpi}, no dithering: black dots burn.\n\
         \n\
         Settings: starting points for a 30 W MOPA (JPT M7 class) with a 110 to 175 mm lens, burning 35 µm (1 oz) copper off FR4. Every laser, lens and board differs: find yours on the test board. In EZCAD the pulse width shows when the laser type is MOPA (Config Parameter). Focus on the copper's surface.\n\
         Pass 1, bulk copper: power 90 to 100 %, pulse width 200 to 350 ns, frequency 40 to 60 kHz, speed 800 to 1500 mm/s, hatch 0.03 mm at 0 and 90 degrees (Auto rotate hatch angle on, if you have it). Loop count 5, look, repeat until the fiberglass shows through nearly everywhere.\n\
         Pass 2, clean-up: power 60 to 80 %, pulse width 100 to 200 ns, frequency 100 to 200 kHz, speed 1500 to 2500 mm/s, hatch 0.02 mm, 2 to 5 loops, until no copper shine is left in the gaps. Then stop: more passes only char the fiberglass.\n\
         - Long pulses at low frequency hit hardest (each pulse carries more energy); short pulses at high frequency clean gently. A 20 W laser: slow down by about a third; 60 W: speed up, or fewer loops.\n\
         - Pass 1 too strong shows as brown, charred fiberglass and thinned fine traces; too weak as copper left after many loops.\n\
         - Clean the board first (isopropyl alcohol); afterwards scrub off the soot and check every gap with a meter or a magnifier before soldering.\n\
         - Burning copper and FR4 makes fumes: extract them.\n",
        name = view.name,
        layer_name = art.layer,
        pilot = f4(s.pilot),
        pilots = art.pilots,
        w = f4(art.size[0]),
        h = f4(art.size[1]),
        burn = f4((art.burn_area * 10.0).round() / 10.0),
        cu = f4((art.copper_area * 10.0).round() / 10.0),
        dpi = f4(s.dpi),
        mirror = if art.mirrored {
            "Mirrored left to right: this is the bottom copper as seen from the copper side, which faces the laser."
        } else {
            "Not mirrored: the copper faces the laser and reads as designed."
        },
    )
}

/// The files for one layer: (name, content type, bytes).
pub fn files(view: &BoardView, settings: &Settings) -> Result<Vec<(String, &'static str, Vec<u8>)>, String> {
    let art = art(view, settings)?;
    let base = format!("{}-laser-{}", gerber::file_stem(&view.name), gerber::file_stem(&art.layer));
    Ok(vec![
        (format!("{base}.dxf"), "application/dxf", dxf(&art).into_bytes()),
        (format!("{base}.svg"), "image/svg+xml", svg(&art).into_bytes()),
        (format!("{base}.bmp"), "image/bmp", bmp(&art)),
        (format!("{base}-README.txt"), "text/plain; charset=utf-8", readme(view, &art).into_bytes()),
    ])
}

/// The names `file` answers for, on the default layer.
pub fn names(view: &BoardView) -> Vec<String> {
    let base = format!("{}-laser-{}", gerber::file_stem(&view.name), gerber::file_stem(&default_layer(view)));
    vec![format!("{base}.dxf"), format!("{base}.svg"), format!("{base}.bmp"), format!("{base}-README.txt"), format!("{}-laser.zip", gerber::file_stem(&view.name))]
}

/// One laser file by name (`<board>-laser-<layer>.dxf|.svg|.bmp|-README.txt`, or
/// `<board>-laser.zip` with all four), its layer read from the name.
pub fn file(view: &BoardView, name: &str, query: &HashMap<String, String>) -> Option<Result<(&'static str, Vec<u8>), String>> {
    let stem = gerber::file_stem(&view.name);
    let rest = name.strip_prefix(&format!("{stem}-laser"))?;
    let mut settings = Settings::from_query(view, query);
    if rest == ".zip" {
        return Some(files(view, &settings).map(|f| ("application/zip", crate::outputs::zip(&f.into_iter().map(|(n, _, b)| (n, b)).collect::<Vec<_>>()))));
    }
    let rest = rest.strip_prefix('-')?;
    let layer = view.layers.iter().find(|l| rest.starts_with(&format!("{}.", gerber::file_stem(l))) || rest.starts_with(&format!("{}-", gerber::file_stem(l))))?;
    settings.layer = layer.clone();
    Some(files(view, &settings).and_then(|f| f.into_iter().find(|(n, _, _)| n == name).map(|(_, kind, b)| (kind, b)).ok_or_else(|| format!("no laser file {name}"))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bmp_header_and_rows() {
        let art = Art {
            layer: "F.Cu".into(),
            burn: vec![vec![[-5.0, -5.0], [0.0, -5.0], [0.0, 5.0], [-5.0, 5.0]]],
            edge: vec![],
            size: [10.0, 10.0],
            mirrored: false,
            burn_area: 50.0,
            copper_area: 50.0,
            pilots: 0,
            settings: Settings { layer: "F.Cu".into(), isolate: 0.0, pilot: 0.0, mirror: None, dpi: 254.0 },
        };
        let b = bmp(&art);
        // 10 mm at 254 dpi = 100 px; 1-bit rows padded to 4 bytes = 16 bytes.
        assert_eq!(&b[0..2], b"BM");
        assert_eq!(i32::from_le_bytes(b[18..22].try_into().unwrap()), 100);
        assert_eq!(b.len(), 62 + 16 * 100);
        // Left half burned: the first row's first 6 bytes black, then the 50th pixel white.
        let first = &b[62..62 + 16];
        assert_eq!(&first[0..6], &[0xff; 6]);
        assert_eq!(first[6] & 0xc0, 0xc0);
        assert_eq!(first[6] & 0x20, 0);
    }

    #[test]
    fn dxf_is_closed_polylines() {
        let art = Art {
            layer: "F.Cu".into(),
            burn: vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]],
            edge: vec![vec![[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]],
            size: [2.0, 2.0],
            mirrored: false,
            burn_area: 0.5,
            copper_area: 0.0,
            pilots: 0,
            settings: Settings { layer: "F.Cu".into(), isolate: 0.0, pilot: 0.0, mirror: None, dpi: 1000.0 },
        };
        let d = dxf(&art);
        assert_eq!(d.matches("0\nPOLYLINE\n").count(), 2);
        assert_eq!(d.matches("0\nVERTEX\n8\nETCH\n").count(), 3);
        assert_eq!(d.matches("0\nSEQEND\n").count(), 2);
        assert!(d.contains("AC1009") && d.ends_with("0\nEOF\n"));
    }
}
