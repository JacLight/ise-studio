// A thin C layer over OpenCascade for Forma: shapes live in an arena and are
// addressed by integer handles, so Rust (and the shape-program runtime) can
// drive exact B-rep geometry with the same vocabulary it uses for meshes.
//
// Every entry point catches OpenCascade's exceptions and reports them through
// occ_error(); a negative handle means failure.

#include <array>
#include <algorithm>
#include <GC_MakeArcOfCircle.hxx>
#include <BRepTools_WireExplorer.hxx>
#include <HLRAlgo_Projector.hxx>
#include <HLRBRep_Algo.hxx>
#include <HLRBRep_HLRToShape.hxx>
#include <GeomConvert.hxx>
#include <GeomConvert_ApproxCurve.hxx>
#include <GeomConvert_BSplineCurveToBezierCurve.hxx>
#include <Geom_BSplineCurve.hxx>
#include <Geom_TrimmedCurve.hxx>
#include <Geom_BezierCurve.hxx>
#include <TColgp_Array1OfPnt.hxx>
#include <BRepAdaptor_Curve.hxx>
#include <BRepAdaptor_Surface.hxx>
#include <BRepAlgoAPI_Common.hxx>
#include <BRepAlgoAPI_Cut.hxx>
#include <BRepAlgoAPI_Fuse.hxx>
#include <BRepAlgoAPI_Section.hxx>
#include <BRepBndLib.hxx>
#include <BRepBuilderAPI_GTransform.hxx>
#include <BRepBuilderAPI_MakeEdge.hxx>
#include <BRepBuilderAPI_MakeFace.hxx>
#include <BRepBuilderAPI_MakePolygon.hxx>
#include <BRepBuilderAPI_MakeSolid.hxx>
#include <BRepBuilderAPI_MakeWire.hxx>
#include <BRepBuilderAPI_Sewing.hxx>
#include <BRepBuilderAPI_Transform.hxx>
#include <BRepCheck_Analyzer.hxx>
#include <BRepExtrema_DistShapeShape.hxx>
#include <BRepFilletAPI_MakeChamfer.hxx>
#include <BRepFilletAPI_MakeFillet.hxx>
#include <BRepGProp.hxx>
#include <BRepGProp_Face.hxx>
#include <BRepMesh_IncrementalMesh.hxx>
#include <BRepOffsetAPI_MakeOffset.hxx>
#include <BRepOffsetAPI_MakeThickSolid.hxx>
#include <BRepOffsetAPI_DraftAngle.hxx>
#include <TopTools_ListOfShape.hxx>
#include <gp_Pln.hxx>
#include <BRepOffset_MakeOffset.hxx>
#include <BRepBuilderAPI_MakeVertex.hxx>
#include <BRepOffsetAPI_ThruSections.hxx>
#include <BRepPrimAPI_MakeBox.hxx>
#include <BRepPrimAPI_MakeCone.hxx>
#include <BRepPrimAPI_MakeCylinder.hxx>
#include <BRepPrimAPI_MakePrism.hxx>
#include <BRepPrimAPI_MakeRevol.hxx>
#include <BRepPrimAPI_MakeSphere.hxx>
#include <BRepTools.hxx>
#include <BRep_Builder.hxx>
#include <BRep_Tool.hxx>
#include <Bnd_Box.hxx>
#include <GCPnts_TangentialDeflection.hxx>
#include <GProp_GProps.hxx>
#include <IFSelect_ReturnStatus.hxx>
#include <Interface_Static.hxx>
#include <Poly_Triangulation.hxx>
#include <IGESCAFControl_Reader.hxx>
#include <IGESCAFControl_Writer.hxx>
#include <Quantity_Color.hxx>
#include <STEPCAFControl_Reader.hxx>
#include <STEPCAFControl_Writer.hxx>
#include <STEPControl_StepModelType.hxx>
#include <ShapeFix_Shape.hxx>
#include <ShapeFix_Solid.hxx>
#include <ShapeUpgrade_UnifySameDomain.hxx>
#include <Standard_Failure.hxx>
#include <TCollection_ExtendedString.hxx>
#include <TDF_ChildIterator.hxx>
#include <TDF_Label.hxx>
#include <TDF_LabelSequence.hxx>
#include <TDataStd_Name.hxx>
#include <TDocStd_Document.hxx>
#include <TopExp.hxx>
#include <TopExp_Explorer.hxx>
#include <TopLoc_Location.hxx>
#include <TopTools_IndexedDataMapOfShapeListOfShape.hxx>
#include <TopTools_IndexedMapOfShape.hxx>
#include <TopTools_ListOfShape.hxx>
#include <TopoDS.hxx>
#include <TopoDS_Compound.hxx>
#include <TopoDS_Face.hxx>
#include <TopoDS_Shape.hxx>
#include <TopoDS_Shell.hxx>
#include <TopoDS_Solid.hxx>
#include <TopoDS_Wire.hxx>
#include <XCAFApp_Application.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <XCAFDoc_DocumentTool.hxx>
#include <XCAFDoc_ShapeTool.hxx>
#include <gp_Ax1.hxx>
#include <gp_Ax2.hxx>
#include <gp_Circ.hxx>
#include <gp_GTrsf.hxx>
#include <gp_Pln.hxx>
#include <gp_Trsf.hxx>

#include <Message.hxx>
#include <Message_Messenger.hxx>
#include <Message_PrinterOStream.hxx>

#include <cmath>
#include <cstring>
#include <tuple>
#include <string>
#include <vector>

namespace {

struct Arena {
  std::vector<TopoDS_Shape> shapes;
  std::vector<std::string> names;  // for shapes read from a file
  std::vector<double> colors;      // r, g, b per shape read from a file (-1: none)
  std::string error;
  std::vector<double> mesh_positions;
  std::vector<int> mesh_triangles;
  std::vector<double> edge_data;
  std::vector<double> loop_data;
  std::vector<double> view_data;
  // Faces or whole shapes given a colour (r g b, 0–1; r < 0 for none) and a name, for STEP (solids.cpp).
  std::vector<std::tuple<TopoDS_Shape, std::array<double, 3>, std::string>> painted;
};

int push(Arena* a, const TopoDS_Shape& s) {
  if (s.IsNull()) {
    a->error = "the operation produced nothing";
    return -1;
  }
  a->shapes.push_back(s);
  return (int)a->shapes.size() - 1;
}

const TopoDS_Shape* get(Arena* a, int h) {
  if (h < 0 || h >= (int)a->shapes.size()) {
    a->error = "unknown shape";
    return nullptr;
  }
  return &a->shapes[h];
}

template <typename F>
int guard(Arena* a, F f) {
  try {
    return f();
  } catch (Standard_Failure& e) {
    a->error = e.GetMessageString() && *e.GetMessageString() ? e.GetMessageString() : e.DynamicType()->Name();
    return -1;
  } catch (std::exception& e) {
    a->error = e.what();
    return -1;
  }
}

/// Rigid moves are recorded as a placement (no geometry copied): a part used
/// forty times costs one shape. Scaling goes through occ_transform's GTrsf path.
TopoDS_Shape transformed(const TopoDS_Shape& s, const gp_Trsf& t) { return BRepBuilderAPI_Transform(s, t, false).Shape(); }

/// Merges coplanar faces and collinear edges left by booleans, so exported
/// files read as a designer would have modelled them.
TopoDS_Shape tidy(const TopoDS_Shape& s) {
  ShapeUpgrade_UnifySameDomain u(s, true, true, true);
  u.Build();
  return u.Shape();
}

/// A planar face (on z = 0) from one closed polyline.
TopoDS_Face polygon_face(const double* xy, int n) {
  BRepBuilderAPI_MakePolygon poly;
  for (int i = 0; i < n; i++) poly.Add(gp_Pnt(xy[2 * i], xy[2 * i + 1], 0));
  poly.Close();
  return BRepBuilderAPI_MakeFace(poly.Wire(), true).Face();
}

double signed_area(const double* xy, int n) {
  double s = 0;
  for (int i = 0; i < n; i++) {
    int j = (i + 1) % n;
    s += xy[2 * i] * xy[2 * j + 1] - xy[2 * j] * xy[2 * i + 1];
  }
  return s / 2;
}

/// Each face of a shape (a profile may be several separate faces).
std::vector<TopoDS_Face> faces_of(const TopoDS_Shape& s) {
  std::vector<TopoDS_Face> out;
  for (TopExp_Explorer e(s, TopAbs_FACE); e.More(); e.Next()) out.push_back(TopoDS::Face(e.Current()));
  return out;
}

TopoDS_Shape fuse_all(const std::vector<TopoDS_Shape>& parts) {
  if (parts.empty()) return TopoDS_Shape();
  if (parts.size() == 1) return parts[0];
  TopTools_ListOfShape args, tools;
  args.Append(parts[0]);
  for (size_t i = 1; i < parts.size(); i++) tools.Append(parts[i]);
  BRepAlgoAPI_Fuse f;
  f.SetArguments(args);
  f.SetTools(tools);
  f.SetRunParallel(true);
  f.Build();
  if (!f.IsDone()) throw Standard_Failure("union failed");
  return f.Shape();
}

/// A wire scaled and turned about Z, lifted to height z (for twisted or tapered extrusions).
TopoDS_Wire section(const TopoDS_Wire& w, double z, double twist_deg, double sx, double sy) {
  gp_GTrsf g;
  g.SetValue(1, 1, sx);
  g.SetValue(2, 2, sy);
  TopoDS_Shape s = BRepBuilderAPI_GTransform(w, g, true).Shape();
  gp_Trsf r;
  r.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), twist_deg * M_PI / 180.0);
  gp_Trsf t;
  t.SetTranslation(gp_Vec(0, 0, z));
  return TopoDS::Wire(transformed(transformed(s, r), t));
}

