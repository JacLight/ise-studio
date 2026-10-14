#!/usr/bin/env node
// Writes steps.json: the robot arm from an empty project, part by part and
// joint by joint. `node examples/robot-arm/build.mjs` regenerates it; then
// `node tools/ise.mjs run examples/robot-arm/steps.json --project <name>`
// builds it in a running engine.
//
// Every printed part is its own component (programs/*.js, own coordinates,
// connectors); every bought part comes from the stock library (stock.insert).
// The "Robot arm" assembly holds them as instances, put together only by
// assembly.joint.create on their connectors: fixed joints seat servos,
// horns, bearings, pins, inserts, washers and screws; hinges at the servo
// outputs are the arm's joints. Each instance carries the assembly "step" it
// goes in at, so the fit check knows which parts are not yet there when a
// screw is tightened.

import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const steps = [];
const op = (operation, parameters) => (steps.push({ operation, parameters }), steps.length - 1);
const ref = (i) => `$${i}.id`;

// ---- Materials (PLA at a typical 20 USD/kg) -----------------------------------------
const white = op("solid.material.create", { name: "PLA white", density: { value: 1240, unit: "kg/m3" }, cost: { per_kg: 20 } });
const charcoal = op("solid.material.create", { name: "PLA charcoal", density: { value: 1240, unit: "kg/m3" }, cost: { per_kg: 20 } });
const materials = { "PLA white": ref(white), "PLA charcoal": ref(charcoal) };

// ---- Printed components ------------------------------------------------------------------
const part = (name, file, params = {}, extra = []) =>
  op("shape.create", {
    name,
    description: `Printed part of the desktop robot arm (examples/robot-arm/programs/${file}).`,
    source: { $file: ["programs/lib.js", ...extra, `programs/${file}`] },
    params: Object.entries(params).map(([key, value]) => ({ key, label: key, value, unit: "" })),
    definition: true,
    display_tolerance: 0.05,
    materials,
  });
const P = {
  base: part("Base", "base.js"),
  cover: part("Base cover", "base-cover.js"),
  turntable: part("Turntable", "turntable.js"),
  yoke: part("Shoulder yoke", "yoke.js"),
  shoulderCheek: part("Shoulder bearing cheek", "shoulder-cheek.js"),
  uaL: part("Upper arm, left half", "upper-arm.js", { half: 0 }),
  uaR: part("Upper arm, right half", "upper-arm.js", { half: 1 }),
  faL: part("Forearm, left half", "forearm.js", { half: 0 }),
  faR: part("Forearm, right half", "forearm.js", { half: 1 }),
  elbowL: part("Elbow cheek, horn side", "elbow-cheek.js", { side: 0 }),
  elbowR: part("Elbow cheek, bearing side", "elbow-cheek.js", { side: 1 }),
  housing: part("Wrist housing", "wrist-housing.js"),
  cover2: part("Wrist cover", "small-parts.js", { which: 0 }),
  spacer: part("Finger pivot spacer", "small-parts.js", { which: 1 }),
  sleeve: part("Finger pivot sleeve", "small-parts.js", { which: 2 }),
  fingerA: part("Finger, driven", "finger.js", { side: 0 }, ["../gearbox/involute.js"]),
  fingerB: part("Finger, idler", "finger.js", { side: 1 }, ["../gearbox/involute.js"]),
};

// ---- The assembly ---------------------------------------------------------------------------
const asm = op("assembly.create", { name: "Robot arm", description: "Desktop 4-axis robot arm with a geared two-finger gripper: base yaw, shoulder, elbow and wrist pitch on MG996R / MG90S servos, every joint on a disc horn and (shoulder, elbow, wrist) a ball bearing on the other side." });
const A = ref(asm);
const inst = (of, name, step) => op("assembly.instance.create", { of: ref(of), parent: A, name, properties: { step: String(step) } });
const stock = (family, size, name, step) => op("stock.insert", { family, size, parent: A, name, properties: { step: String(step) } });
const end = (i, connector) => (i === "ground" ? "ground" : { instance: ref(i), connector });
const joint = (type, a, b, extra = {}) => op("assembly.joint.create", { assembly: A, type, a: end(...a), b: end(...b), ...extra });
const fixed = (a, b, extra) => joint("fixed", a, b, extra);

