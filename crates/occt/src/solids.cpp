// The exact solid vocabulary beyond prisms and revolves: paths (lines, arcs,
// helices, spirals, splines), sweeps along them, lofts through sections,
// offset and thickened solids, variable fillets, defeaturing, painted faces,
// geometry checks and BREP files.
//
// Included at the end of shim.cpp (one translation unit, so it shares the
// arena and its helpers); kept apart so the two grow without colliding.

#include <BOPAlgo_ArgumentAnalyzer.hxx>
#include <BOPAlgo_CheckResult.hxx>
#include <BRepAlgoAPI_Defeaturing.hxx>
#include <BRepBuilderAPI_TransitionMode.hxx>
#include <BRepCheck.hxx>
#include <BRepCheck_ListOfStatus.hxx>
#include <BRepCheck_Result.hxx>
#include <BRepLib.hxx>
#include <BRepOffsetAPI_MakeOffsetShape.hxx>
#include <BRepOffsetAPI_MakePipeShell.hxx>
#include <BRep_Builder.hxx>
#include <GCPnts_AbscissaPoint.hxx>
#include <GC_MakeArcOfCircle.hxx>
#include <GeomAPI_Interpolate.hxx>
#include <Geom2d_Line.hxx>
#include <Geom_ConicalSurface.hxx>
#include <Geom_CylindricalSurface.hxx>
#include <Law_Linear.hxx>
#include <ShapeAnalysis_ShapeTolerance.hxx>
#include <TColgp_HArray1OfPnt.hxx>
#include <Quantity_Color.hxx>
#include <STEPCAFControl_Controller.hxx>
#include <XCAFDoc_ColorTool.hxx>
#include <gp_Ax3.hxx>

#include <Message_ProgressIndicator.hxx>

#include <chrono>
#include <sstream>