/// Paint (colours, face names) onto a part's STEP label (solids.cpp).
void apply_paint(Arena* a, const Handle(XCAFDoc_ShapeTool) & tool, const Handle(XCAFDoc_ColorTool) & colours, const TDF_Label& label, const TopoDS_Shape& part);

}  // namespace

extern "C" {

void* occ_arena_new() {
  // OpenCascade reports every STEP transfer on the console; Forma reports its own way.
  static bool quiet = [] {
    Message::DefaultMessenger()->RemovePrinters(STANDARD_TYPE(Message_PrinterOStream));
    return true;
  }();
  (void)quiet;
  return new Arena();
}
void occ_arena_free(void* a) { delete static_cast<Arena*>(a); }
const char* occ_error(void* a) { return static_cast<Arena*>(a)->error.c_str(); }

// ---- Solids ------------------------------------------------------------------

int occ_box(void* p, double x, double y, double z, int center) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    gp_Pnt o = center ? gp_Pnt(-x / 2, -y / 2, -z / 2) : gp_Pnt(0, 0, 0);
    return push(a, BRepPrimAPI_MakeBox(o, x, y, z).Shape());
  });
}

int occ_cylinder(void* p, double h, double r1, double r2, int center) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    gp_Ax2 ax(gp_Pnt(0, 0, center ? -h / 2 : 0), gp_Dir(0, 0, 1));
    TopoDS_Shape s = std::fabs(r1 - r2) < 1e-12 ? BRepPrimAPI_MakeCylinder(ax, r1, h).Shape() : BRepPrimAPI_MakeCone(ax, r1, r2, h).Shape();
    return push(a, s);
  });
}

int occ_sphere(void* p, double r) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] { return push(a, BRepPrimAPI_MakeSphere(r).Shape()); });
}

/// op: 0 union, 1 difference, 2 intersection.
int occ_bool(void* p, int op, int x, int y) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, x);
    const TopoDS_Shape* t = get(a, y);
    if (!s || !t) return -1;
    TopoDS_Shape r;
    if (op == 0) {
      BRepAlgoAPI_Fuse f(*s, *t);
      if (!f.IsDone()) throw Standard_Failure("union failed");
      r = f.Shape();
    } else if (op == 1) {
      BRepAlgoAPI_Cut f(*s, *t);
      if (!f.IsDone()) throw Standard_Failure("difference failed");
      r = f.Shape();
    } else {
      BRepAlgoAPI_Common f(*s, *t);
      if (!f.IsDone()) throw Standard_Failure("intersection failed");
      r = f.Shape();
    }
    return push(a, tidy(r));
  });
}

int occ_union_all(void* p, const int* hs, int n) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    std::vector<TopoDS_Shape> parts;
    for (int i = 0; i < n; i++) {
      const TopoDS_Shape* s = get(a, hs[i]);
      if (!s) return -1;
      parts.push_back(*s);
    }
    return push(a, tidy(fuse_all(parts)));
  });
}

/// kind: 0 translate (x, y, z), 1 rotate (degrees about X, then Y, then Z),
/// 2 scale (x, y, z), 3 mirror (plane normal x, y, z through the origin).
int occ_transform(void* p, int h, int kind, double x, double y, double z) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (kind == 2 && !(std::fabs(x - y) < 1e-12 && std::fabs(y - z) < 1e-12)) {
      gp_GTrsf g;
      g.SetValue(1, 1, x);
      g.SetValue(2, 2, y);
      g.SetValue(3, 3, z);
      return push(a, BRepBuilderAPI_GTransform(*s, g, true).Shape());
    }
    gp_Trsf t;
    if (kind == 0) {
      t.SetTranslation(gp_Vec(x, y, z));
    } else if (kind == 1) {
      gp_Trsf rx, ry, rz;
      rx.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(1, 0, 0)), x * M_PI / 180.0);
      ry.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 1, 0)), y * M_PI / 180.0);
      rz.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), z * M_PI / 180.0);
      t = rz * ry * rx;
    } else if (kind == 2) {
      t.SetScale(gp_Pnt(0, 0, 0), x);
    } else {
      t.SetMirror(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(x, y, z)));
    }
    return push(a, transformed(*s, t));
  });
}

// ---- Profiles (planar faces on z = 0) -----------------------------------------

/// Contours as flat [x0, y0, x1, y1, …] arrays; counter-clockwise ones add,
/// clockwise ones cut (the same fill rule as the mesh kernel).
int occ_polygons(void* p, const double* xy, const int* counts, int n) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    std::vector<TopoDS_Shape> add, cut;
    const double* at = xy;
    for (int i = 0; i < n; i++) {
      (signed_area(at, counts[i]) >= 0 ? add : cut).push_back(polygon_face(at, counts[i]));
      at += 2 * counts[i];
    }
    if (add.empty()) {
      std::string m = "a profile needs at least one counter-clockwise outline (";
      const double* q = xy;
      for (int i = 0; i < n; i++) {
        m += std::to_string(counts[i]) + " points, area " + std::to_string(signed_area(q, counts[i])) + ":";
        for (int k = 0; k < counts[i] && k < 6; k++) m += " (" + std::to_string(q[2 * k]) + "," + std::to_string(q[2 * k + 1]) + ")";
        m += (i + 1 < n ? "; " : "");
        q += 2 * counts[i];
      }
      throw Standard_Failure((m + ")").c_str());
    }
    TopoDS_Shape s = fuse_all(add);
    for (auto& c : cut) s = BRepAlgoAPI_Cut(s, c).Shape();
    return push(a, tidy(s));
  });
}

/// A sketch's loops with true arcs: [n segments, then per segment: kind and
/// numbers] per loop. Kinds: 0 line (x0 y0 x1 y1), 1 arc (x0 y0 x1 y1 cx cy
/// ccw), 2 circle (cx cy r, negative r = clockwise), 3 cubic Bézier (x0 y0,
/// handle 1, handle 2, x1 y1). Counter-clockwise loops are outlines,
/// clockwise ones holes.
int occ_sketch_profile(void* p, int nloops, const double* d, int len) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    int at = 0;
    auto next = [&]() {
      if (at >= len) throw Standard_Failure("sketch data ends early");
      return d[at++];
    };
    std::vector<TopoDS_Shape> add, cut;
    for (int l = 0; l < nloops; l++) {
      int n = (int)next();
      BRepBuilderAPI_MakeWire wire;
      double signed2 = 0;  // twice the signed area of the loop's chord polygon, for its sense
      bool clockwise_circle = false;
      for (int k = 0; k < n; k++) {
        int kind = (int)next();
        if (kind == 0) {
          gp_Pnt p0(next(), next(), 0), p1(next(), next(), 0);
          wire.Add(BRepBuilderAPI_MakeEdge(p0, p1).Edge());
          signed2 += p0.X() * p1.Y() - p1.X() * p0.Y();
        } else if (kind == 1) {
          gp_Pnt p0(next(), next(), 0), p1(next(), next(), 0);
          gp_Pnt c(next(), next(), 0);
          bool ccw = next() != 0;
          gp_Circ circ(gp_Ax2(c, gp_Dir(0, 0, 1)), c.Distance(p0));
          // An arc runs counter-clockwise from its first point; a clockwise one is that arc reversed.
          Handle(Geom_TrimmedCurve) arc = ccw ? GC_MakeArcOfCircle(circ, p0, p1, true).Value() : GC_MakeArcOfCircle(circ, p1, p0, true).Value();
          TopoDS_Edge e = BRepBuilderAPI_MakeEdge(arc).Edge();
          wire.Add(ccw ? e : TopoDS::Edge(e.Reversed()));
          // The arc's sense counts through its middle point.
          double a0 = std::atan2(p0.Y() - c.Y(), p0.X() - c.X()), a1 = std::atan2(p1.Y() - c.Y(), p1.X() - c.X());
          if (ccw) { while (a1 <= a0) a1 += 2 * M_PI; } else { while (a1 >= a0) a1 -= 2 * M_PI; }
          double am = (a0 + a1) / 2, r = c.Distance(p0);
          gp_Pnt m(c.X() + r * std::cos(am), c.Y() + r * std::sin(am), 0);
          signed2 += p0.X() * m.Y() - m.X() * p0.Y() + m.X() * p1.Y() - p1.X() * m.Y();
        } else if (kind == 3) {
          // A true Bézier edge: exact to the STEP file.
          TColgp_Array1OfPnt poles(1, 4);
          for (int i = 1; i <= 4; i++) {
            double x = next(), y = next();
            poles.SetValue(i, gp_Pnt(x, y, 0));
          }
          wire.Add(BRepBuilderAPI_MakeEdge(new Geom_BezierCurve(poles)).Edge());
          // Its sense counts through points along it (the control polygon would mislead).
          gp_Pnt prev = poles(1);
          Handle(Geom_BezierCurve) bz = new Geom_BezierCurve(poles);
          for (int i = 1; i <= 16; i++) {
            gp_Pnt q = bz->Value(i / 16.0);
            signed2 += prev.X() * q.Y() - q.X() * prev.Y();
            prev = q;
          }
        } else if (kind == 2) {
          gp_Pnt c(next(), next(), 0);
          double r = next();
          clockwise_circle = r < 0;
          wire.Add(BRepBuilderAPI_MakeEdge(gp_Circ(gp_Ax2(c, gp_Dir(0, 0, 1)), std::fabs(r))).Edge());
          signed2 = r < 0 ? -1 : 1;
        } else {
          throw Standard_Failure("unknown sketch segment");
        }
      }
      if (!wire.IsDone()) throw Standard_Failure("a sketch loop does not close");
      TopoDS_Face f = BRepBuilderAPI_MakeFace(wire.Wire(), true).Face();
      bool hole = clockwise_circle || signed2 < 0;
      (hole ? cut : add).push_back(f);
    }
    if (add.empty()) throw Standard_Failure("the sketch has no closed outline");
    TopoDS_Shape s = fuse_all(add);
    for (auto& c : cut) s = BRepAlgoAPI_Cut(s, c).Shape();
    return push(a, tidy(s));
  });
}

