//! Architectural drawings of a building model: a plan of each level, cut a
//! little above its floor; vertical sections; and elevations of each side.
//!
//! Every element's solid is turned into the view's frame (the viewer up
//! +Z), cut by the view's plane (the cut filled solid black for walls,
//! columns, slabs, roofs, beams and stairs; outlined for doors, windows,
//! furniture) and what lies beyond is projected and drawn nearest first,
//! each outline less what is already covered, so hidden edges never show.
//! Plans add door swings, window sills and stair treads (the elements' plan
//! marks), room names with their areas, grid lines with bubbles, overall and
//! grid dimensions and a north arrow; sections and elevations add level
//! datums and grid lines. Sheet, scale and title block as engineering
//! drawings have; written out with `drawing::pdf_pages` and `drawing::dxf`.

use crate::drawing::{Anchor, Drawing, Pen, Stroke, Text};
use de_bim::{geom, Model};
use manifold_csg::{CrossSection, Manifold};
use serde::{Deserialize, Serialize};

/// Which view of the building.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    /// A level's plan, cut `cut` mm above its floor (default 1200).
    Plan { level: String, cut: Option<f64> },
    /// A vertical section through the plane x = `at` (axis "x") or y = `at`
    /// (axis "y"), looking `looking` ("north", "south", "east", "west").
    Section { axis: String, at: f64, looking: Option<String> },
    /// The building seen from one side: "south" (looking north), "north", "east", "west".
    Elevation { side: String },
}

impl View {
    pub fn title(&self) -> String {
        match self {
            View::Plan { level, .. } => format!("{level} plan"),
            View::Section { axis, at, looking } => format!("Section {} = {} looking {}", axis.to_uppercase(), de_bim::num_text(*at), looking.clone().unwrap_or_else(|| default_looking(axis).into())),
            View::Elevation { side } => format!("{} elevation", capital(side)),
        }
    }
}

fn default_looking(axis: &str) -> &'static str {
    if axis == "x" { "east" } else { "north" }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// What the title block says.
#[derive(Clone, Debug, Default)]
pub struct Title {
    pub project: String,
    pub number: String,
    pub drawn_by: String,
    pub date: String,
}

#[derive(Clone, Copy, Debug)]
pub struct Sheet {
    pub width: f64,
    pub height: f64,
    pub name: &'static str,
}

impl Sheet {
    pub fn named(name: &str) -> Sheet {
        match name {
            "A4" => Sheet { width: 297.0, height: 210.0, name: "A4" },
            "A2" => Sheet { width: 594.0, height: 420.0, name: "A2" },
            "A1" => Sheet { width: 841.0, height: 594.0, name: "A1" },
            "A0" => Sheet { width: 1189.0, height: 841.0, name: "A0" },
            _ => Sheet { width: 420.0, height: 297.0, name: "A3" },
        }
    }
}

