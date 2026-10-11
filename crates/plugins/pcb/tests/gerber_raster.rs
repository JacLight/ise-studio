//! An independent check of the copper Gerbers: the files are read back with
//! the strict reader, every flash, draw and region is painted into a raster
//! (0.02 mm pixels) the way a photoplotter would expose it, and the exposed
//! area must match the board's own copper (pads, tracks, vias and pour fills
//! unioned exactly with manifold) within 1 %. A missing or misplaced pad,
//! a wrong aperture, a region with a broken cut-in or a lost pour shows up as
//! an area difference.

use de_core::{Actor, Envelope, Project, Registry, SCHEMA_VERSION, StableId, Value};
use de_pcb::geom::{self, P, Prim};
use de_pcb::reader::{self, Aperture};
use de_pcb::PcbPlugin;
use de_shape::ShapePlugin;
use de_solid::SolidPlugin;
use manifold_csg::CrossSection;
use std::sync::Arc;

fn subst(v: &Value, outputs: &[Value]) -> Value {
    match v {
        Value::String(s) if s.starts_with('$') => {
            let (n, path) = s[1..].split_once('.').unwrap();
            outputs[n.parse::<usize>().unwrap()][path].clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| subst(x, outputs)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), subst(x, outputs))).collect()),
        other => other.clone(),
    }
}

struct Raster {
    x0: f64,
    y0: f64,
    px: f64,
    w: usize,
    h: usize,
    on: Vec<bool>,
}

impl Raster {
    fn new(b: [f64; 4], px: f64) -> Raster {
        let (x0, y0) = (b[0] - 1.0, b[1] - 1.0);
        let w = ((b[2] - b[0] + 2.0) / px).ceil() as usize;
        let h = ((b[3] - b[1] + 2.0) / px).ceil() as usize;
        Raster { x0, y0, px, w, h, on: vec![false; w * h] }
    }
    fn centre(&self, i: usize, j: usize) -> P {
        [self.x0 + (i as f64 + 0.5) * self.px, self.y0 + (j as f64 + 0.5) * self.px]
    }
    fn span(&self, lo: f64, hi: f64, n: usize, o: f64) -> (usize, usize) {
        let a = (((lo - o) / self.px) - 0.5).floor().max(0.0) as usize;
        let b = ((((hi - o) / self.px) + 0.5).ceil() as usize).min(n - 1);
        (a, b)
    }
    /// Paints a rounded convex.
    fn prim(&mut self, p: &Prim) {
        let hull = geom::convex_hull(&p.pts);
        let b = p.bbox();
        let (i0, i1) = self.span(b[0], b[2], self.w, self.x0);
        let (j0, j1) = self.span(b[1], b[3], self.h, self.y0);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let q = self.centre(i, j);
                let inside = (hull.len() >= 3 && geom::in_convex(q, &hull))
                    || match hull.len() {
                        1 => geom::dist(q, hull[0]) <= p.r,
                        2 => geom::point_seg_dist(q, hull[0], hull[1]) <= p.r,
                        _ => geom::dist_to_loop(q, &hull) <= p.r,
                    };
                if inside {
                    self.on[j * self.w + i] = true;
                }
            }
        }
    }
    /// Paints a region (one contour, cut-ins included) by non-zero winding, scanline by scanline.
    fn region(&mut self, c: &[P]) {
        let (ymin, ymax) = c.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |m, p| (m.0.min(p[1]), m.1.max(p[1])));
        let (j0, j1) = self.span(ymin, ymax, self.h, self.y0);
        let n = c.len();
        for j in j0..=j1 {
            let y = self.centre(0, j)[1];
            let mut xs: Vec<(f64, i32)> = Vec::new();
            for k in 0..n {
                let (a, b) = (c[k], c[(k + 1) % n]);
                if (a[1] <= y) != (b[1] <= y) {
                    let x = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
                    xs.push((x, if b[1] > a[1] { 1 } else { -1 }));
                }
            }
            xs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut wind = 0;
            for k in 0..xs.len() {
                wind += xs[k].1;
                if wind != 0 && k + 1 < xs.len() {
                    let (lo, hi) = (xs[k].0, xs[k + 1].0);
                    for i in 0..self.w {
                        let x = self.centre(i, 0)[0];
                        if x >= lo && x < hi {
                            self.on[j * self.w + i] = true;
                        }
                    }
                }
            }
        }
    }
    fn area(&self) -> f64 {
        self.on.iter().filter(|v| **v).count() as f64 * self.px * self.px
    }
}