/** A screw seated on a connector (its head's seat), shank going in. */
const screw = (size, name, step, at) => {
  const s = stock("iso4762", size, name, step);
  fixed(at, [s, "Head seat"]);
  return s;
};
/** A heat-set insert flush in its hole. */
const insert = (size, name, step, at) => {
  const s = stock("heatset_insert", size, name, step);
  fixed(at, [s, "Top face"], { aligned: true });
  return s;
};
/** A servo's tab screws: on washers (MG996R's slotted tabs) or straight on the tab. */
const tabScrews = (servo, model, prefix, step) => {
  const n = model === "MG996R" ? 4 : 2;
  for (let k = 1; k <= n; k++) {
    if (model === "MG996R") {
      const w = stock("iso7089", "M2", `${prefix} washer ${k}`, step);
      fixed([servo, `Hole ${k}`], [w, "Top face"], { aligned: true, offset: -2.5 });
      screw("M2×6", `${prefix} screw ${k}`, step, [w, "Bottom face"]);
    } else {
      const s = stock("iso4762", "M2×6", `${prefix} screw ${k}`, step);
      fixed([servo, `Hole ${k}`], [s, "Head seat"], { aligned: true, offset: -2.5 });
    }
  }
};
const hornScrews = (part, prefix, size, step) => {
  for (let k = 1; k <= 4; k++) screw(size, `${prefix} horn screw ${k}`, step, [part, `${prefix} screw ${k}`]);
};

// Base: the cover with its servo and bearing goes in as a unit; then the base, under it.
const cover = inst(P.cover, "Base cover", 6);
const base = inst(P.base, "Base", 10);
fixed(["ground"], [base, "Ground"]);
for (let k = 1; k <= 4; k++) insert("M3×5.7", `Base insert ${k}`, 9, [base, `Insert ${k}`]);
fixed([base, "Cover seat"], [cover, "Seat"]);
for (let k = 1; k <= 4; k++) insert("M2×4", `Base servo insert ${k}`, 1, [cover, `Servo hole ${k}`]);
const s0 = stock("servo_hobby", "MG996R", "Base servo", 2);
fixed([cover, "Servo mount"], [s0, "Hole 1"]);
tabScrews(s0, "MG996R", "Base servo", 3);
const b0 = stock("bearing_deep_groove", "6807-2RS", "Turntable bearing", 4);
fixed([cover, "Bearing"], [b0, "Face A"]);
for (let k = 1; k <= 4; k++) screw("M3×12", `Base cover screw ${k}`, 11, [cover, `Screw ${k}`]);
const h0 = stock("servo_horn", "25T Ø24", "Base horn", 12);
const baseYaw = joint("hinge", [s0, "Output shaft"], [h0, "Spline"], { name: "Base yaw", value: 0, limits: [-90, 90] });
screw("M3×6", "Base horn screw", 12, [h0, "Face"]);
const turntable = inst(P.turntable, "Turntable", 13);
fixed([h0, "Face"], [turntable, "Base horn"]);
hornScrews(turntable, "Base", "M3×10", 14);
for (let k = 1; k <= 4; k++) insert("M3×5.7", `Turntable insert ${k}`, 13, [turntable, `Yoke insert ${k}`]);

// Shoulder: the yoke, the horn bolted into it; the upper arm (with both big servos) onto the horn.
const yoke = inst(P.yoke, "Shoulder yoke", 15);
fixed([turntable, "Yoke seat"], [yoke, "Seat"]);
for (let k = 1; k <= 4; k++) screw("M3×14", `Yoke screw ${k}`, 16, [yoke, `Foot screw ${k}`]);
for (let k = 1; k <= 2; k++) insert("M3×5.7", `Yoke insert ${k}`, 15, [yoke, `Cheek insert ${k}`]);
const h1 = stock("servo_horn", "25T Ø24", "Shoulder horn", 17);
fixed([yoke, "Shoulder horn"], [h1, "Face"]);
hornScrews(yoke, "Shoulder", "M3×8", 18);