/// The loops of a flat profile (z = 0) read back as sketch segments, in the
/// format occ_sketch_profile takes: per loop, the number of segments, then
/// kind and numbers (0 line, 1 arc counter-clockwise from its first point,
/// 2 circle, 3 cubic Bézier). Free-form edges (an offset Bézier, an ellipse)
/// become cubic Béziers within 1e-4 mm. Returns the number of loops.
int occ_profile_loops(void* p, int h, const double** data, int* len) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    std::vector<double>& out = a->loop_data;
    out.clear();
    int loops = 0;
    for (TopExp_Explorer fe(*s, TopAbs_FACE); fe.More(); fe.Next()) {
      TopoDS_Face face = TopoDS::Face(fe.Current());
      for (TopExp_Explorer we(face, TopAbs_WIRE); we.More(); we.Next()) {
        std::vector<double> segs;
        int n = 0;
        for (BRepTools_WireExplorer ex(TopoDS::Wire(we.Current()), face); ex.More(); ex.Next()) {
          const TopoDS_Edge& e = ex.Current();
          if (BRep_Tool::Degenerated(e)) continue;
          bool reversed = e.Orientation() == TopAbs_REVERSED;
          BRepAdaptor_Curve c(e);
          double t0 = c.FirstParameter(), t1 = c.LastParameter();
          gp_Pnt p0 = c.Value(reversed ? t1 : t0), p1 = c.Value(reversed ? t0 : t1);
          if (c.GetType() == GeomAbs_Line) {
            segs.insert(segs.end(), {0, p0.X(), p0.Y(), p1.X(), p1.Y()});
            n++;
          } else if (c.GetType() == GeomAbs_Circle) {
            gp_Circ ci = c.Circle();
            gp_Pnt ce = ci.Location();
            if (std::fabs(t1 - t0 - 2 * M_PI) < 1e-9) {
              segs.insert(segs.end(), {2, ce.X(), ce.Y(), ci.Radius()});
            } else {
              // Sketch arcs run counter-clockwise: with the axis down, the
              // parameter runs clockwise, so the arc runs from its last point.
              bool up = ci.Axis().Direction().Z() > 0;
              gp_Pnt s0 = c.Value(up ? t0 : t1), s1 = c.Value(up ? t1 : t0);
              segs.insert(segs.end(), {1, s0.X(), s0.Y(), s1.X(), s1.Y(), ce.X(), ce.Y(), 1});
            }
            n++;
          } else {
            Handle(Geom_Curve) g = BRep_Tool::Curve(e, t0, t1);
            Handle(Geom_BSplineCurve) bs = GeomConvert::CurveToBSplineCurve(new Geom_TrimmedCurve(g, t0, t1));
            if (bs->Degree() > 3 || bs->IsRational()) {
              GeomConvert_ApproxCurve approx(bs, 1e-4, GeomAbs_C1, 400, 3);
              if (!approx.HasResult()) throw Standard_Failure("a curve could not be turned into Béziers");
              bs = approx.Curve();
            }
            if (bs->Degree() < 3) bs->IncreaseDegree(3);
            GeomConvert_BSplineCurveToBezierCurve conv(bs);
            std::vector<std::array<gp_Pnt, 4>> parts;
            for (int i = 1; i <= conv.NbArcs(); i++) {
              Handle(Geom_BezierCurve) bz = conv.Arc(i);
              if (bz->Degree() < 3) bz->Increase(3);
              parts.push_back({bz->Pole(1), bz->Pole(2), bz->Pole(3), bz->Pole(4)});
            }
            if (reversed) {
              std::reverse(parts.begin(), parts.end());
              for (auto& q : parts) std::reverse(q.begin(), q.end());
            }
            for (auto& q : parts) {
              segs.push_back(3);
              for (auto& pt : q) segs.insert(segs.end(), {pt.X(), pt.Y()});
              n++;
            }
          }
        }
        if (!n) continue;
        out.push_back(n);
        out.insert(out.end(), segs.begin(), segs.end());
        loops++;
      }
    }
    *data = out.data();
    *len = (int)out.size();
    return loops;
  });
}

/// A view of a shape with hidden lines removed: looking along `dir` (from
/// the eye towards the shape) with `up` pointing up the page. Returns
/// polylines in view coordinates (x right, y up, mm): per polyline its kind
/// (0 visible edge, 1 visible outline of a curved face, 2 hidden edge, 3
/// hidden outline), its point count, then x y pairs. Curves are chords
/// within `deflection` mm.
int occ_hlr_view(void* p, int h, double dx, double dy, double dz, double ux, double uy, double uz, double deflection, const double** data, int* len) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    gp_Dir view(-dx, -dy, -dz);  // the projector looks along -Z of its frame
    gp_Dir up(ux, uy, uz);
    gp_Dir right = up.Crossed(view);
    gp_Ax2 frame(gp_Pnt(0, 0, 0), view, right);
    HLRAlgo_Projector projector(frame);
    Handle(HLRBRep_Algo) algo = new HLRBRep_Algo();
    algo->Add(*s);
    algo->Projector(projector);
    algo->Update();
    algo->Hide();
    HLRBRep_HLRToShape out(algo);
    std::vector<double>& d = a->view_data;
    d.clear();
    int count = 0;
    auto take = [&](const TopoDS_Shape& c, int kind) {
      if (c.IsNull()) return;
      for (TopExp_Explorer e(c, TopAbs_EDGE); e.More(); e.Next()) {
        const TopoDS_Edge& edge = TopoDS::Edge(e.Current());
        if (BRep_Tool::Degenerated(edge)) continue;
        BRepAdaptor_Curve curve(edge);
        std::vector<gp_Pnt> pts;
        if (curve.GetType() == GeomAbs_Line) {
          pts = {curve.Value(curve.FirstParameter()), curve.Value(curve.LastParameter())};
        } else {
          GCPnts_TangentialDeflection t(curve, 0.1, std::max(deflection, 1e-3));
          for (int k = 1; k <= t.NbPoints(); k++) pts.push_back(t.Value(k));
        }
        if (pts.size() < 2) continue;
        d.push_back(kind);
        d.push_back((double)pts.size());
        for (auto& q : pts) d.insert(d.end(), {q.X(), q.Y()});
        count++;
      }
    };
    take(out.VCompound(), 0);
    take(out.OutLineVCompound(), 1);
    take(out.HCompound(), 2);
    take(out.OutLineHCompound(), 3);
    *data = d.data();
    *len = (int)d.size();
    return count;
  });
}

/// Where a shape crosses a plane: the section's edges (true lines, circles
/// and curves where the plane cuts the faces), as one compound.
int occ_section(void* p, int h, double ox, double oy, double oz, double nx, double ny, double nz) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    BRepAlgoAPI_Section sec(*s, gp_Pln(gp_Pnt(ox, oy, oz), gp_Dir(nx, ny, nz)), Standard_False);
    sec.Approximation(Standard_True);
    sec.Build();
    if (!sec.IsDone()) throw Standard_Failure("could not cut the shape with the plane");
    return push(a, sec.Shape());
  });
}

/// Shapes gathered into one compound, untouched (no boolean).
int occ_compound(void* p, const int* hs, int n) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    BRep_Builder b;
    TopoDS_Compound c;
    b.MakeCompound(c);
    for (int i = 0; i < n; i++) {
      const TopoDS_Shape* s = get(a, hs[i]);
      if (!s) return -1;
      b.Add(c, *s);
    }
    return push(a, c);
  });
}

int occ_circle(void* p, double r) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    gp_Circ c(gp_Ax2(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), r);
    TopoDS_Wire w = BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(c).Edge()).Wire();
    return push(a, BRepBuilderAPI_MakeFace(w, true).Face());
  });
}

int occ_rect(void* p, double x, double y, int center) {
  double x0 = center ? -x / 2 : 0, y0 = center ? -y / 2 : 0;
  double pts[8] = {x0, y0, x0 + x, y0, x0 + x, y0 + y, x0, y0 + y};
  int counts[1] = {4};
  return occ_polygons(p, pts, counts, 1);
}

