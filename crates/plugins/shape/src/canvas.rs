//! Canvases: reference pictures placed in the model, to model from.
//!
//! A canvas is a stored picture (a product photo, a scanned drawing) laid on a
//! plane of the model at a known scale, as Fusion 360's Canvas, FreeCAD's
//! image planes and SolidWorks' sketch pictures do. Its pixels map to the
//! plane by a calibration (two points a known distance apart, a scale, or the
//! object's overall size), turned, mirrored and cropped as needed; a photo
//! taken at an angle can be squared up by four points of a rectangle of known
//! size (a perspective correction). Programs read a canvas's mapping and
//! trace its outlines in the model's own millimetres (`canvas("Side")`), and
//! the engine compares the model with it: the model drawn from the canvas's
//! direction over the photo, the silhouettes' overlap (IoU) and where and how
//! far they differ. A canvas is reference only: no mass, no BOM line, never
//! exported as geometry.
//!
//! Pixel coordinates run from the picture's top-left corner (0, 0) to its
//! bottom-right (width, height); pixel (i, j)'s centre is (i + ½, j + ½).
//! The mapping to the plane is one 3 × 3 matrix in homogeneous coordinates:
//!
//!   affine:       plane = D · R · F · Y · (pixel − anchor)
//!   perspective:  plane = R · F · H · pixel
//!
//! with Y mirroring the picture's downward rows upward, F the left–right
//! mirror (`flip`), R the turn (`rotation`, counter-clockwise), D the scale
//! (mm per pixel along the plane's x, and `aspect` times that along y) and H
//! the homography taking four picked points to a rectangle centred on the
//! plane's origin. The plane's origin sits where the anchor pixel (or the
//! rectangle's centre) lands; seen from the plane's normal side, the picture
//! reads the right way round.

use crate::image::{Field, Picture};
use crate::sketch::Plane;
use de_core::{
    Command, CommandError, ErrorCode, IdIndex, Persist, StableId, Tx, Value,
    bevy_ecs::{self, component::Component, world::World},
    json, schemars,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A reference picture placed on a plane of the model.
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Canvas {
    pub name: String,
    /// The stored picture's content hash (`./forma image add`).
    pub image: String,
    /// The picture's size in pixels.
    pub pixels: [u32; 2],
    /// Where it lies: its x to the picture's right, its y up the picture,
    /// seen from the side the normal (x × y) points to.
    pub plane: Plane,
    /// The view it was placed for ("front", "top", "left", …): the direction
    /// comparisons look from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// Millimetres per pixel along the plane's x.
    pub scale: f64,
    /// The plane's y scale over its x scale (1: square pixels). Stretches a
    /// photo foreshortened along one direction.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub aspect: f64,
    /// The pixel that sits at the plane's origin.
    pub anchor: [f64; 2],
    /// Degrees the picture is turned in its plane, counter-clockwise.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation: f64,
    /// Mirrored left to right.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip: bool,
    /// Squared up from four points of a rectangle of known size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective: Option<Perspective>,
    /// Two points a known distance apart that set the scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calibration: Option<Calibration>,
    /// The part of the picture that counts, pixels [left, top, right, bottom].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f64; 4]>,
    /// 0 (invisible) to 1 (opaque) in the view.
    #[serde(default = "half")]
    pub opacity: f64,
    #[serde(default = "yes")]
    pub visible: bool,
    /// Shown through the model (drawn over it) rather than hidden behind it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub through: bool,
    /// The backdrop's colour (sRGB 0–1) for finding the object; default the
    /// median colour round the picture's edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<[f64; 3]>,
    /// How far a colour must stand from the backdrop to count as the object
    /// (0–1, default 0.12).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Whether anything has set its size (a calibration, a scale, a width or
    /// the object's size): until then it is shown 200 mm wide, unsized.
    #[serde(default)]
    pub sized: bool,
    /// The mapping given outright (as programs carry it); never stored.
    #[serde(skip)]
    #[schemars(skip)]
    pub matrix: Option<M3>,
}

fn one() -> f64 {
    1.0
}
fn is_one(v: &f64) -> bool {
    (*v - 1.0).abs() < 1e-12
}
fn is_zero(v: &f64) -> bool {
    *v == 0.0
}
fn half() -> f64 {
    0.5
}
fn yes() -> bool {
    true
}

impl Persist for Canvas {
    const KIND: &'static str = "shape.canvas";
}

/// Two pixels a known distance apart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Calibration {
    pub a: [f64; 2],
    pub b: [f64; 2],
    /// Millimetres between them.
    pub distance: f64,
}

/// Four pixels at the corners of a rectangle in the photo (top-left,
/// top-right, bottom-right, bottom-left as the picture shows them) and the
/// rectangle's true size: the photo is squared up so it is `width` × `height`
/// mm, centred on the plane's origin.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Perspective {
    pub points: [[f64; 2]; 4],
    pub width: f64,
    pub height: f64,
}

// ---- The mapping --------------------------------------------------------------

pub type M3 = [[f64; 3]; 3];

pub fn mul(a: &M3, b: &M3) -> M3 {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

pub fn apply(m: &M3, p: [f64; 2]) -> [f64; 2] {
    let x = m[0][0] * p[0] + m[0][1] * p[1] + m[0][2];
    let y = m[1][0] * p[0] + m[1][1] * p[1] + m[1][2];
    let w = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
    [x / w, y / w]
}

pub fn inverse(m: &M3) -> Option<M3> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if !(det.abs() > 1e-300) || !det.is_finite() {
        return None;
    }
    let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    Some([
        [c(1, 1, 2, 2) / det, -c(0, 1, 2, 2) / det, c(0, 1, 1, 2) / det],
        [-c(1, 0, 2, 2) / det, c(0, 0, 2, 2) / det, -c(0, 0, 1, 2) / det],
        [c(1, 0, 2, 1) / det, -c(0, 0, 2, 1) / det, c(0, 0, 1, 1) / det],
    ])
}

/// The projective map taking each `from` point to its `to` point (four
/// points, no three in a line), with h₃₃ = 1.
pub fn homography(from: &[[f64; 2]; 4], to: &[[f64; 2]; 4]) -> Result<M3, String> {
    let mut a = [[0.0f64; 9]; 8];
    for k in 0..4 {
        let ([x, y], [u, v]) = (from[k], to[k]);
        a[2 * k] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * k + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    // Gaussian elimination with partial pivoting on the 8 × 8 system.
    for col in 0..8 {
        let pivot = (col..8).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs())).unwrap();
        if a[pivot][col].abs() < 1e-12 {
            return Err("the four points must be corners of a rectangle as the photo shows it, no three in a line".into());
        }
        a.swap(col, pivot);
        for row in 0..8 {
            if row != col {
                let f = a[row][col] / a[col][col];
                for k in col..9 {
                    a[row][k] -= f * a[col][k];
                }
            }
        }
    }
    let h: Vec<f64> = (0..8).map(|i| a[i][8] / a[i][i]).collect();
    Ok([[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]])
}