namespace {

// ---- Paths -----------------------------------------------------------------------

/// A path as a wire, from flat segment data. Each segment starts with its kind:
///   0 line   x0 y0 z0  x1 y1 z1
///   1 arc    x0 y0 z0  xm ym zm  x1 y1 z1          (through the middle point)
///   2 helix  ox oy oz  ax ay az  xx xy xz  radius pitch height taper° left
///            (axis through o along a, starting at o + radius·x; taper leans the
///            radius out (positive) or in going up, as a cone's half-angle)
///   3 spline n closed  then n points               (interpolating, C2)
///   4 spiral ox oy oz  ax ay az  xx xy xz  r0 growth turns left
///            (flat Archimedean: radius r0 + growth per turn)
/// Helices are exact (a line on a cylinder or cone); a spiral is a B-spline
/// through 64 points a turn.
struct PathData {
  TopoDS_Wire wire;
  bool corners = false;  // tangent breaks between segments (mitred, not bent)
  // The first segment's helix axis (for profiles placed "axial").
  bool helix = false;
  gp_Ax3 axis;
};

gp_Pnt pt(const double* d) { return gp_Pnt(d[0], d[1], d[2]); }
gp_Dir dir(const double* d) {
  gp_Vec v(d[0], d[1], d[2]);
  if (v.Magnitude() < 1e-12) throw Standard_Failure("a path direction is zero");
  return gp_Dir(v);
}

TopoDS_Edge helix_edge(const gp_Ax3& ax, double r, double pitch, double height, double taper_deg, bool left) {
  if (!(r > 0)) throw Standard_Failure("a helix needs a positive radius");
  if (!(pitch > 0)) throw Standard_Failure("a helix needs a positive pitch");
  if (!(height > 0)) throw Standard_Failure("a helix needs a positive height");
  const double turns = height / pitch;
  const double sign = left ? -1 : 1;
  if (std::fabs(taper_deg) < 1e-9) {
    Handle(Geom_CylindricalSurface) s = new Geom_CylindricalSurface(ax, r);
    // (u, v) = (angle, height): one turn rises a pitch.
    gp_Dir2d d(sign * 2 * M_PI, pitch);
    Handle(Geom2d_Line) l = new Geom2d_Line(gp_Pnt2d(0, 0), d);
    double end = std::hypot(2 * M_PI * turns, height);
    TopoDS_Edge e = BRepBuilderAPI_MakeEdge(l, s, 0, end).Edge();
    BRepLib::BuildCurves3d(e, 1e-7, GeomAbs_C1, 14, 10000);
    return e;
  }
  if (std::fabs(taper_deg) >= 89) throw Standard_Failure("a helix taper must be under 89 degrees");
  const double a = taper_deg * M_PI / 180;
  // On a cone v runs along the slant: height = v·cos a.
  Handle(Geom_ConicalSurface) s = new Geom_ConicalSurface(ax, a, r);
  if (r + height * std::tan(a) <= 0) throw Standard_Failure("the helix tapers to nothing before its top");
  gp_Dir2d d(sign * 2 * M_PI, pitch / std::cos(a));
  Handle(Geom2d_Line) l = new Geom2d_Line(gp_Pnt2d(0, 0), d);
  double end = std::hypot(2 * M_PI * turns, height / std::cos(a));
  TopoDS_Edge e = BRepBuilderAPI_MakeEdge(l, s, 0, end).Edge();
  BRepLib::BuildCurves3d(e, 1e-7, GeomAbs_C1, 14, 10000);
  return e;
}

PathData build_path(const double* d, int len) {
  PathData out;
  BRepBuilderAPI_MakeWire wire;
  int at = 0;
  auto need = [&](int n) {
    if (at + n > len) throw Standard_Failure("path data ends early");
  };
  gp_Pnt last;
  gp_Vec last_t;
  bool any = false;
  while (at < len) {
    int kind = (int)d[at++];
    TopoDS_Edge e;
    if (kind == 0) {
      need(6);
      gp_Pnt a = pt(d + at), b = pt(d + at + 3);
      at += 6;
      if (a.Distance(b) < 1e-9) continue;
      e = BRepBuilderAPI_MakeEdge(a, b).Edge();
    } else if (kind == 1) {
      need(9);
      gp_Pnt a = pt(d + at), m = pt(d + at + 3), b = pt(d + at + 6);
      at += 9;
      GC_MakeArcOfCircle arc(a, m, b);
      if (!arc.IsDone()) throw Standard_Failure("an arc's three points lie on a line");
      e = BRepBuilderAPI_MakeEdge(arc.Value()).Edge();
    } else if (kind == 2) {
      need(14);
      gp_Ax3 ax(pt(d + at), dir(d + at + 3), dir(d + at + 6));
      if (!any) out.helix = true, out.axis = ax;
      e = helix_edge(ax, d[at + 9], d[at + 10], d[at + 11], d[at + 12], d[at + 13] != 0);
      at += 14;
    } else if (kind == 3) {
      need(2);
      int n = (int)d[at];
      bool closed = d[at + 1] != 0;
      at += 2;
      need(3 * n);
      if (n < 2) throw Standard_Failure("a spline needs at least two points");
      Handle(TColgp_HArray1OfPnt) pts = new TColgp_HArray1OfPnt(1, n);
      for (int i = 0; i < n; i++) pts->SetValue(i + 1, pt(d + at + 3 * i));
      at += 3 * n;
      GeomAPI_Interpolate in(pts, closed, 1e-7);
      in.Perform();
      if (!in.IsDone()) throw Standard_Failure("could not fit a spline through those points (two the same?)");
      e = BRepBuilderAPI_MakeEdge(in.Curve()).Edge();
    } else if (kind == 4) {
      need(13);
      gp_Ax3 ax(pt(d + at), dir(d + at + 3), dir(d + at + 6));
      double r0 = d[at + 9], growth = d[at + 10], turns = d[at + 11];
      bool left = d[at + 12] != 0;
      at += 13;
      if (!(turns > 0) || r0 < 0) throw Standard_Failure("a spiral needs turns and a radius of at least 0");
      int n = std::max(8, (int)std::ceil(turns * 64)) + 1;
      Handle(TColgp_HArray1OfPnt) pts = new TColgp_HArray1OfPnt(1, n);
      gp_Trsf to;
      to.SetTransformation(ax);
      to.Invert();
      for (int i = 0; i < n; i++) {
        double t = turns * i / (n - 1), ang = (left ? -1 : 1) * 2 * M_PI * t, r = r0 + growth * t;
        pts->SetValue(i + 1, gp_Pnt(r * std::cos(ang), r * std::sin(ang), 0).Transformed(to));
      }
      GeomAPI_Interpolate in(pts, false, 1e-7);
      in.Perform();
      if (!in.IsDone()) throw Standard_Failure("could not build the spiral");
      e = BRepBuilderAPI_MakeEdge(in.Curve()).Edge();
    } else {
      throw Standard_Failure("unknown path segment");
    }
    BRepAdaptor_Curve c(e);
    gp_Pnt p0;
    gp_Vec t0;
    c.D1(c.FirstParameter(), p0, t0);
    if (any) {
      if (p0.Distance(last) > 1e-5) {
        std::ostringstream m;
        m << "path segments do not meet: one ends at (" << last.X() << ", " << last.Y() << ", " << last.Z() << "), the next starts at (" << p0.X() << ", " << p0.Y() << ", "
          << p0.Z() << ")";
        throw Standard_Failure(m.str().c_str());
      }
      if (t0.Magnitude() > 1e-12 && last_t.Magnitude() > 1e-12 && t0.Angle(last_t) > 1e-4) out.corners = true;
    }
    wire.Add(e);
    if (!wire.IsDone()) throw Standard_Failure("the path's segments do not join into one wire");
    gp_Pnt p1;
    gp_Vec t1;
    c.D1(c.LastParameter(), p1, t1);
    last = p1, last_t = t1, any = true;
  }
  if (!any) throw Standard_Failure("the path is empty");
  out.wire = wire.Wire();
  return out;
}

/// One sample along a path: arclength from the start, the point, and the frame
/// the section rides in (t along the path; n, b across it).
struct Sample {
  double s;
  gp_Pnt p;
  gp_Vec t, n, b;
};

gp_Vec unit(const gp_Vec& v) {
  double m = v.Magnitude();
  return m > 1e-15 ? v / m : v;
}

/// A deterministic frame for a tangent: b toward `up` (Z, or Y when the
/// tangent runs along Z), n = b × t, so (t, n, b) is right-handed.
void frame_for(const gp_Vec& t, gp_Vec up, gp_Vec& n, gp_Vec& b) {
  if (up.Magnitude() < 1e-12 || std::fabs(unit(up).Dot(t)) > 0.999) up = std::fabs(t.Z()) > 0.999 ? gp_Vec(0, 1, 0) : gp_Vec(0, 0, 1);
  b = unit(up - t * up.Dot(t));
  n = b.Crossed(t);
}

/// Frame laws, as the sweep's orientation: 0 corrected Frenet (rotation
/// minimising), 1 Frenet, 2 fixed, 3 constant binormal, 4 auxiliary path.
struct FrameLaw {
  int mode = 0;
  gp_Vec binormal;
  TopoDS_Wire aux;
};

/// Points along each edge in order, dense enough that a section `reach` mm from
/// the path turns by at most `dphi` between samples (path curvature, torsion and
/// twist all count). Corners give two samples at the same place.
std::vector<Sample> sample_path(const PathData& path, const FrameLaw& law, double dphi, double twist_rad) {
  std::vector<BRepAdaptor_Curve> edges;
  std::vector<bool> rev;
  std::vector<double> lens;
  double total = 0;
  for (BRepTools_WireExplorer ex(path.wire); ex.More(); ex.Next()) {
    edges.emplace_back(ex.Current());
    rev.push_back(ex.Current().Orientation() == TopAbs_REVERSED);
    double l = GCPnts_AbscissaPoint::Length(edges.back());
    lens.push_back(l);
    total += l;
  }
  if (total < 1e-9) throw Standard_Failure("the path has no length");
  std::vector<Sample> out;
  double s0 = 0;
  for (size_t k = 0; k < edges.size(); k++) {
    BRepAdaptor_Curve& c = edges[k];
    double u0 = c.FirstParameter(), u1 = c.LastParameter();
    if (rev[k]) std::swap(u0, u1);
    // How much the frame turns along this edge: curvature and torsion, by 64 probes.
    double turn = 0;
    {
      gp_Pnt p;
      gp_Vec d1, d2, d3, prev_t, prev_b;
      for (int i = 0; i <= 64; i++) {
        double u = u0 + (u1 - u0) * i / 64.0;
        c.D3(u, p, d1, d2, d3);
        gp_Vec t = unit(d1), bb = d1.Crossed(d2);
        bb = bb.Magnitude() > 1e-12 ? unit(bb) : prev_b;
        if (i) {
          if (prev_t.Magnitude() > 0 && t.Magnitude() > 0) turn += prev_t.Angle(t);
          if (law.mode == 1 && prev_b.Magnitude() > 0 && bb.Magnitude() > 0) turn += prev_b.Angle(bb);
        }
        prev_t = t, prev_b = bb;
      }
    }
    turn += std::fabs(twist_rad) * lens[k] / total;
    int n = std::max(1, std::min(20000, (int)std::ceil(turn / dphi)));
    if (law.mode == 4) n = std::max(n, 16);  // follows another curve: sample it
    // Even steps in arclength.
    std::vector<double> us(n + 1);
    for (int i = 0; i <= n; i++) {
      double frac = (double)i / n;
      if (i == 0) us[i] = u0;
      else if (i == n) us[i] = u1;
      else {
        GCPnts_AbscissaPoint ap(c, (rev[k] ? -1 : 1) * lens[k] * frac, u0);
        us[i] = ap.IsDone() ? ap.Parameter() : u0 + (u1 - u0) * frac;
      }
    }
    for (int i = 0; i <= n; i++) {
      gp_Pnt p;
      gp_Vec d1, d2;
      c.D2(us[i], p, d1, d2);
      if (rev[k]) d1.Reverse();
      Sample sm;
      sm.s = s0 + lens[k] * i / n;
      sm.p = p;
      sm.t = unit(d1);
      // Curvature normal (for Frenet), carried in n for now.
      gp_Vec kn = d2 - sm.t * d2.Dot(sm.t);
      sm.n = kn.Magnitude() > 1e-9 * std::max(1.0, d1.SquareMagnitude()) ? unit(kn) : gp_Vec();
      // The first sample of a later edge repeats the previous edge's last point:
      // keep it only at a corner (both frames are needed there).
      if (i == 0 && !out.empty() && out.back().t.Angle(sm.t) < 1e-4) continue;
      out.push_back(sm);
    }
    s0 += lens[k];
  }
  // Frames.
  Sample& first = out[0];
  gp_Vec n0, b0;
  frame_for(first.t, gp_Vec(0, 0, 1), n0, b0);
  std::vector<Sample> aux;
  double aux_len = 0;
  if (law.mode == 4) {
    PathData ap;
    ap.wire = law.aux;
    aux = sample_path(ap, FrameLaw(), dphi, 0);
    aux_len = aux.back().s;
  }
  gp_Vec n = n0, b = b0;
  for (size_t i = 0; i < out.size(); i++) {
    Sample& sm = out[i];
    gp_Vec curv = sm.n;
    if (law.mode == 0 || (law.mode == 1 && curv.Magnitude() < 0.5)) {
      // Rotation minimising frame (double reflection, Wang et al. 2008); at a
      // corner the frame turns the least that takes the old tangent to the new.
      if (i > 0) {
        const Sample& pr = out[i - 1];
        gp_Vec v1(pr.p, sm.p);
        double c1 = v1.SquareMagnitude();
        if (c1 < 1e-18) {
          // A corner: turn the frame about the bend the least that takes the old tangent to the new.
          gp_Vec ax = pr.t.Crossed(sm.t);
          if (ax.Magnitude() > 1e-12) n = n.Rotated(gp_Ax1(gp_Pnt(0, 0, 0), gp_Dir(ax)), pr.t.Angle(sm.t));
        } else {
          gp_Vec rl = n - v1 * (2 / c1) * v1.Dot(n);
          gp_Vec tl = pr.t - v1 * (2 / c1) * v1.Dot(pr.t);
          gp_Vec v2 = sm.t - tl;
          double c2 = v2.SquareMagnitude();
          n = c2 > 1e-18 ? rl - v2 * (2 / c2) * v2.Dot(rl) : rl;
        }
        n = unit(n - sm.t * n.Dot(sm.t));
      }
      b = sm.t.Crossed(n);
    } else if (law.mode == 1) {
      // Frenet: n toward the centre of curvature.
      n = curv;
      b = sm.t.Crossed(n);
    } else if (law.mode == 2) {
      n = n0, b = b0;
    } else if (law.mode == 3) {
      gp_Vec bn = unit(law.binormal);
      n = bn.Crossed(sm.t);
      if (n.Magnitude() < 1e-9) throw Standard_Failure("the path runs along the fixed binormal");
      n = unit(n);
      b = sm.t.Crossed(n);
    } else {
      // Toward the auxiliary path's point at the same fraction of its length.
      double want = sm.s / out.back().s * aux_len;
      size_t j = 1;
      while (j + 1 < aux.size() && aux[j].s < want) j++;
      const Sample &a0 = aux[j - 1], &a1 = aux[j];
      double f = a1.s > a0.s ? (want - a0.s) / (a1.s - a0.s) : 0;
      gp_Pnt q(a0.p.XYZ() * (1 - f) + a1.p.XYZ() * f);
      gp_Vec d(sm.p, q);
      d = d - sm.t * d.Dot(sm.t);
      if (d.Magnitude() < 1e-9) throw Standard_Failure("the auxiliary path touches the path");
      n = unit(d);
      b = sm.t.Crossed(n);
    }
    sm.n = n, sm.b = b;
  }
  return out;
}

/// Where the profile's own x, y (and z out of it) go: origin, x, y, z.
struct Placement {
  gp_Pnt o;
  gp_Vec x, y, z;
};

/// place: 0 square to the path at its start (profile y toward `up`), 1 axial
/// (a helix's profile drawn as for revolve: x the radius, y along the axis,
/// in the plane through the axis and the start), 2 as given (o, x, y in args).
Placement placement(const PathData& path, int place, const double* args) {
  Placement pl;
  BRepTools_WireExplorer ex(path.wire);
  BRepAdaptor_Curve c(ex.Current());
  bool rev = ex.Current().Orientation() == TopAbs_REVERSED;
  gp_Pnt p;
  gp_Vec d1;
  c.D1(rev ? c.LastParameter() : c.FirstParameter(), p, d1);
  if (rev) d1.Reverse();
  if (place == 0) {
    gp_Vec t = unit(d1), n, b;
    frame_for(t, gp_Vec(args[0], args[1], args[2]), n, b);
    pl.o = p, pl.x = n, pl.y = b, pl.z = t;
  } else if (place == 1) {
    if (!path.helix) throw Standard_Failure("an axial profile needs a path that starts with a helix");
    gp_Pnt o = path.axis.Location();
    gp_Vec a(path.axis.Direction());
    gp_Vec r(o, p);
    double h = r.Dot(a);
    pl.o = o.Translated(a * h);
    pl.x = unit(r - a * h);
    pl.y = a;
    pl.z = pl.x.Crossed(pl.y);
  } else {
    pl.o = gp_Pnt(args[0], args[1], args[2]);
    pl.x = unit(gp_Vec(args[3], args[4], args[5]));
    gp_Vec y(args[6], args[7], args[8]);
    pl.y = unit(y - pl.x * y.Dot(pl.x));
    pl.z = pl.x.Crossed(pl.y);
  }
  return pl;
}

gp_Trsf trsf_of(const Placement& pl) {
  gp_Trsf t;
  t.SetValues(pl.x.X(), pl.y.X(), pl.z.X(), pl.o.X(), pl.x.Y(), pl.y.Y(), pl.z.Y(), pl.o.Y(), pl.x.Z(), pl.y.Z(), pl.z.Z(), pl.o.Z());
  return t;
}

FrameLaw law_of(int mode, const double* args, int nargs) {
  FrameLaw law;
  law.mode = mode;
  if (mode == 3) {
    if (nargs < 3) throw Standard_Failure("a fixed binormal needs a direction");
    law.binormal = gp_Vec(args[0], args[1], args[2]);
    if (law.binormal.Magnitude() < 1e-12) throw Standard_Failure("the binormal is zero");
  } else if (mode == 4) {
    law.aux = build_path(args, nargs).wire;
  } else if (mode < 0 || mode > 4) {
    throw Standard_Failure("unknown sweep orientation");
  }
  return law;
}

/// Each wire of each face of a profile: (outer?, wire).
std::vector<std::pair<bool, TopoDS_Wire>> profile_wires(const TopoDS_Shape& s) {
  std::vector<std::pair<bool, TopoDS_Wire>> out;
  for (auto& f : faces_of(s)) {
    TopoDS_Wire outer = BRepTools::OuterWire(f);
    for (TopExp_Explorer e(f, TopAbs_WIRE); e.More(); e.Next()) out.push_back({e.Current().IsSame(outer), TopoDS::Wire(e.Current())});
  }
  if (out.empty()) {
    // A bare wire (an open or closed outline that never became a face).
    for (TopExp_Explorer e(s, TopAbs_WIRE); e.More(); e.Next()) out.push_back({true, TopoDS::Wire(e.Current())});
  }
  if (out.empty()) throw Standard_Failure("the profile has no outline");
  return out;
}

/// Stops a long OpenCascade algorithm once its time is up (it asks between steps).
class Deadline : public Message_ProgressIndicator {
 public:
  explicit Deadline(double seconds) : end(std::chrono::steady_clock::now() + std::chrono::milliseconds((long long)(seconds * 1000))) {}
  Standard_Boolean UserBreak() override { return std::chrono::steady_clock::now() > end; }
  void Show(const Message_ProgressScope&, const Standard_Boolean) override {}