/// join: 0 square, 1 round, 2 miter, 3 bevel.
int occ_offset(void* p, int h, double d, int join) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (std::fabs(d) < 1e-12) return push(a, *s);
    std::vector<TopoDS_Shape> out;
    for (auto& f : faces_of(*s)) {
      BRepOffsetAPI_MakeOffset off(f, join == 1 ? GeomAbs_Arc : GeomAbs_Intersection);
      off.Perform(d);
      if (!off.IsDone()) throw Standard_Failure("offset failed");
      // The result is the offset wires: the outer one bounds the face, the rest are holes.
      std::vector<TopoDS_Wire> wires;
      for (TopExp_Explorer e(off.Shape(), TopAbs_WIRE); e.More(); e.Next()) wires.push_back(TopoDS::Wire(e.Current()));
      if (wires.empty()) continue;  // shrunk away
      // The outer wire encloses the largest area.
      size_t outer = 0;
      double best = -1;
      for (size_t i = 0; i < wires.size(); i++) {
        GProp_GProps g;
        TopoDS_Face wf = BRepBuilderAPI_MakeFace(wires[i], true).Face();
        BRepGProp::SurfaceProperties(wf, g);
        if (std::fabs(g.Mass()) > best) best = std::fabs(g.Mass()), outer = i;
      }
      BRepBuilderAPI_MakeFace mf(wires[outer], true);
      for (size_t i = 0; i < wires.size(); i++)
        if (i != outer) mf.Add(TopoDS::Wire(wires[i].Reversed()));
      TopoDS_Shape face = mf.Shape();
      ShapeFix_Shape fix(face);
      fix.Perform();
      out.push_back(fix.Shape());
    }
    if (out.empty()) throw Standard_Failure("the offset leaves nothing");
    return push(a, tidy(fuse_all(out)));
  });
}

/// A profile or solid moved in its own plane / space: same kinds as occ_transform.
int occ_extrude(void* p, int h, double height, double twist_deg, double sx, double sy, int slices) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (std::fabs(twist_deg) < 1e-12 && std::fabs(sx - 1) < 1e-12 && std::fabs(sy - 1) < 1e-12) {
      return push(a, BRepPrimAPI_MakePrism(*s, gp_Vec(0, 0, height)).Shape());
    }
    // Twisted or tapered: a smooth loft through sections (each outer and inner
    // wire separately). The surfaces are B-splines through the sections, so a
    // section per 15° of twist follows the helix closely; the mesh kernel's
    // `slices` (straight facets between sections) would only add faces here.
    (void)slices;
    int n = std::max(2, (int)std::ceil(std::fabs(twist_deg) / 15.0));
    std::vector<TopoDS_Shape> solids, holes;
    for (auto& f : faces_of(*s)) {
      TopoDS_Wire outer = BRepTools::OuterWire(f);
      for (TopExp_Explorer e(f, TopAbs_WIRE); e.More(); e.Next()) {
        TopoDS_Wire w = TopoDS::Wire(e.Current());
        BRepOffsetAPI_ThruSections loft(true, false);
        for (int i = 0; i <= n; i++) {
          double t = (double)i / n;
          loft.AddWire(section(w, t * height, t * twist_deg, 1 + t * (sx - 1), 1 + t * (sy - 1)));
        }
        loft.Build();
        (w.IsSame(outer) ? solids : holes).push_back(loft.Shape());
      }
    }
    TopoDS_Shape r = fuse_all(solids);
    for (auto& hole : holes) r = BRepAlgoAPI_Cut(r, hole).Shape();
    return push(a, r);
  });
}

/// The profile's x is the radius and its y the height along Z (as in the mesh kernel).
int occ_revolve(void* p, int h, double degrees) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    gp_Trsf up;
    up.SetRotation(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(1, 0, 0)), M_PI / 2);  // y → z
    TopoDS_Shape standing = transformed(*s, up);
    double angle = std::fabs(degrees) >= 360 ? 2 * M_PI : degrees * M_PI / 180.0;
    return push(a, BRepPrimAPI_MakeRevol(standing, gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1)), angle).Shape());
  });
}

/// A closed triangle mesh as a (faceted) solid: for geometry a program computed itself.
int occ_from_mesh(void* p, const double* pos, int npos, const int* tri, int ntri) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    BRepBuilderAPI_Sewing sew(1e-6);
    for (int t = 0; t < ntri; t++) {
      gp_Pnt q[3];
      for (int k = 0; k < 3; k++) {
        int i = tri[3 * t + k];
        if (i < 0 || i >= npos) throw Standard_Failure("triangle index out of range");
        q[k] = gp_Pnt(pos[3 * i], pos[3 * i + 1], pos[3 * i + 2]);
      }
      if (q[0].Distance(q[1]) < 1e-12 || q[1].Distance(q[2]) < 1e-12 || q[0].Distance(q[2]) < 1e-12) continue;
      BRepBuilderAPI_MakePolygon poly(q[0], q[1], q[2], true);
      sew.Add(BRepBuilderAPI_MakeFace(poly.Wire(), true).Face());
    }
    sew.Perform();
    TopoDS_Shape sewn = sew.SewedShape();
    TopoDS_Shell shell;
    for (TopExp_Explorer e(sewn, TopAbs_SHELL); e.More(); e.Next()) {
      shell = TopoDS::Shell(e.Current());
      break;
    }
    if (shell.IsNull()) throw Standard_Failure("the mesh does not close into a shell");
    BRepBuilderAPI_MakeSolid ms(shell);
    ShapeFix_Solid fix(ms.Solid());
    fix.Perform();
    return push(a, fix.Solid());
  });
}

// ---- Queries -------------------------------------------------------------------

double occ_volume(void* p, int h) {
  Arena* a = static_cast<Arena*>(p);
  const TopoDS_Shape* s = get(a, h);
  if (!s) return NAN;
  GProp_GProps g;
  BRepGProp::VolumeProperties(*s, g);
  return g.Mass();
}

double occ_area(void* p, int h) {
  Arena* a = static_cast<Arena*>(p);
  const TopoDS_Shape* s = get(a, h);
  if (!s) return NAN;
  GProp_GProps g;
  BRepGProp::SurfaceProperties(*s, g);
  return g.Mass();
}

int occ_bounds(void* p, int h, double* out) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    Bnd_Box b;
    BRepBndLib::AddOptimal(*s, b, false, false);
    if (b.IsVoid()) {
      for (int i = 0; i < 6; i++) out[i] = 0;
      return 0;
    }
    b.Get(out[0], out[1], out[2], out[3], out[4], out[5]);
    return 0;
  });
}

/// Smallest distance between two shapes (0 when they touch or overlap).
double occ_distance(void* p, int x, int y) {
  Arena* a = static_cast<Arena*>(p);
  const TopoDS_Shape* s = get(a, x);
  const TopoDS_Shape* t = get(a, y);
  if (!s || !t) return NAN;
  try {
    BRepExtrema_DistShapeShape d(*s, *t);
    return d.IsDone() ? d.Value() : NAN;
  } catch (Standard_Failure&) {
    return NAN;
  }
}

/// Triangulates a shape so no facet strays more than `deflection` mm from the
/// true surface. Results stay in the arena until the next call: positions
/// (x, y, z per vertex) and triangles (three indices each).
int occ_mesh(void* p, int h, double deflection, const double** positions, int* npos, const int** triangles, int* ntri) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    BRepMesh_IncrementalMesh mesher(*s, deflection, false, 0.2, true);
    a->mesh_positions.clear();
    a->mesh_triangles.clear();
    for (TopExp_Explorer e(*s, TopAbs_FACE); e.More(); e.Next()) {
      TopoDS_Face f = TopoDS::Face(e.Current());
      TopLoc_Location loc;
      Handle(Poly_Triangulation) tri = BRep_Tool::Triangulation(f, loc);
      if (tri.IsNull()) continue;
      int base = (int)a->mesh_positions.size() / 3;
      gp_Trsf t = loc.Transformation();
      for (int i = 1; i <= tri->NbNodes(); i++) {
        gp_Pnt q = tri->Node(i).Transformed(t);
        a->mesh_positions.push_back(q.X());
        a->mesh_positions.push_back(q.Y());
        a->mesh_positions.push_back(q.Z());
      }
      bool reversed = f.Orientation() == TopAbs_REVERSED;
      for (int i = 1; i <= tri->NbTriangles(); i++) {
        int n1, n2, n3;
        tri->Triangle(i).Get(n1, n2, n3);
        if (reversed) std::swap(n2, n3);
        a->mesh_triangles.push_back(base + n1 - 1);
        a->mesh_triangles.push_back(base + n2 - 1);
        a->mesh_triangles.push_back(base + n3 - 1);
      }
    }
    *positions = a->mesh_positions.data();
    *npos = (int)a->mesh_positions.size() / 3;
    *triangles = a->mesh_triangles.data();
    *ntri = (int)a->mesh_triangles.size() / 3;
    return 0;
  });
}

// ---- STEP and IGES ---------------------------------------------------------------

