// Iṣẹ́ Studio's bridge to Apple Object Capture (RealityKit's PhotogrammetrySession,
// macOS 12+ on Apple silicon or a Mac with a capable GPU).
//
// The engine builds this file with `swiftc` the first time it is needed and
// runs it on a folder of photos. Everything it says is one JSON object per
// line on stdout, so the engine can follow along:
//
//   ise-object-capture --check
//       {"supported": true|false, "os": "…"}
//   ise-object-capture <images> <out folder> [--detail preview|reduced|medium|full|raw] [--mask] [--no-poses]
//       {"stage": "imageAlignment", "progress": 0.31}     (repeated)
//       {"invalid": 3, "reason": "…"} / {"skipped": 4}     (photos it could not use)
//       {"depth": 12}                                      (photos carrying depth: the model is metric)
//       {"poses": [{"image": "IMG_1.jpg", "matrix": [16, column-major], "intrinsics": [9, column-major] | null}]}
//       {"points": "<out>/points.ply", "count": n}
//       {"model": "<out>/model.obj", "usdz": "<out>/model.usdz"}
//       {"done": true} or {"error": "…"}
//
// The model and poses are in metres (Object Capture's units); without depth
// data in the photos the size is only nominal and Iṣẹ́ Studio scales it from the
// marker sheet or a known distance.

import Foundation
import ImageIO
import ModelIO
import RealityKit

func say(_ object: [String: Any]) {
    if let data = try? JSONSerialization.data(withJSONObject: object), let line = String(data: data, encoding: .utf8) {
        print(line)
        fflush(stdout)
    }
}

func fail(_ message: String) -> Never {
    say(["error": message])
    exit(1)
}

let args = Array(CommandLine.arguments.dropFirst())
if args.first == "--check" {
    say(["supported": PhotogrammetrySession.isSupported, "os": ProcessInfo.processInfo.operatingSystemVersionString])
    exit(0)
}
guard args.count >= 2 else { fail("usage: ise-object-capture <images> <out folder> [--detail medium] [--mask] [--no-poses]") }
guard PhotogrammetrySession.isSupported else { fail("Object Capture is not supported on this Mac (it needs Apple silicon, or an AMD GPU with 4 GB and ray tracing)") }

let input = URL(fileURLWithPath: args[0], isDirectory: true)
let out = URL(fileURLWithPath: args[1], isDirectory: true)
try? FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
var detail = PhotogrammetrySession.Request.Detail.medium
if let i = args.firstIndex(of: "--detail"), i + 1 < args.count {
    switch args[i + 1] {
    case "preview": detail = .preview
    case "reduced": detail = .reduced
    case "medium": detail = .medium
    case "full": detail = .full
    case "raw": detail = .raw
    default: fail("detail is preview, reduced, medium, full or raw")
    }
}
let wantPoses = !args.contains("--no-poses")
// Two steps: the cameras first, then (after a line on stdin naming the box the object stands in) the model of that box only.
let askBounds = args.contains("--ask-bounds")

// Photos carrying depth (iPhone portrait / LiDAR captures) make the model metric.
var depthCount = 0
if let files = try? FileManager.default.contentsOfDirectory(at: input, includingPropertiesForKeys: nil) {
    for f in files {
        guard let src = CGImageSourceCreateWithURL(f as CFURL, nil) else { continue }
        if CGImageSourceCopyAuxiliaryDataInfoAtIndex(src, 0, kCGImageAuxiliaryDataTypeDepth) != nil || CGImageSourceCopyAuxiliaryDataInfoAtIndex(src, 0, kCGImageAuxiliaryDataTypeDisparity) != nil {
            depthCount += 1
        }
    }
}
if depthCount > 0 { say(["depth": depthCount]) }

var config = PhotogrammetrySession.Configuration()
config.isObjectMaskingEnabled = args.contains("--mask")
config.sampleOrdering = .unordered
config.featureSensitivity = .high
let session: PhotogrammetrySession
do {
    session = try PhotogrammetrySession(input: input, configuration: config)
} catch {
    fail("could not start Object Capture: \(error.localizedDescription)")
}

let usdz = out.appendingPathComponent("model.usdz")
var requests: [PhotogrammetrySession.Request] = askBounds ? [.pointCloud, .poses] : [.modelFile(url: usdz, detail: detail), .pointCloud]
if wantPoses && !askBounds { requests.append(.poses) }

/// The box to model, from a JSON line: {"min": [x, y, z], "max": [x, y, z]} in metres, axis-aligned in the
/// session's frame, and "oriented": {"rotation": [w, x, y, z], "min": …, "max": …}, the box in a frame turned
/// by that rotation (used from macOS 13); {} or nothing for the whole scene.
func geometry(_ line: String?) -> PhotogrammetrySession.Request.Geometry? {
    guard let data = line?.data(using: .utf8), let v = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
    func box(_ v: [String: Any]) -> BoundingBox? {
        guard let lo = v["min"] as? [Double], let hi = v["max"] as? [Double], lo.count == 3, hi.count == 3 else { return nil }
        return BoundingBox(min: SIMD3<Float>(lo.map { Float($0) }), max: SIMD3<Float>(hi.map { Float($0) }))
    }
    if #available(macOS 13.0, *), let o = v["oriented"] as? [String: Any], let q = o["rotation"] as? [Double], q.count == 4, let b = box(o) {
        return PhotogrammetrySession.Request.Geometry(orientedBounds: OrientedBoundingBox(orientation: simd_quatf(ix: Float(q[1]), iy: Float(q[2]), iz: Float(q[3]), r: Float(q[0])), boundingBox: b))
    }
    return box(v).map { PhotogrammetrySession.Request.Geometry(bounds: $0) }
}