 private:
  std::chrono::steady_clock::time_point end;
};

int count_solids(const TopoDS_Shape& s) {
  int n = 0;
  for (TopExp_Explorer e(s, TopAbs_SOLID); e.More(); e.Next()) n++;
  return n;
}

double volume_of(const TopoDS_Shape& s) {
  GProp_GProps g;
  BRepGProp::VolumeProperties(s, g);
  return g.Mass();
}

/// A closed solid from a pipe or loft shell, turned outward if it came inside out.
TopoDS_Shape outward(const TopoDS_Shape& s) {
  TopoDS_Shape r = as_solid(s);
  if (count_solids(r) && volume_of(r) < 0) r.Reverse();
  return r;
}

}  // namespace

extern "C" {

/// Samples a path for the mesh kernel, in the same frames the exact sweep
/// rides in. Returns the number of samples; data: the 12-number placement
/// (origin, x, y, z of the profile), a corners flag, then per sample s, point,
/// t, n, b (13 numbers).
int occ_path_frames(void* p, const double* segs, int nsegs, int mode, const double* margs, int nmargs, int place, const double* pargs, double dphi,
                    double twist_deg, const double** data, int* len) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    PathData path = build_path(segs, nsegs);
    FrameLaw law = law_of(mode, margs, nmargs);
    Placement pl = placement(path, place, pargs);
    std::vector<Sample> sm = sample_path(path, law, std::max(dphi, 1e-4), twist_deg * M_PI / 180);
    std::vector<double>& d = a->view_data;
    d.clear();
    d.insert(d.end(), {pl.o.X(), pl.o.Y(), pl.o.Z(), pl.x.X(), pl.x.Y(), pl.x.Z(), pl.y.X(), pl.y.Y(), pl.y.Z(), pl.z.X(), pl.z.Y(), pl.z.Z()});
    d.push_back(path.corners ? 1 : 0);
    for (auto& q : sm)
      d.insert(d.end(), {q.s, q.p.X(), q.p.Y(), q.p.Z(), q.t.X(), q.t.Y(), q.t.Z(), q.n.X(), q.n.Y(), q.n.Z(), q.b.X(), q.b.Y(), q.b.Z()});
    *data = d.data();
    *len = (int)d.size();
    return (int)sm.size();
  });
}