/// Writes shapes to a STEP (AP214) or IGES file in millimetres, each named,
/// so a shop sees "Left side", "Stretcher", … rather than anonymous solids.
/// `colors` (r, g, b in 0…1 per shape, sRGB; negative for none) may be null.
int occ_write_cad(void* p, const int* hs, const char* const* names, const double* colors, int n, const char* path, int iges) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    Handle(XCAFApp_Application) app = XCAFApp_Application::GetApplication();
    Handle(TDocStd_Document) doc;
    app->NewDocument("MDTV-XCAF", doc);
    Handle(XCAFDoc_ShapeTool) tool = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
    Handle(XCAFDoc_ColorTool) paint = XCAFDoc_DocumentTool::ColorTool(doc->Main());
    // Parts carry the names Forma gives them, nothing auto-generated ("SOLID", "COMPOUND").
    XCAFDoc_ShapeTool::SetAutoNaming(false);
    for (int i = 0; i < n; i++) {
      const TopoDS_Shape* s = get(a, hs[i]);
      if (!s) return -1;
      // A part is one entry: a single solid goes in as itself (booleans often
      // wrap it in a compound), and its sub-shapes are not split out.
      TopoDS_Shape part = *s;
      int solids = 0;
      TopoDS_Shape only;
      for (TopExp_Explorer e(part, TopAbs_SOLID); e.More(); e.Next()) solids++, only = e.Current();
      if (solids == 1) part = only;
      // A placed part's placement goes into its geometry: a located shape
      // would go in as an unnamed product ("Open CASCADE STEP translator …").
      const size_t painted = a->painted.size();
      if (!part.Location().IsIdentity()) {
        const TopoDS_Shape placed = part;
        const TopoDS_Shape unplaced = part.Located(TopLoc_Location());
        BRepBuilderAPI_Transform xf(unplaced, placed.Location().Transformation(), true);
        part = xf.Shape();
        // Paint follows the geometry into its placed copy (faces by their images).
        for (size_t k = 0; k < painted; k++) {
          auto entry = a->painted[k];
          const TopoDS_Shape& shape = std::get<0>(entry);
          if (shape.ShapeType() != TopAbs_FACE) {
            bool same = shape.IsPartner(placed);
            for (TopExp_Explorer e(shape, TopAbs_SOLID); !same && e.More(); e.Next()) same = e.Current().IsPartner(placed);
            if (same) a->painted.push_back({part, std::get<1>(entry), std::get<2>(entry)});
            continue;
          }
          for (TopExp_Explorer e(unplaced, TopAbs_FACE); e.More(); e.Next())
            if (shape.IsPartner(e.Current())) a->painted.push_back({xf.ModifiedShape(e.Current()), std::get<1>(entry), std::get<2>(entry)});
        }
      }
      TDF_Label l = tool->AddShape(part, false, false);
      TDataStd_Name::Set(l, TCollection_ExtendedString(names[i], true));
      if (colors && colors[3 * i] >= 0)
        paint->SetColor(l, Quantity_Color(colors[3 * i], colors[3 * i + 1], colors[3 * i + 2], Quantity_TOC_sRGB), XCAFDoc_ColorGen);
      // What the program painted (the part, or single faces) wins over its material's look.
      apply_paint(a, tool, paint, l, part);
      a->painted.resize(painted);
    }
    if (iges) {
      Interface_Static::SetCVal("write.iges.unit", "MM");
      IGESCAFControl_Writer w;
      w.SetNameMode(true);
      w.SetColorMode(true);
      if (!w.Transfer(doc)) throw Standard_Failure("could not translate the shapes to IGES");
      if (!w.Write(path)) throw Standard_Failure("could not write the IGES file");
    } else {
      Interface_Static::SetCVal("write.step.unit", "MM");
      Interface_Static::SetCVal("write.step.schema", "AP214IS");
      STEPCAFControl_Writer w;
      w.SetNameMode(true);
      w.SetColorMode(true);
      if (!w.Transfer(doc, STEPControl_AsIs)) throw Standard_Failure("could not translate the shapes to STEP");
      if (w.Write(path) != IFSelect_RetDone) throw Standard_Failure("could not write the STEP file");
    }
    app->Close(doc);
    return 0;
  });
}

int occ_write_step(void* p, const int* hs, const char* const* names, int n, const char* path) {
  return occ_write_cad(p, hs, names, nullptr, n, path, 0);
}

namespace {
/// The colour a label carries (surface, else generic), if any.
bool label_color(const TDF_Label& l, Quantity_Color& c) {
  return XCAFDoc_ColorTool::GetColor(l, XCAFDoc_ColorSurf, c) || XCAFDoc_ColorTool::GetColor(l, XCAFDoc_ColorGen, c);
}

/// A body's own colour (set on it as a sub-shape), else its first coloured face's.
bool shape_color(const Handle(XCAFDoc_ColorTool) & paint, const TopoDS_Shape& s, Quantity_Color& c) {
  if (paint->GetColor(s, XCAFDoc_ColorSurf, c) || paint->GetColor(s, XCAFDoc_ColorGen, c)) return true;
  for (TopExp_Explorer e(s, TopAbs_FACE); e.More(); e.Next())
    if (paint->GetColor(e.Current(), XCAFDoc_ColorSurf, c) || paint->GetColor(e.Current(), XCAFDoc_ColorGen, c)) return true;
  return false;
}

void push_body(Arena* a, const TopoDS_Shape& s, const std::string& name, bool has, const Quantity_Color& c) {
  a->shapes.push_back(s);
  a->names.push_back(name);
  a->colors.resize(a->shapes.size() * 3, -1.0);
  if (has) {
    Standard_Real r, g, b;
    c.Values(r, g, b, Quantity_TOC_sRGB);
    size_t i = a->colors.size() - 3;
    a->colors[i] = r, a->colors[i + 1] = g, a->colors[i + 2] = b;
  }
}

/// Faces with no solid around them (IGES carries surfaces, and some STEP
/// files do too) are sewn, and made solids where they close.
static TopoDS_Shape closed(const TopoDS_Shape& s) {
  if (TopExp_Explorer(s, TopAbs_SOLID).More() || !TopExp_Explorer(s, TopAbs_FACE).More()) return s;
  BRepBuilderAPI_Sewing sew(1e-3);
  sew.Add(s);
  sew.Perform();
  TopoDS_Shape sewn = sew.SewedShape();
  BRep_Builder b;
  TopoDS_Compound out;
  b.MakeCompound(out);
  int made = 0;
  for (TopExp_Explorer e(sewn, TopAbs_SHELL); e.More(); e.Next()) {
    BRepBuilderAPI_MakeSolid ms(TopoDS::Shell(e.Current()));
    if (!ms.IsDone()) continue;
    TopoDS_Solid solid = ms.Solid();
    // Outward faces: a solid of negative volume is turned inside out.
    GProp_GProps g;
    BRepGProp::VolumeProperties(solid, g);
    if (g.Mass() < 0) {
      solid.Reverse();
      BRepGProp::VolumeProperties(solid, g);
    }
    if (g.Mass() > 0 && BRepCheck_Analyzer(solid).IsValid()) b.Add(out, solid), made++;
  }
  return made ? TopoDS_Shape(out) : s;
}

void collect(const Handle(XCAFDoc_ShapeTool) & tool, const Handle(XCAFDoc_ColorTool) & paint, const TDF_Label& l, const TopLoc_Location& at, const std::string& prefix,
             bool inherited, const Quantity_Color& inherit, Arena* a) {
  Handle(TDataStd_Name) nm;
  std::string name;
  if (l.FindAttribute(TDataStd_Name::GetID(), nm)) {
    TCollection_AsciiString s(nm->Get(), '_');
    name = s.ToCString();
  }
  // Colours pass down an assembly: the nearest one set wins.
  Quantity_Color color = inherit;
  bool has = label_color(l, color) || inherited;
  TDF_Label ref = l;
  TopLoc_Location loc = at;
  if (XCAFDoc_ShapeTool::IsReference(l)) {
    XCAFDoc_ShapeTool::GetReferredShape(l, ref);
    loc = at * XCAFDoc_ShapeTool::GetLocation(l);
    if (name.empty() && ref.FindAttribute(TDataStd_Name::GetID(), nm)) {
      TCollection_AsciiString s(nm->Get(), '_');
      name = s.ToCString();
    }
    Quantity_Color own;
    if (!label_color(l, own) && label_color(ref, own)) color = own, has = true;
  }
  // A component named like the assembly holding it adds nothing to the path.
  std::string last = prefix.substr(prefix.rfind(" / ") == std::string::npos ? 0 : prefix.rfind(" / ") + 3);
  std::string full = prefix.empty() ? name : (name.empty() || name == last ? prefix : prefix + " / " + name);
  if (XCAFDoc_ShapeTool::IsAssembly(ref)) {
    TDF_LabelSequence comps;
    XCAFDoc_ShapeTool::GetComponents(ref, comps);
    for (int i = 1; i <= comps.Length(); i++) collect(tool, paint, comps.Value(i), loc, full, has, color, a);
    return;
  }
  TopoDS_Shape s = XCAFDoc_ShapeTool::GetShape(ref);
  if (s.IsNull()) return;
  // One entry per solid (a part file may hold several bodies).
  int k = 0;
  for (TopExp_Explorer e(s, TopAbs_SOLID); e.More(); e.Next(), k++) {
    Quantity_Color own = color;
    bool mine = shape_color(paint, e.Current(), own);
    push_body(a, e.Current().Moved(loc), full.empty() ? "Imported body" : full, mine || has, own);
  }
  if (k == 0) {
    Quantity_Color own = color;
    bool mine = shape_color(paint, s, own);
    TopoDS_Shape c = closed(s);
    if (TopExp_Explorer(c, TopAbs_SOLID).More())
      for (TopExp_Explorer e(c, TopAbs_SOLID); e.More(); e.Next()) push_body(a, e.Current().Moved(loc), full.empty() ? "Imported body" : full, mine || has, own);
    else
      push_body(a, s.Moved(loc), full.empty() ? "Imported shape" : full, mine || has, own);
  }
}
}  // namespace