/// R · F as a matrix: the turn after the mirror.
fn turn(rotation: f64, flip: bool) -> M3 {
    let (s, c) = rotation.to_radians().sin_cos();
    let f = if flip { -1.0 } else { 1.0 };
    [[c * f, -s, 0.0], [s * f, c, 0.0], [0.0, 0.0, 1.0]]
}

impl Canvas {
    /// Pixel → plane millimetres.
    pub fn to_plane(&self) -> M3 {
        if let Some(m) = self.matrix {
            return m;
        }
        let rf = turn(self.rotation, self.flip);
        if let Some(p) = &self.perspective {
            let (w, h) = (p.width / 2.0, p.height / 2.0);
            if let Ok(hm) = homography(&p.points, &[[-w, h], [w, h], [w, -h], [-w, -h]]) {
                return mul(&rf, &hm);
            }
        }
        let d = [[self.scale, 0.0, 0.0], [0.0, self.scale * self.aspect, 0.0], [0.0, 0.0, 1.0]];
        let y = [[1.0, 0.0, -self.anchor[0]], [0.0, -1.0, self.anchor[1]], [0.0, 0.0, 1.0]];
        mul(&d, &mul(&rf, &y))
    }

    /// Plane millimetres → pixel.
    pub fn to_pixel(&self) -> M3 {
        inverse(&self.to_plane()).unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
    }

    pub fn pixel_to_world(&self, p: [f64; 2]) -> [f64; 3] {
        self.plane.to_world(apply(&self.to_plane(), p))
    }

    /// Millimetres per pixel near a pixel (the square root of the local area scale).
    pub fn mm_per_px_at(&self, p: [f64; 2]) -> f64 {
        let m = self.to_plane();
        let (a, b, c) = (apply(&m, p), apply(&m, [p[0] + 1.0, p[1]]), apply(&m, [p[0], p[1] + 1.0]));
        let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - a[0], c[1] - a[1]]);
        (u[0] * v[1] - u[1] * v[0]).abs().sqrt()
    }

    /// The counted region in pixels [left, top, right, bottom].
    pub fn region(&self) -> [f64; 4] {
        let (w, h) = (self.pixels[0] as f64, self.pixels[1] as f64);
        match self.crop {
            Some([x0, y0, x1, y1]) => [x0.clamp(0.0, w), y0.clamp(0.0, h), x1.clamp(0.0, w), y1.clamp(0.0, h)],
            None => [0.0, 0.0, w, h],
        }
    }

    /// The four corners of the shown region in the world (top-left, top-right,
    /// bottom-right, bottom-left), for drawing it.
    pub fn corners(&self) -> [[f64; 3]; 4] {
        let [x0, y0, x1, y1] = self.region();
        [[x0, y0], [x1, y0], [x1, y1], [x0, y1]].map(|p| self.pixel_to_world(p))
    }

    /// The JSON programs and the view read: the mapping both ways and the plane.
    pub fn view_json(&self, id: StableId) -> Value {
        let flat = |m: M3| m.iter().flatten().copied().collect::<Vec<f64>>();
        let c = self.region();
        let centre = [(c[0] + c[2]) / 2.0, (c[1] + c[3]) / 2.0];
        json!({
            "id": id,
            "name": self.name,
            "image": self.image,
            "pixels": self.pixels,
            "plane": self.plane,
            "normal": self.plane.normal(),
            "view": self.view,
            "toPlane": flat(self.to_plane()),
            "toPixel": flat(self.to_pixel()),
            "mmPerPx": self.mm_per_px_at(centre),
            "crop": self.crop,
            "corners": self.corners(),
            "background": self.background,
            "threshold": self.threshold,
            "sized": self.sized,
        })
    }

    fn validate(&self) -> Result<(), CommandError> {
        let bad = |m: String| Err(CommandError::new(ErrorCode::InvalidParameters, m));
        if self.name.trim().is_empty() || self.name.contains('.') {
            return bad("a canvas needs a name without dots".into());
        }
        if !(self.scale.is_finite() && self.scale > 0.0) || !(self.aspect.is_finite() && self.aspect > 0.0) {
            return bad("the scale must be a positive number of mm per pixel".into());
        }
        if !(0.0..=1.0).contains(&self.opacity) {
            return bad("opacity is 0 (invisible) to 1 (opaque)".into());
        }
        if self.anchor.iter().chain(&[self.rotation]).any(|v| !v.is_finite()) {
            return bad("the anchor and rotation must be finite".into());
        }
        if let Some([x0, y0, x1, y1]) = self.crop
            && !(x1 - x0 >= 2.0 && y1 - y0 >= 2.0)
        {
            return bad("a crop is [left, top, right, bottom] in pixels, at least 2 × 2".into());
        }
        if let Some(p) = &self.perspective {
            if !(p.width > 0.0 && p.height > 0.0) {
                return bad("a perspective correction needs the rectangle's width and height in mm".into());
            }
            let (w, h) = (p.width / 2.0, p.height / 2.0);
            homography(&p.points, &[[-w, h], [w, h], [w, -h], [-w, -h]]).map_err(|e| CommandError::new(ErrorCode::InvalidParameters, e))?;
        }
        if let Some(t) = self.threshold
            && !(0.01..=0.95).contains(&t)
        {
            return bad("the threshold is a colour difference 0.01 to 0.95".into());
        }
        if inverse(&self.to_plane()).is_none() {
            return bad("the picture's mapping cannot be undone (zero scale?)".into());
        }
        Ok(())
    }
}

/// A plane for a named view, seen from where the camera stands: "front"
/// (from −Y, the XZ plane), "back" (+Y), "left" (−X), "right" (+X), "top"
/// (+Z, the XY plane), "bottom" (−Z); also "XY", "XZ", "YZ". Its x and y are
/// the picture's right and up as that view shows the model.
pub fn view_plane(name: &str) -> Result<(Plane, String), String> {
    let (x, y, view) = match name.to_ascii_lowercase().as_str() {
        "front" | "xz" => ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], "front"),
        "back" | "rear" => ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], "back"),
        "left" => ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0], "left"),
        "right" | "yz" => ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0], "right"),
        "top" | "xy" => ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], "top"),
        "bottom" => ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], "bottom"),
        other => return Err(format!("unknown plane “{other}” (front, back, left, right, top, bottom, XY, XZ, YZ, or give origin, x and y)")),
    };
    Ok((Plane { origin: [0.0; 3], x, y }, view.to_string()))
}

