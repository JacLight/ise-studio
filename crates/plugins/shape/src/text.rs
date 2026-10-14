//! Text as outlines, from real fonts: lettering to engrave, emboss or cut.
//!
//! Inter (SIL Open Font License, bundled) is built in, at any weight from 100
//! to 900; any TrueType or OpenType font stored in the project can be used by
//! its content hash. Glyphs are laid out with the font's advances and pair
//! kerning (GPOS, or the older kern table), and their quadratic and cubic
//! curves are flattened to within the tolerance.

use ttf_parser::{Face, GlyphId, OutlineBuilder, Tag};

static INTER: &[u8] = include_bytes!("../fonts/Inter.ttf");

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Align {
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Baseline {
    /// The first line's baseline at y = 0.
    Baseline,
    /// The top of the first line's capitals at y = 0.
    Top,
    /// Halfway between the first line's cap top and the last line's baseline.
    Middle,
    /// The last line's baseline at y = 0.
    Bottom,
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub text: String,
    /// Height of a capital letter, mm.
    pub size: f64,
    /// "Inter" (the default), or a stored font's content hash.
    pub font: Option<String>,
    /// 100 (thin) to 900 (black); 400 is regular, 700 bold.
    pub weight: f64,
    pub align: Align,
    pub baseline: Baseline,
    /// Extra space between letters, mm.
    pub spacing: f64,
    /// Baseline to baseline, as a multiple of `size`.
    pub line_height: f64,
    /// Set along a circle of this radius (the baseline on it): centred on
    /// +y, reading clockwise over the top; negative reads left to right along
    /// the bottom, the letters' tops toward the centre (as on a coin).
    pub arc: Option<f64>,
    pub tolerance: f64,
}

/// The font's bytes: Inter, or a stored one.
fn font_bytes(font: Option<&str>) -> Result<std::borrow::Cow<'static, [u8]>, String> {
    match font {
        None => Ok(INTER.into()),
        Some(f) if f.eq_ignore_ascii_case("inter") || f.eq_ignore_ascii_case("sans") => Ok(INTER.into()),
        Some(sha) => {
            let path = crate::assets::path(sha).map_err(|e| format!("text: font {e}"))?;
            Ok(std::fs::read(path).map_err(|e| format!("text: could not read the font: {e}"))?.into())
        }
    }
}

/// Whether these bytes are a font text() can set: it parses and draws a capital.
pub fn check_font(bytes: &[u8]) -> Result<(), String> {
    let face = Face::parse(bytes, 0).map_err(|e| format!("not a font ISE Studio reads ({e})"))?;
    let g = face.glyph_index('H').ok_or("the font has no Latin capitals")?;
    struct Count(usize);
    impl OutlineBuilder for Count {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
            self.0 += 1;
        }
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
            self.0 += 1;
        }
        fn close(&mut self) {}
    }
    let mut c = Count(0);
    face.outline_glyph(g, &mut c).filter(|_| c.0 > 0).map(|_| ()).ok_or_else(|| "the font has no outlines (a bitmap or colour font?)".to_string())
}

struct Flatten {
    loops: Vec<Vec<[f64; 2]>>,
    at: [f64; 2],
    /// Font units per mm and where the glyph sits, mm.
    scale: f64,
    origin: [f64; 2],
    /// Largest stray from a curve, in font units.
    tol: f64,
}

impl Flatten {
    fn point(&self, x: f64, y: f64) -> [f64; 2] {
        [self.origin[0] + x * self.scale, self.origin[1] + y * self.scale]
    }
    fn push(&mut self, x: f64, y: f64) {
        let p = self.point(x, y);
        self.loops.last_mut().expect("move_to first").push(p);
        self.at = [x, y];
    }
}

impl OutlineBuilder for Flatten {
    fn move_to(&mut self, x: f32, y: f32) {
        self.loops.push(Vec::new());
        self.push(x as f64, y as f64);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.push(x as f64, y as f64);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (p0, p1, p2) = (self.at, [x1 as f64, y1 as f64], [x as f64, y as f64]);
        // A quadratic's chord error with n pieces is |p0 − 2p1 + p2| / (4n²).
        let dd = ((p0[0] - 2.0 * p1[0] + p2[0]).powi(2) + (p0[1] - 2.0 * p1[1] + p2[1]).powi(2)).sqrt();
        let n = ((dd / (4.0 * self.tol)).sqrt().ceil() as usize).clamp(1, 64);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            self.push(u * u * p0[0] + 2.0 * u * t * p1[0] + t * t * p2[0], u * u * p0[1] + 2.0 * u * t * p1[1] + t * t * p2[1]);
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (p0, p1, p2, p3) = (self.at, [x1 as f64, y1 as f64], [x2 as f64, y2 as f64], [x as f64, y as f64]);
        let d = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| ((a[0] - 2.0 * b[0] + c[0]).powi(2) + (a[1] - 2.0 * b[1] + c[1]).powi(2)).sqrt();
        let dd = d(p0, p1, p2).max(d(p1, p2, p3));
        let n = ((0.75 * dd / self.tol).sqrt().ceil() as usize).clamp(1, 64);
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let u = 1.0 - t;
            let (a, b, c, e) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.push(a * p0[0] + b * p1[0] + c * p2[0] + e * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + e * p3[1]);
        }
    }
    fn close(&mut self) {
        if let Some(l) = self.loops.last_mut()
            && l.len() > 1
            && l.first() == l.last()
        {
            l.pop();
        }
    }
}