/// Reads a STEP or IGES file (any units; converted to millimetres). Returns
/// the number of bodies; they are the arena's shapes from index `*first` on,
/// with names from occ_name() and colours from occ_color().
int occ_read_cad(void* p, const char* path, int iges, int* first) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    Handle(XCAFApp_Application) app = XCAFApp_Application::GetApplication();
    Handle(TDocStd_Document) doc;
    app->NewDocument("MDTV-XCAF", doc);
    if (iges) {
      IGESCAFControl_Reader r;
      r.SetNameMode(true);
      r.SetColorMode(true);
      if (r.ReadFile(path) != IFSelect_RetDone) throw Standard_Failure("not a readable IGES file");
      if (!r.Transfer(doc)) throw Standard_Failure("could not translate the IGES file");
    } else {
      STEPCAFControl_Reader r;
      r.SetNameMode(true);
      r.SetColorMode(true);
      if (r.ReadFile(path) != IFSelect_RetDone) throw Standard_Failure("not a readable STEP file");
      if (!r.Transfer(doc)) throw Standard_Failure("could not translate the STEP file");
    }
    Handle(XCAFDoc_ShapeTool) tool = XCAFDoc_DocumentTool::ShapeTool(doc->Main());
    Handle(XCAFDoc_ColorTool) paint = XCAFDoc_DocumentTool::ColorTool(doc->Main());
    TDF_LabelSequence free;
    tool->GetFreeShapes(free);
    // Names and colours line up with shapes from here on.
    a->names.resize(a->shapes.size());
    a->colors.resize(a->shapes.size() * 3, -1.0);
    *first = (int)a->shapes.size();
    for (int i = 1; i <= free.Length(); i++) collect(tool, paint, free.Value(i), TopLoc_Location(), "", false, Quantity_Color(), a);
    app->Close(doc);
    return (int)a->shapes.size() - *first;
  });
}

int occ_read_step(void* p, const char* path, int* first) { return occ_read_cad(p, path, 0, first); }

/// A read body's colour (sRGB, 0…1) into `out`: 1 if it has one, else 0.
int occ_color(void* p, int h, double* out) {
  Arena* a = static_cast<Arena*>(p);
  if (h < 0 || 3 * h + 2 >= (int)a->colors.size() || a->colors[3 * h] < 0) return 0;
  for (int k = 0; k < 3; k++) out[k] = a->colors[3 * h + k];
  return 1;
}

const char* occ_name(void* p, int h) {
  Arena* a = static_cast<Arena*>(p);
  if (h < 0 || h >= (int)a->names.size()) return "";
  return a->names[h].c_str();
}

// ---- Edges -----------------------------------------------------------------------

/// Every edge of a shape, numbered as TopExp::MapShapes numbers them (from 0),
/// for picking and for fillet/chamfer. Per edge: kind (0 line, 1 circle,
/// 2 other curve, -1 not roundable: a seam or a degenerate edge), seven
/// facts (line: direction x y z, then 0 0 0, then length; circle: center
/// x y z, axis x y z, radius; other: zeros and length), then the number of
/// polyline points and their x y z (within `deflection` mm of the curve).
int occ_edges(void* p, int h, double deflection, const double** data, int* len) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(*s, TopAbs_EDGE, edges);
    TopTools_IndexedDataMapOfShapeListOfShape faces;
    TopExp::MapShapesAndAncestors(*s, TopAbs_EDGE, TopAbs_FACE, faces);
    std::vector<double>& out = a->edge_data;
    out.clear();
    for (int i = 1; i <= edges.Extent(); i++) {
      TopoDS_Edge e = TopoDS::Edge(edges(i));
      double kind = 2, facts[7] = {0, 0, 0, 0, 0, 0, 0};
      bool seam = false;
      if (faces.Contains(e))
        for (TopTools_ListIteratorOfListOfShape f(faces.FindFromKey(e)); f.More(); f.Next())
          if (BRep_Tool::IsClosed(e, TopoDS::Face(f.Value()))) seam = true;
      std::vector<gp_Pnt> pts;
      if (BRep_Tool::Degenerated(e) || seam) {
        kind = -1;
      }
      if (!BRep_Tool::Degenerated(e)) {
        BRepAdaptor_Curve c(e);
        GProp_GProps g;
        BRepGProp::LinearProperties(e, g);
        double length = g.Mass();
        if (c.GetType() == GeomAbs_Line) {
          gp_Pnt a0 = c.Value(c.FirstParameter()), a1 = c.Value(c.LastParameter());
          pts = {a0, a1};
          gp_Vec d(a0, a1);
          if (d.Magnitude() > 1e-12) d.Normalize();
          if (kind != -1) kind = 0;
          facts[0] = d.X(), facts[1] = d.Y(), facts[2] = d.Z(), facts[6] = length;
        } else {
          if (c.GetType() == GeomAbs_Circle) {
            gp_Circ ci = c.Circle();
            if (kind != -1) kind = 1;
            facts[0] = ci.Location().X(), facts[1] = ci.Location().Y(), facts[2] = ci.Location().Z();
            facts[3] = ci.Axis().Direction().X(), facts[4] = ci.Axis().Direction().Y(), facts[5] = ci.Axis().Direction().Z();
            facts[6] = ci.Radius();
          } else {
            facts[6] = length;
          }
          GCPnts_TangentialDeflection d(c, 0.15, std::max(deflection, 1e-4));
          for (int k = 1; k <= d.NbPoints(); k++) pts.push_back(d.Value(k));
        }
      }
      out.push_back(kind);
      for (double f : facts) out.push_back(f);
      out.push_back((double)pts.size());
      for (const gp_Pnt& q : pts) {
        out.push_back(q.X());
        out.push_back(q.Y());
        out.push_back(q.Z());
      }
    }
    *data = out.data();
    *len = (int)out.size();
    return edges.Extent();
  });
}

/// Rounds (kind 0, `size` the radius) or bevels (kind 1, `size` the distance
/// back along each face) the edges numbered as occ_edges numbers them.
int occ_blend(void* p, int h, int kind, double size, const int* idx, int n) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (!(size > 0)) throw Standard_Failure(kind == 0 ? "the radius must be positive" : "the distance must be positive");
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(*s, TopAbs_EDGE, edges);
    const char* what = kind == 0 ? "fillet" : "chamfer";
    TopoDS_Shape r;
    if (kind == 0) {
      BRepFilletAPI_MakeFillet f(*s);
      for (int i = 0; i < n; i++) {
        if (idx[i] < 0 || idx[i] >= edges.Extent()) throw Standard_Failure("unknown edge");
        f.Add(size, TopoDS::Edge(edges(idx[i] + 1)));
      }
      f.Build();
      if (!f.IsDone()) throw Standard_Failure("these edges cannot take that radius (too large for the faces next to them?)");
      r = f.Shape();
    } else {
      BRepFilletAPI_MakeChamfer f(*s);
      for (int i = 0; i < n; i++) {
        if (idx[i] < 0 || idx[i] >= edges.Extent()) throw Standard_Failure("unknown edge");
        f.Add(size, TopoDS::Edge(edges(idx[i] + 1)));
      }
      f.Build();
      if (!f.IsDone()) throw Standard_Failure("these edges cannot take that chamfer (too large for the faces next to them?)");
      r = f.Shape();
    }
    if (!BRepCheck_Analyzer(r).IsValid()) throw Standard_Failure(std::string(what) == "fillet" ? "the rounded result is not a valid solid (try a smaller radius)" : "the bevelled result is not a valid solid (try a smaller distance)");
    return push(a, r);
  });
}

// ---- Faces --------------------------------------------------------------------

/// The flat faces of a shape, numbered as TopExp::MapShapes numbers faces
/// (from 0). Per face, 8 numbers: 1 if planar (else 0), the outward normal
/// x y z, the centre of area x y z, and the area. Results stay in the arena.
int occ_faces(void* p, int h, const double** data, int* len) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(*s, TopAbs_FACE, faces);
    std::vector<double>& out = a->edge_data;
    out.clear();
    for (int i = 1; i <= faces.Extent(); i++) {
      TopoDS_Face f = TopoDS::Face(faces(i));
      BRepAdaptor_Surface surf(f);
      GProp_GProps g;
      BRepGProp::SurfaceProperties(f, g);
      gp_Pnt c = g.CentreOfMass();
      double planar = 0, nx = 0, ny = 0, nz = 0;
      if (surf.GetType() == GeomAbs_Plane) {
        planar = 1;
        gp_Dir n = surf.Plane().Axis().Direction();
        if (f.Orientation() == TopAbs_REVERSED) n.Reverse();
        nx = n.X(), ny = n.Y(), nz = n.Z();
      }
      for (double v : {planar, nx, ny, nz, c.X(), c.Y(), c.Z(), g.Mass()}) out.push_back(v);
    }
    *data = out.data();
    *len = (int)out.size();
    return faces.Extent();
  });
}