/// Where a canvas lies: a named view's plane, a full frame, or a face (its
/// centre and outward normal).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum CanvasPlane {
    Named(String),
    Full(Plane),
    Normal {
        origin: [f64; 3],
        normal: [f64; 3],
        #[serde(default)]
        x: Option<[f64; 3]>,
    },
}

/// The object's overall size in the picture: its outline's bounds become
/// `width` × `height` mm (either or both; both stretch a foreshortened
/// photo), with the corner or middle `align` names at the plane's origin.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct ObjectSize {
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// Which point of the outline's bounds goes to the origin: "left" |
    /// "center" | "right" and "bottom" | "middle" | "top", e.g. "left top".
    /// Default the middle.
    #[serde(default)]
    pub align: Option<String>,
}

/// A turn about a world axis (right-handed, degrees).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct Pivot {
    pub axis: [f64; 3],
    pub degrees: f64,
}

/// `v` turned about the unit `axis` by `deg` degrees (Rodrigues).
fn rotate_about(v: [f64; 3], axis: [f64; 3], deg: f64) -> [f64; 3] {
    let k = unit3(axis);
    let (s, c) = deg.to_radians().sin_cos();
    let kv = [k[1] * v[2] - k[2] * v[1], k[2] * v[0] - k[0] * v[2], k[0] * v[1] - k[1] * v[0]];
    let kd = k[0] * v[0] + k[1] * v[1] + k[2] * v[2];
    [0, 1, 2].map(|i| v[i] * c + kv[i] * s + k[i] * kd * (1.0 - c))
}

/// What `canvas.create` and `canvas.update` set (anything left out stays).
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
pub struct Settings {
    pub name: Option<String>,
    /// The picture (a stored picture's id): replacing it keeps the placement.
    pub image: Option<String>,
    pub plane: Option<CanvasPlane>,
    /// Moves a named plane this far along its normal (mm).
    pub offset: Option<f64>,
    /// Turns the plane about a world axis through its origin: for a photo
    /// taken square on while the object stood pitched or rolled (resting on
    /// a grip, a foot), so comparisons look from where the camera really was.
    pub pivot: Option<Pivot>,
    /// Where the anchor pixel lands in the world (mm); moves the plane there.
    pub at: Option<[f64; 3]>,
    /// The pixel to put at the plane's origin (default the picture's middle).
    pub anchor: Option<[f64; 2]>,
    /// Millimetres per pixel.
    pub scale: Option<f64>,
    /// The whole picture's width in mm (sets the scale).
    pub width: Option<f64>,
    /// The whole picture's height in mm (sets the scale).
    pub height: Option<f64>,
    /// Two pixels and the true distance between them (sets the scale).
    pub calibrate: Option<Calibration>,
    /// The object's overall size: sets the scale (and aspect), and puts the
    /// outline's aligned corner at the origin.
    pub object: Option<ObjectSize>,
    pub aspect: Option<f64>,
    pub rotation: Option<f64>,
    /// Two pixels that should lie level: turns the picture so they do.
    pub level: Option<[[f64; 2]; 2]>,
    pub flip: Option<bool>,
    pub perspective: Option<Perspective>,
    /// [left, top, right, bottom] pixels.
    pub crop: Option<[f64; 4]>,
    pub opacity: Option<f64>,
    pub visible: Option<bool>,
    pub through: Option<bool>,
    pub background: Option<[f64; 3]>,
    pub threshold: Option<f64>,
    /// Settings to clear: "crop", "perspective", "calibration", "rotation",
    /// "background", "threshold", "flip", "aspect".
    #[serde(default)]
    pub reset: Vec<String>,
}

fn invalid(m: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParameters, m)
}

fn unit3(v: [f64; 3]) -> [f64; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    v.map(|x| x / l)
}