const uaL = inst(P.uaL, "Upper arm, left half", 19);
for (let k = 1; k <= 4; k++) insert("M2×4", `Shoulder servo insert ${k}`, 19, [uaL, `Shoulder servo hole ${k}`]);
for (let k = 1; k <= 4; k++) insert("M2×4", `Elbow servo insert ${k}`, 19, [uaL, `Elbow servo hole ${k}`]);
for (let k = 1; k <= 6; k++) insert("M3×5.7", `Upper arm insert ${k}`, 19, [uaL, `Half insert ${k}`]);
const s1 = stock("servo_hobby", "MG996R", "Shoulder servo", 20);
const shoulder = joint("hinge", [h1, "Spline"], [s1, "Output shaft"], { name: "Shoulder", value: 0, limits: [-90, 90] });
fixed([s1, "Hole 1"], [uaL, "Shoulder servo mount"]);
tabScrews(s1, "MG996R", "Shoulder servo", 21);
const s2 = stock("servo_hobby", "MG996R", "Elbow servo", 22);
fixed([uaL, "Elbow servo mount"], [s2, "Hole 1"]);
tabScrews(s2, "MG996R", "Elbow servo", 23);
const uaR = inst(P.uaR, "Upper arm, right half", 24);
fixed([uaL, "Half seat"], [uaR, "Seat"]);
for (let k = 1; k <= 6; k++) screw("M3×35", `Upper arm screw ${k}`, 25, [uaR, `Screw ${k}`]);
const pinS = stock("iso8734", "Ø8×16", "Shoulder pin", 24);
fixed([uaR, "Shoulder pin"], [pinS, "Axis"], { aligned: true });
const pinE = stock("iso8734", "Ø8×16", "Elbow pin", 24);
fixed([uaR, "Elbow pin"], [pinE, "Axis"], { aligned: true });
screw("M3×6", "Shoulder horn screw", 26, [h1, "Face"]);
const sCheek = inst(P.shoulderCheek, "Shoulder bearing cheek", 28);
const b1 = stock("bearing_deep_groove", "608-2RS", "Shoulder bearing", 27);
fixed([sCheek, "Bearing"], [b1, "Face A"], { aligned: true });
fixed([yoke, "Cheek seat"], [sCheek, "Seat"]);
for (let k = 1; k <= 2; k++) screw("M3×12", `Shoulder cheek screw ${k}`, 29, [sCheek, `Foot screw ${k}`]);

// Elbow: the forearm (with the wrist servo inside) between its cheeks, onto the elbow horn.
const faL = inst(P.faL, "Forearm, left half", 30);
for (let k = 1; k <= 2; k++) insert("M3×5.7", `Forearm cheek insert ${k}`, 30, [faL, `Cheek insert ${k}`]);
for (let k = 1; k <= 4; k++) insert("M3×5.7", `Forearm insert ${k}`, 30, [faL, `Half insert ${k}`]);
const s3 = stock("servo_hobby", "MG90S", "Wrist servo", 31);
fixed([faL, "Wrist servo mount"], [s3, "Hole 1"]);
tabScrews(s3, "MG90S", "Wrist servo", 31);
const faR = inst(P.faR, "Forearm, right half", 32);
fixed([faL, "Half seat"], [faR, "Seat"]);
for (let k = 1; k <= 4; k++) screw("M3×20", `Forearm screw ${k}`, 33, [faR, `Screw ${k}`]);
for (let k = 1; k <= 2; k++) insert("M3×5.7", `Forearm cheek insert ${k + 2}`, 32, [faR, `Cheek insert ${k}`]);
const pinW = stock("iso8734", "Ø5×16", "Wrist pin", 32);
fixed([faR, "Wrist pin"], [pinW, "Axis"], { aligned: true });
const h2 = stock("servo_horn", "25T Ø24", "Elbow horn", 34);
const elbow = joint("hinge", [s2, "Output shaft"], [h2, "Spline"], { name: "Elbow", value: 0, limits: [-30, 90] });
const eL = inst(P.elbowL, "Elbow cheek, horn side", 35);
fixed([h2, "Face"], [eL, "Elbow horn"]);
hornScrews(eL, "Elbow", "M3×8", 36);
fixed([eL, "Forearm seat"], [faL, "Cheek seat"]);
for (let k = 1; k <= 2; k++) screw("M3×18", `Elbow cheek screw ${k}`, 37, [eL, `Screw ${k}`]);
const eR = inst(P.elbowR, "Elbow cheek, bearing side", 39);
const b2 = stock("bearing_deep_groove", "608-2RS", "Elbow bearing", 38);
fixed([eR, "Bearing"], [b2, "Face A"], { aligned: true });
fixed([faR, "Cheek seat"], [eR, "Forearm seat"]);
for (let k = 1; k <= 2; k++) screw("M3×18", `Elbow cheek screw ${k + 2}`, 40, [eR, `Screw ${k}`]);
screw("M3×6", "Elbow horn screw", 41, [h2, "Face"]);