/// World → view: p' = R p, the viewer looking down −Z'. Rows of R.
fn rotation(view: &View) -> Result<([[f64; 3]; 3], Option<f64>), String> {
    // (screen x, screen y, toward the viewer)
    let looking = |dir: &str| -> Result<[[f64; 3]; 3], String> {
        Ok(match dir {
            "north" => [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]],
            "south" => [[-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
            "east" => [[0.0, -1.0, 0.0], [0.0, 0.0, 1.0], [-1.0, 0.0, 0.0]],
            "west" => [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            other => return Err(format!("looking is north, south, east or west, not “{other}”")),
        })
    };
    match view {
        View::Plan { .. } => Ok(([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], None)),
        View::Section { axis, at, looking: l } => {
            let dir = l.as_deref().unwrap_or(default_looking(axis));
            let r = looking(dir)?;
            match (axis.as_str(), dir) {
                ("x", "east" | "west") | ("y", "north" | "south") => {}
                ("x" | "y", _) => return Err(format!("a section on {axis} looks {}", if axis == "x" { "east or west" } else { "north or south" })),
                _ => return Err("a section's axis is x or y".into()),
            }
            // The cut plane in view depth: z' = R · (point on the plane).
            let p = if axis == "x" { [*at, 0.0, 0.0] } else { [0.0, *at, 0.0] };
            let zc = r[2][0] * p[0] + r[2][1] * p[1] + r[2][2] * p[2];
            Ok((r, Some(zc)))
        }
        View::Elevation { side } => {
            // Seen from the south means looking north.
            let dir = match side.as_str() {
                "south" => "north",
                "north" => "south",
                "east" => "west",
                "west" => "east",
                other => return Err(format!("an elevation is of the north, south, east or west side, not “{other}”")),
            };
            Ok((looking(dir)?, None))
        }
    }
}

fn turn(r: &[[f64; 3]; 3], p: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| r[i][0] * p[0] + r[i][1] * p[1] + r[i][2] * p[2])
}

/// Stairs and ramps: drawn on the floor they arrive at too.
fn arrives(class: &str) -> bool {
    matches!(class, "IfcStair" | "IfcStairFlight" | "IfcRamp" | "IfcRampFlight")
}

/// Classes whose cut is filled solid (structure); others are outlined.
fn poche(class: &str) -> bool {
    matches!(
        class,
        "IfcWall" | "IfcWallStandardCase" | "IfcWallElementedCase" | "IfcColumn" | "IfcColumnStandardCase" | "IfcSlab" | "IfcSlabStandardCase" | "IfcRoof" | "IfcBeam" | "IfcBeamStandardCase" | "IfcFooting" | "IfcStair" | "IfcStairFlight" | "IfcRamp" | "IfcRampFlight" | "IfcChimney" | "IfcPile"
    )
}

struct Item {
    class: String,
    cut: Option<CrossSection>,
    beyond: Option<CrossSection>,
    near: f64,
}

/// Standard architectural scales, drawing : real.
const SCALES: &[f64] = &[1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0];

/// One view of the model on a sheet.
pub fn draw(model: &Model, view: &View, sheet: Sheet, title: &Title, scale: Option<f64>) -> Result<Drawing, String> {
    let (r, mut cut) = rotation(view)?;
    let m12 = [r[0][0], r[1][0], r[2][0], r[0][1], r[1][1], r[2][1], r[0][2], r[1][2], r[2][2], 0.0, 0.0, 0.0];
    let mut level_name = None;
    let mut floor = f64::NEG_INFINITY;
    if let View::Plan { level, cut: c } = view {
        let l = model.levels.iter().find(|l| l.name.eq_ignore_ascii_case(level)).ok_or_else(|| {
            format!("there is no level “{level}” (levels: {})", model.levels.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", "))
        })?;
        floor = l.elevation;
        cut = Some(l.elevation + c.unwrap_or(1200.0));
        level_name = Some(l.name.clone());
    }
    // Every element, turned, cut and projected.
    let mut items: Vec<Item> = Vec::new();
    for e in &model.elements {
        if e.mesh.triangles.is_empty() {
            continue;
        }
        let (lo, hi) = geom::bounds(&e.mesh);
        if let View::Plan { .. } = view {
            // Above the cut, or the floor itself and below: not on this plan.
            // A stair from below arriving at this floor shows through its well.
            let arriving = arrives(&e.info.class) && (hi[2] - floor).abs() <= 50.0;
            if (hi[2] <= floor + 1.0 && !arriving) || lo[2] >= cut.unwrap() {
                continue;
            }
        }
        let pos: Vec<f64> = e.mesh.positions.iter().flat_map(|p| p.iter().copied()).collect();
        let tri: Vec<u64> = e.mesh.triangles.iter().flat_map(|t| t.iter().map(|i| *i as u64)).collect();
        let Ok(solid) = Manifold::from_mesh_f64(&pos, 3, &tri) else { continue };
        let s = solid.transform(&m12);
        let Some(b) = s.bounding_box() else { continue };
        let (bmin, bmax) = (b.min(), b.max());
        let (cs, beyond, near) = match cut {
            Some(z) if bmin[2] < z && bmax[2] > z => {
                let c = s.slice_to_cross_section(z);
                let rest = s.trim_by_plane([0.0, 0.0, -1.0], -z);
                let proj = CrossSection::from_polygons(&rest.project());
                (Some(c), Some(proj), z)
            }
            Some(z) if bmin[2] >= z => continue, // in front of the cut: removed
            _ => (None, Some(CrossSection::from_polygons(&s.project())), bmax[2]),
        };
        items.push(Item { class: e.info.class.clone(), cut: cs.filter(|c| !c.is_empty()), beyond: beyond.filter(|c| !c.is_empty()), near });
    }
    if items.is_empty() {
        return Err(match &level_name {
            Some(l) => format!("nothing stands on {l}"),
            None => "there is nothing to draw".into(),
        });
    }
    // Extent of what is drawn, in view coordinates.
    let mut ext = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    fn grow(ext: &mut [f64; 4], p: [f64; 2]) {
        ext[0] = ext[0].min(p[0]);
        ext[1] = ext[1].min(p[1]);
        ext[2] = ext[2].max(p[0]);
        ext[3] = ext[3].max(p[1]);
    }
    for it in &items {
        for c in it.cut.iter().chain(it.beyond.iter()) {
            let rb = c.bounds();
            let (a, b) = (rb.min(), rb.max());
            grow(&mut ext, [a[0], a[1]]);
            grow(&mut ext, [b[0], b[1]]);
        }
    }
    // Grid lines in this view: in plan as drawn; elsewhere the ones seen end on.
    let mut grid_lines: Vec<(String, [f64; 2], [f64; 2])> = Vec::new();
    for g in &model.grids {
        if level_name.is_some() && g.level.as_ref().is_some_and(|l| Some(l) != level_name.as_ref()) && model.grids.len() > 1 {
            continue;
        }
        for a in g.u.iter().chain(g.v.iter()) {
            let p = turn(&r, [a.from[0], a.from[1], 0.0]);
            let q = turn(&r, [a.to[0], a.to[1], 0.0]);
            if level_name.is_some() {
                grid_lines.push((a.tag.clone(), [p[0], p[1]], [q[0], q[1]]));
            } else if (p[0] - q[0]).abs() < 1.0 {
                grid_lines.push((a.tag.clone(), [p[0], ext[1]], [p[0], ext[3]]));
            }
        }
    }
    let plan = level_name.is_some();
    // Plans: the grid's extent counts; sections: the grid spans the drawing.
    if plan {
        for (_, a, b) in &grid_lines {
            grow(&mut ext, *a);
            grow(&mut ext, *b);
        }
    }
    // Scale: the largest standard one that fits the drawing area with room for dimensions.
    let margin = 10.0;
    let block = [120.0, 32.0];
    let room = [sheet.width - 2.0 * margin - 50.0, sheet.height - 2.0 * margin - block[1] - 40.0];
    let size = [(ext[2] - ext[0]).max(1.0), (ext[3] - ext[1]).max(1.0)];
    let fit = SCALES.iter().copied().find(|s| size[0] / s <= room[0] && size[1] / s <= room[1]).unwrap_or(5000.0);
    let denom = scale.filter(|s| *s > 0.0).unwrap_or(fit);
    let k = 1.0 / denom;
    let centre = [margin + 25.0 + room[0] / 2.0, margin + block[1] + 20.0 + room[1] / 2.0];
    let mid = [(ext[0] + ext[2]) / 2.0, (ext[1] + ext[3]) / 2.0];
    let to = |p: [f64; 2]| [centre[0] + (p[0] - mid[0]) * k, centre[1] + (p[1] - mid[1]) * k];
    let mut d = Drawing::blank(sheet.width, sheet.height);
    d.scale = k;
    d.scale_text = format!("1:{}", de_bim::num_text(denom));
    if scale.is_some_and(|s| s > 0.0) && denom > fit {
        // fine: smaller than needed
    } else if scale.is_some_and(|s| s > 0.0) && denom < fit {
        d.warnings.push(format!("at 1:{} the drawing runs off the sheet (1:{} fits)", de_bim::num_text(denom), de_bim::num_text(fit)));
    }

    // Cut regions first: solid for structure, outlined for the rest.
    let mut covered = CrossSection::empty();
    for it in &items {
        let Some(c) = &it.cut else { continue };
        let polys = c.to_polygons();
        if poche(&it.class) {
            fill(&mut d, &polys, &to);
        }
        for l in &polys {
            outline(&mut d, l, &to, if poche(&it.class) { Pen::Visible } else { Pen::Thin });
        }
        covered = covered.union(c);
    }
    // Then what lies beyond, nearest first, less what is in front of it.
    let mut order: Vec<&Item> = items.iter().filter(|i| i.beyond.is_some()).collect();
    order.sort_by(|a, b| b.near.total_cmp(&a.near));
    for it in order {
        let p = it.beyond.as_ref().unwrap();
        let vis = p.difference(&covered);
        for l in vis.to_polygons() {
            outline(&mut d, &l, &to, Pen::Thin);
        }
        covered = covered.union(p);
    }

    // Plan marks: door swings, sills, treads.
    if let Some(level) = &level_name {
        for e in &model.elements {
            let (lo, hi) = geom::bounds(&e.mesh);
            let arriving = arrives(&e.info.class) && (hi[2] - floor).abs() <= 50.0 && lo[2] < floor;
            if e.info.level.as_ref() != Some(level) && !arriving {
                continue;
            }
            for m in &e.info.plan {
                let pts: Vec<[f64; 2]> = m.points.iter().map(|p| to(*p)).collect();
                d.strokes.push(Stroke { pen: if m.dashed && !arriving { Pen::Hidden } else { Pen::Thin }, points: pts });
            }
        }
        // Rooms: name, then area.
        for s in model.spaces.iter().filter(|s| s.level.as_ref() == Some(level)) {
            if s.outline.is_empty() || s.outline[0].len() < 3 {
                continue;
            }
            let at = to(geom::label_point(&s.outline[0], &s.outline[1..]));
            d.texts.push(Text { at: [at[0], at[1] + 1.0], size: 2.5, text: s.long_name.clone().unwrap_or_else(|| s.name.clone()).to_uppercase(), anchor: Anchor::Middle, angle: 0.0, bold: true });
            d.texts.push(Text { at: [at[0], at[1] - 3.5], size: 2.0, text: format!("{:.1} m²", s.area()), anchor: Anchor::Middle, angle: 0.0, bold: false });
        }
    }

    // Grid lines and their bubbles.
    let bubble = 4.0;
    for (tag, a, b) in &grid_lines {
        let (pa, pb) = (to(*a), to(*b));
        d.strokes.push(Stroke { pen: Pen::Center, points: vec![pa, pb] });
        let dir = unit2([pb[0] - pa[0], pb[1] - pa[1]]);
        let ends: Vec<[f64; 2]> = if plan { vec![pa, pb] } else { vec![pb] };
        for (i, e) in ends.iter().enumerate() {
            let s = if i == 0 && plan { -1.0 } else { 1.0 };
            let c = [e[0] + dir[0] * bubble * s, e[1] + dir[1] * bubble * s];
            circle(&mut d, c, bubble);
            d.texts.push(Text { at: [c[0], c[1] - 1.25], size: 2.5, text: tag.clone(), anchor: Anchor::Middle, angle: 0.0, bold: true });
        }
    }

    // Dimensions.
    let drawn = cut_extent(&items).unwrap_or(ext);
    if plan {
        // Outside the grid's bubbles: the grid chain nearer, the overall dimension beyond it.
        let clear = if grid_lines.is_empty() { 8.0 } else { 2.0 * bubble + 6.0 };
        let below = to([ext[0], ext[1]])[1] - clear;
        let left = to([ext[0], ext[1]])[0] - clear;
        let mut xs: Vec<f64> = grid_lines.iter().filter(|(_, a, b)| (a[0] - b[0]).abs() < 1.0).map(|(_, a, _)| a[0]).collect();
        let mut ys: Vec<f64> = grid_lines.iter().filter(|(_, a, b)| (a[1] - b[1]).abs() < 1.0).map(|(_, a, _)| a[1]).collect();
        xs.sort_by(f64::total_cmp);
        ys.sort_by(f64::total_cmp);
        let (h_all, v_all) = (if xs.len() > 1 { below - 8.0 } else { below }, if ys.len() > 1 { left - 8.0 } else { left });
        for w in xs.windows(2) {
            dim_h(&mut d, to([w[0], 0.0])[0], to([w[1], 0.0])[0], to([0.0, drawn[1]])[1], below, w[1] - w[0]);
        }
        for w in ys.windows(2) {
            dim_v(&mut d, to([0.0, w[0]])[1], to([0.0, w[1]])[1], to([drawn[0], 0.0])[0], left, w[1] - w[0]);
        }
        dim_h(&mut d, to([drawn[0], 0.0])[0], to([drawn[2], 0.0])[0], to([0.0, drawn[1]])[1], h_all, drawn[2] - drawn[0]);
        dim_v(&mut d, to([0.0, drawn[1]])[1], to([0.0, drawn[3]])[1], to([drawn[0], 0.0])[0], v_all, drawn[3] - drawn[1]);
        // North arrow.
        let north = model.site.north.unwrap_or(0.0).to_radians();
        let c = [sheet.width - margin - 15.0, sheet.height - margin - 15.0];
        circle(&mut d, c, 6.0);
        let dir = [-north.sin(), north.cos()];
        let tip = [c[0] + dir[0] * 6.0, c[1] + dir[1] * 6.0];
        let side = [-dir[1], dir[0]];
        d.fills.push(vec![tip, [c[0] - dir[0] * 4.0 + side[0] * 2.5, c[1] - dir[1] * 4.0 + side[1] * 2.5], [c[0] - dir[0] * 2.0, c[1] - dir[1] * 2.0]]);
        d.texts.push(Text { at: [tip[0] + dir[0] * 3.0, tip[1] + dir[1] * 3.0 - 1.0], size: 2.5, text: "N".into(), anchor: Anchor::Middle, angle: 0.0, bold: true });
    } else {
        // Level datums: a line across with the level's name and height.
        let x0 = to([ext[0], 0.0])[0] - 6.0;
        let x1 = to([ext[2], 0.0])[0] + 6.0;
        for l in &model.levels {
            let y = to([0.0, l.elevation])[1];
            d.strokes.push(Stroke { pen: Pen::Center, points: vec![[x0, y], [x1 + 18.0, y]] });
            d.fills.push(vec![[x1 + 2.0, y], [x1 - 0.5, y + 3.0], [x1 + 4.5, y + 3.0]]);
            d.texts.push(Text { at: [x1 + 6.0, y + 1.0], size: 2.0, text: format!("{:+.3}", l.elevation / 1000.0), anchor: Anchor::Start, angle: 0.0, bold: true });
            d.texts.push(Text { at: [x1 + 6.0, y - 3.0], size: 2.0, text: l.name.clone(), anchor: Anchor::Start, angle: 0.0, bold: false });
        }
        // Storey heights up the left.
        let mut zs: Vec<f64> = model.levels.iter().map(|l| l.elevation).collect();
        zs.sort_by(f64::total_cmp);
        for w in zs.windows(2) {
            dim_v(&mut d, to([0.0, w[0]])[1], to([0.0, w[1]])[1], x0, x0 - 8.0, w[1] - w[0]);
        }
        if zs.last().is_some_and(|z| ext[3] > z + 1.0) {
            dim_v(&mut d, to([0.0, *zs.last().unwrap()])[1], to([0.0, ext[3]])[1], x0, x0 - 8.0, ext[3] - zs.last().unwrap());
        }
        let below = to([0.0, ext[1]])[1] - 12.0;
        dim_h(&mut d, to([ext[0], 0.0])[0], to([ext[2], 0.0])[0], to([0.0, ext[1]])[1], below, ext[2] - ext[0]);
    }

    // Frame and title block.
    d.strokes.push(Stroke { pen: Pen::Frame, points: vec![[margin, margin], [sheet.width - margin, margin], [sheet.width - margin, sheet.height - margin], [margin, sheet.height - margin], [margin, margin]] });
    let bx = sheet.width - margin - block[0];
    let by = margin;
    d.strokes.push(Stroke { pen: Pen::Visible, points: vec![[bx, by], [bx, by + block[1]], [bx + block[0], by + block[1]]] });
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[bx, by + 20.0], [bx + block[0], by + 20.0]] });
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[bx, by + 10.0], [bx + block[0], by + 10.0]] });
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[bx + 60.0, by], [bx + 60.0, by + 10.0]] });
    let building = if model.building.name.is_empty() { model.project.clone() } else { model.building.name.clone() };
    d.texts.push(Text { at: [bx + 3.0, by + 25.0], size: 3.5, text: view.title().to_uppercase(), anchor: Anchor::Start, angle: 0.0, bold: true });
    d.texts.push(Text { at: [bx + 3.0, by + 13.5], size: 2.5, text: format!("{building} · {}", title.project), anchor: Anchor::Start, angle: 0.0, bold: false });
    d.texts.push(Text { at: [bx + 3.0, by + 3.5], size: 2.0, text: format!("Scale {} @ {} · {}", d.scale_text, sheet.name, title.number), anchor: Anchor::Start, angle: 0.0, bold: false });
    d.texts.push(Text { at: [bx + 63.0, by + 3.5], size: 2.0, text: format!("{} · {}", title.drawn_by, title.date), anchor: Anchor::Start, angle: 0.0, bold: false });
    Ok(d)
}