/// Applies settings to a canvas (a new one starts from `base`).
fn settle(c: &mut Canvas, s: Settings, picture: &Picture) -> Result<(), CommandError> {
    for r in &s.reset {
        match r.as_str() {
            "crop" => c.crop = None,
            "perspective" => c.perspective = None,
            "calibration" => c.calibration = None,
            "rotation" => c.rotation = 0.0,
            "background" => c.background = None,
            "threshold" => c.threshold = None,
            "flip" => c.flip = false,
            "aspect" => c.aspect = 1.0,
            other => return Err(invalid(format!("cannot reset “{other}” (crop, perspective, calibration, rotation, background, threshold, flip, aspect)"))),
        }
    }
    if let Some(n) = s.name {
        c.name = n;
    }
    if let Some(p) = s.plane {
        let keep = c.plane.origin;
        match p {
            CanvasPlane::Named(n) => {
                let (plane, view) = view_plane(&n).map_err(invalid)?;
                c.plane = Plane { origin: keep, ..plane };
                c.view = Some(view);
            }
            CanvasPlane::Full(p) => {
                if (p.x.iter().map(|v| v * v).sum::<f64>().sqrt() - 1.0).abs() > 1e-6 || (p.y.iter().map(|v| v * v).sum::<f64>().sqrt() - 1.0).abs() > 1e-6 || (p.x[0] * p.y[0] + p.x[1] * p.y[1] + p.x[2] * p.y[2]).abs() > 1e-6 {
                    return Err(invalid("a plane's x and y must be unit length and square to each other"));
                }
                c.plane = p;
                c.view = None;
            }
            CanvasPlane::Normal { origin, normal, x } => {
                c.plane = crate::sketch::plane_from_normal(origin, normal, x, 0.0).map_err(invalid)?;
                c.view = None;
            }
        }
    }
    if let Some(o) = s.offset {
        let n = unit3(c.plane.normal());
        // Along the normal from where it lies within the plane.
        let d = c.plane.origin[0] * n[0] + c.plane.origin[1] * n[1] + c.plane.origin[2] * n[2];
        c.plane.origin = [0, 1, 2].map(|k| c.plane.origin[k] + n[k] * (o - d));
    }
    if let Some(p) = s.pivot {
        if !(p.axis.iter().map(|v| v * v).sum::<f64>() > 1e-18) || !p.degrees.is_finite() {
            return Err(invalid("a pivot needs an axis and an angle in degrees"));
        }
        c.plane.x = unit3(rotate_about(c.plane.x, p.axis, p.degrees));
        c.plane.y = unit3(rotate_about(c.plane.y, p.axis, p.degrees));
    }
    if let Some(at) = s.at {
        if at.iter().any(|v| !v.is_finite()) {
            return Err(invalid("at must be a finite point"));
        }
        c.plane.origin = at;
    }
    if let Some(img) = s.image {
        c.image = img;
        c.pixels = [picture.width as u32, picture.height as u32];
    }
    if let Some(v) = s.flip {
        c.flip = v;
    }
    if let Some(v) = s.rotation {
        c.rotation = v;
    }
    if let Some(v) = s.aspect {
        c.aspect = v;
    }
    if let Some(v) = s.crop {
        c.crop = Some(v);
    }
    if let Some([a, b]) = s.level {
        // The line a → b, as the picture shows it (rows down), turned level.
        let (dx, dy) = (b[0] - a[0], -(b[1] - a[1]));
        if dx.hypot(dy) < 1e-9 {
            return Err(invalid("level needs two different points"));
        }
        let dx = if c.flip { -dx } else { dx };
        let mut ang = -dy.atan2(dx).to_degrees();
        // The smallest turn that makes the line level (either way along it).
        if ang > 90.0 {
            ang -= 180.0;
        } else if ang < -90.0 {
            ang += 180.0;
        }
        c.rotation = if c.flip { -ang } else { ang };
    }
    if let Some(p) = s.perspective {
        c.perspective = Some(p);
        c.sized = true;
    }
    if let Some(a) = s.anchor {
        c.anchor = a;
    }
    // The scale: the last word wins in this order (a calibration is kept and
    // re-applied when the aspect changes).
    if let Some(v) = s.scale {
        c.scale = v;
        c.calibration = None;
        c.sized = true;
    }
    if let Some(w) = s.width {
        c.scale = w / picture.width as f64;
        c.calibration = None;
        c.sized = true;
    }
    if let Some(h) = s.height {
        c.scale = h / (picture.height as f64 * c.aspect);
        c.calibration = None;
        c.sized = true;
    }
    if let Some(cal) = s.calibrate {
        c.calibration = Some(cal);
        c.sized = true;
    }
    if let Some(cal) = &c.calibration {
        if !(cal.distance > 0.0) {
            return Err(invalid("a calibration needs the true distance in mm"));
        }
        let rf = turn(c.rotation, c.flip);
        let d = [cal.b[0] - cal.a[0], -(cal.b[1] - cal.a[1])];
        let t = [rf[0][0] * d[0] + rf[0][1] * d[1], rf[1][0] * d[0] + rf[1][1] * d[1]];
        let px = t[0].hypot(t[1] * c.aspect);
        if px < 1.0 {
            return Err(invalid("the two calibration points must be at least a pixel apart"));
        }
        c.scale = cal.distance / px;
    }
    if let Some(o) = s.object {
        fit_object(c, &o, picture)?;
        c.sized = true;
    }
    if let Some(v) = s.opacity {
        c.opacity = v;
    }
    if let Some(v) = s.visible {
        c.visible = v;
    }
    if let Some(v) = s.through {
        c.through = v;
    }
    if let Some(v) = s.background {
        c.background = Some(v);
    }
    if let Some(v) = s.threshold {
        c.threshold = Some(v);
    }
    if c.perspective.is_some() && (c.calibration.is_some()) {
        c.calibration = None;
    }
    c.validate()
}

/// The object's ink (0–1) over the whole picture, outside the crop cleared.
pub fn object_ink(c: &Canvas, picture: &Picture) -> Field {
    let bg = c.background.map(|b| b.map(|v| v as f32));
    let mut f = Field { w: picture.width, h: picture.height, v: picture.object(bg, c.threshold.unwrap_or(crate::image::OBJECT_THRESHOLD as f64) as f32) };
    if c.crop.is_some() {
        let [x0, y0, x1, y1] = c.region();
        for y in 0..f.h {
            for x in 0..f.w {
                let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
                if cx < x0 || cx > x1 || cy < y0 || cy > y1 {
                    f.v[y * f.w + x] = 0.0;
                }
            }
        }
    }
    f
}

/// The object's outline bounds in the turned (not yet scaled) frame: sets
/// the scale and aspect from its true size and anchors its aligned point.
fn fit_object(c: &mut Canvas, o: &ObjectSize, picture: &Picture) -> Result<(), CommandError> {
    if c.perspective.is_some() {
        return Err(invalid("a squared-up (perspective) canvas takes its size from the rectangle; give that instead"));
    }
    let ink = object_ink(c, picture);
    let rf = turn(c.rotation, c.flip);
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut count = 0usize;
    for y in 0..ink.h {
        for x in 0..ink.w {
            if ink.v[y * ink.w + x] > 0.5 {
                count += 1;
                for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                    let d = [x as f64 + dx, -(y as f64 + dy)];
                    let t = [rf[0][0] * d[0] + rf[0][1] * d[1], rf[1][0] * d[0] + rf[1][1] * d[1]];
                    lo = [lo[0].min(t[0]), lo[1].min(t[1])];
                    hi = [hi[0].max(t[0]), hi[1].max(t[1])];
                }
            }
        }
    }
    if count < 16 {
        return Err(invalid("no object stands out from the backdrop here (crop to it, or set background / threshold)"));
    }
    let (bw, bh) = (hi[0] - lo[0], hi[1] - lo[1]);
    match (o.width, o.height) {
        (Some(w), Some(h)) => {
            if !(w > 0.0 && h > 0.0) {
                return Err(invalid("the object's size must be positive"));
            }
            c.scale = w / bw;
            c.aspect = (h / bh) / c.scale;
        }
        (Some(w), None) if w > 0.0 => c.scale = w / bw,
        (None, Some(h)) if h > 0.0 => c.scale = h / (bh * c.aspect),
        _ => return Err(invalid("give the object's width or height (or both) in mm")),
    }
    c.calibration = None;
    // The aligned point of the bounds, in the turned frame, back to a pixel.
    let words = o.align.clone().unwrap_or_default().to_ascii_lowercase();
    let has = |w: &str| words.split(|ch: char| !ch.is_alphanumeric()).any(|x| x == w);
    let tx = if has("left") { lo[0] } else if has("right") { hi[0] } else { (lo[0] + hi[0]) / 2.0 };
    let ty = if has("bottom") { lo[1] } else if has("top") { hi[1] } else { (lo[1] + hi[1]) / 2.0 };
    // Undo the turn (R·F is orthogonal: its inverse is its transpose).
    let d = [rf[0][0] * tx + rf[1][0] * ty, rf[0][1] * tx + rf[1][1] * ty];
    c.anchor = [d[0], -d[1]];
    Ok(())
}

fn load_picture(sha: &str) -> Result<std::sync::Arc<Picture>, CommandError> {
    crate::image::load(sha).map_err(|e| CommandError::new(ErrorCode::UnresolvedReference, format!("the canvas's picture: {e} (add it with ./forma image add <file>)")))
}