// Wrist and gripper.
const h3 = stock("servo_horn", "20T Ø16", "Wrist horn", 42);
const wrist = joint("hinge", [s3, "Output shaft"], [h3, "Spline"], { name: "Wrist", value: 0, limits: [-90, 90] });
const housing = inst(P.housing, "Wrist housing", 44);
fixed([h3, "Face"], [housing, "Wrist horn"]);
hornScrews(housing, "Wrist", "M2×6", 45);
screw("M2×5", "Wrist horn screw", 46, [h3, "Face"]);
const b3 = stock("bearing_deep_groove", "625-2RS", "Wrist bearing", 43);
fixed([housing, "Wrist bearing"], [b3, "Face A"], { aligned: true });
insert("M3×3 short", "Finger pivot insert", 43, [housing, "Pivot insert"]);
for (let k = 1; k <= 4; k++) insert("M3×5.7", `Wrist cover insert ${k}`, 43, [housing, `Cover insert ${k}`]);
const s5 = stock("servo_hobby", "MG90S", "Gripper servo", 47);
fixed([housing, "Grip servo mount"], [s5, "Hole 1"]);
tabScrews(s5, "MG90S", "Gripper servo", 48);
const h5 = stock("servo_horn", "20T Ø16", "Gripper horn", 49);
const grip = joint("hinge", [s5, "Output shaft"], [h5, "Spline"], { name: "Grip", value: 0, limits: [0, 30] });
screw("M2×5", "Gripper horn screw", 49, [h5, "Face"]);
const fA = inst(P.fingerA, "Finger, driven", 50);
fixed([h5, "Face"], [fA, "Grip horn"]);
hornScrews(fA, "Grip", "M2×8", 51);
const spacer = inst(P.spacer, "Finger pivot spacer", 52);
fixed([housing, "Finger pivot"], [spacer, "Bottom"]);
const fB = inst(P.fingerB, "Finger, idler", 53);
joint("hinge", [housing, "Finger pivot"], [fB, "Pivot"], { name: "Grip idler", offset: 9.7, value: "=-Grip.angle", limits: [-30, 0] });
const sleeve = inst(P.sleeve, "Finger pivot sleeve", 54);
fixed([housing, "Finger pivot"], [sleeve, "Bottom"]);
screw("M3×20", "Finger pivot screw", 55, [sleeve, "Screw"]);
const wcover = inst(P.cover2, "Wrist cover", 56);
fixed([housing, "Cover seat"], [wcover, "Seat"]);
for (let k = 1; k <= 4; k++) screw("M3×8", `Wrist cover screw ${k}`, 57, [wcover, `Screw ${k}`]);

// The arm in the project, and what it must meet.
op("assembly.instance.create", { of: A, name: "Robot arm", at: [0, 0, 0] });
for (const j of [baseYaw, shoulder, elbow, wrist, grip]) op("intent.requirement.create", { rule: { kind: "no_clashes_in_motion", joint: ref(j), steps: 36 } });
op("intent.requirement.create", { name: "Everything fits", rule: { kind: "assembly_fits", assembly: A, motion: false } });

writeFileSync(join(here, "steps.json"), JSON.stringify(steps, null, 1) + "\n");
console.log(`steps.json: ${steps.length} steps`);