/// The path itself as a shape (a wire), e.g. to measure its length or show it.
int occ_path(void* p, const double* segs, int nsegs) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] { return push(a, build_path(segs, nsegs).wire); });
}

/// Sweeps a profile (a face on z = 0, holes and all) along a path, exactly.
/// mode/margs: the frame law (see FrameLaw); place/pargs: where the profile
/// sits (see placement); twist: degrees about the path over its length; scale:
/// the section's size at the end relative to the start (linear in between).
int occ_sweep(void* p, int h, const double* segs, int nsegs, int mode, const double* margs, int nmargs, int place, const double* pargs, double twist_deg,
              double scale) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (!(scale > 0)) throw Standard_Failure("the end scale must be positive");
    PathData path = build_path(segs, nsegs);
    FrameLaw law = law_of(mode, margs, nmargs);
    gp_Trsf to = trsf_of(placement(path, place, pargs));
    const bool twisted = std::fabs(twist_deg) > 1e-9;
    TopoDS_Wire aux;
    if (twisted) {
      // A twist rides on an auxiliary path a little off the spine, turning about
      // it: each of its points in the plane square to the path there.
      std::vector<Sample> sm = sample_path(path, law, 2 * M_PI / 180, twist_deg * M_PI / 180);
      Bnd_Box bb;
      BRepBndLib::Add(*s, bb);
      double rho = 1.0;
      Handle(TColgp_HArray1OfPnt) pts = new TColgp_HArray1OfPnt(1, (int)sm.size());
      int k = 0;
      double L = sm.back().s;
      for (auto& q : sm) {
        double th = twist_deg * M_PI / 180 * q.s / L;
        pts->SetValue(++k, q.p.Translated((q.n * std::cos(th) + q.b * std::sin(th)) * rho));
      }
      GeomAPI_Interpolate in(pts, false, 1e-7);
      in.Perform();
      if (!in.IsDone()) throw Standard_Failure("could not build the twist");
      aux = BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(in.Curve()).Edge()).Wire();
    }
    std::vector<TopoDS_Shape> solids, holes;
    for (auto& [outer, w0] : profile_wires(*s)) {
      TopoDS_Wire w = TopoDS::Wire(transformed(w0, to));
      BRepOffsetAPI_MakePipeShell pipe(path.wire);
      if (twisted) pipe.SetMode(aux, false, BRepFill_NoContact);
      else if (law.mode == 0) pipe.SetMode(false);
      else if (law.mode == 1) pipe.SetMode(true);
      else if (law.mode == 2) {
        BRepTools_WireExplorer ex(path.wire);
        BRepAdaptor_Curve c(ex.Current());
        gp_Pnt o;
        gp_Vec d1;
        c.D1(c.FirstParameter(), o, d1);
        pipe.SetMode(gp_Ax2(o, gp_Dir(0, 0, 1), std::fabs(gp_Dir(d1).Z()) > 0.999 ? gp_Dir(1, 0, 0) : gp_Dir(gp_Vec(0, 0, 1).Crossed(d1))));
      } else if (law.mode == 3) pipe.SetMode(gp_Dir(law.binormal));
      else pipe.SetMode(law.aux, true, BRepFill_NoContact);
      if (path.corners) pipe.SetTransitionMode(BRepBuilderAPI_RightCorner);
      if (std::fabs(scale - 1) > 1e-12) {
        Handle(Law_Linear) l = new Law_Linear();
        l->Set(0, 1, 1, scale);
        pipe.SetLaw(w, l, false, false);
      } else {
        pipe.Add(w, false, false);
      }
      pipe.Build();
      if (!pipe.IsDone()) throw Standard_Failure("could not sweep the profile along that path (does it cross itself, or bend tighter than the profile is wide?)");
      if (!pipe.MakeSolid()) throw Standard_Failure("the sweep does not close into a solid");
      (outer ? solids : holes).push_back(outward(pipe.Shape()));
    }
    TopoDS_Shape r = fuse_all(solids);
    for (auto& hole : holes) r = BRepAlgoAPI_Cut(r, hole).Shape();
    if (!BRepCheck_Analyzer(r).IsValid()) {
      ShapeFix_Shape fix(r);
      fix.Perform();
      r = fix.Shape();
    }
    return push(a, r);
  });
}