fn unit2(v: [f64; 2]) -> [f64; 2] {
    let n = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-12);
    [v[0] / n, v[1] / n]
}

/// The extent of the cut (the building's footprint on a plan), if anything was cut.
fn cut_extent(items: &[Item]) -> Option<[f64; 4]> {
    let mut e = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for c in items.iter().filter(|i| poche(&i.class)).filter_map(|i| i.cut.as_ref()) {
        let b = c.bounds();
        e = [e[0].min(b.min()[0]), e[1].min(b.min()[1]), e[2].max(b.max()[0]), e[3].max(b.max()[1])];
    }
    e[0].is_finite().then_some(e)
}

fn outline(d: &mut Drawing, l: &[[f64; 2]], to: &dyn Fn([f64; 2]) -> [f64; 2], pen: Pen) {
    if l.len() < 2 {
        return;
    }
    let mut pts: Vec<[f64; 2]> = l.iter().map(|p| to(*p)).collect();
    pts.push(pts[0]);
    d.strokes.push(Stroke { pen, points: pts });
}

/// Fills a region (outer loops anticlockwise, holes clockwise) with triangles.
fn fill(d: &mut Drawing, polys: &[Vec<[f64; 2]>], to: &dyn Fn([f64; 2]) -> [f64; 2]) {
    let mut outers: Vec<&Vec<[f64; 2]>> = polys.iter().filter(|p| geom::area2(p) > 0.0).collect();
    let holes: Vec<&Vec<[f64; 2]>> = polys.iter().filter(|p| geom::area2(p) < 0.0).collect();
    outers.sort_by(|a, b| geom::area2(a).total_cmp(&geom::area2(b)));
    let mut used = vec![false; holes.len()];
    for o in outers {
        // Holes inside this outer loop (smallest outers claim first).
        let mut mine: Vec<Vec<[f64; 2]>> = Vec::new();
        for (i, h) in holes.iter().enumerate() {
            if !used[i] && inside(h[0], o) {
                used[i] = true;
                mine.push((*h).clone());
            }
        }
        let pts: Vec<[f64; 2]> = o.iter().chain(mine.iter().flatten()).copied().collect();
        for t in geom::triangulate(o, &mine) {
            d.fills.push(vec![to(pts[t[0]]), to(pts[t[1]]), to(pts[t[2]])]);
        }
    }
}