fn aperture_prim(a: &Aperture, at: P) -> Prim {
    match a {
        Aperture::Circle(d) => Prim::circle(at, d / 2.0),
        Aperture::Rect(w, h) => Prim::rect(at, *w, *h, 0.0, 0.0),
        Aperture::Obround(w, h) => Prim::rect(at, *w, *h, 0.0, w.min(*h) / 2.0),
        Aperture::Macro(name, p) if name == "RoundRect" => Prim { pts: (0..4).map(|k| [at[0] + p[1 + 2 * k], at[1] + p[2 + 2 * k]]).collect(), r: p[0] },
        Aperture::Macro(name, _) => panic!("unknown macro {name}"),
    }
}

#[test]
fn copper_gerbers_expose_the_boards_copper() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/pcb-esp32");
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(root.join("steps.json")).unwrap()).unwrap();
    let mut p = Project::new("p", Arc::new(Registry::with_plugins(&[&SolidPlugin, &ShapePlugin, &PcbPlugin])));
    let mut outs = Vec::new();
    for s in &steps {
        let env = Envelope { schema_version: SCHEMA_VERSION, command_id: StableId::new().to_string(), project_id: "p".into(), expected_revision: p.revision(), operation: s["operation"].as_str().unwrap().into(), parameters: subst(&s["parameters"], &outs), actor: Actor { user_id: "u".into(), session_id: "s".into() } };
        outs.push(p.commit(env).unwrap().output);
    }
    let board: StableId = serde_json::from_value(outs[0]["id"].clone()).unwrap();
    let v = de_pcb::view(p.world(), board).unwrap();
    for layer in v.layers.clone() {
        // The board's own copper on this layer, unioned exactly.
        let mut parts: Vec<CrossSection> = Vec::new();
        for pad in v.pads.iter().filter(|q| q.layers.contains(&layer)) {
            parts.push(pad.prim.section(0.0, 0.002));
        }
        for t in v.tracks.iter().filter(|t| t.layer == layer) {
            for w in t.points.windows(2) {
                parts.push(Prim::capsule(w[0], w[1], t.width / 2.0).section(0.0, 0.002));
            }
        }
        for via in &v.vias {
            parts.push(Prim::circle(via.at, via.diameter / 2.0).section(0.0, 0.002));
        }
        for z in v.zones.iter().filter(|z| z.layer == layer) {
            for island in &z.fill {
                parts.push(CrossSection::from_polygons(island));
            }
        }
        let want = CrossSection::batch_union(&parts).area();

        // What the Gerber exposes.
        let text = de_pcb::gerber::copper(&v, &layer);
        let g = reader::gerber(&text).unwrap_or_else(|e| panic!("{layer}: {e}"));
        let mut r = Raster::new(v.bounds, 0.02);
        for (d, at) in &g.flashes {
            r.prim(&aperture_prim(&g.apertures[d], *at));
        }
        for (d, a, b) in &g.draws {
            let Aperture::Circle(w) = g.apertures[d] else { panic!("draws use round apertures") };
            r.prim(&Prim::capsule(*a, *b, w / 2.0));
        }
        for c in &g.regions {
            r.region(c);
        }
        let got = r.area();
        let diff = (got - want).abs() / want;
        eprintln!("{layer}: board copper {want:.2} mm², Gerber exposes {got:.2} mm² ({:.3} %)", diff * 100.0);
        assert!(want > 50.0, "{layer} has copper");
        assert!(diff < 0.01, "{layer}: Gerber {got:.2} mm² vs board {want:.2} mm²");
        // Every pad, however small, is exposed where it is (a lost 0402 pad is below 1 % of the area).
        for pad in v.pads.iter().filter(|q| q.layers.contains(&layer)) {
            let mut only = Raster::new(v.bounds, 0.02);
            only.prim(&pad.prim);
            let (mut inside, mut lit) = (0usize, 0usize);
            for (k, on) in only.on.iter().enumerate() {
                if *on {
                    inside += 1;
                    lit += r.on[k] as usize;
                }
            }
            assert!(inside > 0 && lit as f64 >= inside as f64 * 0.99, "{layer}: {}.{} exposed {lit}/{inside} px", pad.reference, pad.number);
        }
    }
}
