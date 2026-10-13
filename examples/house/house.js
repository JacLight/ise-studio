// A small two-storey house as building elements: ground slab with its
// reinforcement, external walls with doors and windows, partitions, a first
// floor slab with a stairwell, a straight stair and its balustrade, gable
// walls and a gable roof; rooms as spaces, zoned day and night; a grid on the
// external wall centre lines. Every part carries its IFC class, level,
// property sets and base quantities, so schedules, plans and IFC follow.
//
// x runs east, y north; the external wall centre lines are the grid.
const W = p.width, D = p.depth, T = p.wall, t = p.partition;
const g = p.groundHeight, f = p.firstHeight, fs = p.floorSlab;

site({ name: "Plot 12, Orchard Lane", latitude: 51.752, longitude: -1.2577, elevation: 64, north: 0 });
building({ name: "Orchard House", address: "12 Orchard Lane, Oxford" });
const L0 = level("Ground floor", 0, { height: g });
const L1 = level("First floor", g, { height: f });
const roofLevel = level("Roof", g + f);
grid("Grid", { x: [0, W / 2, W], y: [0, D / 2, D], level: L0 });

const parts = [];
const outer = [[-T / 2, -T / 2], [W + T / 2, -T / 2], [W + T / 2, D + T / 2], [-T / 2, D + T / 2]];
const inner = (x0, y0, x1, y1) => [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
const i0 = T / 2, hx = W / 2, hy = p.hallDepth; // inner face, the partition lines

// Ground slab on the ground, with a bottom mesh of bars both ways.
parts.push(...slab("Ground slab", { outline: outer, thickness: 250, level: L0, kind: "BASESLAB", external: true }));
const cover = 40;
parts.push(...rebar("Slab mesh X", { path: [[-T / 2 + cover, -T / 2 + cover, -250 + cover], [W + T / 2 - cover, -T / 2 + cover, -250 + cover]], diameter: 10, count: Math.floor((D + T - 2 * cover) / 200) + 1, spacing: 200, direction: [0, 1, 0], mark: "01", level: L0, embedded: "Ground slab" }));
parts.push(...rebar("Slab mesh Y", { path: [[-T / 2 + cover, -T / 2 + cover, -250 + cover + 10], [-T / 2 + cover, D + T / 2 - cover, -250 + cover + 10]], diameter: 10, count: Math.floor((W + T - 2 * cover) / 200) + 1, spacing: 200, direction: [1, 0, 0], mark: "02", level: L0, embedded: "Ground slab" }));

// Ground floor: external walls anticlockwise (their left is inside, so doors open in).
parts.push(
  ...walls("GF wall", {
    points: [[0, 0], [W, 0], [W, D], [0, D]], closed: true, thickness: T, height: g - fs, level: L0, external: true, loadBearing: true, material: "Brick",
    names: ["GF south wall", "GF east wall", "GF north wall", "GF west wall"],
    openings: [
      [window({ at: 2500, preset: "double", width: 1500 }), door({ at: W - 2500, preset: "entrance" })],
      [window({ at: 1500, preset: "casement" }), window({ at: 5500, preset: "double" })],
      [door({ at: W - 6500, preset: "sliding", width: 1800, name: "Garden door" }), window({ at: W - 2500, preset: "double" })],
      [window({ at: D / 2, preset: "triple" })],
    ],
  }),
);
// Partitions: living | hall and kitchen; hall | kitchen.
parts.push(...wall("GF partition A", { from: [hx, i0], to: [hx, D - i0], thickness: t, height: g - fs, level: L0, material: "Block", openings: [door({ at: 1700 - i0, width: 900 })] }));
parts.push(...wall("GF partition B", { from: [hx + t / 2, hy], to: [W - i0, hy], thickness: t, height: g - fs, level: L0, material: "Block", openings: [door({ at: 450, width: 800, swing: "right" })] }));

// The stair rises west along the hall's north side, arriving on the landing.
const stairW = 1000, going = 250;
const sy = hy - t / 2 - stairW / 2;
parts.push(...stair("Stair", { at: [W - i0 - 50, sy], dir: 180, width: stairW, rise: g, going, level: L0, material: "Oak" }));
const run = (Math.ceil(g / 180 - 1e-9) - 1) * going;
const top = W - i0 - 50 - run;

// First floor slab, open over the stair, with a balustrade along the well.
const well = inner(top, sy - stairW / 2 - 50, W - i0, sy + stairW / 2);
parts.push(...slab("First floor slab", { outline: outer, thickness: fs, level: L1, holes: [well] }));
parts.push(...railing("Stairwell balustrade", { path: [[W - i0, sy - stairW / 2 - 50], [top, sy - stairW / 2 - 50]], height: 1000, level: L1, material: "Steel" }));

// First floor: north and south walls run to the eaves past the corners; the
// gable walls stand between them, pitched up under the roof.
const slope = p.roofSlope, overhang = 400, roofT = 250;
const tan = Math.tan(rad(slope)), under = (s) => s * tan - roofT / Math.cos(rad(slope)) - 20;
// A gable's elevation; s0: how far its end is in from the eave line.
const s0 = overhang + T;
const gable = (len) => [[0, 0], [len, 0], [len, f + under(s0)], [len / 2, f + under(s0 + len / 2)], [0, f + under(s0)]];
const brick = { thickness: T, level: L1, external: true, loadBearing: true, material: "Brick" };
parts.push(...wall("FF south wall", { ...brick, from: [-T / 2, 0], to: [W + T / 2, 0], height: f, openings: [window({ at: 2500 + T / 2, preset: "double" }), window({ at: 8000 + T / 2, preset: "casement" })] }));
parts.push(...wall("FF north wall", { ...brick, from: [W + T / 2, D], to: [-T / 2, D], height: f, openings: [window({ at: W + T / 2 - 8500, preset: "high", name: "Bathroom window" }), window({ at: W + T / 2 - 2500, preset: "double" })] }));
parts.push(...wall("FF east gable", { ...brick, from: [W, T / 2], to: [W, D - T / 2], profile: gable(D - T) }));
parts.push(...wall("FF west gable", { ...brick, from: [0, D - T / 2], to: [0, T / 2], profile: gable(D - T), openings: [window({ at: 2000 - T / 2, preset: "casement" })] }));

// First floor partitions: bedrooms | landing; bed 1 | bed 2; landing | bathroom.
const fy = D / 2;
parts.push(...wall("FF partition A", { from: [hx, i0], to: [hx, D - i0], thickness: t, height: f, level: L1, material: "Block", openings: [door({ at: 1500, width: 800 }), door({ at: D - 2200, width: 800, swing: "right" })] }));
parts.push(...wall("FF partition B", { from: [i0, fy], to: [hx - t / 2, fy], thickness: t, height: f, level: L1, material: "Block" }));
parts.push(...wall("FF partition C", { from: [hx + 2000, hy], to: [W - i0, hy], thickness: t, height: f, level: L1, material: "Block" }));
parts.push(...wall("FF partition D", { from: [hx + 2000, hy + t / 2], to: [hx + 2000, D - i0], thickness: t, height: f, level: L1, material: "Block", openings: [door({ at: 900, width: 800, side: -1 })] }));

// Roof over the whole, ridge east–west.
parts.push(...roof("Roof", { x: -T / 2, y: -T / 2, width: W + T, depth: D + T, kind: "gable", slope, thickness: roofT, overhang, ridge: "x", level: roofLevel, material: "Timber" }));

// Kitchen worktop along the east wall: furniture, tagged by hand.
parts.push(part("Kitchen worktop", box(600, 3000, 900).translate(W - i0 - 600, hy + 600, 0), { material: "Oak", process: "cnc_router", ifc: { class: "IfcFurniture", predefined: "TABLE", level: L0, type: "Worktop 3000 × 600" } }));

// Rooms.
const ht = t / 2;
space("Living", { outline: inner(i0, i0, hx - ht, D - i0), level: L0, height: g - fs, longName: "Living room", zone: "Day" });
space("Hall", { outline: inner(hx + ht, i0, W - i0, hy - ht), level: L0, height: g - fs, longName: "Entrance hall", zone: "Day" });
space("Kitchen", { outline: inner(hx + ht, hy + ht, W - i0, D - i0), level: L0, height: g - fs, longName: "Kitchen and dining", zone: "Day" });
space("Bedroom 2", { outline: inner(i0, i0, hx - ht, fy - ht), level: L1, height: f - 200, zone: "Night" });
space("Bedroom 1", { outline: inner(i0, fy + ht, hx - ht, D - i0), level: L1, height: f - 200, zone: "Night" });
space("Landing", { outline: [[hx + ht, i0], [W - i0, i0], [W - i0, hy - ht], [hx + 2000 - ht, hy - ht], [hx + 2000 - ht, D - i0], [hx + ht, D - i0]], level: L1, height: f - 200, zone: "Night" });
space("Bathroom", { outline: inner(hx + 2000 + ht, hy + ht, W - i0, D - i0), level: L1, height: f - 200, zone: "Night" });

return parts;