fn inside(p: [f64; 2], poly: &[[f64; 2]]) -> bool {
    let mut c = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
            c = !c;
        }
    }
    c
}

fn circle(d: &mut Drawing, c: [f64; 2], r: f64) {
    let pts = (0..=32).map(|i| {
        let t = std::f64::consts::TAU * i as f64 / 32.0;
        [c[0] + r * t.cos(), c[1] + r * t.sin()]
    });
    d.strokes.push(Stroke { pen: Pen::Thin, points: pts.collect() });
}

/// A horizontal dimension: extension lines from `from_y` to the line at `y`, ticks, the value above.
fn dim_h(d: &mut Drawing, x0: f64, x1: f64, from_y: f64, y: f64, value: f64) {
    if (x1 - x0).abs() < 0.5 {
        return;
    }
    let s = if y < from_y { -1.0 } else { 1.0 };
    for x in [x0, x1] {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x, from_y + s * 2.0], [x, y + s * 2.0]] });
        d.strokes.push(Stroke { pen: Pen::Visible, points: vec![[x - 1.0, y - 1.0], [x + 1.0, y + 1.0]] });
    }
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x0 - 2.0, y], [x1 + 2.0, y]] });
    d.texts.push(Text { at: [(x0 + x1) / 2.0, y + 1.0], size: 2.0, text: format!("{:.0}", value), anchor: Anchor::Middle, angle: 0.0, bold: false });
}