/// `canvas.create`: a reference picture on a plane of the model.
#[derive(Deserialize, JsonSchema)]
pub struct CreateCanvas {
    pub name: String,
    /// A stored picture's id (`./forma image add <file>`).
    pub image: String,
    #[serde(flatten)]
    pub settings: Settings,
}

impl Command for CreateCanvas {
    const NAME: &'static str = "canvas.create";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let picture = load_picture(&self.image)?;
        let (w, h) = (picture.width as f64, picture.height as f64);
        let (plane, view) = view_plane("front").unwrap();
        let mut c = Canvas {
            name: self.name.clone(),
            image: self.image.clone(),
            pixels: [picture.width as u32, picture.height as u32],
            plane,
            view: Some(view),
            scale: 200.0 / w,
            aspect: 1.0,
            anchor: [w / 2.0, h / 2.0],
            rotation: 0.0,
            flip: false,
            perspective: None,
            calibration: None,
            crop: None,
            opacity: 0.5,
            visible: true,
            through: false,
            background: None,
            threshold: None,
            sized: false,
            matrix: None,
        };
        let mut s = self.settings;
        s.name = Some(self.name);
        s.image = None;
        // A crop's middle is the default anchor.
        if s.anchor.is_none()
            && let Some([x0, y0, x1, y1]) = s.crop
        {
            c.anchor = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
        }
        settle(&mut c, s, &picture)?;
        let id = tx.create();
        let out = summary(id, &c, &picture);
        tx.set(id, c)?;
        Ok(out)
    }
}

/// `canvas.update`: change a canvas's placement, calibration or look.
#[derive(Deserialize, JsonSchema)]
pub struct UpdateCanvas {
    pub target: StableId,
    #[serde(flatten)]
    pub settings: Settings,
}

impl Command for UpdateCanvas {
    const NAME: &'static str = "canvas.update";

    fn apply(self, tx: &mut Tx) -> Result<Value, CommandError> {
        let mut c = tx.get::<Canvas>(self.target).cloned().ok_or_else(|| CommandError::new(ErrorCode::UnresolvedReference, format!("{} is not a canvas", self.target)))?;
        let picture = load_picture(self.settings.image.as_deref().unwrap_or(&c.image))?;
        settle(&mut c, self.settings, &picture)?;
        let out = summary(self.target, &c, &picture);
        tx.set(self.target, c)?;
        Ok(out)
    }
}

fn summary(id: StableId, c: &Canvas, picture: &Picture) -> Value {
    let [x0, y0, x1, y1] = c.region();
    let m = c.to_plane();
    let (a, b, d) = (apply(&m, [x0, y0]), apply(&m, [x1, y0]), apply(&m, [x0, y1]));
    let size = [(b[0] - a[0]).hypot(b[1] - a[1]), (d[0] - a[0]).hypot(d[1] - a[1])];
    let mut out = json!({ "id": id, "name": c.name, "view": c.view, "mm_per_px": c.mm_per_px_at([(x0 + x1) / 2.0, (y0 + y1) / 2.0]), "size_mm": size, "sized": c.sized });
    // Where the object stands in the picture and on the plane: what to line views up by.
    if picture.width as u32 == c.pixels[0] && picture.height as u32 == c.pixels[1] {
        let ink = object_ink(c, picture);
        let (mut lo, mut hi) = ([usize::MAX; 2], [0usize; 2]);
        let (mut plo, mut phi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for y in 0..ink.h {
            for x in 0..ink.w {
                if ink.v[y * ink.w + x] > 0.5 {
                    lo = [lo[0].min(x), lo[1].min(y)];
                    hi = [hi[0].max(x + 1), hi[1].max(y + 1)];
                    for (dx, dy) in [(0.0, 0.0), (1.0, 1.0), (1.0, 0.0), (0.0, 1.0)] {
                        let q = apply(&m, [x as f64 + dx, y as f64 + dy]);
                        plo = [plo[0].min(q[0]), plo[1].min(q[1])];
                        phi = [phi[0].max(q[0]), phi[1].max(q[1])];
                    }
                }
            }
        }
        if lo[0] <= hi[0] && hi[0] > 0 {
            out["object_px"] = json!([lo[0], lo[1], hi[0], hi[1]]);
            out["object_mm"] = json!({ "min": plo, "max": phi, "size": [phi[0] - plo[0], phi[1] - plo[1]] });
        }
    }
    out
}

// ---- Tracing in the model's millimetres ---------------------------------------

/// How to trace a canvas (a program's `canvas(…).trace({ … })`).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceOptions {
    /// "object" (default: what stands out from the backdrop), "brightness"
    /// (dark is ink), "alpha", "red", "green", "blue".
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default = "half")]
    pub threshold: f64,
    #[serde(default)]
    pub invert: bool,
    /// Gaussian softening in pixels.
    #[serde(default = "one")]
    pub smooth: f64,
    /// Outlines and holes smaller than this many mm² are dropped.
    #[serde(default)]
    pub min_area: Option<f64>,
    /// How far the thinned outline may stray (mm).
    #[serde(default)]
    pub simplify: Option<f64>,
    /// Keep holes (a trigger guard's opening); false fills them.
    #[serde(default = "yes")]
    pub holes: bool,
    /// Only the largest outline (and its holes).
    #[serde(default)]
    pub largest: bool,
    /// Only this region, pixels [left, top, right, bottom] (inside the crop).
    #[serde(default)]
    pub region: Option<[f64; 4]>,
}

