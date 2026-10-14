//! Pictures a program reads: a photo to carve as a relief, a logo to trace, a
//! drawing to measure. Stored by content like imported files (see `assets`);
//! decoded once (PNG at 8 or 16 bits, JPEG) into channels of 0–1 values and
//! cached, since a relief re-evaluates on every parameter change.

use std::sync::{Arc, Mutex, OnceLock};

/// A decoded picture, top row first.
pub struct Picture {
    pub width: usize,
    pub height: usize,
    /// Red, green, blue, 0–1 as stored (sRGB), `width × height` each.
    rgb: [Vec<f32>; 3],
    /// Coverage, 0–1; None when every pixel is opaque.
    alpha: Option<Vec<f32>>,
    /// Bits per sample in the file (8 or 16): 16-bit depth maps keep their steps.
    pub bits: u8,
}

/// Which value of each pixel a program reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    /// Brightness as seen (Rec. 709 luma), transparent pixels read as white.
    Brightness,
    /// Coverage: 1 where the picture is opaque.
    Alpha,
    Red,
    Green,
    Blue,
    /// The object standing out from its backdrop: 1 where a pixel's colour
    /// differs from the backdrop (the median of the picture's border), as a
    /// product photo on white or a plain wall shows it; opaque pixels on a
    /// see-through picture.
    Object,
}

impl Channel {
    pub fn parse(name: &str) -> Result<Channel, String> {
        Ok(match name {
            "brightness" | "luminance" | "gray" | "grey" => Channel::Brightness,
            "alpha" => Channel::Alpha,
            "red" => Channel::Red,
            "green" => Channel::Green,
            "blue" => Channel::Blue,
            "object" | "silhouette" => Channel::Object,
            other => return Err(format!("unknown channel “{other}” (use brightness, alpha, red, green, blue or object)")),
        })
    }
}

impl Picture {
    /// Whether any pixel is see-through (a logo on a transparent background).
    pub fn has_alpha(&self) -> bool {
        self.alpha.is_some()
    }

    /// One channel as a grid of 0–1 values, top row first.
    pub fn channel(&self, c: Channel) -> Vec<f32> {
        match c {
            Channel::Red => self.rgb[0].clone(),
            Channel::Green => self.rgb[1].clone(),
            Channel::Blue => self.rgb[2].clone(),
            Channel::Object => self.object(None, OBJECT_THRESHOLD),
            Channel::Alpha => self.alpha.clone().unwrap_or_else(|| vec![1.0; self.width * self.height]),
            Channel::Brightness => {
                let [r, g, b] = &self.rgb;
                (0..self.width * self.height)
                    .map(|i| {
                        let y = 0.2126 * r[i] + 0.7152 * g[i] + 0.0722 * b[i];
                        // Over white: what a see-through pixel looks like on paper.
                        match &self.alpha {
                            Some(a) => y * a[i] + (1.0 - a[i]),
                            None => y,
                        }
                    })
                    .collect()
            }
        }
    }
}

/// Decodes PNG or JPEG bytes.
pub fn decode(bytes: &[u8]) -> Result<Picture, String> {
    match crate::assets::sniff(bytes) {
        "png" => decode_png(bytes),
        "jpg" => decode_jpeg(bytes),
        _ => Err("not a picture ISE Studio reads (PNG or JPEG)".into()),
    }
}

fn decode_png(bytes: &[u8]) -> Result<Picture, String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Palettes and low-bit greys to 8 bits, transparency chunks to alpha; 16 bits stay.
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| format!("PNG: {e}"))?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("PNG: {e}"))?;
    let (width, height) = (info.width as usize, info.height as usize);
    let channels = info.color_type.samples();
    let sixteen = info.bit_depth == png::BitDepth::Sixteen;
    let sample = |i: usize| -> f32 {
        if sixteen { u16::from_be_bytes([buf[2 * i], buf[2 * i + 1]]) as f32 / 65535.0 } else { buf[i] as f32 / 255.0 }
    };
    let n = width * height;
    let mut rgb = [Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n)];
    let mut alpha = Vec::with_capacity(n);
    for row in 0..height {
        let start = row * (info.line_size / if sixteen { 2 } else { 1 });
        for x in 0..width {
            let at = start + x * channels;
            let (r, g, b, a) = match channels {
                1 => (sample(at), sample(at), sample(at), 1.0),
                2 => (sample(at), sample(at), sample(at), sample(at + 1)),
                3 => (sample(at), sample(at + 1), sample(at + 2), 1.0),
                _ => (sample(at), sample(at + 1), sample(at + 2), sample(at + 3)),
            };
            rgb[0].push(r);
            rgb[1].push(g);
            rgb[2].push(b);
            alpha.push(a);
        }
    }
    let alpha = alpha.iter().any(|a| *a < 0.999).then_some(alpha);
    Ok(Picture { width, height, rgb, alpha, bits: if sixteen { 16 } else { 8 } })
}