/// A loft through sections: n profiles (handles; -1 for a point section at
/// the placement's origin), each placed by 12 numbers (origin, x, y, z).
/// ruled: flat between sections; closed: back to the first; solid: closed
/// ends (else a skin). continuity 0 C0, 1 C1, 2 C2 (smooth lofts).
int occ_loft(void* p, const int* hs, const double* mats, int n, int ruled, int closed, int solid, int continuity) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    if (n < 2) throw Standard_Failure("a loft needs at least two sections");
    // Each section's outer wire, and its holes.
    std::vector<std::vector<TopoDS_Shape>> layers;  // [0] outer, [1..] holes; a vertex for a point
    for (int i = 0; i < n; i++) {
      const double* m = mats + 12 * i;
      gp_Trsf t;
      t.SetValues(m[3], m[6], m[9], m[0], m[4], m[7], m[10], m[1], m[5], m[8], m[11], m[2]);
      if (hs[i] < 0) {
        layers.push_back({BRepBuilderAPI_MakeVertex(gp_Pnt(m[0], m[1], m[2])).Vertex()});
        continue;
      }
      const TopoDS_Shape* s = get(a, hs[i]);
      if (!s) return -1;
      std::vector<TopoDS_Shape> ws;
      std::vector<TopoDS_Shape> inner;
      auto wires = profile_wires(*s);
      int outers = 0;
      for (auto& [outer, w] : wires) {
        TopoDS_Wire placed = TopoDS::Wire(transformed(w, t));
        if (outer) ws.insert(ws.begin(), placed), outers++;
        else inner.push_back(placed);
      }
      if (outers != 1) throw Standard_Failure("each loft section must be one outline (holes allowed)");
      ws.insert(ws.end(), inner.begin(), inner.end());
      layers.push_back(ws);
    }
    auto loft = [&](size_t k) {
      BRepOffsetAPI_ThruSections ts(solid != 0, ruled != 0, 1e-6);
      ts.CheckCompatibility(true);
      if (!ruled) {
        ts.SetContinuity(continuity <= 0 ? GeomAbs_C0 : continuity == 1 ? GeomAbs_C1 : GeomAbs_C2);
        ts.SetSmoothing(false);
      }
      auto add = [&](const std::vector<TopoDS_Shape>& L) {
        const TopoDS_Shape& x = L.size() == 1 && L[0].ShapeType() == TopAbs_VERTEX ? L[0] : L[k];
        if (x.ShapeType() == TopAbs_VERTEX) ts.AddVertex(TopoDS::Vertex(x));
        else ts.AddWire(TopoDS::Wire(x));
      };
      for (auto& L : layers) add(L);
      if (closed) add(layers[0]);
      ts.Build();
      if (!ts.IsDone()) throw Standard_Failure("could not loft through those sections (do they cross, or twist too far from one to the next?)");
      return solid ? outward(ts.Shape()) : ts.Shape();
    };
    size_t holes = SIZE_MAX;
    for (auto& L : layers)
      if (!(L.size() == 1 && L[0].ShapeType() == TopAbs_VERTEX)) holes = std::min(holes, L.size() - 1);
    for (auto& L : layers)
      if (!(L.size() == 1 && L[0].ShapeType() == TopAbs_VERTEX) && L.size() - 1 != holes) throw Standard_Failure("every loft section needs the same number of holes");
    TopoDS_Shape r = loft(0);
    if (solid)
      for (size_t k = 1; k <= holes; k++) r = BRepAlgoAPI_Cut(r, loft(k)).Shape();
    return push(a, r);
  });
}