/// A vertical dimension, its value reading up the line.
fn dim_v(d: &mut Drawing, y0: f64, y1: f64, from_x: f64, x: f64, value: f64) {
    if (y1 - y0).abs() < 0.5 {
        return;
    }
    let s = if x < from_x { -1.0 } else { 1.0 };
    for y in [y0, y1] {
        d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[from_x + s * 2.0, y], [x + s * 2.0, y]] });
        d.strokes.push(Stroke { pen: Pen::Visible, points: vec![[x - 1.0, y - 1.0], [x + 1.0, y + 1.0]] });
    }
    d.strokes.push(Stroke { pen: Pen::Thin, points: vec![[x, y0 - 2.0], [x, y1 + 2.0]] });
    d.texts.push(Text { at: [x - 1.0, (y0 + y1) / 2.0], size: 2.0, text: format!("{:.0}", value), anchor: Anchor::Middle, angle: 90.0, bold: false });
}

#[cfg(test)]
mod tests {
    use super::*;
    use de_bim::{Element, Level, Mesh, ModelElement};

    fn block(name: &str, class: &str, lo: [f64; 3], hi: [f64; 3]) -> ModelElement {
        let m = geom::extrude(&[[lo[0], lo[1]], [hi[0], lo[1]], [hi[0], hi[1]], [lo[0], hi[1]]], &[], [0.0, 0.0, hi[2] - lo[2]]);
        let mut m: Mesh = geom::solid(&m, 1e-9).unwrap();
        for p in &mut m.positions {
            p[2] += lo[2];
        }
        ModelElement { name: name.into(), key: name.into(), info: Element { class: class.into(), level: Some("L0".into()), ..Default::default() }, material: None, process: None, volume: 0.0, mesh: m }
    }