/// Kerning between two glyphs, in font units: the font's "kern" feature
/// (GPOS pair adjustment), else its kern table.
fn kerning(face: &Face, left: GlyphId, right: GlyphId) -> f64 {
    use ttf_parser::gpos::{PairAdjustment, PositioningSubtable};
    if let Some(gpos) = face.tables().gpos
        && let Some(feature) = gpos.features.find(Tag::from_bytes(b"kern"))
    {
        for index in feature.lookup_indices {
            let Some(lookup) = gpos.lookups.get(index) else { continue };
            for i in 0..lookup.subtables.len() {
                let Some(PositioningSubtable::Pair(pair)) = lookup.subtables.get::<PositioningSubtable>(i) else { continue };
                let Some(cov) = pair.coverage().get(left) else { continue };
                let records = match pair {
                    PairAdjustment::Format1 { sets, .. } => sets.get(cov).and_then(|s| s.get(right)),
                    PairAdjustment::Format2 { classes, matrix, .. } => matrix.get((classes.0.get(left), classes.1.get(right))),
                };
                if let Some((first, _)) = records {
                    return first.x_advance as f64;
                }
            }
        }
        return 0.0;
    }
    if let Some(kern) = face.tables().kern {
        for sub in kern.subtables {
            if sub.horizontal
                && !sub.variable
                && let Some(k) = sub.glyphs_kerning(left, right)
            {
                return k as f64;
            }
        }
    }
    0.0
}

/// The text's outlines in mm (fill them non-zero: glyphs of variable fonts
/// may overlap themselves), and its size: (width, height) of the block.
pub fn outlines(spec: &Spec) -> Result<(Vec<Vec<[f64; 2]>>, [f64; 2]), String> {
    if !(spec.size.is_finite() && spec.size > 0.0) {
        return Err("text: size must be a positive number of mm (the height of a capital)".into());
    }
    let bytes = font_bytes(spec.font.as_deref())?;
    let mut face = Face::parse(&bytes, 0).map_err(|e| format!("text: not a font ISE Studio reads ({e})"))?;
    if face.is_variable() {
        let w = spec.weight.clamp(100.0, 900.0) as f32;
        face.set_variation(Tag::from_bytes(b"wght"), w);
    }
    let cap = face.capital_height().filter(|c| *c > 0).map(|c| c as f64).unwrap_or(face.ascender() as f64 * 0.7);
    let scale = spec.size / cap;
    let tol = (spec.tolerance.max(1e-4) / scale).max(0.5);
    let lines: Vec<&str> = spec.text.split('\n').collect();
    let advance_line = spec.line_height * spec.size;
    // Lay out each line: glyph ids and pen positions, in font units then mm.
    let mut placed: Vec<(GlyphId, [f64; 2])> = Vec::new();
    let mut widest: f64 = 0.0;
    for (li, line) in lines.iter().enumerate() {
        let mut pen = 0.0f64;
        let mut glyphs: Vec<(GlyphId, f64)> = Vec::new();
        let mut prev: Option<GlyphId> = None;
        for ch in line.chars() {
            let g = face.glyph_index(ch).or_else(|| face.glyph_index('?')).unwrap_or(GlyphId(0));
            if let Some(p) = prev {
                pen += kerning(&face, p, g) * scale;
            }
            glyphs.push((g, pen));
            pen += face.glyph_hor_advance(g).unwrap_or(0) as f64 * scale + spec.spacing;
            prev = Some(g);
        }
        let width = if glyphs.is_empty() { 0.0 } else { pen - spec.spacing };
        widest = widest.max(width);
        let shift = match spec.align {
            Align::Start => 0.0,
            Align::Center => -width / 2.0,
            Align::End => -width,
        };
        let y = -(li as f64) * advance_line;
        placed.extend(glyphs.into_iter().map(|(g, x)| (g, [x + shift, y])));
    }
    let last = -((lines.len() - 1) as f64) * advance_line;
    let lift = match spec.baseline {
        Baseline::Baseline => 0.0,
        Baseline::Top => -spec.size,
        Baseline::Middle => -(spec.size + last) / 2.0,
        Baseline::Bottom => -last,
    };
    let mut loops = Vec::new();
    for (g, [x, y]) in placed {
        let mut f = Flatten { loops: Vec::new(), at: [0.0, 0.0], scale, origin: [0.0, 0.0], tol };
        if face.outline_glyph(g, &mut f).is_none() {
            continue; // a space
        }
        let advance = face.glyph_hor_advance(g).unwrap_or(0) as f64 * scale;
        for l in f.loops.into_iter().filter(|l| l.len() >= 3) {
            loops.push(match spec.arc {
                None => l.iter().map(|p| [p[0] + x, p[1] + y + lift]).collect(),
                Some(r) => on_arc(&l, x + advance / 2.0, advance / 2.0, y + lift, r),
            });
        }
    }
    Ok((loops, [widest, spec.size - last]))
}