fn decode_jpeg(bytes: &[u8]) -> Result<Picture, String> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    let pixels = decoder.decode().map_err(|e| format!("JPEG: {e}"))?;
    let info = decoder.info().ok_or("JPEG: no image")?;
    let (width, height) = (info.width as usize, info.height as usize);
    let n = width * height;
    let mut rgb = [Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n)];
    use jpeg_decoder::PixelFormat::*;
    for i in 0..n {
        let (r, g, b) = match info.pixel_format {
            L8 => {
                let v = pixels[i] as f32 / 255.0;
                (v, v, v)
            }
            L16 => {
                let v = u16::from_be_bytes([pixels[2 * i], pixels[2 * i + 1]]) as f32 / 65535.0;
                (v, v, v)
            }
            RGB24 => (pixels[3 * i] as f32 / 255.0, pixels[3 * i + 1] as f32 / 255.0, pixels[3 * i + 2] as f32 / 255.0),
            CMYK32 => {
                let [c, m, y, k] = [0, 1, 2, 3].map(|j| pixels[4 * i + j] as f32 / 255.0);
                ((1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k))
            }
        };
        rgb[0].push(r);
        rgb[1].push(g);
        rgb[2].push(b);
    }
    Ok(Picture { width, height, rgb, alpha: None, bits: if info.pixel_format == L16 { 16 } else { 8 } })
}

