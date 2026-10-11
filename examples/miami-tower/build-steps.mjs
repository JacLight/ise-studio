#!/usr/bin/env node
// Writes steps.json: the tower built the way it is designed and sold.
//
//   0. The footprint: one parameter set every storey links to.
//   1. Components: one floor component per storey type (lobby, parking,
//      amenity deck, typical hotel floor, typical residential floor, roof).
//   2. The tower: an assembly with one instance per storey, stacked.
//   3. Sales (sample data): one buyer takes L25 as a full-floor home; another
//      takes L30–L32, which become one three-storey residence linked by
//      internal stairs; L36 is fitted out as serviced suites; a few floors are
//      sold or reserved. Every customised floor is made unique first, so the
//      typical floors stay shared.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const footprint = [
  { key: "towerWidth", label: "Tower width", value: 52000, min: 20000, max: 90000 },
  { key: "towerDepth", label: "Tower depth", value: 26000, min: 14000, max: 50000 },
  { key: "cornerRadius", label: "Corner radius", value: 4000, min: 500, max: 10000 },
  { key: "balconyDepth", label: "Balcony depth", value: 2400, min: 0, max: 4000 },
  { key: "podiumWidth", label: "Podium width", value: 76000, min: 30000, max: 150000 },
  { key: "podiumDepth", label: "Podium depth", value: 56000, min: 20000, max: 120000 },
  { key: "towerSetback", label: "Tower setback from podium back", value: 4000, min: 0, max: 20000 },
  { key: "coreWidth", label: "Core width", value: 18000, min: 8000, max: 30000 },
  { key: "coreDepth", label: "Core depth", value: 9000, min: 5000, max: 16000 },
  { key: "coreWall", label: "Core wall", value: 400, min: 200, max: 1000 },
  { key: "columnSize", label: "Column size", value: 900, min: 400, max: 1800 },
  { key: "bay", label: "Column bay", value: 8800, min: 6000, max: 12000 },
  { key: "slab", label: "Slab thickness", value: 250, min: 180, max: 450 },
  { key: "glassInset", label: "Glass line inset", value: 150, min: 50, max: 600 },
  { key: "glassThickness", label: "Glass unit thickness", value: 32, min: 10, max: 60 },
  { key: "crownHeight", label: "Crown height", value: 6000, min: 1000, max: 20000 },
  { key: "crownThickness", label: "Crown thickness", value: 400, min: 200, max: 1500 },
  { key: "poolLength", label: "Pool length", value: 25000, min: 8000, max: 50000 },
  { key: "poolDepth", label: "Pool width", value: 8000, min: 4000, max: 20000 },
].map((q) => ({ unit: "mm", ...q }));

// Every floor follows the footprint: one change to the set moves the whole building.
const linked = footprint.map((q) => ({ ...q, expr: `=Tower footprint.${q.key}` }));
const floorParams = (kind, height, layout) => [
  { key: "kind", label: "Storey type (0 lobby … 5 roof)", value: kind, unit: "count", min: 0, max: 5, step: 1 },
  { key: "height", label: "Floor to floor", value: height, unit: "mm", min: 2800, max: 12000 },
  { key: "layout", label: "Fit-out (0 shell, 1 home, 2 suites)", value: layout, unit: "count", min: 0, max: 2, step: 1 },
  { key: "stairUp", label: "Stair to the floor above", value: 0, unit: "count", min: 0, max: 1, step: 1 },
  { key: "stairDown", label: "Opening for a stair from below", value: 0, unit: "count", min: 0, max: 1, step: 1 },
  ...linked,
];

const steps = [];
const add = (operation, parameters) => steps.push({ operation, parameters }) - 1;
const ref = (i, key = "id") => `$${i}.${key}`;

const mat = {
  Concrete: add("solid.material.create", { name: "Concrete", density: { value: 2400, unit: "kg/m3" } }),
  Glass: add("solid.material.create", { name: "Glass", density: { value: 2500, unit: "kg/m3" } }),
  "Pool water": add("solid.material.create", { name: "Pool water", density: { value: 1000, unit: "kg/m3" } }),
  Gypsum: add("solid.material.create", { name: "Gypsum", density: { value: 700, unit: "kg/m3" } }),
};
const materials = Object.fromEntries(Object.entries(mat).map(([k, i]) => [k, ref(i)]));

add("parameters.create", {
  name: "Tower footprint",
  description: "The plan dimensions every storey follows: tower and podium, core, column grid, slab and façade.",
  params: footprint,
});

const component = (name, description, kind, height, layout = 0) =>
  add("shape.create", {
    name,
    description,
    source: { $file: ["common.js", "floor.js"] },
    params: floorParams(kind, height, layout),
    display_tolerance: 5,
    definition: true,
    materials,
  });

