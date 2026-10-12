// Files the engine imports (POST /import/<name>): exact solids, meshes and 2D drawings.

/** Extensions an import takes. */
export const IMPORT_EXTENSIONS = ["step", "stp", "iges", "igs", "stl", "obj", "ply", "gltf", "glb", "3mf", "dxf", "svg"];

/** For a file input's `accept`. */
export const IMPORT_ACCEPT = IMPORT_EXTENSIONS.map((e) => `.${e}`).join(",");

/** For messages: what can be imported. */
export const IMPORT_FORMATS = "STEP, IGES, STL, OBJ, PLY, glTF, 3MF, DXF or SVG";

/** Whether a file looks like something the engine imports (it reads the bytes to be sure). */
export const importable = (name: string) => IMPORT_EXTENSIONS.includes(name.split(".").pop()?.toLowerCase() ?? "");

/** What the engine says an import became. */
export type ImportReply =
  | { kind: "shape"; id: string; bodies: number; format: string; exact: boolean; colors?: { part: string; color: string }[] }
  | { kind: "sketch"; id: string; name: string; format: string; counts: { lines: number; arcs: number; circles: number; curves: number }; size: [number, number] };

/** One line for the person: what came in. */
export function importSummary(file: string, r: ImportReply): string {
  if (r.kind === "sketch") {
    const c = r.counts;
    const parts = [c.lines && `${c.lines} lines`, c.arcs && `${c.arcs} arcs`, c.circles && `${c.circles} circles`, c.curves && `${c.curves} curves`].filter(Boolean).join(", ");
    return `Imported ${file} as sketch “${r.name}”: ${parts}, ${r.size.map((v) => v.toFixed(1)).join(" × ")} mm`;
  }
  return `Imported ${file}: ${r.bodies} ${r.bodies === 1 ? "body" : "bodies"}${r.exact ? "" : " (mesh)"}`;
}
