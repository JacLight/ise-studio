//! A study written as a CalculiX input deck (.inp), to cross-check Iṣẹ́ Studio's
//! answer in an independent solver: the same cubes as 8-node bricks with
//! incompatible modes (C3D8I, the element Iṣẹ́ Studio uses), the same holds and the
//! same forces on the nodes. Only the file format is written; nothing of
//! CalculiX is used here.

use super::*;

/// Which analysis the deck asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InpKind {
    Static,
    Frequency,
    Buckle,
}

/// The deck for a study. Partly filled cubes get a material whose stiffness
/// (and density) is scaled by how full they are, in tenths, as Iṣẹ́ Studio counts them.
pub fn calculix_inp(bodies: &[Body], study: &Study, kind: InpKind, count: usize) -> Result<String, String> {
    let s = Setup::build(bodies, study, kind != InpKind::Frequency)?;
    let m = &s.model;
    let mut out = String::new();
    let mut notes: Vec<String> = vec![];
    out += "** Written by ISE Studio: a stress study as a CalculiX deck, to cross-check.\n** Units: mm, N, MPa, tonne/mm³.\n*HEADING\nForma study\n";
    // Nodes, numbered from 1.
    out += "*NODE, NSET=NALL\n";
    for n in (0..m.nodes).filter(|&n| m.used[n]) {
        let p = m.position(n);
        out += &format!("{}, {:.6}, {:.6}, {:.6}\n", n + 1, p[0], p[1], p[2]);
    }
    // Elements by part and fill (tenths), each set its own material.
    let mut sets: std::collections::BTreeMap<(usize, u32), Vec<usize>> = Default::default();
    for (e, (&p, &w)) in m.part.iter().zip(&m.weight).enumerate() {
        sets.entry((p, ((w * 10.0).round() as u32).clamp(1, 10))).or_default().push(e);
    }
    for ((p, tenth), cubes) in &sets {
        out += &format!("*ELEMENT, TYPE=C3D8I, ELSET=P{p}F{tenth}\n");
        for &e in cubes {
            let c = m.corners[e];
            out += &format!("{}, {}\n", e + 1, c.iter().map(|n| (n + 1).to_string()).collect::<Vec<_>>().join(", "));
        }
    }
    out += "*ELSET, ELSET=EALL\n";
    for (p, tenth) in sets.keys() {
        out += &format!("P{p}F{tenth}\n");
    }
    for (p, tenth) in sets.keys() {
        let mat = &s.materials[*p];
        let w = *tenth as f64 / 10.0;
        out += &format!("*MATERIAL, NAME=M{p}F{tenth}\n*ELASTIC\n{:.6}, {:.4}\n*DENSITY\n{:.6e}\n", mat.e * w, mat.nu, mat.density * 1e-12 * w);
        if mat.alpha > 0.0 {
            out += &format!("*EXPANSION\n{:.6e}\n", mat.alpha);
        }
        out += &format!("*SOLID SECTION, ELSET=P{p}F{tenth}, MATERIAL=M{p}F{tenth}\n");
    }
    // Held directions: zero, or the set movement.
    out += "*BOUNDARY\n";
    for n in (0..m.nodes).filter(|&n| m.used[n]) {
        for d in 0..3 {
            if m.fixed[3 * n + d] {
                let v = s.u0[3 * n + d];
                if v == 0.0 {
                    out += &format!("{}, {}, {}\n", n + 1, d + 1, d + 1);
                } else {
                    out += &format!("{}, {}, {}, {:.8}\n", n + 1, d + 1, d + 1, v);
                }
            }
        }
    }
    // Springs along one axis as SPRING1 elements, one set per stiffness.
    let mut next = m.cells.len() + 1;
    let mut slanted = 0;
    for (i, sp) in m.base.iter().enumerate() {
        if sp.g.len() == 1 || sp.g.iter().filter(|(_, c)| c.abs() > 1e-12).count() == 1 {
            let &(dof, c) = sp.g.iter().find(|(_, c)| c.abs() > 1e-12).unwrap_or(&sp.g[0]);
            out += &format!("*ELEMENT, TYPE=SPRING1, ELSET=SPR{i}\n{next}, {}\n*SPRING, ELSET=SPR{i}\n{}\n{:.6e}\n", dof / 3 + 1, dof % 3 + 1, sp.k * c * c);
            next += 1;
        } else {
            slanted += 1;
        }
    }
    if slanted > 0 {
        notes.push(format!("{slanted} springs at a slant (sliding or elastic supports on slanted or round faces) are not written: add them as *EQUATION or SPRINGA by hand."));
    }
    if s.pairs.is_some() {
        notes.push("The parts touch: contact is not written, so the parts are separate here (add *CONTACT PAIR or *TIE to match ISE Studio).".into());
    }
    if s.eps_heat.iter().any(|e| e[0] != 0.0) || !study.bolts.is_empty() || study.plastic {
        notes.push("Heat, bolt preload and plasticity are not written (only the loads on faces and the body forces).".into());
    }
    out += "*STEP\n";
    match kind {
        InpKind::Static => out += "*STATIC\n",
        InpKind::Frequency => out += &format!("*FREQUENCY\n{}\n", count.clamp(1, 20)),
        InpKind::Buckle => out += &format!("*BUCKLE\n{}\n", count.clamp(1, 20)),
    }
    if kind != InpKind::Frequency {
        out += "*CLOAD\n";
        for n in (0..m.nodes).filter(|&n| m.used[n]) {
            for d in 0..3 {
                let v = s.f[3 * n + d];
                if v != 0.0 && !m.fixed[3 * n + d] {
                    out += &format!("{}, {}, {:.8e}\n", n + 1, d + 1, v);
                }
            }
        }
    }
    out += "*NODE FILE\nU\n*EL FILE\nS\n*NODE PRINT, NSET=NALL, TOTALS=NO\nU\n*END STEP\n";
    let mut head = String::new();
    for n in &notes {
        head += &format!("** Note: {n}\n");
    }
    Ok(head + &out)
}