/// A stored picture by its content hash, decoded once and kept for reuse.
pub fn load(sha: &str) -> Result<Arc<Picture>, String> {
    static CACHE: OnceLock<Mutex<Vec<(String, Arc<Picture>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Some((_, p)) = cache.lock().unwrap().iter().find(|(s, _)| s == sha) {
        return Ok(p.clone());
    }
    let path = crate::assets::path(sha)?;
    let bytes = std::fs::read(&path).map_err(|e| format!("could not read the picture: {e}"))?;
    let picture = Arc::new(decode(&bytes)?);
    let mut c = cache.lock().unwrap();
    if c.len() >= 6 {
        c.remove(0);
    }
    c.push((sha.to_string(), picture.clone()));
    Ok(picture)
}

/// A grid of values (row-major, `w × h`), as reliefs and tracing read it.
#[derive(Clone)]
pub struct Field {
    pub w: usize,
    pub h: usize,
    pub v: Vec<f32>,
}

impl Field {
    pub fn at(&self, x: usize, y: usize) -> f32 {
        self.v[y * self.w + x]
    }

    /// Gaussian blur, `sigma` in pixels (0 leaves it as it is).
    pub fn blur(&mut self, sigma: f64) {
        if !(sigma > 0.05) {
            return;
        }
        let radius = (3.0 * sigma).ceil() as isize;
        let kernel: Vec<f32> = (-radius..=radius).map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp() as f32).collect();
        let sum: f32 = kernel.iter().sum();
        let kernel: Vec<f32> = kernel.iter().map(|k| k / sum).collect();
        let (w, h) = (self.w as isize, self.h as isize);
        let mut tmp = vec![0.0f32; self.v.len()];
        // Edges extend their last pixel, so the border does not darken or lighten.
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for (k, kv) in kernel.iter().enumerate() {
                    let xx = (x + k as isize - radius).clamp(0, w - 1);
                    acc += kv * self.v[(y * w + xx) as usize];
                }
                tmp[(y * w + x) as usize] = acc;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for (k, kv) in kernel.iter().enumerate() {
                    let yy = (y + k as isize - radius).clamp(0, h - 1);
                    acc += kv * tmp[(yy * w + x) as usize];
                }
                self.v[(y * w + x) as usize] = acc;
            }
        }
    }

    /// Resampled to `nx × ny` points spanning the picture edge to edge: each
    /// point averages the pixels around it when shrinking (no aliasing), and
    /// interpolates between pixel centres when growing.
    pub fn resample(&self, nx: usize, ny: usize) -> Field {
        let (w, h) = (self.w, self.h);
        // Summed-area table: any box average in constant time.
        let mut sat = vec![0.0f64; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0.0;
            for x in 0..w {
                row += self.v[y * w + x] as f64;
                sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
            }
        }
        let boxed = |x0: f64, y0: f64, x1: f64, y1: f64| -> f64 {
            // Fractional box over pixel cells [x0, x1) × [y0, y1), by bilinear SAT lookups.
            let s = |x: f64, y: f64| -> f64 {
                let (xi, yi) = (x.floor() as usize, y.floor() as usize);
                let (xi, yi) = (xi.min(w - 1), yi.min(h - 1));
                let (fx, fy) = (x - xi as f64, y - yi as f64);
                let at = |xx: usize, yy: usize| sat[yy * (w + 1) + xx];
                let a = at(xi, yi) + (at(xi + 1, yi) - at(xi, yi)) * fx;
                let b = at(xi, yi + 1) + (at(xi + 1, yi + 1) - at(xi, yi + 1)) * fx;
                a + (b - a) * fy
            };
            (s(x1, y1) - s(x0, y1) - s(x1, y0) + s(x0, y0)) / ((x1 - x0) * (y1 - y0))
        };
        let (sx, sy) = (w as f64 / (nx - 1).max(1) as f64, h as f64 / (ny - 1).max(1) as f64);
        let mut v = Vec::with_capacity(nx * ny);
        for j in 0..ny {
            for i in 0..nx {
                // Grid point in pixel-edge coordinates: 0 is the picture's left edge, w its right.
                let (px, py) = (i as f64 * sx, j as f64 * sy);
                let (hx, hy) = ((sx / 2.0).max(0.5), (sy / 2.0).max(0.5));
                let value = if sx > 1.0 || sy > 1.0 {
                    let (x0, x1) = ((px - hx).max(0.0), (px + hx).min(w as f64));
                    let (y0, y1) = ((py - hy).max(0.0), (py + hy).min(h as f64));
                    boxed(x0, y0, x1, y1)
                } else {
                    self.bilinear(px - 0.5, py - 0.5) as f64
                };
                v.push(value as f32);
            }
        }
        Field { w: nx, h: ny, v }
    }

    /// The value between pixel centres (pixel (0, 0)'s centre is at 0, 0).
    pub fn bilinear(&self, x: f64, y: f64) -> f32 {
        let x = x.clamp(0.0, (self.w - 1) as f64);
        let y = y.clamp(0.0, (self.h - 1) as f64);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let a = self.at(x0, y0) + (self.at(x1, y0) - self.at(x0, y0)) * fx;
        let b = self.at(x0, y1) + (self.at(x1, y1) - self.at(x0, y1)) * fx;
        a + (b - a) * fy
    }
}

/// How far (0–1, the largest difference of red, green or blue) a colour must
/// stand from the backdrop to count as the object.
pub const OBJECT_THRESHOLD: f32 = 0.12;

impl Picture {
    /// The backdrop's colour: the median of a band two percent wide round the
    /// picture's edge (a product shot on white, a plain wall, a cutting mat).
    pub fn backdrop(&self) -> [f32; 3] {
        let (w, h) = (self.width, self.height);
        let band = ((w.min(h) as f64 * 0.02).round() as usize).clamp(1, w.min(h) / 2);
        let mut picks = [Vec::new(), Vec::new(), Vec::new()];
        for y in 0..h {
            for x in 0..w {
                if x < band || y < band || x + band >= w || y + band >= h {
                    for c in 0..3 {
                        picks[c].push(self.rgb[c][y * w + x]);
                    }
                }
            }
        }
        picks.map(|mut v| {
            if v.is_empty() {
                return 1.0;
            }
            let m = v.len() / 2;
            *v.select_nth_unstable_by(m, f32::total_cmp).1
        })
    }

    /// The object as 0–1 values (1 is the object): how far each pixel's
    /// colour stands from the backdrop (`background`, else measured), as a
    /// soft ramp centred on `threshold` so outlines land between pixels.
    /// On a picture with see-through pixels, its coverage.
    pub fn object(&self, background: Option<[f32; 3]>, threshold: f32) -> Vec<f32> {
        if let Some(a) = &self.alpha {
            return a.clone();
        }
        let bg = background.unwrap_or_else(|| self.backdrop());
        let t = threshold.clamp(0.01, 0.95);
        let (lo, hi) = (t * 0.5, t * 1.5);
        (0..self.width * self.height)
            .map(|i| {
                let d = (0..3).map(|c| (self.rgb[c][i] - bg[c]).abs()).fold(0.0f32, f32::max);
                ((d - lo) / (hi - lo)).clamp(0.0, 1.0)
            })
            .collect()
    }