/// A canvas's outlines in its plane's millimetres: counter-clockwise round
/// the ink, clockwise round holes.
pub fn trace(c: &Canvas, o: &TraceOptions, tolerance: f64) -> Result<Vec<Vec<[f64; 2]>>, String> {
    let picture = crate::image::load(&c.image)?;
    let mut f = match o.channel.as_deref().unwrap_or("object") {
        "object" | "silhouette" => object_ink(c, &picture),
        other => {
            let ch = crate::image::Channel::parse(other)?;
            let dark = ch == crate::image::Channel::Brightness;
            let mut f = picture.field(ch);
            if dark {
                for v in &mut f.v {
                    *v = 1.0 - *v;
                }
            }
            f
        }
    };
    if o.invert {
        for v in &mut f.v {
            *v = 1.0 - *v;
        }
    }
    // Outside the crop (and the asked region) is never ink.
    let [mut x0, mut y0, mut x1, mut y1] = c.region();
    if let Some([a, b, cc, d]) = o.region {
        (x0, y0, x1, y1) = (x0.max(a), y0.max(b), x1.min(cc), y1.min(d));
    }
    for y in 0..f.h {
        for x in 0..f.w {
            let (cx, cy) = (x as f64 + 0.5, y as f64 + 0.5);
            if cx < x0 || cx > x1 || cy < y0 || cy > y1 {
                f.v[y * f.w + x] = 0.0;
            }
        }
    }
    f.blur(o.smooth.max(0.0));
    let m = c.to_plane();
    // The mapping keeps the pixel frame's turning sense (rows run down) when
    // its determinant is positive: then outlines come out clockwise, so reverse.
    let centre = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
    let (p, q, r) = (apply(&m, centre), apply(&m, [centre[0] + 1.0, centre[1]]), apply(&m, [centre[0], centre[1] + 1.0]));
    let det = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
    let mm = c.mm_per_px_at(centre);
    let min_area = o.min_area.unwrap_or(16.0 * mm * mm);
    let eps = o.simplify.unwrap_or(tolerance.max(mm * 0.25)).max(1e-6);
    let mut loops: Vec<Vec<[f64; 2]>> = Vec::new();
    for l in crate::trace::contours(&f, o.threshold as f32) {
        let mut pts: Vec<[f64; 2]> = l.iter().map(|p| apply(&m, *p)).collect();
        if det > 0.0 {
            pts.reverse();
        }
        let pts = crate::trace::simplify(&pts, eps);
        if pts.len() >= 3 && crate::trace::area(&pts).abs() >= min_area {
            loops.push(pts);
        }
    }
    if !o.holes {
        loops.retain(|l| crate::trace::area(l) > 0.0);
    }
    if o.largest {
        if let Some(big) = loops.iter().filter(|l| crate::trace::area(l) > 0.0).max_by(|a, b| crate::trace::area(a).total_cmp(&crate::trace::area(b))).cloned() {
            let holes: Vec<Vec<[f64; 2]>> = loops.iter().filter(|l| crate::trace::area(l) < 0.0 && inside(&big, l[0])).cloned().collect();
            loops = std::iter::once(big).chain(holes).collect();
        }
    }
    if loops.is_empty() {
        return Err(format!("canvas “{}”: nothing traced (no object stands out from the backdrop{})", c.name, if o.channel.is_some() { "; try another threshold or invert" } else { "; set its background or threshold, or crop to the object" }));
    }
    Ok(loops)
}

/// Where a straight line across outlines runs inside them: `vertical` (x =
/// `c`) or horizontal (y = `c`) → spans along the line, in order.
pub fn spans(loops: &[Vec<[f64; 2]>], c: f64, vertical: bool) -> Vec<[f64; 2]> {
    let (i, j) = if vertical { (0, 1) } else { (1, 0) };
    let mut hits = Vec::new();
    for l in loops {
        for k in 0..l.len() {
            let (a, b) = (l[k], l[(k + 1) % l.len()]);
            if (a[i] <= c && c < b[i]) || (b[i] <= c && c < a[i]) {
                hits.push(a[j] + (c - a[i]) / (b[i] - a[i]) * (b[j] - a[j]));
            }
        }
    }
    hits.sort_by(f64::total_cmp);
    hits.chunks_exact(2).map(|p| [p[0], p[1]]).collect()
}

/// A canvas's outline measured in plane mm: its bounds and area, and where
/// it is at stations every `every` mm across and up (what `./forma canvas
/// measure` prints).
pub fn measure(c: &Canvas, o: &TraceOptions, every: f64, tolerance: f64) -> Result<Value, String> {
    let loops = trace(c, o, tolerance)?;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for p in loops.iter().flatten() {
        lo = [lo[0].min(p[0]), lo[1].min(p[1])];
        hi = [hi[0].max(p[0]), hi[1].max(p[1])];
    }
    let area: f64 = loops.iter().map(|l| crate::trace::area(l)).sum();
    let step = if every > 0.0 { every } else { ((hi[0] - lo[0]).max(hi[1] - lo[1]) / 20.0).max(0.5) };
    let round = |v: f64| (v * 100.0).round() / 100.0;
    let stations = |a: f64, b: f64, vertical: bool| -> Vec<Value> {
        let first = (a / step).ceil() * step;
        let mut out = Vec::new();
        let mut t = first;
        while t <= b + 1e-9 && out.len() < 2000 {
            let s: Vec<[f64; 2]> = spans(&loops, t, vertical).into_iter().map(|[p, q]| [round(p), round(q)]).collect();
            out.push(json!({ "at": round(t), "spans": s }));
            t += step;
        }
        out
    };
    Ok(json!({
        "canvas": c.name,
        "view": c.view,
        "plane": c.plane,
        "bounds": { "min": [round(lo[0]), round(lo[1])], "max": [round(hi[0]), round(hi[1])], "size": [round(hi[0] - lo[0]), round(hi[1] - lo[1])] },
        "area_mm2": round(area),
        "outlines": loops.iter().filter(|l| crate::trace::area(l) > 0.0).count(),
        "holes": loops.iter().filter(|l| crate::trace::area(l) < 0.0).count(),
        "step": step,
        "across": stations(lo[0], hi[0], true),
        "up": stations(lo[1], hi[1], false),
    }))
}

fn inside(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
    }
    c
}

/// `canvasTrace` in programs: the canvas as programs see it (its JSON) and
/// trace options, to loops in plane mm (JSON).
pub fn trace_json(canvas: &str, options: &str, tolerance: f64) -> Result<String, String> {
    // A program is evaluated several times (the view, the exact solid, its
    // mass) and on every change, while its canvases rarely change: keep the
    // last traces.
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Vec<(u64, String)>>> = std::sync::OnceLock::new();
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (canvas, options, tolerance.to_bits()).hash(&mut h);
        h.finish()
    };
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, v)) = cache.lock().unwrap().iter().find(|(k, _)| *k == key) {
        return Ok(v.clone());
    }
    let out = trace_json_uncached(canvas, options, tolerance)?;
    let mut c = cache.lock().unwrap();
    if c.len() >= 48 {
        c.remove(0);
    }
    c.push((key, out.clone()));
    Ok(out)
}

fn trace_json_uncached(canvas: &str, options: &str, tolerance: f64) -> Result<String, String> {
    let c = from_view_json(canvas)?;
    let o: TraceOptions = serde_json::from_str(if options.trim().is_empty() { "{}" } else { options }).map_err(|e| format!("canvas trace: {e}"))?;
    let loops = trace(&c, &o, tolerance)?;
    Ok(serde_json::to_string(&loops).unwrap())
}