/// Offsets a whole solid outward (positive) or inward (negative): join 0 round
/// (edges grow arcs, a true offset), 1 sharp (faces extended to meet).
int occ_offset_solid(void* p, int h, double d, int join) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (std::fabs(d) < 1e-9) return push(a, *s);
    BRepOffsetAPI_MakeOffsetShape mo;
    mo.PerformByJoin(*s, d, 1e-6, BRepOffset_Skin, Standard_False, Standard_False, join == 1 ? GeomAbs_Intersection : GeomAbs_Arc);
    if (!mo.IsDone()) throw Standard_Failure("could not offset the solid that far");
    TopoDS_Shape r = as_solid(mo.Shape());
    if (!BRepCheck_Analyzer(r).IsValid()) throw Standard_Failure("offsetting that far breaks the solid (a hole or round smaller than the offset?)");
    return push(a, r);
  });
}

/// Gives a sheet (a face or open shell, e.g. an imported surface) a thickness
/// `t` along its normal (negative: the other side); or, for a solid, hollows it
/// to walls `t` thick opening the listed faces, with the chosen corner join
/// (0 round, 1 sharp).
int occ_thicken(void* p, int h, double t, const int* idx, int n, int join) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (std::fabs(t) < 1e-9) throw Standard_Failure("the thickness must not be zero");
    GeomAbs_JoinType j = join == 1 ? GeomAbs_Intersection : GeomAbs_Arc;
    TopoDS_Shape r;
    if (count_solids(*s)) {
      TopTools_IndexedMapOfShape faces;
      TopExp::MapShapes(*s, TopAbs_FACE, faces);
      TopTools_ListOfShape open;
      for (int i = 0; i < n; i++) {
        if (idx[i] < 0 || idx[i] >= faces.Extent()) throw Standard_Failure("unknown face");
        open.Append(faces(idx[i] + 1));
      }
      BRepOffsetAPI_MakeThickSolid mk;
      mk.MakeThickSolidByJoin(*s, open, -t, 1e-6, BRepOffset_Skin, Standard_False, Standard_False, j);
      if (!mk.IsDone()) throw Standard_Failure("could not hollow the solid with walls that thick");
      r = as_solid(mk.Shape());
      // The algorithm can hand the part back untouched rather than fail.
      if (std::fabs(volume_of(r) - volume_of(*s)) <= 1e-9 * std::fabs(volume_of(*s)))
        throw Standard_Failure("could not hollow this solid (its faces do not offset cleanly: loft an inner skin and subtract it instead)");
    } else {
      BRepOffset_MakeOffset mo;
      mo.Initialize(*s, t, 1e-6, BRepOffset_Skin, Standard_False, Standard_False, j, Standard_True);
      mo.MakeOffsetShape();
      if (!mo.IsDone()) throw Standard_Failure("could not thicken the sheet");
      r = outward(mo.Shape());
    }
    if (!BRepCheck_Analyzer(r).IsValid()) throw Standard_Failure("that thickness breaks the solid");
    return push(a, r);
  });
}

/// Rounds (kind 0) with a radius going from r1 to r2 along each edge, or
/// bevels (kind 1) r1 back along one face and r2 along the other.
int occ_blend_var(void* p, int h, int kind, double r1, double r2, const int* idx, int n) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (!(r1 > 0) || !(r2 > 0)) throw Standard_Failure("both sizes must be positive");
    TopTools_IndexedMapOfShape edges;
    TopExp::MapShapes(*s, TopAbs_EDGE, edges);
    TopTools_IndexedDataMapOfShapeListOfShape owners;
    TopExp::MapShapesAndAncestors(*s, TopAbs_EDGE, TopAbs_FACE, owners);
    TopoDS_Shape r;
    if (kind == 0) {
      BRepFilletAPI_MakeFillet f(*s);
      for (int i = 0; i < n; i++) {
        if (idx[i] < 0 || idx[i] >= edges.Extent()) throw Standard_Failure("unknown edge");
        f.Add(r1, r2, TopoDS::Edge(edges(idx[i] + 1)));
      }
      f.Build();
      if (!f.IsDone()) throw Standard_Failure("these edges cannot take those radii");
      r = f.Shape();
    } else {
      BRepFilletAPI_MakeChamfer f(*s);
      for (int i = 0; i < n; i++) {
        if (idx[i] < 0 || idx[i] >= edges.Extent()) throw Standard_Failure("unknown edge");
        TopoDS_Edge e = TopoDS::Edge(edges(idx[i] + 1));
        f.Add(r1, r2, e, TopoDS::Face(owners.FindFromKey(e).First()));
      }
      f.Build();
      if (!f.IsDone()) throw Standard_Failure("these edges cannot take that chamfer");
      r = f.Shape();
    }
    if (!BRepCheck_Analyzer(r).IsValid()) throw Standard_Failure("the result is not a valid solid (try smaller sizes)");
    return push(a, r);
  });
}

/// What each face is, numbered as occ_faces numbers them: per face 10
/// numbers: surface kind (0 plane, 1 cylinder, 2 cone, 3 sphere, 4 torus,
/// 5 other), radius (cylinder, sphere; a cone's at its reference; a torus's
/// minor), axis x y z, axis point x y z, outward bulge (1 convex, -1 concave
/// such as a hole, 0 flat or neither), and the angle it spans round its axis.
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
      bool forward = f.Orientation() == TopAbs_FORWARD;
      double u1, u2, v1, v2;
      BRepTools::UVBounds(f, u1, u2, v1, v2);
      double kind = 5, r = 0, bulge = 0, span = u2 - u1;
      gp_Ax1 ax(gp_Pnt(0, 0, 0), gp_Dir(0, 0, 1));
      switch (surf.GetType()) {
        case GeomAbs_Plane:
          kind = 0, span = 0, ax = surf.Plane().Axis();
          break;
        case GeomAbs_Cylinder:
          kind = 1, r = surf.Cylinder().Radius(), ax = surf.Cylinder().Axis();
          bulge = forward == surf.Cylinder().Direct() ? 1 : -1;
          break;
        case GeomAbs_Cone:
          kind = 2, r = surf.Cone().RefRadius(), ax = surf.Cone().Axis();
          bulge = forward == surf.Cone().Direct() ? 1 : -1;
          break;
        case GeomAbs_Sphere:
          kind = 3, r = surf.Sphere().Radius(), ax = surf.Sphere().Position().Axis();
          bulge = forward == surf.Sphere().Direct() ? 1 : -1;
          break;
        case GeomAbs_Torus:
          kind = 4, r = surf.Torus().MinorRadius(), ax = surf.Torus().Axis();
          bulge = forward == surf.Torus().Direct() ? 1 : -1;
          span = v2 - v1;  // the round's own sweep
          break;
        default:
          span = 0;
          break;
      }
      gp_Dir d = ax.Direction();
      gp_Pnt o = ax.Location();
      for (double v : {kind, r, d.X(), d.Y(), d.Z(), o.X(), o.Y(), o.Z(), bulge, span}) out.push_back(v);
    }
    *data = out.data();
    *len = (int)out.size();
    return faces.Extent();
  });
}