    /// A white picture (for placing before the real one is read).
    pub fn blank(width: usize, height: usize) -> Picture {
        let n = width * height;
        Picture { width, height, rgb: [vec![1.0; n], vec![1.0; n], vec![1.0; n]], alpha: None, bits: 8 }
    }

    /// One pixel's colour (sRGB 0–1).
    pub fn rgb_at(&self, x: usize, y: usize) -> [f32; 3] {
        let i = y * self.width + x;
        [self.rgb[0][i], self.rgb[1][i], self.rgb[2][i]]
    }

    pub fn field(&self, c: Channel) -> Field {
        Field { w: self.width, h: self.height, v: self.channel(c) }
    }
}

/// A picture no larger than `max_edge` pixels on its long side, as PNG
/// (None when it is already small enough and can be sent as it is).
pub fn downscale(bytes: &[u8], max_edge: usize) -> Result<Option<Vec<u8>>, String> {
    let p = decode(bytes)?;
    let long = p.width.max(p.height);
    if long <= max_edge {
        return Ok(None);
    }
    let k = max_edge as f64 / long as f64;
    let (w, h) = (((p.width as f64 * k).round() as usize).max(2), ((p.height as f64 * k).round() as usize).max(2));
    let chans: Vec<Field> = [Channel::Red, Channel::Green, Channel::Blue, Channel::Alpha].iter().map(|c| p.field(*c).resample(w, h)).collect();
    let mut px = Vec::with_capacity(w * h * 4);
    for i in 0..w * h {
        for c in &chans {
            px.push((c.v[i].clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Ok(Some(encode_png(w as u32, h as u32, &px)))
}

/// Encodes 8-bit RGBA pixels (top row first) as a PNG.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().expect("PNG header");
        w.write_image_data(rgba).expect("PNG data");
    }
    out
}

/// One row of an object's silhouette: where the object starts and ends
/// across the picture (pixels from the left), `y` pixels from the top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SilhouetteRow {
    pub y: f64,
    pub left: f64,
    pub right: f64,
}

/// The outline of the one object a photo or drawing shows (a vase on a
/// table, a bottle on white): what sets it apart from the background, row
/// by row.
#[derive(Clone, Debug)]
pub struct Silhouette {
    /// Its rows, top first, `rows` of them evenly from its top to its bottom.
    pub rows: Vec<SilhouetteRow>,
    /// Its top and bottom (pixels from the top).
    pub top: f64,
    pub bottom: f64,
    /// Its axis: the median of the rows' middles (pixels from the left).
    pub axis: f64,
    /// How far the rows' middles stray from the axis at most (pixels): large
    /// when the object is not symmetric or the photo is taken at an angle.
    pub drift: f64,
    /// Where each end first reaches its full width (pixels from the top):
    /// below `top` when the top is a rim seen from a little above (its widest
    /// line is the rim) or a rounded end; likewise above `bottom`.
    pub cap_top: f64,
    pub cap_bottom: f64,
}

/// Finds the main object in a picture and measures its outline across `rows`
/// rows. A pixel is the object where it differs (by more than `threshold`, a
/// colour distance 0–1, default 0.12) from both ends of its row, which are
/// taken to be background (a wall above, a table below, paper); on a picture
/// with transparency, where it is opaque. Of the runs this leaves in a row,
/// the one over the object's main column counts, small gaps in it (a
/// highlight) bridged. The object spans the longest band of rows that have it,
/// less rows at its ends off its axis (a shadow); with `rim`, from where its
/// top reaches full width (the line of a rim seen from a little above).
pub fn silhouette(p: &Picture, rows: usize, threshold: Option<f32>, rim: bool) -> Result<Silhouette, String> {
    let (w, h) = (p.width, p.height);
    if w < 8 || h < 8 {
        return Err("silhouette: the picture is too small".into());
    }
    let rows = rows.clamp(2, h);
    let t = threshold.unwrap_or(0.12).clamp(0.01, 1.0);
    let [r, g, b] = &p.rgb;
    let edge = (w / 60).max(3);
    let mut mask = vec![false; w * h];
    for y in 0..h {
        let mean = |xs: std::ops::Range<usize>| {
            let n = xs.len() as f32;
            let mut c = [0.0f32; 3];
            for x in xs {
                let i = y * w + x;
                c[0] += r[i];
                c[1] += g[i];
                c[2] += b[i];
            }
            c.map(|v| v / n)
        };
        let (bl, br) = (mean(0..edge), mean(w - edge..w));
        for x in 0..w {
            let i = y * w + x;
            mask[i] = match &p.alpha {
                Some(a) => a[i] > 0.5,
                None => {
                    let d = |c: [f32; 3]| ((r[i] - c[0]).powi(2) + (g[i] - c[1]).powi(2) + (b[i] - c[2]).powi(2)).sqrt() / 3f32.sqrt();
                    d(bl) > t && d(br) > t
                }
            };
        }
    }
    // The main column: where the object covers the most rows (smoothed).
    let mut cols = vec![0usize; w];
    for y in 0..h {
        for x in 0..w {
            if mask[y * w + x] {
                cols[x] += 1;
            }
        }
    }
    let k = (w / 100).max(1);
    let main = (0..w).max_by_key(|&x| (x.saturating_sub(k)..(x + k + 1).min(w)).map(|j| cols[j]).sum::<usize>()).unwrap_or(w / 2);
    if cols[main] == 0 {
        return Err("silhouette: no object stands out from the background (try a lower threshold, or a photo against a plain background)".into());
    }
    // Each row's run over the main column, gaps up to 1.5 % of the width bridged.
    let gap = (w * 3 / 200).max(2);
    let run = |y: usize| -> Option<(usize, usize)> {
        let at = |x: usize| mask[y * w + x];
        let near = (main.saturating_sub(gap)..(main + gap + 1).min(w)).find(|&x| at(x))?;
        let (mut lo, mut hi) = (near, near);
        loop {
            match (lo.saturating_sub(gap)..lo).rev().find(|&x| at(x)) {
                Some(x) if x < lo => lo = x,
                _ => break,
            }
        }
        loop {
            match (hi + 1..(hi + gap + 1).min(w)).find(|&x| at(x)) {
                Some(x) => hi = x,
                None => break,
            }
        }
        Some((lo, hi))
    };
    let runs: Vec<Option<(usize, usize)>> = (0..h).map(run).collect();
    // The longest band of rows that have the object.
    let (mut best, mut start) = ((0, 0), None);
    for y in 0..=h {
        match (y < h && runs[y].is_some(), start) {
            (true, None) => start = Some(y),
            (false, Some(s)) => {
                if y - s > best.1 - best.0 {
                    best = (s, y);
                }
                start = None;
            }
            _ => {}
        }
    }
    let (mut top, mut bottom) = (best.0, best.1 - 1);
    if bottom <= top {
        return Err("silhouette: the object is less than two rows tall".into());
    }
    // Rows at the ends whose middle is off the object's axis are not the
    // object: a shadow cast beside its foot, a reflection on the table.
    let mid = |y: usize| runs[y].map(|(a, b)| (a + b + 1) as f64 / 2.0).unwrap();
    let wid = |y: usize| runs[y].map(|(a, b)| (b + 1 - a) as f64).unwrap();
    let mut mids: Vec<f64> = (top..=bottom).map(mid).collect();
    mids.sort_by(|a, b| a.total_cmp(b));
    let axis0 = mids[mids.len() / 2];
    let widest0 = (top..=bottom).map(wid).fold(0.0, f64::max);
    let off = (0.02 * widest0).max(2.0);
    while bottom > top + 1 && (mid(bottom) - axis0).abs() > off {
        bottom -= 1;
    }
    while top + 1 < bottom && (mid(top) - axis0).abs() > off {
        top += 1;
    }
    // Where each end reaches its full width: below a rim seen from a little
    // above (an ellipse whose widest line is the rim), or a rounded end.
    let k = ((bottom - top) / 60).max(2);
    let full = |y: usize, inward: &dyn Fn(usize) -> Option<usize>| -> bool {
        let ahead = (1..=k).filter_map(inward).map(wid).fold(0.0, f64::max);
        wid(y) >= 0.95 * ahead
    };
    let mut cap_top = top;
    while cap_top < bottom && !full(cap_top, &|i| Some(cap_top + i).filter(|y| *y <= bottom)) {
        cap_top += 1;
    }
    let mut cap_bottom = bottom;
    while cap_bottom > top && !full(cap_bottom, &|i| cap_bottom.checked_sub(i).filter(|y| *y >= top)) {
        cap_bottom -= 1;
    }
    // A rim seen from above: the object is measured from the rim's line down.
    let top = if rim { cap_top.min(bottom.saturating_sub(1)).max(top) } else { top };
    let at = |y: f64| -> SilhouetteRow {
        let i = (y.round() as usize).clamp(top, bottom);
        let (lo, hi) = runs[i].unwrap();
        // Pixel edges, not centres: a run of one pixel is one pixel wide.
        SilhouetteRow { y: i as f64, left: lo as f64, right: hi as f64 + 1.0 }
    };
    let out: Vec<SilhouetteRow> = (0..rows).map(|j| at(top as f64 + (bottom - top) as f64 * j as f64 / (rows - 1) as f64)).collect();
    let mut mids: Vec<f64> = (top..=bottom).map(mid).collect();
    mids.sort_by(|a, b| a.total_cmp(b));
    let axis = mids[mids.len() / 2];
    let drift = mids.iter().map(|m| (m - axis).abs()).fold(0.0, f64::max);
    Ok(Silhouette { rows: out, top: top as f64, bottom: bottom as f64 + 1.0, axis, drift, cap_top: cap_top as f64, cap_bottom: cap_bottom as f64 + 1.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_silhouette_finds_a_bottle_on_a_wall_and_table() {
        // 200 × 300: a beige wall above y = 250, a brown table below, a blue
        // body of radius 30 + 20·sin, axis x = 100, from y = 40 to y = 249, a
        // white highlight 2 px wide down it.
        let (w, h) = (200usize, 300usize);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let mut c = if y < 250 { [214, 206, 194] } else { [140, 100, 70] };
                if (40..250).contains(&y) {
                    let rad = 30.0 + 20.0 * ((y - 40) as f64 / 210.0 * std::f64::consts::PI).sin();
                    let dx = x as f64 + 0.5 - 100.0;
                    if dx.abs() < rad {
                        c = if (80..82).contains(&x) { [235, 235, 240] } else { [60, 90, 150] };
                    }
                }
                px[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let p = decode(&encode_png(w as u32, h as u32, &px)).unwrap();
        let s = silhouette(&p, 22, None, false).unwrap();
        assert_eq!((s.top, s.bottom), (40.0, 250.0));
        assert!((s.axis - 100.0).abs() <= 0.5, "axis {}", s.axis);
        // The highlight is bridged, the table not taken.
        for row in &s.rows {
            let rad = 30.0 + 20.0 * ((row.y - 40.0) / 210.0 * std::f64::consts::PI).sin();
            assert!(((row.right - row.left) / 2.0 - rad).abs() <= 1.0, "row {} half width {} vs {rad}", row.y, (row.right - row.left) / 2.0);
        }
    }

    #[test]
    fn a_png_reads_back_as_written() {
        // 2 × 2: black, white / half grey, transparent red.
        let px = [0, 0, 0, 255, 255, 255, 255, 255, 128, 128, 128, 255, 255, 0, 0, 0];
        let p = decode(&encode_png(2, 2, &px)).unwrap();
        assert_eq!((p.width, p.height, p.bits), (2, 2, 8));
        let b = p.channel(Channel::Brightness);
        assert!(b[0].abs() < 1e-6 && (b[1] - 1.0).abs() < 1e-6 && (b[2] - 128.0 / 255.0).abs() < 1e-3);
        assert!((b[3] - 1.0).abs() < 1e-6, "transparent reads as white paper");
        assert_eq!(p.channel(Channel::Alpha), vec![1.0, 1.0, 1.0, 0.0]);
    }

    #[test]
    fn shrinking_averages_and_growing_interpolates() {
        let f = Field { w: 4, h: 1, v: vec![0.0, 1.0, 0.0, 1.0] };
        let small = f.resample(2, 2);
        assert!(small.v.iter().all(|v| (0.0..=1.0).contains(v)));
        let g = Field { w: 2, h: 2, v: vec![0.0, 1.0, 0.0, 1.0] };
        let big = g.resample(5, 5);
        // Edge to edge: the first column is the left pixel, the last the right.
        assert!((big.at(0, 2) - 0.0).abs() < 1e-6 && (big.at(4, 2) - 1.0).abs() < 1e-6);
        assert!((big.at(2, 2) - 0.5).abs() < 1e-6);
    }
}
