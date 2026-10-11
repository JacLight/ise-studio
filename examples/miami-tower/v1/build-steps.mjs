#!/usr/bin/env node
// Writes steps.json for the tower: the storey levels come from the same
// stack() the program uses, so plans and structure always agree.
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const params = [
  { key: "lobbyHeight", label: "Lobby height", value: 6000, min: 4000, max: 12000 },
  { key: "podiumFloors", label: "Podium floors (lobby + parking)", value: 5, unit: "count", min: 1, max: 10, step: 1 },
  { key: "podiumFloorHeight", label: "Parking floor height", value: 3200, min: 2800, max: 4500 },
  { key: "amenityHeight", label: "Amenity deck storey", value: 5000, min: 3500, max: 9000 },
  { key: "hotelFloors", label: "Hotel floors", value: 12, unit: "count", min: 0, max: 40, step: 1 },
  { key: "hotelFloorHeight", label: "Hotel floor height", value: 3300, min: 3000, max: 4500 },
  { key: "condoFloors", label: "Residential floors", value: 22, unit: "count", min: 0, max: 60, step: 1 },
  { key: "condoFloorHeight", label: "Residential floor height", value: 3500, min: 3000, max: 5000 },
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

const common = readFileSync(join(here, "common.js"), "utf8");
const p = Object.fromEntries(params.map((q) => [q.key, q.value]));
const { levels, roof } = new Function("p", `${common}\nreturn stack(p);`)(p);
const storeys = levels.map((L) => ({ name: `L${String(L.n).padStart(2, "0")} · ${L.label}`, elevation: L.z + p.slab }));
storeys.push({ name: "Roof", elevation: roof + p.slab });

const steps = [
  { operation: "solid.material.create", parameters: { name: "Concrete", density: { value: 2400, unit: "kg/m3" } } },
  { operation: "solid.material.create", parameters: { name: "Glass", density: { value: 2500, unit: "kg/m3" } } },
  { operation: "solid.material.create", parameters: { name: "Pool water", density: { value: 1000, unit: "kg/m3" } } },
  {
    operation: "shape.create",
    parameters: {
      name: "Miami condo + hotel tower",
      description:
        `${levels.length}-storey mixed-use tower: double-height lobby, ${p.podiumFloors - 1} parking floors in the podium, an amenity deck with pool on the podium roof, ` +
        `${p.hotelFloors} hotel floors and ${p.condoFloors} residential floors with deep balconies and glass balustrades at 42″. ` +
        `Concrete flat-plate floors on perimeter columns and a central core with an elevator bank between two stairs; unitized curtain wall. ` +
        `Every storey's slab, columns, core lift and glazing is a separate part.`,
      source: { $file: ["common.js", "tower.js"] },
      params,
      display_tolerance: 5,
      levels: storeys,
      materials: { Concrete: "$0.id", Glass: "$1.id", "Pool water": "$2.id" },
    },
  },
  {
    operation: "intent.requirement.create",
    parameters: {
      name: "Within the site and the height limit",
      rule: { kind: "fits_within", parts: ["$3.id"], width: { value: 80, unit: "m" }, depth: { value: 60, unit: "m" }, height: { value: 160, unit: "m" } },
    },
  },
  { operation: "intent.requirement.create", parameters: { name: "No clashes between building parts", rule: { kind: "no_collisions", parts: ["$3.id"] } } },
];
writeFileSync(join(here, "steps.json"), JSON.stringify(steps, null, 2) + "\n");
console.log(`${levels.length} storeys, roof at ${(roof / 1000).toFixed(1)} m; wrote steps.json`);