    #[test]
    fn plan_cuts_walls_and_skips_the_floor() {
        let model = Model {
            project: "t".into(),
            levels: vec![Level { name: "L0".into(), elevation: 0.0, height: Some(3000.0) }],
            elements: vec![block("Wall", "IfcWall", [0.0, 0.0, 0.0], [5000.0, 200.0, 2700.0]), block("Floor", "IfcSlab", [0.0, 0.0, -200.0], [5000.0, 4000.0, 0.0]), block("Desk", "IfcFurniture", [1000.0, 1000.0, 0.0], [2000.0, 1600.0, 750.0])],
            ..Default::default()
        };
        let d = draw(&model, &View::Plan { level: "L0".into(), cut: None }, Sheet::named("A3"), &Title::default(), None).unwrap();
        assert_eq!(d.scale_text, "1:20");
        // The wall is cut (filled); the desk below the cut is outlined; the floor is not drawn.
        assert!(!d.fills.is_empty());
        let thin = d.strokes.iter().filter(|s| s.pen == Pen::Thin && s.points.len() == 5).count();
        assert!(thin >= 1);
        let e = draw(&model, &View::Elevation { side: "south".into() }, Sheet::named("A3"), &Title::default(), None).unwrap();
        assert!(e.fills.len() < d.fills.len() + 4);
        let s = draw(&model, &View::Section { axis: "x".into(), at: 2500.0, looking: None }, Sheet::named("A3"), &Title::default(), None).unwrap();
        assert!(!s.fills.is_empty());
    }
}