/// A canvas back from the JSON programs receive (only what tracing needs;
/// the mapping is carried outright).
fn from_view_json(text: &str) -> Result<Canvas, String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct V {
        name: String,
        image: String,
        pixels: [u32; 2],
        plane: Plane,
        to_plane: Vec<f64>,
        #[serde(default)]
        crop: Option<[f64; 4]>,
        #[serde(default)]
        background: Option<[f64; 3]>,
        #[serde(default)]
        threshold: Option<f64>,
    }
    let v: V = serde_json::from_str(text).map_err(|e| format!("canvas: {e}"))?;
    if v.to_plane.len() != 9 || v.to_plane.iter().any(|x| !x.is_finite()) {
        return Err("canvas: its mapping is missing".into());
    }
    let t = &v.to_plane;
    Ok(Canvas {
        name: v.name,
        image: v.image,
        pixels: v.pixels,
        plane: v.plane,
        view: None,
        scale: 1.0,
        aspect: 1.0,
        anchor: [0.0, 0.0],
        rotation: 0.0,
        flip: false,
        perspective: None,
        calibration: None,
        crop: v.crop,
        opacity: 0.5,
        visible: true,
        through: false,
        background: v.background,
        threshold: v.threshold,
        sized: true,
        matrix: Some([[t[0], t[1], t[2]], [t[3], t[4], t[5]], [t[6], t[7], t[8]]]),
    })
}

// ---- Programs reading canvases -------------------------------------------------

/// Canvases sit in a program's `uses` beside its sketches, under this prefix.
pub const USE_PREFIX: &str = "canvas:";

