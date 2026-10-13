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
        _ => Err("not a picture Forma reads (PNG or JPEG)".into()),
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

#[cfg(test)]
mod tests {
    use super::*;

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