const H = { lobby: 6000, parking: 3200, amenity: 5000, hotel: 3300, residence: 3500 };
const defs = {
  lobby: component("Lobby level", "Double-height lobby: podium slab, parking-grid and tower columns, core, storefront glazing.", 0, H.lobby),
  parking: component("Parking level", "Podium parking deck on the parking grid.", 1, H.parking),
  amenity: component("Amenity deck", "Podium roof with the pool in front of the tower; amenity storey in the tower footprint.", 2, H.amenity),
  hotel: component("Hotel floor", "Typical hotel floor: 12 keys either side of a corridor around the core.", 3, H.hotel, 2),
  residence: component("Residential floor", "Typical residential floor delivered as a shell, with deep balconies and glass balustrades at 42″.", 4, H.residence, 0),
  roof: component("Roof and crown", "Roof slab, parapet crown and machine room.", 5, H.residence),
};

const tower = add("assembly.create", { name: "Tower", description: "40 storeys: lobby, 4 parking levels, amenity deck, 12 hotel floors, 22 residential floors, roof." });
const storeys = [["lobby"], ...Array(4).fill(["parking"]), ["amenity"], ...Array(12).fill(["hotel"]), ...Array(22).fill(["residence"])].map((x) => x[0]);
let z = 0;
const floorAt = {};
const use = { lobby: "Lobby", parking: "Parking", amenity: "Amenities", hotel: "Hotel", residence: "Residence" };
storeys.forEach((type, i) => {
  const n = i + 1;
  const name = `L${String(n).padStart(2, "0")}`;
  const properties = { use: use[type] };
  if (type === "hotel") Object.assign(properties, { status: "Hotel", owner: "Hotel operator" });
  if (type === "residence") properties.status = "Available";
  floorAt[n] = { step: add("assembly.instance.create", { of: ref(defs[type]), parent: ref(tower), name, at: [0, 0, z], properties }), z };
  z += H[type];
});
add("assembly.instance.create", { of: ref(defs.roof), parent: ref(tower), name: "Roof", at: [0, 0, z] });
const roofTop = z;
const site = add("assembly.instance.create", { of: ref(tower), name: "Miami condo + hotel tower", at: [0, 0, 0] });

// ---- Sales ----------------------------------------------------------------
const update = (n, properties) => add("assembly.instance.update", { target: ref(floorAt[n].step), properties });
// A full floor, fitted out as one home.
const l25 = add("assembly.instance.make_unique", { target: ref(floorAt[25].step), name: "L25 · full-floor residence" });
add("shape.update", { target: ref(l25, "definition"), description: "L25 sold whole: one full-floor home, bedrooms along the private back side, open living to the front." });
add("shape.set_params", { target: ref(l25, "definition"), values: { layout: 1 } });
update(25, { status: "Sold", owner: "Buyer A", use: "Full-floor residence" });

// Three storeys bought together: one home over L30–L32, linked by internal stairs.
const home = add("assembly.create", { name: "Residence L30–L32", description: "Three consecutive storeys combined into one home, linked by internal stairs through openings in the slabs." });
[30, 31, 32].forEach((n, k) => {
  const inst = add("assembly.instance.create", { of: ref(defs.residence), parent: ref(home), name: `L${n}`, at: [0, 0, k * H.residence] });
  const own = add("assembly.instance.make_unique", { target: ref(inst), name: `L${n} · Residence L30–L32` });
  add("shape.update", { target: ref(own, "definition"), description: `L${n} of the three-storey home: ${["stair up to L31", "stair down from L30 and up to L32", "opening for the stair from L31"][k]}.` });
  add("shape.set_params", { target: ref(own, "definition"), values: { layout: 1, stairUp: k < 2 ? 1 : 0, stairDown: k > 0 ? 1 : 0 } });
  add("assembly.instance.delete", { target: ref(floorAt[n].step) });
});
add("assembly.instance.create", {
  of: ref(home),
  parent: ref(tower),
  name: "L30–L32",
  at: [0, 0, floorAt[30].z],
  properties: { status: "Sold", owner: "Buyer B", use: "Three-storey residence" },
});

// A residential floor partitioned as serviced suites, run like the hotel.
const l36 = add("assembly.instance.make_unique", { target: ref(floorAt[36].step), name: "L36 · serviced suites" });
add("shape.update", { target: ref(l36, "definition"), description: "A residential floor fitted out as 12 serviced suites around a corridor, run by the hotel operator." });
add("shape.set_params", { target: ref(l36, "definition"), values: { layout: 2 } });
update(36, { status: "Reserved", owner: "Hotel operator", use: "Serviced suites" });

update(21, { status: "Sold", owner: "Buyer C" });
update(27, { status: "Reserved", owner: "Buyer D" });
update(38, { status: "Reserved", owner: "Buyer E" });

// ---- Requirements ------------------------------------------------------------
add("intent.requirement.create", {
  name: "Within the site and the height limit",
  rule: { kind: "fits_within", parts: [ref(site)], width: { value: 80, unit: "m" }, depth: { value: 60, unit: "m" }, height: { value: 160, unit: "m" } },
});
add("intent.requirement.create", { name: "No clashes anywhere in the building", rule: { kind: "no_collisions", parts: [ref(site)] } });

writeFileSync(join(here, "steps.json"), JSON.stringify(steps, null, 2) + "\n");
console.log(`${storeys.length} storeys + roof, roof at ${(roofTop / 1000).toFixed(1)} m; ${steps.length} steps → steps.json`);