/// What each face is, for reading features off a solid (drawings: holes,
/// turned diameters, steps). Per face, 20 numbers: the kind (0 plane,
/// 1 cylinder, 2 cone, 3 torus, 4 sphere, 5 other); a point (a plane's centre
/// of area, else the axis's origin); a direction (a plane's outward normal,
/// else the axis); the radius at the low and high end along the axis (a
/// torus: major and minor radius); 1 when the face is concave about its axis
/// (a hole's wall: the material is outside it); the face's extent along the
/// axis from the point (low, high); the angle it sweeps round the axis
/// (radians, 2π for a whole one); its area; and its bounding box.
int occ_face_info(void* p, int h, const double** data, int* len) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(*s, TopAbs_FACE, faces);
    std::vector<double>& out = a->edge_data;
    out.clear();
    for (int i = 1; i <= faces.Extent(); i++) {
      TopoDS_Face f = TopoDS::Face(faces(i));
      BRepAdaptor_Surface surf(f);
      GProp_GProps g;
      BRepGProp::SurfaceProperties(f, g);
      gp_Pnt c = g.CentreOfMass();
      double umin = 0, umax = 0, vmin = 0, vmax = 0;
      BRepTools::UVBounds(f, umin, umax, vmin, vmax);
      double kind = 5, r0 = 0, r1 = 0, concave = 0, lo = 0, hi = 0, sweep = 0;
      gp_Pnt o = c;
      gp_Dir d(0, 0, 1);
      // Whether the face's outward normal at the middle of its parameters points at the axis.
      auto inward = [&](const gp_Ax1& ax) {
        gp_Pnt pt;
        gp_Vec du, dv;
        surf.D1((umin + umax) / 2, (vmin + vmax) / 2, pt, du, dv);
        gp_Vec n = du.Crossed(dv);
        if (f.Orientation() == TopAbs_REVERSED) n.Reverse();
        gp_Vec along(ax.Direction());
        gp_Vec rel(ax.Location(), pt);
        gp_Vec radial = rel - along * rel.Dot(along);
        return n.Dot(radial) < 0 ? 1.0 : 0.0;
      };
      switch (surf.GetType()) {
        case GeomAbs_Plane: {
          kind = 0;
          gp_Dir n = surf.Plane().Axis().Direction();
          if (f.Orientation() == TopAbs_REVERSED) n.Reverse();
          d = n;
          lo = hi = gp_Vec(c.XYZ()).Dot(gp_Vec(n));
          break;
        }
        case GeomAbs_Cylinder: {
          kind = 1;
          gp_Cylinder cy = surf.Cylinder();
          o = cy.Location(), d = cy.Axis().Direction();
          r0 = r1 = cy.Radius();
          concave = inward(cy.Axis());
          lo = vmin, hi = vmax;
          sweep = umax - umin;
          break;
        }
        case GeomAbs_Cone: {
          kind = 2;
          gp_Cone co = surf.Cone();
          o = co.Location(), d = co.Axis().Direction();
          double sa = co.SemiAngle();
          // The cone's v runs along its side: v·cos along the axis, R + v·sin out from it.
          r0 = co.RefRadius() + vmin * sin(sa), r1 = co.RefRadius() + vmax * sin(sa);
          lo = vmin * cos(sa), hi = vmax * cos(sa);
          concave = inward(co.Axis());
          sweep = umax - umin;
          break;
        }
        case GeomAbs_Torus: {
          kind = 3;
          gp_Torus t = surf.Torus();
          o = t.Location(), d = t.Axis().Direction();
          r0 = t.MajorRadius(), r1 = t.MinorRadius();
          concave = inward(t.Axis());
          sweep = umax - umin;
          break;
        }
        case GeomAbs_Sphere: {
          kind = 4;
          gp_Sphere sp = surf.Sphere();
          o = sp.Location();
          r0 = r1 = sp.Radius();
          break;
        }
        default:
          break;
      }
      Bnd_Box box;
      BRepBndLib::AddOptimal(f, box, false, false);
      double x0 = 0, y0 = 0, z0 = 0, x1 = 0, y1 = 0, z1 = 0;
      if (!box.IsVoid()) box.Get(x0, y0, z0, x1, y1, z1);
      for (double v : {kind, o.X(), o.Y(), o.Z(), d.X(), d.Y(), d.Z(), r0, r1, concave, lo, hi, sweep, g.Mass(), x0, y0, z0, x1, y1, z1}) out.push_back(v);
    }
    *data = out.data();
    *len = (int)out.size();
    return faces.Extent();
  });
}

/// The solid of a shape (an offset can come back as a shell or a compound of one solid).
static TopoDS_Shape as_solid(const TopoDS_Shape& r) {
  TopExp_Explorer solids(r, TopAbs_SOLID);
  if (solids.More()) {
    TopoDS_Shape first = solids.Current();
    solids.Next();
    if (!solids.More()) return first;
    return r;
  }
  TopExp_Explorer shells(r, TopAbs_SHELL);
  if (shells.More()) {
    BRepBuilderAPI_MakeSolid ms(TopoDS::Shell(shells.Current()));
    if (ms.IsDone()) {
      ShapeFix_Solid fix(ms.Solid());
      fix.Perform();
      return fix.Solid();
    }
  }
  return r;
}

/// The face nearest a point (index as faces() numbers them), or -1.
int occ_face_near(void* p, int h, double x, double y, double z) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(*s, TopAbs_FACE, faces);
    TopoDS_Vertex v = BRepBuilderAPI_MakeVertex(gp_Pnt(x, y, z));
    int best = -1;
    double dist = 1e300;
    for (int i = 1; i <= faces.Extent(); i++) {
      BRepExtrema_DistShapeShape d(v, faces(i));
      if (d.IsDone() && d.Value() < dist) {
        dist = d.Value();
        best = i - 1;
      }
    }
    return best;
  });
}

/// Pushes (distance > 0, adds material) or pulls (distance < 0, cuts) one
/// face along its outward normal. A flat face: its outline swept that far. A
/// curved one (a cylinder's side, a hole, a sphere): the face offset that far,
/// its neighbours extended or trimmed to meet it.
int occ_push_face(void* p, int h, int index, double distance) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(*s, TopAbs_FACE, faces);
    if (index < 0 || index >= faces.Extent()) throw Standard_Failure("unknown face");
    TopoDS_Face f = TopoDS::Face(faces(index + 1));
    BRepAdaptor_Surface surf(f);
    if (std::fabs(distance) < 1e-9) return push(a, *s);
    if (surf.GetType() != GeomAbs_Plane) {
      BRepOffset_MakeOffset mo;
      mo.Initialize(*s, 0.0, 1e-6, BRepOffset_Skin, Standard_True, Standard_False, GeomAbs_Intersection);
      mo.SetOffsetOnFace(f, distance);
      mo.MakeOffsetShape();
      if (!mo.IsDone()) throw Standard_Failure("could not offset this curved face that far");
      TopoDS_Shape r = as_solid(mo.Shape());
      BRepCheck_Analyzer check(r);
      if (!check.IsValid()) throw Standard_Failure("offsetting this curved face that far breaks the solid");
      return push(a, tidy(r));
    }
    gp_Dir n = surf.Plane().Axis().Direction();
    if (f.Orientation() == TopAbs_REVERSED) n.Reverse();
    TopoDS_Face base = TopoDS::Face(f.Oriented(TopAbs_FORWARD));
    TopoDS_Shape prism = BRepPrimAPI_MakePrism(base, gp_Vec(n) * distance).Shape();
    TopoDS_Shape r;
    if (distance > 0) {
      BRepAlgoAPI_Fuse op(*s, prism);
      if (!op.IsDone()) throw Standard_Failure("could not push the face");
      r = op.Shape();
    } else {
      BRepAlgoAPI_Cut op(*s, prism);
      if (!op.IsDone()) throw Standard_Failure("could not pull the face");
      r = op.Shape();
    }
    return push(a, tidy(r));
  });
}


/// The tightest outside rounding of a solid (cylinders, cones, spheres and
/// tori bulging outward), mm; infinity when it has none.
static double convex_round(const TopoDS_Shape& s) {
  double best = std::numeric_limits<double>::infinity();
  for (TopExp_Explorer e(s, TopAbs_FACE); e.More(); e.Next()) {
    TopoDS_Face f = TopoDS::Face(e.Current());
    BRepAdaptor_Surface surf(f);
    const bool forward = f.Orientation() == TopAbs_FORWARD;
    double u1, u2, v1, v2;
    BRepTools::UVBounds(f, u1, u2, v1, v2);
    switch (surf.GetType()) {
      case GeomAbs_Cylinder:
        if (forward == surf.Cylinder().Direct()) best = std::min(best, surf.Cylinder().Radius());
        break;
      case GeomAbs_Cone: {
        gp_Cone c = surf.Cone();
        if (forward == c.Direct())
          for (double v : {v1, v2}) best = std::min(best, std::fabs(c.RefRadius() + v * std::sin(c.SemiAngle())));
        break;
      }
      case GeomAbs_Sphere:
        if (forward == surf.Sphere().Direct()) best = std::min(best, surf.Sphere().Radius());
        break;
      case GeomAbs_Torus:
        if (forward == surf.Torus().Direct()) best = std::min(best, surf.Torus().MinorRadius());
        break;
      default:
        break;
    }
  }
  return best;
}