/// One glyph (its own coordinates, pen at 0) bent onto a circle: its middle
/// at arc length `s` from the top, its baseline at `radius + lift`.
fn on_arc(glyph: &[[f64; 2]], s: f64, half: f64, lift: f64, radius: f64) -> Vec<[f64; 2]> {
    let r = radius.abs();
    // Clockwise along the top for a positive radius, counter-clockwise along the bottom otherwise.
    let (sign, start) = if radius > 0.0 { (-1.0, std::f64::consts::FRAC_PI_2) } else { (1.0, -std::f64::consts::FRAC_PI_2) };
    let a = start + sign * s / r;
    // Up for the letter is outward on top, inward along the bottom.
    let up = if radius > 0.0 { [a.cos(), a.sin()] } else { [-a.cos(), -a.sin()] };
    let along = [up[1], -up[0]];
    let base = [r * a.cos(), r * a.sin()];
    let base = [base[0] + up[0] * lift, base[1] + up[1] * lift];
    glyph
        .iter()
        .map(|p| {
            let (u, v) = (p[0] - half, p[1]);
            [base[0] + along[0] * u + up[0] * v, base[1] + along[1] * u + up[1] * v]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use manifold_csg::{CrossSection, FillRule};

    fn spec(text: &str) -> Spec {
        Spec { text: text.into(), size: 10.0, font: None, weight: 400.0, align: Align::Start, baseline: Baseline::Baseline, spacing: 0.0, line_height: 1.6, arc: None, tolerance: 0.01 }
    }

    fn profile(s: &Spec) -> CrossSection {
        CrossSection::from_polygons_with_fill_rule(&outlines(s).unwrap().0, FillRule::NonZero)
    }

    #[test]
    fn capitals_are_the_size_asked_for() {
        let p = profile(&spec("H"));
        let b = p.bounds();
        assert!((b.max()[1] - 10.0).abs() < 0.05 && b.min()[1].abs() < 0.05, "{:?} {:?}", b.min(), b.max());
        // An O has a hole: less area than its outline's box would hold.
        let o = profile(&spec("O"));
        assert_eq!(o.decompose().len(), 1);
        let (lo, hi) = (o.bounds().min(), o.bounds().max());
        assert!(o.area() < 0.5 * (hi[0] - lo[0]) * (hi[1] - lo[1]));
    }

    #[test]
    fn bold_is_heavier_and_kerning_tightens_pairs() {
        let regular = profile(&spec("ISE")).area();
        let mut b = spec("ISE");
        b.weight = 700.0;
        assert!(profile(&b).area() > regular * 1.2);
        // "AV" kerns tighter than the two letters' advances.
        let (_, [kerned, _]) = outlines(&spec("AV")).unwrap();
        let (_, [a, _]) = outlines(&spec("A")).unwrap();
        let (_, [v, _]) = outlines(&spec("V")).unwrap();
        assert!(kerned < a + v - 0.2, "AV {kerned} vs A+V {}", a + v);
    }

    #[test]
    fn alignment_and_arcs_place_the_block() {
        let mut s = spec("Studio");
        s.align = Align::Center;
        s.baseline = Baseline::Middle;
        let b = profile(&s).bounds();
        assert!((b.min()[0] + b.max()[0]).abs() < 0.5, "centred: {:?} {:?}", b.min(), b.max());
        s.arc = Some(30.0);
        s.baseline = Baseline::Baseline;
        let b = profile(&s).bounds();
        // On top of a 30 mm circle, letters standing outward.
        assert!(b.min()[1] > 20.0 && b.max()[1] < 42.0, "{:?} {:?}", b.min(), b.max());
    }
}