/// Removes features (holes, rounds, bosses: the listed faces) and heals the
/// solid over where they were, as if they had never been modelled.
int occ_defeature(void* p, int h, const int* idx, int n) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (n <= 0) return push(a, *s);
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(*s, TopAbs_FACE, faces);
    BRepAlgoAPI_Defeaturing df;
    df.SetShape(*s);
    for (int i = 0; i < n; i++) {
      if (idx[i] < 0 || idx[i] >= faces.Extent()) throw Standard_Failure("unknown face");
      df.AddFaceToRemove(faces(idx[i] + 1));
    }
    df.SetRunParallel(true);
    df.Build();
    if (!df.IsDone() || df.HasErrors()) throw Standard_Failure("could not remove those faces (the faces around them cannot be extended to close the gap)");
    TopoDS_Shape r = df.Shape();
    if (!BRepCheck_Analyzer(r).IsValid()) throw Standard_Failure("removing those faces leaves an invalid solid");
    return push(a, tidy(r));
  });
}

/// Gives faces (the listed ones; n < 0 the whole shape) a colour (r g b,
/// 0–1; negative r for none) and a name (may be empty), carried into STEP.
/// The shape itself is unchanged: returns the same handle.
int occ_paint(void* p, int h, const int* idx, int n, double r, double g, double b, const char* name) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    std::array<double, 3> c = {r, g, b};
    std::string nm = name ? name : "";
    if (n < 0) {
      // n = -2: a default (a part's material colour) that later paint overrides.
      if (n == -2) a->painted.insert(a->painted.begin(), {*s, c, nm});
      else a->painted.push_back({*s, c, nm});
      return h;
    }
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(*s, TopAbs_FACE, faces);
    for (int i = 0; i < n; i++) {
      if (idx[i] < 0 || idx[i] >= faces.Extent()) throw Standard_Failure("unknown face");
      a->painted.push_back({faces(idx[i] + 1), c, nm});
    }
    return h;
  });
}

/// Checks a shape as FreeCAD's Check geometry does: topology and geometry
/// (BRepCheck), then what booleans would trip on (self-intersection, tiny
/// edges, curves off their surfaces). Returns a JSON report (in the arena
/// until the next call).
const char* occ_check(void* p, int h) {
  Arena* a = static_cast<Arena*>(p);
  static thread_local std::string json;
  int ok = guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    auto esc = [](const std::string& x) {
      std::string o;
      for (char c : x) {
        if (c == '"' || c == '\\') o += '\\';
        if (c == '\n') o += "\\n";
        else o += c;
      }
      return o;
    };
    auto kind = [](TopAbs_ShapeEnum t) {
      switch (t) {
        case TopAbs_COMPOUND: return "compound";
        case TopAbs_COMPSOLID: return "compsolid";
        case TopAbs_SOLID: return "solid";
        case TopAbs_SHELL: return "shell";
        case TopAbs_FACE: return "face";
        case TopAbs_WIRE: return "wire";
        case TopAbs_EDGE: return "edge";
        case TopAbs_VERTEX: return "vertex";
        default: return "shape";
      }
    };
    std::ostringstream o;
    o.precision(10);
    // Counts.
    int counts[8] = {0};
    TopAbs_ShapeEnum types[] = {TopAbs_SOLID, TopAbs_SHELL, TopAbs_FACE, TopAbs_WIRE, TopAbs_EDGE, TopAbs_VERTEX};
    for (int i = 0; i < 6; i++) {
      TopTools_IndexedMapOfShape m;
      TopExp::MapShapes(*s, types[i], m);
      counts[i] = m.Extent();
    }
    // Topology and geometry.
    BRepCheck_Analyzer an(*s);
    bool valid = an.IsValid();
    std::vector<std::string> problems;
    if (!valid) {
      for (TopAbs_ShapeEnum t : {TopAbs_VERTEX, TopAbs_EDGE, TopAbs_WIRE, TopAbs_FACE, TopAbs_SHELL, TopAbs_SOLID}) {
        TopTools_IndexedMapOfShape m;
        TopExp::MapShapes(*s, t, m);
        for (int i = 1; i <= m.Extent() && problems.size() < 50; i++) {
          Handle(BRepCheck_Result) res = an.Result(m(i));
          if (res.IsNull()) continue;
          for (res->InitContextIterator(); res->MoreShapeInContext(); res->NextShapeInContext()) {
            for (BRepCheck_ListIteratorOfListOfStatus it(res->StatusOnShape()); it.More(); it.Next()) {
              if (it.Value() == BRepCheck_NoError) continue;
              std::ostringstream st;
              BRepCheck::Print(it.Value(), st);
              std::string text = st.str();
              while (!text.empty() && (text.back() == '\n' || text.back() == ' ')) text.pop_back();
              problems.push_back(std::string("{\"shape\":\"") + kind(t) + " " + std::to_string(i) + "\",\"status\":\"" + esc(text) + "\"}");
            }
          }
        }
      }
    }
    // What booleans would trip on.
    std::vector<std::string> faults;
    bool stopped = false;
    {
      BOPAlgo_ArgumentAnalyzer ba;
      ba.SetShape1(*s);
      ba.ArgumentTypeMode() = Standard_True;
      ba.SelfInterMode() = Standard_True;
      ba.SmallEdgeMode() = Standard_True;
      ba.RebuildFaceMode() = Standard_True;
      ba.ContinuityMode() = Standard_True;
      ba.CurveOnSurfaceMode() = Standard_True;
      ba.StopOnFirstFaulty() = Standard_False;
      // Self-intersection of many curved faces can take minutes: it gets 20 s.
      Handle(Deadline) clock = new Deadline(20);
      ba.Perform(clock->Start());
      stopped = clock->UserBreak();
      if (ba.HasFaulty()) {
        static const char* names[] = {"unknown",
                                      "bad type",
                                      "self-intersection",
                                      "too small edge",
                                      "non-recoverable face",
                                      "incompatible vertices",
                                      "incompatible edges",
                                      "incompatible faces",
                                      "aborted",
                                      "C0 geometry (not tangent-continuous)",
                                      "curve off its surface",
                                      "not valid"};
        for (BOPAlgo_ListIteratorOfListOfCheckResult it(ba.GetCheckResult()); it.More() && faults.size() < 50; it.Next()) {
          const BOPAlgo_CheckResult& r = it.Value();
          int st = (int)r.GetCheckStatus();
          if (r.GetCheckStatus() == BOPAlgo_OperationAborted) continue;
          std::string shapes;
          for (TopTools_ListIteratorOfListOfShape f(r.GetFaultyShapes1()); f.More(); f.Next()) shapes += std::string(shapes.empty() ? "" : ", ") + kind(f.Value().ShapeType());
          std::ostringstream fo;
          fo.precision(6);
          fo << "{\"check\":\"" << names[std::max(0, std::min(11, st))] << "\",\"shapes\":\"" << shapes << "\"";
          if (r.GetMaxDistance1() > 0) fo << ",\"distance\":" << r.GetMaxDistance1();
          fo << "}";
          faults.push_back(fo.str());
        }
      }
    }
    ShapeAnalysis_ShapeTolerance tol;
    double tv = tol.Tolerance(*s, 1, TopAbs_VERTEX), te = tol.Tolerance(*s, 1, TopAbs_EDGE), tf = tol.Tolerance(*s, 1, TopAbs_FACE);
    // Free edges (on one face only): a solid should have none.
    TopTools_IndexedDataMapOfShapeListOfShape ef;
    TopExp::MapShapesAndAncestors(*s, TopAbs_EDGE, TopAbs_FACE, ef);
    int free = 0;
    for (int i = 1; i <= ef.Extent(); i++)
      if (!BRep_Tool::Degenerated(TopoDS::Edge(ef.FindKey(i))) && ef(i).Extent() == 1) free++;
    double vol = counts[0] ? volume_of(*s) : 0;
    o << "{\"valid\":" << (valid ? "true" : "false") << ",\"boolean_ready\":" << (faults.empty() ? "true" : "false") << ",\"complete\":" << (stopped ? "false" : "true") << ",\"kind\":\"" << kind(s->ShapeType())
      << "\",\"solids\":" << counts[0] << ",\"shells\":" << counts[1] << ",\"faces\":" << counts[2] << ",\"wires\":" << counts[3] << ",\"edges\":" << counts[4]
      << ",\"vertices\":" << counts[5] << ",\"free_edges\":" << free << ",\"volume\":" << vol << ",\"max_tolerance\":{\"vertex\":" << tv << ",\"edge\":" << te
      << ",\"face\":" << tf << "},\"problems\":[";
    for (size_t i = 0; i < problems.size(); i++) o << (i ? "," : "") << problems[i];
    o << "],\"faults\":[";
    for (size_t i = 0; i < faults.size(); i++) o << (i ? "," : "") << faults[i];
    o << "]}";
    json = o.str();
    return 0;
  });
  return ok < 0 ? nullptr : json.c_str();
}

