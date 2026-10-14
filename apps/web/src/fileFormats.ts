// Files the engine imports (POST /import/<name>): exact solids, meshes, 2D drawings, OpenSCAD and tables; .ise projects open as new projects.

/** Extensions an import takes. */
export const IMPORT_EXTENSIONS = ["step", "stp", "stpz", "stepz", "gz", "brep", "brp", "iges", "igs", "stl", "obj", "ply", "gltf", "glb", "3mf", "dxf", "svg", "scad", "csv", "tsv", "xlsx", "xlsm", "ifc", "ise"];

/** For a file input's `accept`. */
export const IMPORT_ACCEPT = IMPORT_EXTENSIONS.map((e) => `.${e}`).join(",");

/** For messages: what can be imported. */
export const IMPORT_FORMATS = "STEP (also compressed), BREP, IGES, STL, OBJ, PLY, glTF, 3MF, DXF, SVG, OpenSCAD, IFC buildings, tables (CSV, XLSX) or project (.ise)";

/** Whether a file looks like something the engine imports (it reads the bytes to be sure). */
export const importable = (name: string) => IMPORT_EXTENSIONS.includes(name.split(".").pop()?.toLowerCase() ?? "");

/** What the engine says an import became. */
export type ImportReply =
  | { kind: "shape"; id: string; bodies: number; format: string; exact: boolean; colors?: { part: string; color: string }[] }
  | { kind: "sketch"; id: string; name: string; format: string; counts: { lines: number; arcs: number; circles: number; curves: number }; size: [number, number] }
  | { kind: "ifc"; id: string; bodies: number };

/** One line for the person: what came in. */
export function importSummary(file: string, r: ImportReply): string {
  if (r.kind === "sketch") {
    const c = r.counts;
    const parts = [c.lines && `${c.lines} lines`, c.arcs && `${c.arcs} arcs`, c.circles && `${c.circles} circles`, c.curves && `${c.curves} curves`].filter(Boolean).join(", ");
    return `Imported ${file} as sketch “${r.name}”: ${parts}, ${r.size.map((v) => v.toFixed(1)).join(" × ")} mm`;
  }
  if (r.kind === "ifc") return `Imported ${file}: ${r.bodies} building elements`;
  return `Imported ${file}: ${r.bodies} ${r.bodies === 1 ? "body" : "bodies"}${r.exact ? "" : " (mesh)"}`;
}
