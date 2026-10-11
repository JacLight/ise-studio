//! ISO 286 limits and fits: the deviations of a tolerance class ("H7", "g6",
//! "js6", "K7") for a nominal size, from the standard's tables (ISO 286-1
//! tables 1–3, sizes up to 500 mm, grades IT5–IT12).

/// Upper limits of the size ranges (over the previous one, up to and including this).
const RANGES: [f64; 13] = [3.0, 6.0, 10.0, 18.0, 30.0, 50.0, 80.0, 120.0, 180.0, 250.0, 315.0, 400.0, 500.0];

/// Standard tolerance grades IT5–IT12 (µm), per size range.
const IT: [[i32; 13]; 8] = [
    [4, 5, 6, 8, 9, 11, 13, 15, 18, 20, 23, 25, 27],                       // IT5
    [6, 8, 9, 11, 13, 16, 19, 22, 25, 29, 32, 36, 40],                     // IT6
    [10, 12, 15, 18, 21, 25, 30, 35, 40, 46, 52, 57, 63],                  // IT7
    [14, 18, 22, 27, 33, 39, 46, 54, 63, 72, 81, 89, 97],                  // IT8
    [25, 30, 36, 43, 52, 62, 74, 87, 100, 115, 130, 140, 155],             // IT9
    [40, 48, 58, 70, 84, 100, 120, 140, 160, 185, 210, 230, 250],          // IT10
    [60, 75, 90, 110, 130, 160, 190, 220, 250, 290, 320, 360, 400],        // IT11
    [100, 120, 150, 180, 210, 250, 300, 350, 400, 460, 520, 570, 630],     // IT12
];

/// Fundamental deviations of shafts (µm): the upper deviation es for d–h,
/// the lower ei for k–p (k: its value for grades 4 to 7; 0 otherwise).
const D_ES: [i32; 13] = [-20, -30, -40, -50, -65, -80, -100, -120, -145, -170, -190, -210, -230];
const E_ES: [i32; 13] = [-14, -20, -25, -32, -40, -50, -60, -72, -85, -100, -110, -125, -135];
const F_ES: [i32; 13] = [-6, -10, -13, -16, -20, -25, -30, -36, -43, -50, -56, -62, -68];
const G_ES: [i32; 13] = [-2, -4, -5, -6, -7, -9, -10, -12, -14, -15, -17, -18, -20];
const K_EI: [i32; 13] = [0, 1, 1, 1, 2, 2, 2, 3, 3, 4, 4, 4, 5];
const M_EI: [i32; 13] = [2, 4, 6, 7, 8, 9, 11, 13, 15, 17, 20, 21, 23];
const N_EI: [i32; 13] = [4, 8, 10, 12, 15, 17, 20, 23, 27, 31, 34, 37, 40];
const P_EI: [i32; 13] = [6, 12, 15, 18, 22, 26, 32, 37, 43, 50, 56, 62, 68];

/// The classes these tables cover, for pickers: holes then shafts.
pub const COMMON: [&str; 24] = ["H6", "H7", "H8", "H9", "H10", "H11", "G7", "F8", "JS7", "K7", "N7", "P7", "h6", "h7", "h8", "h9", "h11", "g6", "f7", "js6", "k6", "m6", "n6", "p6"];

/// A tolerance class split into its letters and grade ("js6" → ("js", 6)).
fn parse(class: &str) -> Option<(&str, usize)> {
    let class = class.trim();
    let split = class.find(|c: char| c.is_ascii_digit())?;
    let (letters, grade) = class.split_at(split);
    let grade: usize = grade.parse().ok()?;
    (!letters.is_empty() && letters.chars().all(|c| c.is_ascii_alphabetic())).then_some((letters, grade))
}

/// Whether a tolerance class names a hole (capital letters, "H7") rather than a shaft ("h7").
pub fn is_hole(class: &str) -> bool {
    parse(class).is_some_and(|(l, _)| l.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
}

/// The upper and lower deviations (mm) of `class` at `nominal` mm:
/// ⌀25 H7 → (+0.021, 0), ⌀25 g6 → (−0.007, −0.020).
pub fn deviations(class: &str, nominal: f64) -> Result<(f64, f64), String> {
    let (letters, grade) = parse(class).ok_or_else(|| format!("“{class}” is not a tolerance class (like H7 or g6)"))?;
    if !(nominal > 0.0 && nominal <= 500.0) {
        return Err(format!("ISO 286 fits are tabled for sizes up to 500 mm, not {nominal} mm"));
    }
    if !(5..=12).contains(&grade) {
        return Err(format!("grade IT{grade} is not tabled here (IT5 to IT12)"));
    }
    let r = RANGES.iter().position(|u| nominal <= *u + 1e-9).unwrap_or(RANGES.len() - 1);
    let it = |g: usize| IT[g - 5][r];
    let t = it(grade);
    // Δ for holes K–P of the finer grades: ITn − IT(n−1); none up to 3 mm.
    let delta = || if r == 0 || grade == 5 { 0 } else { t - it(grade - 1) };
    // js / JS: ±IT/2, rounded down to an even µm for grades 7 to 11 when IT is odd.
    let half = || -> f64 {
        if (7..=11).contains(&grade) && t % 2 == 1 {
            ((t - 1) / 2) as f64
        } else {
            t as f64 / 2.0
        }
    };
    let (upper, lower): (f64, f64) = match letters {
        "d" | "e" | "f" | "g" | "h" => {
            let es = match letters {
                "d" => D_ES[r],
                "e" => E_ES[r],
                "f" => F_ES[r],
                "g" => G_ES[r],
                _ => 0,
            };
            (es as f64, (es - t) as f64)
        }
        "js" => (half(), -half()),
        "k" | "m" | "n" | "p" => {
            let ei = match letters {
                "k" => {
                    if (4..=7).contains(&grade) {
                        K_EI[r]
                    } else {
                        0
                    }
                }
                "m" => M_EI[r],
                "n" => N_EI[r],
                _ => P_EI[r],
            };
            ((ei + t) as f64, ei as f64)
        }
        "D" | "E" | "F" | "G" | "H" => {
            let ei = match letters {
                "D" => -D_ES[r],
                "E" => -E_ES[r],
                "F" => -F_ES[r],
                "G" => -G_ES[r],
                _ => 0,
            };
            ((ei + t) as f64, ei as f64)
        }
        "JS" => (half(), -half()),
        "K" | "M" | "N" => {
            let es = match (letters, grade <= 8) {
                ("K", true) => -K_EI[r] + delta(),
                ("K", false) => 0,
                ("M", true) => -M_EI[r] + delta(),
                ("M", false) => -M_EI[r],
                ("N", true) => -N_EI[r] + delta(),
                _ => 0,
            };
            (es as f64, (es - t) as f64)
        }
        "P" => {
            let es = if grade <= 7 { -P_EI[r] + delta() } else { -P_EI[r] };
            (es as f64, (es - t) as f64)
        }
        _ => return Err(format!("tolerance class “{class}” is not tabled here (holes D–H, JS, K, M, N, P; shafts d–h, js, k, m, n, p)")),
    };
    Ok((upper / 1000.0, lower / 1000.0))
}