/// The canvases a program names (`canvas("Side")`).
pub fn named_in(source: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = source;
    while let Some(i) = rest.find("canvas(") {
        let before = rest[..i].chars().last();
        rest = &rest[i + 7..];
        // Not part of a longer name (fromCanvas(…), mycanvas(…)).
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$' || c == '.') {
            continue;
        }
        let t = rest.trim_start();
        let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'' || *c == '`') else { continue };
        if let Some(end) = t[1..].find(q) {
            out.push(t[1..1 + end].to_string());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Finds the canvases a program reads, by name, keeping links it has.
pub fn resolve_uses(tx: &Tx, source: &str, existing: &BTreeMap<String, StableId>) -> Result<BTreeMap<String, StableId>, CommandError> {
    let world = tx.world();
    let mut out = BTreeMap::new();
    for name in named_in(source) {
        let key = format!("{USE_PREFIX}{name}");
        if let Some(id) = existing.get(&key).filter(|id| tx.get::<Canvas>(**id).is_some_and(|c| c.name == name)) {
            out.insert(key, *id);
            continue;
        }
        let found: Vec<StableId> = world.resource::<IdIndex>().iter().filter(|(_, e)| world.get::<Canvas>(*e).is_some_and(|c| c.name == name)).map(|(id, _)| id).collect();
        match found.as_slice() {
            [id] => {
                out.insert(key, *id);
            }
            [] => return Err(CommandError::new(ErrorCode::UnresolvedReference, format!("there is no canvas called “{name}” (./forma canvas list)"))),
            _ => return Err(CommandError::new(ErrorCode::InvalidParameters, format!("more than one canvas is called “{name}”; rename one"))),
        }
    }
    Ok(out)
}

/// What a program reads from its canvases, as the runtime receives it (JSON).
pub fn inputs(world: &World, uses: &BTreeMap<String, StableId>) -> Result<String, String> {
    let mut m = serde_json::Map::new();
    for (key, id) in uses {
        let Some(name) = key.strip_prefix(USE_PREFIX) else { continue };
        let e = world.resource::<IdIndex>().get(*id).ok_or_else(|| format!("canvas “{name}” was deleted"))?;
        let c = world.get::<Canvas>(e).ok_or_else(|| format!("“{name}” is not a canvas"))?;
        m.insert(name.to_string(), c.view_json(*id));
    }
    Ok(Value::Object(m).to_string())
}

/// A canvas by id or name.
pub fn find(world: &World, what: &str) -> Option<(StableId, Canvas)> {
    let index = world.resource::<IdIndex>();
    index
        .iter()
        .filter_map(|(id, e)| world.get::<Canvas>(e).map(|c| (id, c.clone())))
        .find(|(id, c)| id.to_string() == what || c.name == what)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(w: u32, h: u32) -> Canvas {
        let (plane, view) = view_plane("front").unwrap();
        Canvas {
            name: "Side".into(),
            image: String::new(),
            pixels: [w, h],
            plane,
            view: Some(view),
            scale: 0.5,
            aspect: 1.0,
            anchor: [w as f64 / 2.0, h as f64 / 2.0],
            rotation: 0.0,
            flip: false,
            perspective: None,
            calibration: None,
            crop: None,
            opacity: 0.5,
            visible: true,
            through: false,
            background: None,
            threshold: None,
            sized: true,
            matrix: None,
        }
    }

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    #[test]
    fn pixels_map_to_the_plane_and_back() {
        let mut c = canvas(200, 100);
        // The anchor lands on the origin; right is +x, up the picture is +y.
        assert!(close(apply(&c.to_plane(), [100.0, 50.0]), [0.0, 0.0]));
        assert!(close(apply(&c.to_plane(), [120.0, 30.0]), [10.0, 10.0]));
        c.rotation = 90.0;
        assert!(close(apply(&c.to_plane(), [120.0, 50.0]), [0.0, 10.0]));
        c.rotation = 0.0;
        c.flip = true;
        assert!(close(apply(&c.to_plane(), [120.0, 50.0]), [-10.0, 0.0]));
        c.aspect = 2.0;
        assert!(close(apply(&c.to_plane(), [100.0, 40.0]), [0.0, 10.0]));
        let back = apply(&c.to_pixel(), apply(&c.to_plane(), [33.0, 71.0]));
        assert!(close(back, [33.0, 71.0]));
        // On the front plane, plane y is world z.
        let w = c.pixel_to_world([100.0, 40.0]);
        assert!((w[2] - 10.0).abs() < 1e-9 && w[1].abs() < 1e-12);
    }

    #[test]
    fn two_points_and_a_distance_set_the_scale() {
        let mut c = canvas(1000, 500);
        let picture = Picture::blank(1000, 500);
        settle(&mut c, Settings { calibrate: Some(Calibration { a: [100.0, 200.0], b: [400.0, 600.0], distance: 250.0 }), ..Default::default() }, &picture).unwrap();
        // 300-400-500 px: 250 mm over 500 px.
        assert!((c.scale - 0.5).abs() < 1e-12, "{}", c.scale);
        // Kept and re-applied when the picture is stretched.
        settle(&mut c, Settings { aspect: Some(2.0), ..Default::default() }, &picture).unwrap();
        let d = 300.0f64.hypot(800.0);
        assert!((c.scale - 250.0 / d).abs() < 1e-12, "{}", c.scale);
        // A width sets it outright and drops the calibration.
        settle(&mut c, Settings { width: Some(100.0), ..Default::default() }, &picture).unwrap();
        assert!((c.scale - 0.1).abs() < 1e-12 && c.calibration.is_none());
    }

    #[test]
    fn level_turns_a_tilted_line_level() {
        let mut c = canvas(400, 400);
        let picture = Picture::blank(400, 400);
        // A line rising 10° to the right in the photo.
        let a = [100.0, 200.0];
        let t = 10f64.to_radians();
        let b = [100.0 + 200.0 * t.cos(), 200.0 - 200.0 * t.sin()];
        settle(&mut c, Settings { level: Some([a, b]), ..Default::default() }, &picture).unwrap();
        let (pa, pb) = (apply(&c.to_plane(), a), apply(&c.to_plane(), b));
        assert!((pa[1] - pb[1]).abs() < 1e-9, "{pa:?} {pb:?}");
        assert!((c.rotation + 10.0).abs() < 1e-9);
    }

    #[test]
    fn a_pivot_turns_the_plane_about_a_world_axis() {
        let mut c = canvas(100, 100);
        let picture = Picture::blank(100, 100);
        // The end view (from −X) of something standing nose-up by 10°: turn the plane about Y.
        let (plane, _) = view_plane("left").unwrap();
        c.plane = plane;
        settle(&mut c, Settings { pivot: Some(Pivot { axis: [0.0, 1.0, 0.0], degrees: -10.0 }), ..Default::default() }, &picture).unwrap();
        let n = c.plane.normal();
        let t = 10f64.to_radians();
        // Right-handed: −10° about +Y tips the −X normal down by 10°.
        assert!((n[0] + t.cos()).abs() < 1e-9 && (n[2] + t.sin()).abs() < 1e-9 && n[1].abs() < 1e-12, "{n:?}");
        // Still a unit, square frame; x untouched (it lies along the axis).
        assert!((c.plane.x[1] + 1.0).abs() < 1e-12);
    }

    #[test]
    fn four_points_square_up_a_photo() {
        let mut c = canvas(800, 600);
        // A rectangle seen at an angle: its far edge shorter.
        let pts = [[300.0, 100.0], [500.0, 100.0], [600.0, 500.0], [200.0, 500.0]];
        c.perspective = Some(Perspective { points: pts, width: 40.0, height: 160.0 });
        let m = c.to_plane();
        assert!(close(apply(&m, pts[0]), [-20.0, 80.0]));
        assert!(close(apply(&m, pts[2]), [20.0, -80.0]));
        // Lines stay lines: the middle of the near edge is the middle.
        let mid = apply(&m, [400.0, 500.0]);
        assert!(close(mid, [0.0, -80.0]), "{mid:?}");
        let back = apply(&c.to_pixel(), [5.0, 5.0]);
        assert!(close(apply(&m, back), [5.0, 5.0]));
    }

    /// Timing on a real photo: FORMA_PHOTO=<file.jpg> cargo test … -- --ignored trace_timing --nocapture
    #[test]
    #[ignore]
    fn trace_timing() {
        let Ok(path) = std::env::var("FORMA_PHOTO") else { return };
        crate::assets::set_dir(std::env::temp_dir().join(format!("forma-timing-{}", std::process::id())));
        let sha = crate::assets::store(&std::fs::read(path).unwrap()).unwrap();
        let p = crate::image::load(&sha).unwrap();
        let mut c = canvas(p.width as u32, p.height as u32);
        c.image = sha;
        c.scale = 0.2;
        let t = std::time::Instant::now();
        let ink = object_ink(&c, &p);
        eprintln!("object ink {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let o: TraceOptions = serde_json::from_str(r#"{"largest": true}"#).unwrap();
        let l = trace(&c, &o, 0.05).unwrap();
        eprintln!("trace {:?}: {} loops, {} points", t.elapsed(), l.len(), l.iter().map(|x| x.len()).sum::<usize>());
        let _ = ink;
    }

    #[test]
    fn spans_read_an_outline_with_a_hole() {
        // A 40 × 20 rectangle with a 10 × 10 hole in its middle.
        let outer = vec![[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [0.0, 20.0]];
        let hole = vec![[15.0, 5.0], [15.0, 15.0], [25.0, 15.0], [25.0, 5.0]];
        let loops = vec![outer, hole];
        assert_eq!(spans(&loops, 20.0, true), vec![[0.0, 5.0], [15.0, 20.0]]);
        assert_eq!(spans(&loops, 5.5, true), vec![[0.0, 20.0]]);
        assert_eq!(spans(&loops, 10.0, false), vec![[0.0, 15.0], [25.0, 40.0]]);
        assert!(spans(&loops, 50.0, true).is_empty());
    }

    #[test]
    fn a_program_names_its_canvases() {
        assert_eq!(named_in("const s = canvas(\"Side\"); const t = canvas('Top'); fromCanvas(\"x\"); canvas(\"Side\")"), vec!["Side".to_string(), "Top".to_string()]);
    }

    #[test]
    fn the_object_sets_size_and_origin() {
        // A dark 300 × 100 px bar on white, its left end at x = 50, bottom at y = 300.
        let mut px = vec![255u8; 600 * 400 * 4];
        for y in 200..300 {
            for x in 50..350 {
                let i = (y * 600 + x) * 4;
                px[i..i + 3].copy_from_slice(&[20, 20, 20]);
            }
        }
        let picture = crate::image::decode(&crate::image::encode_png(600, 400, &px)).unwrap();
        let mut c = canvas(600, 400);
        settle(&mut c, Settings { object: Some(ObjectSize { width: Some(150.0), height: None, align: Some("left bottom".into()) }), ..Default::default() }, &picture).unwrap();
        assert!((c.scale - 0.5).abs() < 0.01, "{}", c.scale);
        let p = apply(&c.to_plane(), [50.0, 300.0]);
        assert!(p[0].abs() < 0.6 && p[1].abs() < 0.6, "{p:?}");
        let q = apply(&c.to_plane(), [350.0, 200.0]);
        assert!((q[0] - 150.0).abs() < 1.0 && (q[1] - 50.0).abs() < 1.0, "{q:?}");
        // Both sizes stretch it.
        settle(&mut c, Settings { object: Some(ObjectSize { width: Some(150.0), height: Some(100.0), align: Some("left bottom".into()) }), ..Default::default() }, &picture).unwrap();
        let q = apply(&c.to_plane(), [350.0, 200.0]);
        assert!((q[1] - 100.0).abs() < 1.5 && (c.aspect - 2.0).abs() < 0.05, "{q:?} {}", c.aspect);
    }
}