/// Writes shapes to an OpenCascade BREP file (exact, unnamed) as one compound.
int occ_write_brep(void* p, const int* hs, int n, const char* path) {
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
    if (!BRepTools::Write(n == 1 ? *get(a, hs[0]) : c, path)) throw Standard_Failure("could not write the BREP file");
    return 0;
  });
}

/// Reads a BREP file: every solid (or the shape itself when it has none).
/// Returns how many; they are the arena's shapes from `*first` on.
int occ_read_brep(void* p, const char* path, int* first) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    TopoDS_Shape s;
    BRep_Builder b;
    if (!BRepTools::Read(s, path, b) || s.IsNull()) throw Standard_Failure("not a readable BREP file");
    a->names.resize(a->shapes.size());
    *first = (int)a->shapes.size();
    int k = 0;
    for (TopExp_Explorer e(s, TopAbs_SOLID); e.More(); e.Next(), k++) a->shapes.push_back(e.Current()), a->names.push_back("Imported body");
    if (!k) a->shapes.push_back(s), a->names.push_back("Imported shape");
    return (int)a->shapes.size() - *first;
  });
}

}  // extern "C"

namespace {
/// Colours and names painted on a part (or on its faces) onto its STEP label.
void apply_paint(Arena* a, const Handle(XCAFDoc_ShapeTool) & tool, const Handle(XCAFDoc_ColorTool) & colours, const TDF_Label& label, const TopoDS_Shape& part) {
  if (a->painted.empty()) return;
  auto mark = [&](const TDF_Label& l, const std::array<double, 3>& c, const std::string& name) {
    if (c[0] >= 0) colours->SetColor(l, Quantity_Color(c[0], c[1], c[2], Quantity_TOC_sRGB), XCAFDoc_ColorSurf);
    if (!name.empty()) {
      TDataStd_Name::Set(l, TCollection_ExtendedString(name.c_str(), true));
      // Face names go into the file only when asked for.
      STEPCAFControl_Controller::Init();
      Interface_Static::SetIVal("write.stepcaf.subshapes.name", 1);
    }
  };
  // The whole part: painted as it is, or as the solid it holds.
  for (auto& [shape, c, name] : a->painted) {
    if (shape.ShapeType() == TopAbs_FACE) continue;
    bool same = shape.IsPartner(part);
    for (TopExp_Explorer e(shape, TopAbs_SOLID); !same && e.More(); e.Next()) same = e.Current().IsPartner(part);
    if (same && c[0] >= 0) colours->SetColor(label, Quantity_Color(c[0], c[1], c[2], Quantity_TOC_sRGB), XCAFDoc_ColorSurf);
  }
  // The faces as the document holds them (a placed part is stored as a
  // reference to its unplaced shape, whose faces are the ones to label).
  TDF_Label held = label;
  if (XCAFDoc_ShapeTool::IsReference(label)) XCAFDoc_ShapeTool::GetReferredShape(label, held);
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(XCAFDoc_ShapeTool::GetShape(held), TopAbs_FACE, faces);
  for (int i = 1; i <= faces.Extent(); i++) {
    for (auto& [shape, c, name] : a->painted) {
      if (shape.ShapeType() != TopAbs_FACE || !shape.IsPartner(faces(i))) continue;
      TDF_Label sub = tool->AddSubShape(held, faces(i));
      if (!sub.IsNull()) mark(sub, c, name);
    }
  }
}
}  // namespace