func stageName(_ s: PhotogrammetrySession.Output.ProcessingStage?) -> String {
    guard let s else { return "processing" }
    switch s {
    case .preProcessing: return "preProcessing"
    case .imageAlignment: return "imageAlignment"
    case .pointCloudGeneration: return "pointCloudGeneration"
    case .meshGeneration: return "meshGeneration"
    case .textureMapping: return "textureMapping"
    case .optimization: return "optimization"
    @unknown default: return "processing"
    }
}

func writePoints(_ cloud: PhotogrammetrySession.PointCloud) {
    let url = out.appendingPathComponent("points.ply")
    var data = Data("ply\nformat binary_little_endian 1.0\nelement vertex \(cloud.points.count)\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n".utf8)
    data.reserveCapacity(data.count + cloud.points.count * 15)
    for p in cloud.points {
        var xyz = [p.position.x, p.position.y, p.position.z]
        data.append(Data(bytes: &xyz, count: 12))
        data.append(contentsOf: [p.color.x, p.color.y, p.color.z])
    }
    try? data.write(to: url)
    say(["points": url.path, "count": cloud.points.count])
}

func writePoses(_ poses: PhotogrammetrySession.Poses) {
    var list: [[String: Any]] = []
    let urls = poses.urlsBySample
    for (id, pose) in poses.posesBySample.sorted(by: { $0.key < $1.key }) {
        let m = pose.transform.matrix
        var entry: [String: Any] = ["sample": id, "matrix": [m.columns.0, m.columns.1, m.columns.2, m.columns.3].flatMap { [Double($0.x), Double($0.y), Double($0.z), Double($0.w)] }]
        if let url = urls[id] { entry["image"] = url.lastPathComponent }
        if let k = pose.intrinsics {
            entry["intrinsics"] = [k.columns.0, k.columns.1, k.columns.2].flatMap { [Double($0.x), Double($0.y), Double($0.z)] }
        }
        list.append(entry)
    }
    say(["poses": list])
}

func exportObj() {
    let asset = MDLAsset(url: usdz)
    let obj = out.appendingPathComponent("model.obj")
    do {
        try asset.export(to: obj)
        say(["model": obj.path, "usdz": usdz.path])
    } catch {
        say(["model": NSNull(), "usdz": usdz.path, "warning": "could not write OBJ: \(error.localizedDescription)"])
    }
}

let finished = DispatchSemaphore(value: 0)
var failed: String? = nil
Task {
    do {
        for try await output in session.outputs {
            switch output {
            case .requestProgress(_, let fraction):
                say(["progress": fraction])
            case .requestProgressInfo(_, let info):
                say(["stage": stageName(info.processingStage)])
            case .invalidSample(let id, let reason):
                say(["invalid": id, "reason": reason])
            case .skippedSample(let id):
                say(["skipped": id])
            case .automaticDownsampling:
                say(["note": "the photos were downsampled to fit in memory"])
            case .requestError(let request, let error):
                if case .modelFile = request { failed = "Object Capture could not build the model: \(error.localizedDescription)" }
                else { say(["warning": "\(request): \(error.localizedDescription)"]) }
            case .requestComplete(_, let result):
                switch result {
                case .modelFile: exportObj()
                case .pointCloud(let cloud): writePoints(cloud)
                case .poses(let poses): writePoses(poses)
                default: break
                }
            case .processingComplete:
                // Each process(requests:) ends so; the session stays for the next.
                finished.signal()
                if !askBounds { return }
            case .processingCancelled:
                failed = "cancelled"
                finished.signal()
                return
            case .inputComplete:
                say(["stage": "inputComplete"])
            case .stitchingIncomplete:
                say(["warning": "not every photo could be joined into one model (too little overlap)"])
            @unknown default:
                break
            }
        }
    } catch {
        failed = error.localizedDescription
    }
    finished.signal()
}

signal(SIGTERM) { _ in exit(130) }
do {
    try session.process(requests: requests)
} catch {
    fail("could not start processing: \(error.localizedDescription)")
}
finished.wait()
if let failed { fail(failed) }
if askBounds {
    say(["awaiting": "bounds"])
    let g = geometry(readLine())
    say(["stage": g == nil ? "modelling the whole scene" : "modelling the box the object stands in"])
    do {
        try session.process(requests: [.modelFile(url: usdz, detail: detail, geometry: g)])
    } catch {
        fail("could not start the model: \(error.localizedDescription)")
    }
    finished.wait()
    if let failed { fail(failed) }
}
say(["done": true])