/// Hollows a solid to walls `thickness` thick. Positive: the walls grow
/// inward from the outer skin (the outside stays put); negative: outward (the
/// inside keeps the original shape). The listed faces are removed, leaving the
/// shell open there; none gives a closed hollow, an inner void.
int occ_shell(void* p, int h, double thickness, const int* idx, int n) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (std::fabs(thickness) < 1e-9) throw Standard_Failure("the wall needs a thickness");
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(*s, TopAbs_FACE, faces);
    TopTools_ListOfShape open;
    for (int i = 0; i < n; i++) {
      if (idx[i] < 0 || idx[i] >= faces.Extent()) throw Standard_Failure("unknown face");
      open.Append(faces(idx[i] + 1));
    }
    // OCCT offsets along the outward normal: inward walls are a negative offset.
    double off = -thickness;
    GProp_GProps g0;
    BRepGProp::VolumeProperties(*s, g0);
    const double v0 = g0.Mass();
    // A hollow worth keeping: valid, one solid, and not the part handed back untouched.
    auto good = [&](const TopoDS_Shape& c) {
      if (c.IsNull() || !BRepCheck_Analyzer(c).IsValid()) return false;
      int solids = 0;
      for (TopExp_Explorer e(c, TopAbs_SOLID); e.More(); e.Next()) solids++;
      if (solids != 1) return false;
      GProp_GProps g;
      BRepGProp::VolumeProperties(c, g);
      return g.Mass() > 1e-9 && std::fabs(g.Mass() - v0) > v0 * 1e-9 && (thickness < 0 || g.Mass() < v0);
    };
    /// The part offset by `off` everywhere, except the open faces by `open_off`.
    auto offset_body = [&](double open_off) {
      BRepOffset_MakeOffset mo;
      mo.Initialize(*s, off, 1e-6, BRepOffset_Skin, Standard_False, Standard_False, GeomAbs_Intersection, Standard_False);
      for (TopTools_ListIteratorOfListOfShape it(open); it.More(); it.Next()) mo.SetOffsetOnFace(TopoDS::Face(it.Value()), open_off);
      mo.MakeOffsetShape();
      if (!mo.IsDone()) throw Standard_Failure("could not hollow this solid with walls that thick");
      return as_solid(mo.Shape());
    };
    auto cut = [&](const TopoDS_Shape& x, const TopoDS_Shape& y) {
      BRepAlgoAPI_Cut op(x, y);
      if (!op.IsDone()) throw Standard_Failure("could not hollow this solid");
      return op.Shape();
    };
    auto hollow = [&]() -> TopoDS_Shape {
    TopoDS_Shape r;
    if (n > 0) {
      try {
        BRepOffsetAPI_MakeThickSolid mk;
        mk.MakeThickSolidByJoin(*s, open, off, 1e-6, BRepOffset_Skin, Standard_False, Standard_False, GeomAbs_Intersection);
        if (mk.IsDone()) r = as_solid(mk.Shape());
      } catch (const Standard_Failure&) {
      }
      if (!good(r)) {
        // The thick-solid algorithm can hand back the part untouched (drafted
        // cones, say): build the same hollow as an offset body and a cut. Inward,
        // the cavity pokes out through the open faces so the cut opens them;
        // outward, the grown body stays flush with them.
        try {
          r = thickness > 0 ? cut(*s, offset_body(thickness)) : cut(offset_body(0.0), *s);
        } catch (const Standard_Failure&) {
          r.Nullify();
        }
      }
      if (!good(r) && thickness > 0) {
        // Again, from the sealed cavity: each flat open face's twin on it swept
        // out through the wall, and all of it cut from the part.
        TopoDS_Shape cavity = offset_body(off);
        TopTools_IndexedMapOfShape inner;
        TopExp::MapShapes(cavity, TopAbs_FACE, inner);
        for (TopTools_ListIteratorOfListOfShape it(open); it.More(); it.Next()) {
          TopoDS_Face f = TopoDS::Face(it.Value());
          BRepAdaptor_Surface fs(f);
          if (fs.GetType() != GeomAbs_Plane) throw Standard_Failure("could not open this curved face: open flat faces, or hollow it sealed");
          gp_Dir n = fs.Plane().Axis().Direction();
          if (f.Orientation() == TopAbs_REVERSED) n.Reverse();
          const double d0 = gp_Vec(fs.Plane().Location().XYZ()).Dot(gp_Vec(n)) - thickness;
          int best = 0;
          double err = 1e300;
          for (int i = 1; i <= inner.Extent(); i++) {
            TopoDS_Face g = TopoDS::Face(inner(i));
            BRepAdaptor_Surface gs(g);
            if (gs.GetType() != GeomAbs_Plane) continue;
            gp_Dir m = gs.Plane().Axis().Direction();
            if (g.Orientation() == TopAbs_REVERSED) m.Reverse();
            if (m.Dot(n) < 1 - 1e-9) continue;
            double e = std::fabs(gp_Vec(gs.Plane().Location().XYZ()).Dot(gp_Vec(n)) - d0);
            if (e < err) err = e, best = i;
          }
          if (!best || err > 1e-4) throw Standard_Failure("could not open that face with walls that thick");
          TopoDS_Shape through = BRepPrimAPI_MakePrism(TopoDS::Face(inner(best).Oriented(TopAbs_FORWARD)), gp_Vec(n) * (thickness * 2)).Shape();
          BRepAlgoAPI_Fuse op(cavity, through);
          if (!op.IsDone()) throw Standard_Failure("could not open that face");
          cavity = op.Shape();
        }
        r = cut(*s, cavity);
      }
    } else {
      // Closed: the solid with an offset copy of itself as an inner void.
      TopoDS_Shape body = offset_body(off);
      r = thickness > 0 ? cut(*s, body) : cut(body, *s);
    }
    return r;
    };
    TopoDS_Shape r;
    std::string why = "walls that thick break the solid";
    try {
      r = hollow();
    } catch (const Standard_Failure& e) {
      why = e.GetMessageString();
    }
    if (!good(r)) {
      // The usual cause: an outside corner rounded tighter than the wall,
      // which cannot be offset inward. Say which.
      double tightest = convex_round(*s);
      if (thickness > 0 && tightest < thickness) {
        char msg[200];
        std::snprintf(msg, sizeof msg, "the %.3g mm wall is thicker than a rounded corner here (%.3g mm radius): use a thinner wall, or round after shelling", thickness,
                      tightest);
        throw Standard_Failure(msg);
      }
      throw Standard_Failure(why.c_str());
    }
    return push(a, r);
  });
}

/// Tapers faces by `degrees` relative to the pull direction (dx,dy,dz),
/// hinging about the neutral plane through (px,py,pz) normal to the pull: the
/// faces keep their size on that plane and lean inward going along the pull
/// (positive angle), as a moulded part's walls do so it releases from the mould.
int occ_draft(void* p, int h, const int* idx, int n, double degrees, double dx, double dy, double dz, double px, double py,
              double pz) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (std::fabs(degrees) < 1e-9) return push(a, *s);
    if (std::fabs(degrees) >= 89.0) throw Standard_Failure("a draft must be under 89 degrees");
    gp_Vec v(dx, dy, dz);
    if (v.Magnitude() < 1e-12) throw Standard_Failure("the pull direction is zero");
    gp_Dir dir(v);
    gp_Pln neutral(gp_Pnt(px, py, pz), dir);
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(*s, TopAbs_FACE, faces);
    std::vector<int> pick(idx, idx + std::max(n, 0));
    if (pick.empty()) {
      // Every wall running along the pull: flat faces square to it, and
      // cylinders (bosses, holes) whose axis is the pull.
      for (int i = 1; i <= faces.Extent(); i++) {
        BRepAdaptor_Surface surf(TopoDS::Face(faces(i)));
        bool wall = (surf.GetType() == GeomAbs_Plane && std::fabs(surf.Plane().Axis().Direction().Dot(dir)) < 1e-7) ||
                    (surf.GetType() == GeomAbs_Cylinder && surf.Cylinder().Axis().Direction().IsParallel(dir, 1e-7));
        if (wall) pick.push_back(i - 1);
      }
      if (pick.empty()) throw Standard_Failure("no faces run along the pull direction");
    }
    BRepOffsetAPI_DraftAngle draft(*s);
    double ang = degrees * M_PI / 180.0;
    for (int i : pick) {
      if (i < 0 || i >= faces.Extent()) throw Standard_Failure("unknown face");
      TopoDS_Face f = TopoDS::Face(faces(i + 1));
      draft.Add(f, dir, ang, neutral);
      if (!draft.AddDone()) throw Standard_Failure("this face cannot be drafted from that direction (is it parallel to the pull?)");
    }
    draft.Build();
    if (!draft.IsDone()) throw Standard_Failure("could not draft these faces");
    TopoDS_Shape r = as_solid(draft.Shape());
    BRepCheck_Analyzer check(r);
    if (!check.IsValid()) throw Standard_Failure("these faces cannot be drafted cleanly: try a smaller angle, fewer faces, or drafting before shelling");
    return push(a, r);
  });
}

}  // extern "C"

// Sweeps, lofts, offsets, defeaturing, paint, checks and BREP files.
#include "solids.cpp"
