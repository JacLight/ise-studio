// Surface modelling: sheets (open faces and shells) built from curves and
// edges, joined, trimmed and measured. Filling an N-sided hole with G0/G1/G2
// to its neighbours (GeomPlate through BRepOffsetAPI_MakeFilling), surfaces
// through curve networks (a Gordon surface: sections and guides), boundary
// patches (GeomFill), ruled and section surfaces, G2 blend surfaces and blend
// curves, sewing into shells and solids, extending, splitting and trimming,
// offset sheets, projection of curves, isoparametric curves, and the
// analysis a surfacing designer lives by: continuity measured across edges,
// deviation between surfaces, curvature and draft samples.
//
// Included at the end of shim.cpp (one translation unit, so it shares the
// arena and its helpers, and solids.cpp's paths). The Gordon construction
// follows the textbook form (Gordon 1971; as in TiGL's CTiglGordonSurface
// and FreeCAD's Surface workbench, both studied, none copied).

#include <BRepAdaptor_CompCurve.hxx>
#include <BRepAlgoAPI_Splitter.hxx>
#include <BRepAlgo_NormalProjection.hxx>
#include <BRepClass_FaceClassifier.hxx>
#include <BRepLProp_SLProps.hxx>
#include <Geom2d_Curve.hxx>
#include <BRepLib_MakeWire.hxx>
#include <Adaptor3d_CurveOnSurface.hxx>
#include <BRepBuilderAPI_Copy.hxx>
#include <Geom2dAdaptor_Curve.hxx>
#include <GeomPlate_BuildPlateSurface.hxx>
#include <GeomPlate_CurveConstraint.hxx>
#include <GeomPlate_MakeApprox.hxx>
#include <GeomPlate_PointConstraint.hxx>
#include <GeomPlate_Surface.hxx>
#include <ShapeFix_Face.hxx>
#include <BRepProj_Projection.hxx>
#include <BRepTopAdaptor_FClass2d.hxx>
#include <GeomAPI_ProjectPointOnCurve.hxx>
#include <GeomAPI_ProjectPointOnSurf.hxx>
#include <GeomAdaptor_Surface.hxx>
#include <GeomConvert_CompCurveToBSplineCurve.hxx>
#include <GeomFill_BSplineCurves.hxx>
#include <GeomProjLib.hxx>
#include <Geom2dAPI_Interpolate.hxx>
#include <Geom2d_BSplineCurve.hxx>
#include <GeomAdaptor_Curve.hxx>
#include <TColgp_HArray1OfPnt2d.hxx>
#include <GeomLProp_SLProps.hxx>
#include <Geom_BSplineSurface.hxx>
#include <Geom_RectangularTrimmedSurface.hxx>
#include <ShapeAnalysis_FreeBounds.hxx>
#include <ShapeFix_Shell.hxx>
#include <TColStd_Array2OfReal.hxx>
#include <TColgp_Array2OfPnt.hxx>
#include <TopTools_HSequenceOfShape.hxx>

#include <functional>

namespace {

// ---- Curves sampled by arc length -----------------------------------------------

/// A wire (or edge) walked by the fraction of its length, so curves made of
/// unlike pieces (lines, splines, arcs) are paced evenly.
struct ArcCurve {
  BRepAdaptor_CompCurve c;
  std::vector<double> s, u;  // fraction along → curve parameter (table)
  bool closed = false;
  double length = 0;
  std::vector<gp_Pnt> samples;  // at fractions i / 400

  explicit ArcCurve(const TopoDS_Wire& w) : c(w, Standard_True) {
    const int n = 400;
    double f = c.FirstParameter(), l = c.LastParameter();
    gp_Pnt prev = c.Value(f);
    s.push_back(0), u.push_back(f);
    double acc = 0;
    for (int i = 1; i <= n; i++) {
      double t = f + (l - f) * i / n;
      gp_Pnt q = c.Value(t);
      acc += q.Distance(prev);
      prev = q;
      s.push_back(acc), u.push_back(t);
    }
    length = acc;
    if (!(length > 1e-9)) throw Standard_Failure("a curve has no length");
    for (double& v : s) v /= length;
    closed = c.Value(f).Distance(c.Value(l)) < 1e-6 * std::max(1.0, length);
    for (int i = 0; i <= n; i++) samples.push_back(at((double)i / n));
  }
  double param(double t) const {
    if (closed) t -= std::floor(t);
    t = std::min(1.0, std::max(0.0, t));
    size_t i = std::upper_bound(s.begin(), s.end(), t) - s.begin();
    if (i == 0) return u.front();
    if (i >= s.size()) return u.back();
    double k = (t - s[i - 1]) / std::max(1e-15, s[i] - s[i - 1]);
    return u[i - 1] + k * (u[i] - u[i - 1]);
  }
  gp_Pnt at(double t) const { return c.Value(param(t)); }
  /// Nearest fraction to a point (sampled, then refined by golden section).
  double nearest(const gp_Pnt& q) const {
    const int n = (int)samples.size() - 1;
    int best = 0;
    double bd = 1e300;
    for (int i = 0; i <= n; i++) {
      double d = samples[i].SquareDistance(q);
      if (d < bd) bd = d, best = i;
    }
    double lo = (best - 1.0) / n, hi = (best + 1.0) / n;
    if (!closed) lo = std::max(0.0, lo), hi = std::min(1.0, hi);
    for (int k = 0; k < 60; k++) {
      double m1 = lo + (hi - lo) * 0.382, m2 = lo + (hi - lo) * 0.618;
      if (at(m1).SquareDistance(q) < at(m2).SquareDistance(q)) hi = m2;
      else lo = m1;
    }
    double t = (lo + hi) / 2;
    if (closed) t -= std::floor(t);
    return t;
  }
};

TopoDS_Wire wire_of(const TopoDS_Shape& s) {
  if (s.ShapeType() == TopAbs_WIRE) return TopoDS::Wire(s);
  if (s.ShapeType() == TopAbs_EDGE) return BRepBuilderAPI_MakeWire(TopoDS::Edge(s)).Wire();
  TopExp_Explorer w(s, TopAbs_WIRE);
  if (w.More()) return TopoDS::Wire(w.Current());
  BRepLib_MakeWire mk;
  for (TopExp_Explorer e(s, TopAbs_EDGE); e.More(); e.Next()) mk.Add(TopoDS::Edge(e.Current()));
  if (!mk.IsDone()) throw Standard_Failure("those edges do not join into one curve");
  return mk.Wire();
}

/// A curve (wire) as one B-spline, for the GeomFill patches.
Handle(Geom_BSplineCurve) bspline_of(const TopoDS_Wire& w) {
  GeomConvert_CompCurveToBSplineCurve join;
  for (BRepTools_WireExplorer x(w); x.More(); x.Next()) {
    const TopoDS_Edge& e = x.Current();
    if (BRep_Tool::Degenerated(e)) continue;
    double f, l;
    Handle(Geom_Curve) c = BRep_Tool::Curve(e, f, l);
    if (c.IsNull()) continue;
    Handle(Geom_TrimmedCurve) t = new Geom_TrimmedCurve(c, f, l);
    Handle(Geom_BSplineCurve) b = GeomConvert::CurveToBSplineCurve(t);
    gp_Trsf tr = e.Location().Transformation();
    if (tr.Form() != gp_Identity) b->Transform(tr);
    if (e.Orientation() == TopAbs_REVERSED) b->Reverse();
    if (!join.Add(b, 1e-5)) throw Standard_Failure("a curve's pieces do not meet end to end");
  }
  Handle(Geom_BSplineCurve) r = join.BSplineCurve();
  if (r.IsNull()) throw Standard_Failure("an empty curve");
  return r;
}

// ---- Cardinal cubic splines -------------------------------------------------------

/// Weights w_k(x) such that the cubic spline through values y_k at the knots
/// is Σ w_k(x) y_k: natural end conditions, or periodic with `period`.
/// Linear for two knots (open).
struct Cardinal {
  std::vector<double> x;
  bool periodic;
  double period;
  std::vector<std::vector<double>> m;  // second derivatives per unit vector: m[k][i]

  Cardinal(std::vector<double> knots, bool per, double prd) : x(std::move(knots)), periodic(per), period(prd) {
    const int n = (int)x.size();
    m.assign(n, std::vector<double>(n, 0.0));
    if (n < 2 || (!periodic && n == 2)) return;
    // Solve A·M = B·y for each unit y (dense; n is small).
    auto h = [&](int i) {
      if (periodic && i == n - 1) return x[0] + period - x[n - 1];
      return x[i + 1] - x[i];
    };
    std::vector<std::vector<double>> A(n, std::vector<double>(n, 0.0)), Bm(n, std::vector<double>(n, 0.0));
    for (int i = 0; i < n; i++) {
      if (!periodic && (i == 0 || i == n - 1)) {
        A[i][i] = 1;
        continue;
      }
      int ip = (i + n - 1) % n, in = (i + 1) % n;
      double h0 = h(ip), h1 = h(i);
      A[i][ip] += h0 / 6, A[i][i] += (h0 + h1) / 3, A[i][in] += h1 / 6;
      Bm[i][ip] += 1 / h0, Bm[i][i] += -1 / h0 - 1 / h1, Bm[i][in] += 1 / h1;
    }
    for (int k = 0; k < n; k++) {
      std::vector<std::vector<double>> a = A;
      std::vector<double> b(n);
      for (int i = 0; i < n; i++) b[i] = Bm[i][k];
      for (int c = 0; c < n; c++) {
        int piv = c;
        for (int r = c + 1; r < n; r++)
          if (std::fabs(a[r][c]) > std::fabs(a[piv][c])) piv = r;
        std::swap(a[c], a[piv]), std::swap(b[c], b[piv]);
        if (std::fabs(a[c][c]) < 1e-300) throw Standard_Failure("the curves sit at the same place along the network");
        for (int r = 0; r < n; r++) {
          if (r == c) continue;
          double f = a[r][c] / a[c][c];
          if (f == 0) continue;
          for (int q = c; q < n; q++) a[r][q] -= f * a[c][q];
          b[r] -= f * b[c];
        }
      }
      for (int i = 0; i < n; i++) m[k][i] = b[i] / a[i][i];
    }
  }

  std::vector<double> weights(double t) const {
    const int n = (int)x.size();
    std::vector<double> w(n, 0.0);
    if (n == 1) {
      w[0] = 1;
      return w;
    }
    int i;
    double x0, x1;
    if (periodic) {
      double t0 = x[0];
      t = t0 + std::fmod(std::fmod(t - t0, period) + period, period);
      i = n - 1;
      for (int k = 0; k + 1 < n; k++)
        if (t < x[k + 1]) {
          i = k;
          break;
        }
      x0 = x[i], x1 = i == n - 1 ? x[0] + period : x[i + 1];
    } else {
      i = 0;
      while (i + 2 < n && t > x[i + 1]) i++;
      x0 = x[i], x1 = x[i + 1];
    }
    int j = (i + 1) % n;
    double hh = x1 - x0, A = (x1 - t) / hh, B = (t - x0) / hh;
    double C = (A * A * A - A) * hh * hh / 6, D = (B * B * B - B) * hh * hh / 6;
    // Outside the knots (open): carry on along the end's straight run.
    if (!periodic && (t < x.front() || t > x.back())) C = D = 0;
    w[i] += A, w[j] += B;
    for (int k = 0; k < n; k++) w[k] += C * m[k][i] + D * m[k][j];
    return w;
  }
};

/// A monotone cubic (Fritsch–Carlson) through (xs, ys): how one curve's own
/// fractions are bent so it meets the network's crossings at shared parameters.
struct Monotone {
  std::vector<double> x, y, d;
  Monotone(std::vector<double> xs, std::vector<double> ys) : x(std::move(xs)), y(std::move(ys)) {
    const int n = (int)x.size();
    d.assign(n, 0.0);
    std::vector<double> sl(n - 1);
    for (int i = 0; i + 1 < n; i++) sl[i] = (y[i + 1] - y[i]) / (x[i + 1] - x[i]);
    for (int i = 0; i < n; i++) {
      if (i == 0) d[i] = sl[0];
      else if (i == n - 1) d[i] = sl[n - 2];
      else d[i] = sl[i - 1] * sl[i] <= 0 ? 0 : (sl[i - 1] + sl[i]) / 2;
    }
    for (int i = 0; i + 1 < n; i++) {
      if (sl[i] == 0) {
        d[i] = d[i + 1] = 0;
        continue;
      }
      double a = d[i] / sl[i], b = d[i + 1] / sl[i], r = a * a + b * b;
      if (r > 9) {
        double t = 3 / std::sqrt(r);
        d[i] = t * a * sl[i], d[i + 1] = t * b * sl[i];
      }
    }
  }
  double at(double t) const {
    const int n = (int)x.size();
    int i = 0;
    while (i + 2 < n && t > x[i + 1]) i++;
    double h = x[i + 1] - x[i], s = (t - x[i]) / h;
    double h00 = (1 + 2 * s) * (1 - s) * (1 - s), h10 = s * (1 - s) * (1 - s), h01 = s * s * (3 - 2 * s), h11 = s * s * (s - 1);
    return h00 * y[i] + h10 * h * d[i] + h01 * y[i + 1] + h11 * h * d[i + 1];
  }
};

// ---- Tensor interpolation -----------------------------------------------------------

/// The C2 B-spline surface through a grid of points (nu × nv, row-major in u)
/// at the given parameters, periodic in u when asked (the first column is
/// then the junction, not repeated).
Handle(Geom_BSplineSurface) interpolate_grid(const std::vector<gp_Pnt>& g, int nu, int nv, const std::vector<double>& up, const std::vector<double>& vp, bool uper) {
  // Rows along u, one per v.
  std::vector<Handle(Geom_BSplineCurve)> rows;
  for (int j = 0; j < nv; j++) {
    Handle(TColgp_HArray1OfPnt) pts = new TColgp_HArray1OfPnt(1, nu);
    for (int i = 0; i < nu; i++) pts->SetValue(i + 1, g[j * nu + i]);
    Handle(TColStd_HArray1OfReal) par = new TColStd_HArray1OfReal(1, uper ? nu + 1 : nu);
    for (int i = 0; i < (uper ? nu + 1 : nu); i++) par->SetValue(i + 1, up[i]);
    GeomAPI_Interpolate in(pts, par, uper, 1e-9);
    in.Perform();
    if (!in.IsDone()) throw Standard_Failure("could not fit the surface (a row of points collapses)");
    rows.push_back(in.Curve());
  }
  const Handle(Geom_BSplineCurve)& r0 = rows[0];
  const int np = r0->NbPoles();
  // Poles columns along v.
  std::vector<Handle(Geom_BSplineCurve)> cols;
  for (int k = 1; k <= np; k++) {
    Handle(TColgp_HArray1OfPnt) pts = new TColgp_HArray1OfPnt(1, nv);
    for (int j = 0; j < nv; j++) {
      if (rows[j]->NbPoles() != np) throw Standard_Failure("rows came out unlike");
      pts->SetValue(j + 1, rows[j]->Pole(k));
    }
    Handle(TColStd_HArray1OfReal) par = new TColStd_HArray1OfReal(1, nv);
    for (int j = 0; j < nv; j++) par->SetValue(j + 1, vp[j]);
    GeomAPI_Interpolate in(pts, par, false, 1e-12);
    in.Perform();
    if (!in.IsDone()) throw Standard_Failure("could not fit the surface across its rows");
    cols.push_back(in.Curve());
  }
  const Handle(Geom_BSplineCurve)& c0 = cols[0];
  const int nq = c0->NbPoles();
  TColgp_Array2OfPnt poles(1, np, 1, nq);
  for (int k = 1; k <= np; k++)
    for (int j = 1; j <= nq; j++) poles.SetValue(k, j, cols[k - 1]->Pole(j));
  TColStd_Array1OfReal uk(1, r0->NbKnots()), vk(1, c0->NbKnots());
  TColStd_Array1OfInteger um(1, r0->NbKnots()), vm(1, c0->NbKnots());
  r0->Knots(uk), r0->Multiplicities(um), c0->Knots(vk), c0->Multiplicities(vm);
  return new Geom_BSplineSurface(poles, uk, vk, um, vm, r0->Degree(), c0->Degree(), uper, false);
}

/// Distance from q to a surface near (u, v): Gauss–Newton from that guess
/// (u, v left at the foot).
double local_foot(const Handle(Geom_Surface)& s, const gp_Pnt& q, double& u, double& v) {
  double u0, u1, v0, v1;
  s->Bounds(u0, u1, v0, v1);
  for (int it = 0; it < 6; it++) {
    gp_Pnt p;
    gp_Vec su, sv;
    s->D1(u, v, p, su, sv);
    gp_Vec r(p, q);
    double a = su.Dot(su), b = su.Dot(sv), c = sv.Dot(sv), det = a * c - b * b;
    if (std::fabs(det) < 1e-30) break;
    double ru = r.Dot(su), rv = r.Dot(sv);
    double du = (ru * c - rv * b) / det, dv = (rv * a - ru * b) / det;
    u = std::min(u1, std::max(u0, u + du)), v = std::min(v1, std::max(v0, v + dv));
    if (std::fabs(du) + std::fabs(dv) < 1e-14) break;
  }
  return s->Value(u, v).Distance(q);
}

double local_distance(const Handle(Geom_Surface)& s, const gp_Pnt& q, double u, double v) { return local_foot(s, q, u, v); }

/// A curve's image on a surface as a 2D curve: points projected (each from
/// the last's foot) and interpolated, periodic when the curve closes.
Handle(Geom2d_Curve) pcurve_by_projection(const Handle(Geom_Curve)& c, double f, double l, const Handle(Geom_Surface)& s) {
  GeomAdaptor_Curve ad(c, f, l);
  const int n = std::min(400, std::max(64, 8 * ad.NbIntervals(GeomAbs_C3)));
  bool closed = c->Value(f).Distance(c->Value(l)) < 1e-7;
  int m = closed ? n : n + 1;
  Handle(TColgp_HArray1OfPnt2d) pts = new TColgp_HArray1OfPnt2d(1, m);
  Handle(TColStd_HArray1OfReal) par = new TColStd_HArray1OfReal(1, closed ? m + 1 : m);
  GeomAPI_ProjectPointOnSurf first(c->Value(f), s);
  if (!first.NbPoints()) throw Standard_Failure("the boundary does not land on the surface");
  double u, v;
  first.LowerDistanceParameters(u, v);
  for (int i = 0; i < m; i++) {
    double t = f + (l - f) * i / n;
    local_foot(s, c->Value(t), u, v);
    pts->SetValue(i + 1, gp_Pnt2d(u, v));
    par->SetValue(i + 1, t);
  }
  if (closed) par->SetValue(m + 1, l);
  Geom2dAPI_Interpolate in(pts, par, closed, 1e-12);
  in.Perform();
  if (!in.IsDone()) throw Standard_Failure("could not lay the boundary on the surface");
  return in.Curve();
}

TopoDS_Face face_of_surface(const Handle(Geom_Surface)& s) {
  BRepBuilderAPI_MakeFace mf(s, 1e-7);
  if (!mf.IsDone()) throw Standard_Failure("could not make a face of the surface");
  TopoDS_Face f = mf.Face();
  BRepLib::BuildCurves3d(f);
  return f;
}

// ---- Faces, edges and their neighbourhoods ----------------------------------------

struct Edges {
  TopTools_IndexedMapOfShape map;
  TopTools_IndexedDataMapOfShapeListOfShape faces;
  explicit Edges(const TopoDS_Shape& s) {
    TopExp::MapShapes(s, TopAbs_EDGE, map);
    TopExp::MapShapesAndAncestors(s, TopAbs_EDGE, TopAbs_FACE, faces);
  }
  TopoDS_Edge edge(int i) const {
    if (i < 0 || i >= map.Extent()) throw Standard_Failure("unknown edge");
    return TopoDS::Edge(map(i + 1));
  }
  std::vector<TopoDS_Face> around(const TopoDS_Edge& e) const {
    std::vector<TopoDS_Face> out;
    if (!faces.Contains(e)) return out;
    for (TopTools_ListIteratorOfListOfShape it(faces.FindFromKey(e)); it.More(); it.Next()) {
      TopoDS_Face f = TopoDS::Face(it.Value());
      bool dup = false;
      for (auto& g : out) dup |= g.IsSame(f);
      if (!dup) out.push_back(f);
    }
    return out;
  }
};

/// Surface point, unit normal (as the face faces) and derivatives at (u, v).
struct SurfPt {
  gp_Pnt p;
  gp_Vec su, sv, suu, suv, svv;
  gp_Vec n;  // unit, the face's outward side
};

SurfPt eval(const TopoDS_Face& f, double u, double v) {
  BRepAdaptor_Surface s(f, Standard_False);
  SurfPt r;
  s.D2(u, v, r.p, r.su, r.sv, r.suu, r.svv, r.suv);
  gp_Vec n = r.su.Crossed(r.sv);
  if (n.Magnitude() < 1e-14) {
    // A pole (a sphere's tip): look a little in.
    BRepLProp_SLProps pr(s, u, v, 1, 1e-9);
    n = pr.IsNormalDefined() ? gp_Vec(pr.Normal()) : gp_Vec(0, 0, 1);
  }
  n.Normalize();
  if (f.Orientation() == TopAbs_REVERSED) n.Reverse();
  r.n = n;
  return r;
}

/// Normal curvature vector of the surface in tangent direction d (curvature × normal).
gp_Vec curvature_vector(const SurfPt& s, const gp_Vec& d) {
  // d = a·Su + b·Sv (least squares in the tangent plane).
  double E = s.su.Dot(s.su), F = s.su.Dot(s.sv), G = s.sv.Dot(s.sv);
  double r1 = d.Dot(s.su), r2 = d.Dot(s.sv), det = E * G - F * F;
  if (std::fabs(det) < 1e-20) return gp_Vec(0, 0, 0);
  double a = (r1 * G - r2 * F) / det, b = (r2 * E - r1 * F) / det;
  gp_Vec dd = s.suu * (a * a) + s.suv * (2 * a * b) + s.svv * (b * b);
  double dl2 = (s.su * a + s.sv * b).SquareMagnitude();
  if (dl2 < 1e-30) return gp_Vec(0, 0, 0);
  double k = dd.Dot(s.n) / dl2;
  return s.n * k;
}

/// (u, v) of a point on a face's surface (projection).
bool uv_of(const TopoDS_Face& f, const gp_Pnt& q, double& u, double& v, double* dist = nullptr) {
  Handle(Geom_Surface) s = BRep_Tool::Surface(f);
  double u0, u1, v0, v1;
  BRepTools::UVBounds(f, u0, u1, v0, v1);
  GeomAPI_ProjectPointOnSurf pr(q, s, u0 - (u1 - u0) * 0.05, u1 + (u1 - u0) * 0.05, v0 - (v1 - v0) * 0.05, v1 + (v1 - v0) * 0.05);
  if (!pr.NbPoints()) {
    GeomAPI_ProjectPointOnSurf all(q, s);
    if (!all.NbPoints()) return false;
    all.LowerDistanceParameters(u, v);
    if (dist) *dist = all.LowerDistance();
    return true;
  }
  pr.LowerDistanceParameters(u, v);
  if (dist) *dist = pr.LowerDistance();
  return true;
}

/// (u, v) on a face at an edge's parameter t, from its curve on that face.
bool uv_on_edge(const TopoDS_Edge& e, const TopoDS_Face& f, double t, double& u, double& v) {
  double f0, l0;
  Handle(Geom2d_Curve) pc = BRep_Tool::CurveOnSurface(e, f, f0, l0);
  if (pc.IsNull()) return false;
  gp_Pnt2d q = pc->Value(t);
  u = q.X(), v = q.Y();
  return true;
}

/// The direction leaving a face across its edge at a point: in the tangent
/// plane, square to the edge, away from the face.
gp_Vec outward_across(const TopoDS_Face& f, const TopoDS_Edge& e, double t, const SurfPt& s, const gp_Vec& tangent, double u, double v) {
  gp_Vec d = tangent.Crossed(s.n);
  if (d.Magnitude() < 1e-14) throw Standard_Failure("the edge runs along the surface normal");
  d.Normalize();
  // Step a little along ±d in (u, v) and ask which side the face is on.
  double E = s.su.Dot(s.su), F = s.su.Dot(s.sv), G = s.sv.Dot(s.sv), det = E * G - F * F;
  double r1 = d.Dot(s.su), r2 = d.Dot(s.sv);
  double a = (r1 * G - r2 * F) / det, b = (r2 * E - r1 * F) / det;
  double u0, u1, v0, v1;
  BRepTools::UVBounds(f, u0, u1, v0, v1);
  double step = 1e-3 * std::max(std::hypot(u1 - u0, v1 - v0), 1e-9) / std::max(std::hypot(a, b), 1e-12);
  BRepTopAdaptor_FClass2d cls(f, 1e-9);
  for (int k = 0; k < 4; k++, step *= 4) {
    TopAbs_State plus = cls.Perform(gp_Pnt2d(u + a * step, v + b * step));
    TopAbs_State minus = cls.Perform(gp_Pnt2d(u - a * step, v - b * step));
    if (plus == TopAbs_IN && minus != TopAbs_IN) return -d;
    if (minus == TopAbs_IN && plus != TopAbs_IN) return d;
  }
  (void)e, (void)t;
  return d;
}

// ---- Blend curves -------------------------------------------------------------------

/// The five-degree Bezier from p0 leaving along d0 with curvature vector k0
/// (G0: straight; G1: tangent; G2: tangent and curvature) to p1 arriving
/// along -d1 (d1 leaves the far side) with curvature vector k1.
std::array<gp_Pnt, 6> blend_poles(const gp_Pnt& p0, const gp_Vec& d0, const gp_Vec& k0, int c0, double s0, const gp_Pnt& p1, const gp_Vec& d1, const gp_Vec& k1, int c1, double s1) {
  double chord = p0.Distance(p1);
  gp_Vec span(p0, p1);
  std::array<gp_Pnt, 6> P;
  P[0] = p0, P[5] = p1;
  double l0 = s0 * chord, l1 = s1 * chord;
  if (c0 >= 1) P[1] = p0.Translated(d0 * (l0 / 5));
  else P[1] = p0.Translated(span * 0.2);
  if (c0 >= 2) P[2] = gp_Pnt(gp_XYZ(2 * P[1].XYZ() - p0.XYZ()) + k0.XYZ() * (l0 * l0 / 20));
  else if (c0 >= 1) P[2] = P[1].Translated(d0 * (l0 / 5));
  else P[2] = p0.Translated(span * 0.4);
  if (c1 >= 1) P[4] = p1.Translated(d1 * (l1 / 5));
  else P[4] = p1.Translated(span * -0.2);
  if (c1 >= 2) P[3] = gp_Pnt(gp_XYZ(2 * P[4].XYZ() - p1.XYZ()) + k1.XYZ() * (l1 * l1 / 20));
  else if (c1 >= 1) P[3] = P[4].Translated(d1 * (l1 / 5));
  else P[3] = p1.Translated(span * -0.4);
  return P;
}

/// Where a chain of edges of one body runs: the curve and, per point, the
/// face beside it with its outward direction and curvature across.
struct Rim {
  TopoDS_Wire wire;
  std::unique_ptr<ArcCurve> arc;
  const TopoDS_Shape* body;
  Edges* edges;
  struct Probe {
    gp_Pnt p;
    gp_Vec out, k, n;
  };
  Probe probe(double frac) const {
    double U = arc->param(frac), t;
    TopoDS_Edge e;
    arc->c.Edge(U, e, t);
    std::vector<TopoDS_Face> fs = edges->around(e);
    if (fs.empty()) throw Standard_Failure("a blend edge has no face beside it");
    TopoDS_Face f = fs[0];
    double u, v;
    gp_Pnt p = arc->c.Value(U);
    if (!uv_on_edge(e, f, t, u, v) && !uv_of(f, p, u, v)) throw Standard_Failure("could not find the edge on its face");
    SurfPt s = eval(f, u, v);
    gp_Pnt q;
    gp_Vec tan;
    arc->c.D1(U, q, tan);
    if (tan.Magnitude() < 1e-14) throw Standard_Failure("a blend edge has a cusp");
    tan.Normalize();
    gp_Vec out = outward_across(f, e, t, s, tan, u, v);
    // The surface's own point: an edge's 3D curve may sit off it by its tolerance.
    (void)p;
    return {s.p, out, curvature_vector(s, out), s.n};
  }
};

std::vector<TopoDS_Edge> picked_edges(const Edges& ed, const int* idx, int n) {
  std::vector<TopoDS_Edge> out;
  for (int i = 0; i < n; i++) out.push_back(ed.edge(idx[i]));
  return out;
}

TopoDS_Wire chain(const std::vector<TopoDS_Edge>& list) {
  Handle(TopTools_HSequenceOfShape) seq = new TopTools_HSequenceOfShape;
  for (auto& e : list) seq->Append(e);
  Handle(TopTools_HSequenceOfShape) wires;
  ShapeAnalysis_FreeBounds::ConnectEdgesToWires(seq, 1e-5, Standard_False, wires);
  if (wires.IsNull() || wires->Length() != 1) throw Standard_Failure("the picked edges do not run end to end as one chain");
  return TopoDS::Wire(wires->Value(1));
}

// ---- Bodies -----------------------------------------------------------------------

/// A body's faces gathered into one shell (or the solid it closes into).
TopoDS_Shape sew_faces(const std::vector<TopoDS_Shape>& parts, double tol, bool solid, int* free_edges) {
  BRepBuilderAPI_Sewing sew(tol, Standard_True, Standard_True, Standard_True, Standard_False);
  for (auto& s : parts) sew.Add(s);
  sew.Perform();
  TopoDS_Shape r = sew.SewedShape();
  if (r.IsNull()) throw Standard_Failure("nothing to sew");
  int nfree = sew.NbFreeEdges();
  if (free_edges) *free_edges = nfree;
  if (solid && nfree == 0) {
    TopExp_Explorer sh(r, TopAbs_SHELL);
    if (sh.More()) {
      ShapeFix_Shell fixs(TopoDS::Shell(sh.Current()));
      fixs.Perform();
      BRepBuilderAPI_MakeSolid ms(fixs.Shell());
      if (ms.IsDone()) {
        ShapeFix_Solid fix(ms.Solid());
        fix.Perform();
        TopoDS_Shape s = fix.Solid();
        if (volume_of(s) < 0) s.Reverse();
        return s;
      }
    }
  }
  return r;
}

/// The boundary edge (one face only) of a body nearest a point, its face, and the distance.
double nearest_free(const Edges& ed, const gp_Pnt& q, TopoDS_Edge& edge, TopoDS_Face& face) {
  double best = 1e300;
  TopoDS_Vertex v = BRepBuilderAPI_MakeVertex(q).Vertex();
  for (int j = 1; j <= ed.map.Extent(); j++) {
    TopoDS_Edge e = TopoDS::Edge(ed.map(j));
    if (BRep_Tool::Degenerated(e)) continue;
    std::vector<TopoDS_Face> fs = ed.around(e);
    if (fs.size() != 1 || BRep_Tool::IsClosed(e, fs[0])) continue;
    BRepExtrema_DistShapeShape d(v, e);
    if (d.IsDone() && d.Value() < best) best = d.Value(), edge = e, face = fs[0];
  }
  return best;
}


// ---- Four-sided G2 fill -------------------------------------------------------------

/// Quintic Hermite basis on [0, 1]: value, first and second derivative at 0,
/// then at 1.
std::array<double, 6> hermite5(double t) {
  double t2 = t * t, t3 = t2 * t, t4 = t3 * t, t5 = t4 * t;
  return {1 - 10 * t3 + 15 * t4 - 6 * t5, t - 6 * t3 + 8 * t4 - 3 * t5, (t2 - 3 * t3 + 3 * t4 - t5) / 2,
          10 * t3 - 15 * t4 + 6 * t5,     -4 * t3 + 7 * t4 - 3 * t5,     (t3 - 2 * t4 + t5) / 2};
}

/// One side of a four-sided hole: a neighbour's edge, walked by its own
/// (smooth) parameter mapped to [0, 1], with the neighbour's surface beside it.
struct HoleSide {
  TopoDS_Edge e;
  TopoDS_Face f;
  int order = 0;
  bool rev = false;
  double f0 = 0, l0 = 1;
  Handle(Geom_Curve) c;
  Handle(Geom2d_Curve) pc;
  double t_of(double s) const { return f0 + (rev ? 1 - s : s) * (l0 - f0); }
  /// Position and its first two derivatives in s.
  void d2(double s, gp_Pnt& p, gp_Vec& v1, gp_Vec& v2) const {
    c->D2(t_of(s), p, v1, v2);
    double k = (rev ? -1 : 1) * (l0 - f0);
    v1 *= k, v2 *= k * k;
  }
  gp_Pnt at(double s) const { return c->Value(t_of(s)); }
  SurfPt surf(double s) const {
    gp_Pnt2d q = pc->Value(t_of(s));
    return eval(f, q.X(), q.Y());
  }
};

/// A Coons patch through four sides (bottom v = 0, right u = 1, top v = 1,
/// left u = 0) matching each neighbour's position, tangent plane and
/// curvature across (quintic Hermite blending), with Gregory's rational
/// corners so that neighbours' twists need not agree. Sampled on a
/// Chebyshev grid and interpolated as one C2 B-spline.
Handle(Geom_BSplineSurface) coons_g2(const HoleSide& B, const HoleSide& R, const HoleSide& T, const HoleSide& L) {
  // Side positions and their derivatives along the side.
  auto pos = [&](const HoleSide& sd, double s, int k) -> gp_Vec {
    gp_Pnt p;
    gp_Vec v1, v2;
    sd.d2(s, p, v1, v2);
    return k == 0 ? gp_Vec(p.XYZ()) : k == 1 ? v1 : v2;
  };
  // Cross data of order i (1, 2) across a side at s; `a0`/`a1` the
  // adjacent sides' derivatives (orders 1 and 2) where this side starts and ends.
  struct Cross {
    const HoleSide* sd;
    gp_Vec a1[2], a2[2];  // first, second derivative of the adjacent sides at s = 0, 1
  };
  auto cross = [&](const Cross& cr, double s, int i) -> gp_Vec {
    SurfPt sp = cr.sd->surf(s);
    const gp_Vec& n = sp.n;
    gp_Vec w1 = cr.a1[0] * (1 - s) + cr.a1[1] * s;
    if (cr.sd->order >= 1) w1 -= n * w1.Dot(n);
    if (i == 1) return w1;
    auto tang = [&](const gp_Vec& x) { return x - n * x.Dot(n); };
    gp_Vec w2 = tang(cr.a2[0]) * (1 - s) + tang(cr.a2[1]) * s;
    if (cr.sd->order >= 1 && w1.Magnitude() > 1e-12) {
      w2 += curvature_vector(sp, w1.Normalized()) * w1.SquareMagnitude();
    } else {
      w2 += n * ((1 - s) * cr.a2[0].Dot(n) + s * cr.a2[1].Dot(n));
    }
    return w2;
  };
  // v-sides (bottom, top) run along u; u-sides (left, right) along v.
  Cross cb{&B, {pos(L, 0, 1), pos(R, 0, 1)}, {pos(L, 0, 2), pos(R, 0, 2)}};
  Cross ct{&T, {pos(L, 1, 1), pos(R, 1, 1)}, {pos(L, 1, 2), pos(R, 1, 2)}};
  Cross cl{&L, {pos(B, 0, 1), pos(T, 0, 1)}, {pos(B, 0, 2), pos(T, 0, 2)}};
  Cross cr{&R, {pos(B, 1, 1), pos(T, 1, 1)}, {pos(B, 1, 2), pos(T, 1, 2)}};
  // Derivative of order k (0–2) of a function of s at an end (one-sided differences).
  auto dend = [](const std::function<gp_Vec(double)>& f, double s0, int k) -> gp_Vec {
    if (k == 0) return f(s0);
    double h = k == 1 ? 1e-4 : 1e-3, sg = s0 < 0.5 ? 1 : -1;
    gp_Vec f0 = f(s0), f1 = f(s0 + sg * h), f2 = f(s0 + 2 * sg * h), f3 = f(s0 + 3 * sg * h);
    if (k == 1) return (f0 * -3 + f1 * 4 - f2) / (2 * h * sg);
    return (f0 * 2 - f1 * 5 + f2 * 4 - f3) / (h * h);
  };
  // Corner mixed derivatives: [corner u][corner v][i][j], with both readings where i, j ≥ 1.
  gp_Vec pure[2][2][3][3], fromU[2][2][3][3], fromV[2][2][3][3];
  const HoleSide* vside[2] = {&B, &T};
  const HoleSide* uside[2] = {&L, &R};
  const Cross* vcross[2] = {&cb, &ct};
  const Cross* ucross[2] = {&cl, &cr};
  for (int cu = 0; cu < 2; cu++)
    for (int cv = 0; cv < 2; cv++)
      for (int i = 0; i < 3; i++)
        for (int j = 0; j < 3; j++) {
          if (j == 0) pure[cu][cv][i][j] = pos(*vside[cv], cu, i);
          else if (i == 0) pure[cu][cv][i][j] = pos(*uside[cu], cv, j);
          else {
            const Cross* uc = ucross[cu];
            const Cross* vc = vcross[cv];
            fromU[cu][cv][i][j] = dend([&](double s) { return cross(*uc, s, i); }, cv, j);
            fromV[cu][cv][i][j] = dend([&](double s) { return cross(*vc, s, j); }, cu, i);
          }
        }
  auto S = [&](double u, double v, const std::array<gp_Vec, 6>& bu, const std::array<gp_Vec, 6>& lv) {
    std::array<double, 6> hu = hermite5(u), hv = hermite5(v);
    // bu: bottom position, cross 1, 2, then top; lv: left, then right.
    gp_Vec P1(0, 0, 0), P2(0, 0, 0);
    for (int k = 0; k < 6; k++) P1 += bu[k] * hv[k], P2 += lv[k] * hu[k];
    gp_Vec Tt(0, 0, 0);
    for (int a = 0; a < 6; a++)
      for (int b = 0; b < 6; b++) {
        int cu = a / 3, i = a % 3, cv = b / 3, j = b % 3;
        gp_Vec m;
        if (i == 0 || j == 0) m = pure[cu][cv][i][j];
        else {
          double du = u - cu, dv = v - cv, wu = du * du, wv = dv * dv;
          m = wu + wv < 1e-30 ? (fromU[cu][cv][i][j] + fromV[cu][cv][i][j]) / 2 : (fromU[cu][cv][i][j] * wu + fromV[cu][cv][i][j] * wv) / (wu + wv);
        }
        Tt += m * (hu[a] * hv[b]);
      }
    return gp_Pnt((P1 + P2 - Tt).XYZ());
  };
  // Chebyshev–Lobatto nodes: dense at the sides, where the cross data must hold.
  const int n = 41;
  std::vector<double> nodes;
  for (int k = 0; k < n; k++) nodes.push_back(0.5 - 0.5 * std::cos(M_PI * k / (n - 1)));
  std::vector<std::array<gp_Vec, 6>> ub(n), vb(n);
  for (int k = 0; k < n; k++) {
    double t = nodes[k];
    ub[k] = {pos(B, t, 0), cross(cb, t, 1), cross(cb, t, 2), pos(T, t, 0), cross(ct, t, 1), cross(ct, t, 2)};
    vb[k] = {pos(L, t, 0), cross(cl, t, 1), cross(cl, t, 2), pos(R, t, 0), cross(cr, t, 1), cross(cr, t, 2)};
  }
  std::vector<gp_Pnt> grid(n * n);
  for (int j = 0; j < n; j++)
    for (int i = 0; i < n; i++) grid[j * n + i] = S(nodes[i], nodes[j], ub[i], vb[j]);
  return interpolate_grid(grid, n, n, nodes, nodes, false);
}

/// Orders four edges round a hole as bottom, right, top, left (see coons_g2).
std::array<HoleSide, 4> hole_sides(std::vector<HoleSide> sides) {
  auto ends = [](const HoleSide& sd, bool end) {
    double f, l;
    Handle(Geom_Curve) c = BRep_Tool::Curve(sd.e, f, l);
    return c->Value(end ? l : f);
  };
  // Chain them: each next side starts where the last ended.
  std::vector<HoleSide> loop{sides[0]};
  loop[0].rev = false;
  std::vector<bool> used(4, false);
  used[0] = true;
  for (int k = 1; k < 4; k++) {
    gp_Pnt last = ends(loop.back(), !loop.back().rev);
    int best = -1;
    bool brev = false;
    double bd = 1e300;
    for (int m = 0; m < 4; m++) {
      if (used[m]) continue;
      double d0 = ends(sides[m], false).Distance(last), d1 = ends(sides[m], true).Distance(last);
      if (std::min(d0, d1) < bd) bd = std::min(d0, d1), best = m, brev = d1 < d0;
    }
    if (bd > 1e-3) throw Standard_Failure("the four edges do not run round one hole");
    used[best] = true;
    loop.push_back(sides[best]);
    loop.back().rev = brev;
  }
  // Loop: Q0 →B→ Q1 →R→ Q2 →(T reversed)→ Q3 →(L reversed)→ Q0.
  std::array<HoleSide, 4> out{loop[0], loop[1], loop[2], loop[3]};
  out[2].rev = !out[2].rev;
  out[3].rev = !out[3].rev;
  return out;
}

TopoDS_Face face_at(const TopoDS_Shape& s, int i) {
  TopTools_IndexedMapOfShape faces;
  TopExp::MapShapes(s, TopAbs_FACE, faces);
  if (i < 0 || i >= faces.Extent()) throw Standard_Failure("unknown face");
  return TopoDS::Face(faces(i + 1));
}


}  // namespace

extern "C" {

/// Fills a hole bounded by curves and edges with one smooth face
/// (GeomPlate, approximated as a C2 B-spline). Constraint data, one after another:
///   0 order nsegs segs…   a bounding curve (a path), G0
///   1 order h e           a bounding edge e of body h, with continuity
///                         `order` (0 G0, 1 G1, 2 G2) to the face beside it
///   2 order nsegs segs…   a curve the surface passes through (not a bound)
///   3 x y z               a point it passes through
/// opts: degree, points per curve, iterations, max degree, max segments,
/// 3D tolerance. surf_data gets the G0 (mm), G1 (radians) and G2 errors.
int occ_fill(void* p, const double* d, int len, const double* opts) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    // GeomPlate directly (BRepFill_Filling hands G2 to GeomPlate as the enum's
    // value, which GeomPlate rejects).
    const int npts = (int)opts[1];
    const double tol3d = opts[5], tolang = 0.01, tolcurv = 0.1;
    GeomPlate_BuildPlateSurface plate((int)opts[0], npts, (int)opts[2], 1e-5, tol3d, tolang, tolcurv);
    std::vector<TopoDS_Edge> bound;
    std::vector<std::unique_ptr<Edges>> owned;
    std::vector<HoleSide> g2_sides;
    bool other_constraints = false, any_g1 = false, inner = false;
    int at = 0;
    // Points along a curve: enough for its shape (a long spline has many spans).
    auto points_for = [&](const Adaptor3d_Curve& c) { return std::min(150, std::max(npts, 4 * c.NbIntervals(GeomAbs_C3))); };
    auto free_curve = [&](const TopoDS_Edge& e) {
      Handle(BRepAdaptor_Curve) c = new BRepAdaptor_Curve(e);
      plate.Add(new GeomPlate_CurveConstraint(c, 0, points_for(*c), tol3d));
    };
    while (at < len) {
      int kind = (int)d[at++];
      if (kind == 0 || kind == 2) {
        at++;  // (order: curves are passed through)
        int n = (int)d[at++];
        if (at + n > len) throw Standard_Failure("fill data ends early");
        PathData path = build_path(d + at, n);
        at += n;
        other_constraints = true;
        inner |= kind == 2;
        for (BRepTools_WireExplorer x(path.wire); x.More(); x.Next()) {
          free_curve(x.Current());
          if (kind == 0) bound.push_back(x.Current());
        }
      } else if (kind == 1) {
        int order = std::min(2, std::max(0, (int)d[at])), h = (int)d[at + 1], ei = (int)d[at + 2];
        at += 3;
        const TopoDS_Shape* s = get(a, h);
        if (!s) throw Standard_Failure("unknown shape");
        owned.push_back(std::make_unique<Edges>(*s));
        Edges& ed = *owned.back();
        TopoDS_Edge e = ed.edge(ei);
        bound.push_back(e);
        std::vector<TopoDS_Face> fs = ed.around(e);
        if (!fs.empty()) {
          HoleSide hs;
          hs.e = e, hs.f = fs[0], hs.order = order;
          hs.c = BRep_Tool::Curve(e, hs.f0, hs.l0);
          hs.pc = BRep_Tool::CurveOnSurface(e, fs[0], hs.f0, hs.l0);
          if (!hs.c.IsNull() && !hs.pc.IsNull()) g2_sides.push_back(hs);
          else other_constraints = true;
        } else {
          other_constraints = true;
        }
        any_g1 |= order >= 1;
        if (order == 0) {
          free_curve(e);
          continue;
        }
        if (fs.empty()) throw Standard_Failure("a G1/G2 edge needs a face beside it");
        double f0, l0;
        Handle(Geom2d_Curve) pc = BRep_Tool::CurveOnSurface(e, fs[0], f0, l0);
        if (pc.IsNull()) throw Standard_Failure("an edge has no curve on its face");
        Handle(GeomAdaptor_Surface) S = new GeomAdaptor_Surface(BRep_Tool::Surface(fs[0]));
        Handle(Geom2dAdaptor_Curve) C2 = new Geom2dAdaptor_Curve(pc, f0, l0);
        Handle(Adaptor3d_CurveOnSurface) cos = new Adaptor3d_CurveOnSurface(C2, S);
        plate.Add(new GeomPlate_CurveConstraint(cos, order, points_for(BRepAdaptor_Curve(e)), tol3d, tolang, tolcurv));
      } else if (kind == 3) {
        other_constraints = inner = true;
        plate.Add(new GeomPlate_PointConstraint(gp_Pnt(d[at], d[at + 1], d[at + 2]), 0, tol3d));
        at += 3;
      } else {
        throw Standard_Failure("unknown fill constraint");
      }
    }
    if (bound.empty()) throw Standard_Failure("a filling needs its boundary");
    // Four neighbour edges met G1 or G2: a Coons–Gregory patch (G2 across
    // each side; tangent sides get the neighbour's curvature too) rather
    // than the plate, which drifts at G2.
    if (g2_sides.size() == 4 && !other_constraints && any_g1) {
      std::array<HoleSide, 4> sd = hole_sides(g2_sides);
      Handle(Geom_BSplineSurface) bs = coons_g2(sd[0], sd[1], sd[2], sd[3]);
      TopoDS_Face f = face_of_surface(bs);
      // Measure it against each neighbour along its side.
      double e0 = 0, e1 = 0, e2 = 0;
      for (int k = 0; k < 4; k++)
        for (int q = 1; q < 40; q++) {
          double t = q / 40.0;
          double u = k == 0 || k == 2 ? t : k == 1 ? 1 : 0, v = k == 1 || k == 3 ? t : k == 2 ? 1 : 0;
          SurfPt P = eval(f, u, v), N = sd[k].surf(t);
          e0 = std::max(e0, P.p.Distance(N.p));
          if (sd[k].order < 1) continue;
          e1 = std::max(e1, std::acos(std::min(1.0, std::fabs(P.n.Dot(N.n)))));
          gp_Pnt c;
          gp_Vec tan, d2;
          sd[k].d2(t, c, tan, d2);
          gp_Vec across = tan.Crossed(N.n);
          if (sd[k].order < 2 || across.Magnitude() < 1e-12) continue;
          across.Normalize();
          e2 = std::max(e2, (curvature_vector(P, across) - curvature_vector(N, across)).Magnitude());
        }
      a->surf_data = {e0, e1, e2};
      return push(a, f);
    }
    // A flat boundary met only in position: the plane itself (exact).
    if (!any_g1 && !inner) {
      Handle(TopTools_HSequenceOfShape) seq = new TopTools_HSequenceOfShape, wires;
      for (auto& e : bound) seq->Append(BRepBuilderAPI_Copy(e, Standard_True, Standard_False).Shape());
      ShapeAnalysis_FreeBounds::ConnectEdgesToWires(seq, 1e-4, Standard_False, wires);
      if (!wires.IsNull() && wires->Length() == 1) {
        BRepBuilderAPI_MakeFace flat(TopoDS::Wire(wires->Value(1)), Standard_True);
        if (flat.IsDone()) {
          a->surf_data = {0, 0, 0};
          return push(a, flat.Face());
        }
      }
    }
    plate.Perform();
    if (!plate.IsDone()) throw Standard_Failure("could not fill the boundary");
    Handle(GeomPlate_Surface) ps = plate.Surface();
    double dmax = std::max(1e-4, 10 * plate.G0Error());
    GeomPlate_MakeApprox ap(ps, tol3d, (int)opts[4], (int)opts[3], dmax, 0, GeomAbs_C2);
    Handle(Geom_BSplineSurface) bs = ap.Surface();
    if (bs.IsNull()) throw Standard_Failure("could not approximate the filling");
    // Bounded by the boundary's own image on the surface (each curve
    // projected onto it), so the face's edges lie exactly on it: what meets
    // this face later (a blend, a sew) meets the surface itself.
    Handle(TopTools_HSequenceOfShape) seq = new TopTools_HSequenceOfShape, wires;
    for (auto& e : bound) {
      double f0, l0;
      Handle(Geom_Curve) c3 = BRep_Tool::Curve(e, f0, l0);
      TopoDS_Shape onto;
      if (!c3.IsNull()) {
        try {
          Handle(Geom2d_Curve) c2 = pcurve_by_projection(c3, f0, l0, bs);
          if (!c2.IsNull()) {
            TopoDS_Edge ne = BRepBuilderAPI_MakeEdge(c2, bs, c2->FirstParameter(), c2->LastParameter()).Edge();
            BRepLib::BuildCurves3d(ne, 1e-7, GeomAbs_C2, 14, 400);
            if (e.Orientation() == TopAbs_REVERSED) ne.Reverse();
            onto = ne;
          }
        } catch (Standard_Failure&) {
        }
      }
      if (onto.IsNull()) onto = BRepBuilderAPI_Copy(e, Standard_True, Standard_False).Shape();
      seq->Append(onto);
    }
    ShapeAnalysis_FreeBounds::ConnectEdgesToWires(seq, std::max(1e-4, 4 * plate.G0Error()), Standard_False, wires);
    if (wires.IsNull() || wires->Length() != 1) throw Standard_Failure("the boundary does not close into one loop");
    TopoDS_Wire w = TopoDS::Wire(wires->Value(1));
    BRepBuilderAPI_MakeFace mf(bs, w, Standard_True);
    if (!mf.IsDone()) throw Standard_Failure("could not bound the filling by its edges");
    ShapeFix_Face fix(mf.Face());
    fix.SetPrecision(1e-6);
    fix.Perform();
    TopoDS_Face f = fix.Face();
    // A wire running the wrong way round bounds everything but the patch.
    GProp_GProps gs;
    BRepGProp::SurfaceProperties(f, gs);
    double u0, u1, v0, v1;
    bs->Bounds(u0, u1, v0, v1);
    GProp_GProps whole;
    BRepGProp::SurfaceProperties(BRepBuilderAPI_MakeFace(bs, u0, u1, v0, v1, 1e-7).Face(), whole);
    if (gs.Mass() <= 0 || gs.Mass() > whole.Mass() * 1.0001) {
      w.Reverse();
      ShapeFix_Face fix2(BRepBuilderAPI_MakeFace(bs, w, Standard_True).Face());
      fix2.Perform();
      f = fix2.Face();
    }
    a->surf_data = {plate.G0Error(), plate.G1Error(), plate.G2Error()};
    return push(a, f);
  });
}

/// A patch bounded by 2–4 curves (GeomFill): style 0 stretch (flattest),
/// 1 Coons, 2 curved (roundest). Two curves: a ruled-like Coons strip.
int occ_boundary_surface(void* p, const int* hs, int n, int style) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    if (n < 2 || n > 4) throw Standard_Failure("a boundary patch takes two, three or four curves");
    std::vector<Handle(Geom_BSplineCurve)> c;
    for (int i = 0; i < n; i++) {
      const TopoDS_Shape* s = get(a, hs[i]);
      if (!s) return -1;
      c.push_back(bspline_of(wire_of(*s)));
    }
    if (style == 1 && n >= 3) {
      // Coons (bilinearly blended): ruled both ways less the corners'
      // bilinear patch. OCCT's B-spline Coons is not available for every
      // case, so built here and interpolated as one C2 B-spline.
      std::vector<std::unique_ptr<ArcCurve>> side;
      for (int i = 0; i < n; i++) side.push_back(std::make_unique<ArcCurve>(BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(c[i]).Edge()).Wire()));
      // Chain them: each starts where the last ends (turned round if need be).
      std::vector<bool> rev(n, false);
      for (int i = 1; i < n; i++) {
        gp_Pnt end = rev[i - 1] ? side[i - 1]->at(0) : side[i - 1]->at(1);
        if (side[i]->at(0).Distance(end) > side[i]->at(1).Distance(end)) rev[i] = true;
      }
      auto at = [&](int i, double t) { return side[i]->at(rev[i] ? 1 - t : t); };
      // Bottom c0 (u), right c1 (v), top c2 (reversed u), left c3 (reversed v); three sides: the left collapses.
      gp_Pnt P00 = at(0, 0), P10 = at(0, 1), P11 = at(1, 1), P01 = n == 4 ? at(2, 1) : at(2, 1);
      auto bottom = [&](double u) { return at(0, u); };
      auto right = [&](double v) { return at(1, v); };
      auto top = [&](double u) { return at(2, 1 - u); };
      auto left = [&](double v) { return n == 4 ? at(3, 1 - v) : P00.Translated(gp_Vec(P00, P01) * v); };
      if (n == 3) P01 = at(2, 1);
      const int nu = 40, nv = 40;
      std::vector<double> up, vp;
      for (int i = 0; i < nu; i++) up.push_back((double)i / (nu - 1));
      for (int j = 0; j < nv; j++) vp.push_back((double)j / (nv - 1));
      std::vector<gp_Pnt> grid(nu * nv);
      for (int j = 0; j < nv; j++)
        for (int i = 0; i < nu; i++) {
          double u = up[i], v = vp[j];
          gp_XYZ r = bottom(u).XYZ() * (1 - v) + top(u).XYZ() * v + left(v).XYZ() * (1 - u) + right(v).XYZ() * u;
          r -= P00.XYZ() * ((1 - u) * (1 - v)) + P10.XYZ() * (u * (1 - v)) + P01.XYZ() * ((1 - u) * v) + P11.XYZ() * (u * v);
          grid[j * nu + i] = gp_Pnt(r);
        }
      return push(a, face_of_surface(interpolate_grid(grid, nu, nv, up, vp, false)));
    }
    GeomFill_FillingStyle st = style == 0 ? GeomFill_StretchStyle : style == 2 ? GeomFill_CurvedStyle : GeomFill_CoonsStyle;
    GeomFill_BSplineCurves f;
    if (n == 2) f.Init(c[0], c[1], st);
    else if (n == 3) f.Init(c[0], c[1], c[2], st);
    else f.Init(c[0], c[1], c[2], c[3], st);
    Handle(Geom_BSplineSurface) s = f.Surface();
    if (s.IsNull()) throw Standard_Failure("the curves do not bound a patch (do they meet corner to corner?)");
    return push(a, face_of_surface(s));
  });
}

/// A sheet through curves in order (ruled: flat between them), or closed
/// back to the first. Not capped.
int occ_section_surface(void* p, const int* hs, int n, int ruled, int closed) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    if (n < 2) throw Standard_Failure("a section surface needs two curves or more");
    BRepOffsetAPI_ThruSections ts(Standard_False, ruled != 0, 1e-6);
    ts.CheckCompatibility(Standard_True);
    for (int i = 0; i < n; i++) {
      const TopoDS_Shape* s = get(a, hs[i]);
      if (!s) return -1;
      ts.AddWire(wire_of(*s));
    }
    if (closed) {
      const TopoDS_Shape* s = get(a, hs[0]);
      ts.AddWire(wire_of(*s));
    }
    ts.Build();
    if (!ts.IsDone()) throw Standard_Failure("could not skin those curves");
    return push(a, ts.Shape());
  });
}

/// A Gordon surface through a network: profiles (running along u, in order
/// along v) and guides (running along v, in order along u) that cross each
/// other. Closed profiles make a tube (periodic in u). The network is
/// sampled (samples × samples per span) and interpolated as one C2
/// B-spline; refined until it lies within `tol` of every curve.
/// surf_data: worst gap where curves cross (mm), worst deviation from the
/// curves (mm), samples used.
int occ_network_surface(void* p, const int* ph, int np, const int* gh, int ng, double tol) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    if (np < 2 || ng < 2) throw Standard_Failure("a network needs two profiles and two guides at least");
    std::vector<std::unique_ptr<ArcCurve>> prof, guide;
    for (int i = 0; i < np; i++) {
      const TopoDS_Shape* s = get(a, ph[i]);
      if (!s) return -1;
      prof.push_back(std::make_unique<ArcCurve>(wire_of(*s)));
    }
    for (int j = 0; j < ng; j++) {
      const TopoDS_Shape* s = get(a, gh[j]);
      if (!s) return -1;
      guide.push_back(std::make_unique<ArcCurve>(wire_of(*s)));
      if (guide.back()->closed) throw Standard_Failure("a guide must not be closed (close the profiles instead)");
    }
    const bool per = prof[0]->closed;
    for (auto& c : prof)
      if (c->closed != per) throw Standard_Failure("the profiles must be all closed or all open");
    // Where each profile crosses each guide.
    std::vector<std::vector<double>> s(np, std::vector<double>(ng)), t(np, std::vector<double>(ng));
    std::vector<std::vector<gp_Pnt>> P(np, std::vector<gp_Pnt>(ng));
    double gap = 0;
    for (int i = 0; i < np; i++)
      for (int j = 0; j < ng; j++) {
        // Alternate projections from the closest sampled pair.
        double bi = 0, bj = 0, bd = 1e300;
        for (int x = 0; x <= 400; x += 5)
          for (int y = 0; y <= 400; y += 5) {
            double dd = prof[i]->samples[x].SquareDistance(guide[j]->samples[y]);
            if (dd < bd) bd = dd, bi = x / 400.0, bj = y / 400.0;
          }
        for (int k = 0; k < 8; k++) {
          bj = guide[j]->nearest(prof[i]->at(bi));
          bi = prof[i]->nearest(guide[j]->at(bj));
        }
        gp_Pnt q1 = prof[i]->at(bi), q2 = guide[j]->at(bj);
        gap = std::max(gap, q1.Distance(q2));
        s[i][j] = bi, t[i][j] = bj;
        P[i][j] = gp_Pnt((q1.XYZ() + q2.XYZ()) / 2);
      }
    double scale = 0;
    for (auto& c : prof) scale = std::max(scale, c->length);
    if (gap > std::max(1e-3, 1e-3 * scale)) {
      std::ostringstream m;
      m << "the network's curves miss each other by up to " << gap << " mm: every profile must cross every guide";
      throw Standard_Failure(m.str().c_str());
    }
    // Common parameters: u_j for the guides, v_i for the profiles.
    std::vector<double> uj(ng, 0.0), vi(np, 0.0);
    std::vector<std::vector<double>> sun(np, std::vector<double>(ng));  // unwrapped profile fractions
    for (int i = 0; i < np; i++) {
      double base = s[i][0];
      for (int j = 0; j < ng; j++) {
        double v = s[i][j];
        if (per) {
          v -= base;
          v -= std::floor(v);
          if (j > 0 && v < 1e-9) v += 1;  // a guide at the seam
        }
        sun[i][j] = v;
      }
      for (int j = 1; j < ng; j++)
        if (sun[i][j] <= sun[i][j - 1]) throw Standard_Failure("the guides must be given in order along the profiles");
    }
    for (int j = 0; j < ng; j++) {
      for (int i = 0; i < np; i++) uj[j] += sun[i][j];
      uj[j] /= np;
    }
    for (int i = 0; i < np; i++) {
      for (int j = 0; j < ng; j++) vi[i] += t[i][j];
      vi[i] /= ng;
    }
    for (int i = 1; i < np; i++)
      if (vi[i] <= vi[i - 1]) throw Standard_Failure("the profiles must be given in order along the guides");
    // Each curve's own fraction as a function of the common parameter.
    std::vector<std::unique_ptr<Monotone>> pmap, gmap;
    std::vector<double> pbase(np);
    for (int i = 0; i < np; i++) {
      std::vector<double> xs, ys;
      if (per) {
        pbase[i] = s[i][0];
        for (int j = 0; j < ng; j++) xs.push_back(uj[j]), ys.push_back(sun[i][j]);
        xs.push_back(1), ys.push_back(1);
        // Periodic: extend one span either side so the slopes wrap.
        std::vector<double> X{uj[ng - 1] - 1}, Y{sun[i][ng - 1] - 1};
        X.insert(X.end(), xs.begin(), xs.end()), Y.insert(Y.end(), ys.begin(), ys.end());
        X.push_back(1 + uj[1]), Y.push_back(1 + sun[i][1]);
        pmap.push_back(std::make_unique<Monotone>(X, Y));
      } else {
        pbase[i] = 0;
        if (uj[0] > 1e-9) xs.push_back(0), ys.push_back(0);
        for (int j = 0; j < ng; j++) xs.push_back(uj[j]), ys.push_back(s[i][j]);
        if (uj[ng - 1] < 1 - 1e-9) xs.push_back(1), ys.push_back(1);
        for (size_t k = 1; k < ys.size(); k++)
          if (ys[k] <= ys[k - 1]) throw Standard_Failure("a profile runs the other way: draw them all the same way round");
        pmap.push_back(std::make_unique<Monotone>(xs, ys));
      }
    }
    for (int j = 0; j < ng; j++) {
      std::vector<double> xs, ys;
      if (vi[0] > 1e-9) xs.push_back(0), ys.push_back(0);
      for (int i = 0; i < np; i++) xs.push_back(vi[i]), ys.push_back(t[i][j]);
      if (vi[np - 1] < 1 - 1e-9) xs.push_back(1), ys.push_back(1);
      for (size_t k = 1; k < ys.size(); k++)
        if (ys[k] <= ys[k - 1]) throw Standard_Failure("a guide runs the other way: draw them all the same way round");
      gmap.push_back(std::make_unique<Monotone>(xs, ys));
    }
    Cardinal Lu(uj, per, 1.0), Lv(vi, false, 1.0);
    auto prof_at = [&](int i, double u) { return prof[i]->at(pbase[i] + pmap[i]->at(u)); };
    auto guide_at = [&](int j, double v) { return guide[j]->at(gmap[j]->at(v)); };
    Handle(Geom_BSplineSurface) best;
    double dev = 0;
    int used = 0;
    for (int per_span = 6; per_span <= 48; per_span *= 2) {
      int nu = std::min(400, std::max(24, per_span * ng)), nv = std::min(400, std::max(16, per_span * (np - 1)));
      Handle(Geom_BSplineSurface) tried;
      // Even samples, plus the curves' own parameters: each profile is then a
      // row of the grid and each guide a column (the curves' bent pacing is
      // only C1 there; a node on it keeps the fit exact along the curve).
      auto merged = [](std::vector<double> xs, const std::vector<double>& extra, double gap) {
        for (double e : extra) {
          auto it = std::lower_bound(xs.begin(), xs.end(), e);
          size_t i = it - xs.begin();
          if (i < xs.size() && xs[i] - e < gap) xs[i] = e;
          else if (i > 0 && e - xs[i - 1] < gap) xs[i - 1] = e;
          else xs.insert(it, e);
        }
        return xs;
      };
      std::vector<double> up, vp;
      for (int i = 0; i < nu; i++) up.push_back(per ? (double)i / nu : (double)i / (nu - 1));
      for (int j = 0; j < nv; j++) vp.push_back((double)j / (nv - 1));
      std::vector<double> ujs(uj.begin(), uj.end());
      up = merged(up, ujs, 0.35 / nu);
      vp = merged(vp, vi, 0.35 / nv);
      nu = (int)up.size(), nv = (int)vp.size();
      if (per) up.push_back(1.0);
      std::vector<std::vector<double>> wu(nu), wv(nv);
      for (int i = 0; i < nu; i++) wu[i] = Lu.weights(up[i]);
      for (int j = 0; j < nv; j++) wv[j] = Lv.weights(vp[j]);
      // The curves at the sample parameters.
      std::vector<std::vector<gp_Pnt>> C(np, std::vector<gp_Pnt>(nu)), G(ng, std::vector<gp_Pnt>(nv));
      for (int i = 0; i < np; i++)
        for (int k = 0; k < nu; k++) C[i][k] = prof_at(i, up[k]);
      for (int j = 0; j < ng; j++)
        for (int k = 0; k < nv; k++) G[j][k] = guide_at(j, vp[k]);
      std::vector<gp_Pnt> grid(nu * nv);
      for (int jv = 0; jv < nv; jv++)
        for (int iu = 0; iu < nu; iu++) {
          gp_XYZ su(0, 0, 0), sv(0, 0, 0), tt(0, 0, 0);
          for (int i = 0; i < np; i++) su += C[i][iu].XYZ() * wv[jv][i];
          for (int j = 0; j < ng; j++) sv += G[j][jv].XYZ() * wu[iu][j];
          for (int i = 0; i < np; i++)
            for (int j = 0; j < ng; j++) tt += P[i][j].XYZ() * (wv[jv][i] * wu[iu][j]);
          grid[jv * nu + iu] = gp_Pnt(su + sv - tt);
        }
      tried = interpolate_grid(grid, nu, nv, up, vp, per);
      // How far the surface strays from each curve, between nodes (found
      // from where on the surface each curve point should lie).
      double d = 0;
      const int m = 4 * std::max(nu, nv);
      for (int k = 0; k <= m; k++) {
        double t = (double)k / m;
        for (int i = 0; i < np; i++) d = std::max(d, local_distance(tried, prof_at(i, t), t, vi[i]));
        for (int j = 0; j < ng; j++) d = std::max(d, local_distance(tried, guide_at(j, t), uj[j], t));
      }
      // Keep the best; stop when finer sampling stops helping.
      bool better = best.IsNull() || d < dev;
      if (better) best = tried, dev = d, used = per_span;
      if (dev <= tol || !better) break;
    }
    a->surf_data = {gap, dev, (double)used};
    return push(a, face_of_surface(best));
  });
}

/// A blend surface between edge chains of two bodies (each chain the indices
/// of one body's edges, running end to end, closed or open), leaving each
/// with continuity 0–2 to the face beside it; `bulge` scales how far the
/// blend runs along each side before turning. `samples` rows across (0:
/// doubled from 32 until the blend keeps to the edges within 1e-5 mm).
/// surf_data: rows used.
int occ_blend_surface(void* p, int ha, const int* ea, int na, int ca, double ba, int hb, const int* eb, int nb, int cb, double bb, int samples) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* sa = get(a, ha);
    const TopoDS_Shape* sb = get(a, hb);
    if (!sa || !sb) return -1;
    Edges eda(*sa), edb(*sb);
    Rim A, B;
    A.wire = chain(picked_edges(eda, ea, na)), A.body = sa, A.edges = &eda;
    B.wire = chain(picked_edges(edb, eb, nb)), B.body = sb, B.edges = &edb;
    A.arc = std::make_unique<ArcCurve>(A.wire);
    B.arc = std::make_unique<ArcCurve>(B.wire);
    if (A.arc->closed != B.arc->closed) throw Standard_Failure("blend: one side is a closed loop and the other is not");
    const bool per = A.arc->closed;
    // Line B up with A: start opposite A's start, running the same way.
    double off = 0;
    bool flip = false;
    if (per) {
      off = B.arc->nearest(A.arc->at(0));
      double ahead = B.arc->nearest(A.arc->at(0.1));
      double fwd = ahead - off;
      fwd -= std::floor(fwd);
      flip = fwd > 0.5;
    } else {
      gp_Pnt a0 = A.arc->at(0), a1 = A.arc->at(1), b0 = B.arc->at(0), b1 = B.arc->at(1);
      flip = a0.Distance(b0) + a1.Distance(b1) > a0.Distance(b1) + a1.Distance(b0);
    }
    auto bfrac = [&](double t) {
      double v = flip ? off - t : off + t;
      if (!per) v = flip ? 1 - t : t;
      return v;
    };
    Handle(Geom_BSplineSurface) s;
    int n = samples > 0 ? std::max(8, samples) : 32;
    for (;; n *= 2) {
      std::vector<double> up;
      for (int i = 0; i < n; i++) up.push_back(per ? (double)i / n : (double)i / (n - 1));
      if (per) up.push_back(1.0);
      std::vector<double> vp;
      for (int j = 0; j < 6; j++) vp.push_back(j / 5.0);
      // Rows of Bezier poles; interpolated across, they are the surface's poles in v.
      std::vector<gp_Pnt> grid(n * 6);
      for (int i = 0; i < n; i++) {
        Rim::Probe pa = A.probe(up[i]), pb = B.probe(bfrac(up[i]));
        std::array<gp_Pnt, 6> P = blend_poles(pa.p, pa.out, pa.k, ca, ba, pb.p, pb.out, pb.k, cb, bb);
        for (int j = 0; j < 6; j++) grid[j * n + i] = P[j];
      }
      // Interpolate each pole row along u, then use them as Bezier poles in v.
      std::vector<Handle(Geom_BSplineCurve)> rows;
      for (int j = 0; j < 6; j++) {
        Handle(TColgp_HArray1OfPnt) pts = new TColgp_HArray1OfPnt(1, n);
        for (int i = 0; i < n; i++) pts->SetValue(i + 1, grid[j * n + i]);
        Handle(TColStd_HArray1OfReal) par = new TColStd_HArray1OfReal(1, per ? n + 1 : n);
        for (int i = 0; i < (per ? n + 1 : n); i++) par->SetValue(i + 1, up[i]);
        GeomAPI_Interpolate in(pts, par, per, 1e-9);
        in.Perform();
        if (!in.IsDone()) throw Standard_Failure("could not run the blend along the edges");
        rows.push_back(in.Curve());
      }
      const int np = rows[0]->NbPoles();
      TColgp_Array2OfPnt poles(1, np, 1, 6);
      for (int k = 1; k <= np; k++)
        for (int j = 1; j <= 6; j++) poles.SetValue(k, j, rows[j - 1]->Pole(k));
      TColStd_Array1OfReal uk(1, rows[0]->NbKnots()), vk(1, 2);
      TColStd_Array1OfInteger um(1, rows[0]->NbKnots()), vm(1, 2);
      rows[0]->Knots(uk), rows[0]->Multiplicities(um);
      vk(1) = 0, vk(2) = 1, vm(1) = vm(2) = 6;
      s = new Geom_BSplineSurface(poles, uk, vk, um, vm, rows[0]->Degree(), 5, per, false);

      // Rows match the edges exactly; between them the fit may stray: check halfway.
      double worst = 0;
      for (int i = 0; i + 1 < (int)up.size(); i++) {
        double mid = (up[i] + up[i + 1]) / 2;
        worst = std::max(worst, s->Value(mid, 0).Distance(A.probe(mid).p));
        worst = std::max(worst, s->Value(mid, 1).Distance(B.probe(bfrac(mid)).p));
      }
      if (worst < 1e-5 || n >= 512 || samples > 0) break;
    }
    a->surf_data = {(double)n};
    return push(a, face_of_surface(s));
  });
}

/// A blend curve between the ends of two curves (end 0 start, 1 end), G0–G2
/// to each (Bezier, degree 5); bulge scales the tangent runs.
int occ_blend_curve(void* p, int ha, int enda, int ca, double ba, int hb, int endb, int cb, double bb) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* sa = get(a, ha);
    const TopoDS_Shape* sb = get(a, hb);
    if (!sa || !sb) return -1;
    auto end_of = [](const TopoDS_Shape& s, int end, gp_Pnt& q, gp_Vec& d, gp_Vec& k) {
      BRepAdaptor_CompCurve c(wire_of(s), Standard_True);
      double t = end ? c.LastParameter() : c.FirstParameter();
      gp_Vec d1, d2;
      c.D2(t, q, d1, d2);
      if (d1.Magnitude() < 1e-14) throw Standard_Failure("a curve has no direction at its end");
      // Leaving the curve: forward past its end, backward past its start.
      gp_Vec tan = d1.Normalized();
      // Curvature vector: (d2 − (d2·T)T)/|d1|².
      k = (d2 - tan * d2.Dot(tan)) / d1.SquareMagnitude();
      d = end ? tan : -tan;
    };
    gp_Pnt p0, p1;
    gp_Vec d0, d1, k0, k1;
    end_of(*sa, enda, p0, d0, k0);
    end_of(*sb, endb, p1, d1, k1);
    std::array<gp_Pnt, 6> P = blend_poles(p0, d0, k0, ca, ba, p1, d1, k1, cb, bb);
    TColgp_Array1OfPnt poles(1, 6);
    for (int j = 0; j < 6; j++) poles.SetValue(j + 1, P[j]);
    Handle(Geom_BezierCurve) bz = new Geom_BezierCurve(poles);
    return push(a, BRepBuilderAPI_MakeWire(BRepBuilderAPI_MakeEdge(bz).Edge()).Wire());
  });
}

/// Sews bodies (sheets, faces) into one shell, and closes it into a solid
/// when it has no free edge and `solid` is set. surf_data: free edges left.
int occ_sew(void* p, const int* hs, int n, double tol, int solid) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    std::vector<TopoDS_Shape> parts;
    for (int i = 0; i < n; i++) {
      const TopoDS_Shape* s = get(a, hs[i]);
      if (!s) return -1;
      parts.push_back(*s);
    }
    int free_edges = 0;
    TopoDS_Shape r = sew_faces(parts, tol, solid != 0, &free_edges);
    a->surf_data = {(double)free_edges};
    return push(a, r);
  });
}

/// The pieces of a body: kind 0 faces, 1 shells, 2 solids, 3 edges, 4 wires
/// (closed free boundaries). Pushes each; returns how many, the first in *first.
int occ_pieces(void* p, int h, int kind, int* first) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s0 = get(a, h);
    if (!s0) return -1;
    TopoDS_Shape s = *s0;
    std::vector<TopoDS_Shape> out;
    if (kind == 4) {
      ShapeAnalysis_FreeBounds fb(s, 1e-5, Standard_False, Standard_False);
      TopoDS_Compound closed = fb.GetClosedWires(), open = fb.GetOpenWires();
      for (TopExp_Explorer w(closed, TopAbs_WIRE); w.More(); w.Next()) out.push_back(w.Current());
      for (TopExp_Explorer w(open, TopAbs_WIRE); w.More(); w.Next()) out.push_back(w.Current());
    } else {
      TopAbs_ShapeEnum t = kind == 0 ? TopAbs_FACE : kind == 1 ? TopAbs_SHELL : kind == 2 ? TopAbs_SOLID : TopAbs_EDGE;
      TopTools_IndexedMapOfShape m;
      TopExp::MapShapes(s, t, m);
      for (int i = 1; i <= m.Extent(); i++) out.push_back(m(i));
    }
    *first = (int)a->shapes.size();
    for (auto& x : out) a->shapes.push_back(x);
    return (int)out.size();
  });
}

/// What a body is: solids, shells, faces, edges, free edges (on one face only), closed (0/1).
int occ_body_info(void* p, int h, double* out) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    Edges ed(*s);
    int solids = count_solids(*s), shells = 0, faces = 0, free_edges = 0;
    for (TopExp_Explorer x(*s, TopAbs_SHELL); x.More(); x.Next()) shells++;
    TopTools_IndexedMapOfShape fm;
    TopExp::MapShapes(*s, TopAbs_FACE, fm);
    faces = fm.Extent();
    for (int i = 1; i <= ed.map.Extent(); i++) {
      TopoDS_Edge e = TopoDS::Edge(ed.map(i));
      if (BRep_Tool::Degenerated(e)) continue;
      std::vector<TopoDS_Face> fs = ed.around(e);
      if (fs.size() == 1 && !BRep_Tool::IsClosed(e, fs[0])) free_edges++;
    }
    out[0] = solids, out[1] = shells, out[2] = faces, out[3] = ed.map.Extent(), out[4] = free_edges, out[5] = (solids > 0 || (faces > 0 && free_edges == 0)) ? 1 : 0;
    return 0;
  });
}

/// Indices of the edges on the body's open boundary (one face only).
int occ_free_edges(void* p, int h, const double** data, int* len) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    Edges ed(*s);
    a->surf_data.clear();
    for (int i = 1; i <= ed.map.Extent(); i++) {
      TopoDS_Edge e = TopoDS::Edge(ed.map(i));
      if (BRep_Tool::Degenerated(e)) continue;
      std::vector<TopoDS_Face> fs = ed.around(e);
      if (fs.size() == 1 && !BRep_Tool::IsClosed(e, fs[0])) a->surf_data.push_back(i - 1);
    }
    *data = a->surf_data.data();
    *len = (int)a->surf_data.size();
    return *len;
  });
}

/// A face extended by `length` mm past its natural bounds on the chosen
/// sides (bit 1 u-min, 2 u-max, 4 v-min, 8 v-max), as a new sheet.
int occ_extend_face(void* p, int h, int fi, double length, int sides) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    TopoDS_Face f = face_at(*s, fi), r;
    BRepLib::ExtendFace(f, length, (sides & 1) != 0, (sides & 2) != 0, (sides & 4) != 0, (sides & 8) != 0, r);
    if (r.IsNull()) throw Standard_Failure("could not extend the face");
    return push(a, r);
  });
}

/// Offsets a sheet (or each face of a body) by d mm along its normal: the offset surface.
int occ_offset_sheet(void* p, int h, double d) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    BRepOffsetAPI_MakeOffsetShape mo;
    mo.PerformBySimple(*s, d);
    if (!mo.IsDone()) throw Standard_Failure("could not offset the surface that far (it would fold over itself?)");
    return push(a, mo.Shape());
  });
}

/// Projects a curve (wire) onto a body: along its normals (mode 0) or along a
/// direction (mode 1). The projected curve nearest the original.
int occ_project(void* p, int hc, int hb, int mode, double dx, double dy, double dz) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* c = get(a, hc);
    const TopoDS_Shape* b = get(a, hb);
    if (!c || !b) return -1;
    TopoDS_Shape result;
    if (mode == 0) {
      BRepAlgo_NormalProjection np(*b);
      np.Add(*c);
      np.SetParams(1e-6, 1e-6, GeomAbs_C2, 14, 16);
      np.Build();
      if (!np.IsDone()) throw Standard_Failure("could not project the curve onto the surface");
      TopTools_ListOfShape wires;
      if (np.BuildWire(wires) && !wires.IsEmpty()) {
        TopoDS_Compound comp;
        BRep_Builder bb;
        bb.MakeCompound(comp);
        for (TopTools_ListIteratorOfListOfShape it(wires); it.More(); it.Next()) bb.Add(comp, it.Value());
        result = comp;
      } else {
        result = np.Projection();
      }
    } else {
      BRepProj_Projection pr(*c, *b, gp_Dir(dx, dy, dz));
      if (!pr.IsDone()) throw Standard_Failure("the curve does not land on the surface along that direction");
      result = pr.Shape();
    }
    // The wire nearest the original (a projection can land front and back).
    std::vector<TopoDS_Shape> wires;
    for (TopExp_Explorer w(result, TopAbs_WIRE); w.More(); w.Next()) wires.push_back(w.Current());
    if (wires.empty()) {
      Handle(TopTools_HSequenceOfShape) seq = new TopTools_HSequenceOfShape, out;
      for (TopExp_Explorer e(result, TopAbs_EDGE); e.More(); e.Next()) seq->Append(e.Current());
      if (seq->IsEmpty()) throw Standard_Failure("the curve does not land on the surface");
      ShapeAnalysis_FreeBounds::ConnectEdgesToWires(seq, 1e-4, Standard_False, out);
      for (int i = 1; i <= out->Length(); i++) wires.push_back(out->Value(i));
    }
    TopoDS_Shape best;
    double bd = 1e300;
    for (auto& w : wires) {
      BRepExtrema_DistShapeShape dist(w, *c);
      double dd = dist.IsDone() ? dist.Value() : 1e300;
      if (dd < bd) bd = dd, best = w;
    }
    return push(a, best);
  });
}

/// The iso-parametric curve of a face at fraction t of its u (dir 0) or v
/// (dir 1) range, kept to the face.
int occ_iso(void* p, int h, int fi, int dir, double t) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    TopoDS_Face f = face_at(*s, fi);
    double u0, u1, v0, v1;
    BRepTools::UVBounds(f, u0, u1, v0, v1);
    Handle(Geom_Surface) g = BRep_Tool::Surface(f);
    Handle(Geom_Curve) c;
    double c0, c1;
    if (dir == 0) c = g->UIso(u0 + t * (u1 - u0)), c0 = v0, c1 = v1;
    else c = g->VIso(v0 + t * (v1 - v0)), c0 = u0, c1 = u1;
    TopoDS_Edge e = BRepBuilderAPI_MakeEdge(c, c0, c1).Edge();
    // Keep the parts on the face (it may be trimmed).
    BRepAlgoAPI_Common common(e, f);
    TopoDS_Shape r = e;
    if (common.IsDone()) {
      TopExp_Explorer x(common.Shape(), TopAbs_EDGE);
      if (x.More()) r = common.Shape();
    }
    Handle(TopTools_HSequenceOfShape) seq = new TopTools_HSequenceOfShape, out;
    for (TopExp_Explorer x(r, TopAbs_EDGE); x.More(); x.Next()) seq->Append(x.Current());
    ShapeAnalysis_FreeBounds::ConnectEdgesToWires(seq, 1e-5, Standard_False, out);
    if (out.IsNull() || out->IsEmpty()) throw Standard_Failure("the iso-curve misses the face");
    // The longest run.
    TopoDS_Shape best;
    double bl = -1;
    for (int i = 1; i <= out->Length(); i++) {
      GProp_GProps gp;
      BRepGProp::LinearProperties(out->Value(i), gp);
      if (gp.Mass() > bl) bl = gp.Mass(), best = out->Value(i);
    }
    return push(a, best);
  });
}

/// Splits a body by tools (bodies, sheets, or curves lying on it): every piece.
int occ_split(void* p, int h, const int* ts, int n) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    BRepAlgoAPI_Splitter sp;
    TopTools_ListOfShape args, tools;
    args.Append(*s);
    for (int i = 0; i < n; i++) {
      const TopoDS_Shape* t = get(a, ts[i]);
      if (!t) return -1;
      tools.Append(*t);
    }
    sp.SetArguments(args);
    sp.SetTools(tools);
    sp.SetNonDestructive(Standard_True);
    sp.Build();
    if (!sp.IsDone()) throw Standard_Failure("could not split the body there");
    return push(a, sp.Shape());
  });
}

/// Trims a sheet by tools (as split), keeping the side holding `keep` (the
/// faces reached from the face nearest it without crossing a cut), or
/// dropping it (`drop` set). The kept faces are sewn back into one sheet.
int occ_trim(void* p, int h, const int* ts, int n, double x, double y, double z, int drop) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    int sh = occ_split(p, h, ts, n);
    if (sh < 0) return -1;
    TopoDS_Shape split = a->shapes[sh];
    std::vector<TopoDS_Shape> tools;
    for (int i = 0; i < n; i++) tools.push_back(a->shapes[ts[i]]);
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(split, TopAbs_FACE, faces);
    if (faces.Extent() == 0) throw Standard_Failure("trim: the body has no faces (trim sheets; split solids)");
    Edges ed(split);
    // An edge is a cut where it lies on a tool.
    auto on_tool = [&](const TopoDS_Edge& e) {
      BRepAdaptor_Curve c(e);
      gp_Pnt m = c.Value((c.FirstParameter() + c.LastParameter()) / 2);
      TopoDS_Vertex v = BRepBuilderAPI_MakeVertex(m).Vertex();
      for (auto& t : tools) {
        BRepExtrema_DistShapeShape d(v, t);
        if (d.IsDone() && d.Value() < 1e-4) return true;
      }
      return false;
    };
    gp_Pnt q(x, y, z);
    TopoDS_Vertex qv = BRepBuilderAPI_MakeVertex(q).Vertex();
    int start = 1;
    double bd = 1e300;
    for (int i = 1; i <= faces.Extent(); i++) {
      BRepExtrema_DistShapeShape d(qv, faces(i));
      if (d.IsDone() && d.Value() < bd) bd = d.Value(), start = i;
    }
    std::vector<bool> seen(faces.Extent() + 1, false);
    std::vector<int> stack{start};
    seen[start] = true;
    while (!stack.empty()) {
      int f = stack.back();
      stack.pop_back();
      for (TopExp_Explorer ex(faces(f), TopAbs_EDGE); ex.More(); ex.Next()) {
        TopoDS_Edge e = TopoDS::Edge(ex.Current());
        std::vector<TopoDS_Face> around = ed.around(e);
        if (around.size() < 2 || on_tool(e)) continue;
        for (auto& g : around) {
          int gi = faces.FindIndex(g);
          if (gi > 0 && !seen[gi]) seen[gi] = true, stack.push_back(gi);
        }
      }
    }
    std::vector<TopoDS_Shape> kept;
    for (int i = 1; i <= faces.Extent(); i++)
      if (seen[i] != (drop != 0)) kept.push_back(faces(i));
    if (kept.empty()) throw Standard_Failure("trim: nothing is left");
    if (kept.size() == (size_t)faces.Extent()) throw Standard_Failure("trim: the tool does not cut the body into two sides there");
    TopoDS_Shape r = kept.size() == 1 ? kept[0] : sew_faces(kept, 1e-6, false, nullptr);
    return push(a, r);
  });
}

/// Continuity across edges: every edge two faces of body `h` share, or (with
/// h2 ≥ 0) every boundary edge of `h` lying on a boundary edge of `h2`.
/// surf_text: JSON { edges: [{ edge, faces, g0, g1, g2, length }], max: { g0, g1, g2 } }
/// with g0 the gap (mm), g1 the crease angle (degrees), g2 the jump in
/// normal curvature across the edge (1/mm).
const char* occ_continuity(void* p, int h, int h2, int samples) {
  Arena* a = static_cast<Arena*>(p);
  int ok = guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    const TopoDS_Shape* s2 = h2 >= 0 ? get(a, h2) : nullptr;
    if (h2 >= 0 && !s2) return -1;
    Edges ed(*s);
    std::unique_ptr<Edges> ed2;
    if (s2) ed2 = std::make_unique<Edges>(*s2);
    struct Pair {
      int index;
      TopoDS_Edge e;
      TopoDS_Face fa, fb;
      TopoDS_Edge eb;  // the matching edge on the other body (h2)
      bool shared;
    };
    std::vector<Pair> pairs;
    for (int i = 1; i <= ed.map.Extent(); i++) {
      TopoDS_Edge e = TopoDS::Edge(ed.map(i));
      if (BRep_Tool::Degenerated(e)) continue;
      std::vector<TopoDS_Face> fs = ed.around(e);
      if (!s2) {
        if (fs.size() == 2 && !fs[0].IsSame(fs[1])) pairs.push_back({i - 1, e, fs[0], fs[1], TopoDS_Edge(), true});
        continue;
      }
      if (fs.size() != 1) continue;
      // A boundary edge of h lying along the boundary of h2.
      BRepAdaptor_Curve c(e);
      bool on = true;
      for (int k = 0; k <= 6 && on; k++) {
        gp_Pnt q = c.Value(c.FirstParameter() + (c.LastParameter() - c.FirstParameter()) * (0.02 + 0.96 * k / 6));
        TopoDS_Edge near;
        TopoDS_Face nf;
        on = nearest_free(*ed2, q, near, nf) < 1e-2;
      }
      if (on) pairs.push_back({i - 1, e, fs[0], TopoDS_Face(), TopoDS_Edge(), false});
    }
    std::ostringstream js;
    js.precision(6);
    js << "{\"edges\":[";
    double m0 = 0, m1 = 0, m2 = 0;
    bool firstp = true;
    for (auto& pr : pairs) {
      BRepAdaptor_Curve c(pr.e);
      double f = c.FirstParameter(), l = c.LastParameter();
      double g0 = 0, g1 = 0, g2 = 0, f0b, l0b;
      bool any = false;
      for (int k = 0; k <= samples; k++) {
        double t = f + (l - f) * (0.002 + 0.996 * k / samples);
        gp_Pnt q;
        gp_Vec tan;
        c.D1(t, q, tan);
        if (tan.Magnitude() < 1e-12) continue;
        tan.Normalize();
        double ua, va, ub, vb;
        if (!uv_on_edge(pr.e, pr.fa, t, ua, va) && !uv_of(pr.fa, q, ua, va)) continue;
        TopoDS_Face fb = pr.fb;
        bool okb = false;
        if (pr.shared) {
          okb = uv_on_edge(pr.e, fb, t, ub, vb);
        } else {
          TopoDS_Edge eb;
          if (nearest_free(*ed2, q, eb, fb) > 1e-2) continue;
          double tb;
          BRepAdaptor_Curve cb(eb);
          GeomAPI_ProjectPointOnCurve pc(q, BRep_Tool::Curve(eb, f0b, l0b));
          if (pc.NbPoints()) tb = pc.LowerDistanceParameter(), okb = uv_on_edge(eb, fb, tb, ub, vb);
        }
        if (!okb && !uv_of(fb, q, ub, vb)) continue;
        SurfPt A = eval(pr.fa, ua, va), B = eval(fb, ub, vb);
        any = true;
        g0 = std::max(g0, A.p.Distance(B.p));
        double cosang = A.n.Dot(B.n);
        if (!pr.shared) cosang = std::fabs(cosang);  // two bodies: either may face either way
        g1 = std::max(g1, std::acos(std::max(-1.0, std::min(1.0, cosang))) * 180 / M_PI);
        gp_Vec across = tan.Crossed(A.n);
        if (across.Magnitude() < 1e-12) continue;
        across.Normalize();
        g2 = std::max(g2, (curvature_vector(A, across) - curvature_vector(B, across)).Magnitude());
      }
      if (!any) continue;
      GProp_GProps gp;
      BRepGProp::LinearProperties(pr.e, gp);
      TopTools_IndexedMapOfShape fm;
      TopExp::MapShapes(*s, TopAbs_FACE, fm);
      int fa = fm.FindIndex(pr.fa) - 1, fb = pr.shared ? fm.FindIndex(pr.fb) - 1 : -1;
      if (!firstp) js << ",";
      firstp = false;
      js << "{\"edge\":" << pr.index << ",\"faces\":[" << fa << "," << fb << "],\"g0\":" << g0 << ",\"g1\":" << g1 << ",\"g2\":" << g2 << ",\"length\":" << gp.Mass() << "}";
      m0 = std::max(m0, g0), m1 = std::max(m1, g1), m2 = std::max(m2, g2);
    }
    js << "],\"max\":{\"g0\":" << m0 << ",\"g1\":" << m1 << ",\"g2\":" << m2 << "}}";
    a->surf_text = js.str();
    return 0;
  });
  return ok < 0 ? nullptr : a->surf_text.c_str();
}

/// Samples every face on an n × n grid of its (u, v) range, keeping points on
/// the face: per sample face, x y z, unit normal (as the face faces), the two
/// principal curvatures (1/mm, signed against the normal: positive bulges
/// out). Returns the count.
int occ_face_samples(void* p, int h, int n, const double** data, int* len) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    TopTools_IndexedMapOfShape faces;
    TopExp::MapShapes(*s, TopAbs_FACE, faces);
    std::vector<double>& out = a->surf_data;
    out.clear();
    int count = 0;
    for (int fi = 1; fi <= faces.Extent(); fi++) {
      TopoDS_Face f = TopoDS::Face(faces(fi));
      double u0, u1, v0, v1;
      BRepTools::UVBounds(f, u0, u1, v0, v1);
      BRepAdaptor_Surface ad(f, Standard_False);
      BRepTopAdaptor_FClass2d cls(f, 1e-9);
      for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j++) {
          double u = u0 + (u1 - u0) * (i + 0.5) / n, v = v0 + (v1 - v0) * (j + 0.5) / n;
          if (cls.Perform(gp_Pnt2d(u, v)) != TopAbs_IN) continue;
          BRepLProp_SLProps pr(ad, u, v, 2, 1e-9);
          if (!pr.IsNormalDefined()) continue;
          gp_Dir nd = pr.Normal();
          double k1 = 0, k2 = 0;
          if (pr.IsCurvatureDefined()) k1 = pr.MaxCurvature(), k2 = pr.MinCurvature();
          // OCCT's curvature is signed against its own normal; flip both with the face.
          // Positive: bulging out (a sphere seen from outside).
          double sgn = f.Orientation() == TopAbs_REVERSED ? -1 : 1;
          gp_Vec nn(nd);
          nn *= sgn;
          k1 *= -sgn, k2 *= -sgn;
          if (k1 < k2) std::swap(k1, k2);
          gp_Pnt q = pr.Value();
          out.insert(out.end(), {(double)(fi - 1), q.X(), q.Y(), q.Z(), nn.X(), nn.Y(), nn.Z(), k1, k2});
          count++;
        }
    }
    *data = out.data();
    *len = (int)out.size();
    return count;
  });
}

/// How far body b lies from body a: sampled over a's faces (n × n each).
/// out: max, mean, rms (mm), the point on a where it is worst (x y z).
int occ_deviation(void* p, int ha, int hb, int n, double* out) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* sa = get(a, ha);
    const TopoDS_Shape* sb = get(a, hb);
    if (!sa || !sb) return -1;
    double mx = 0, sum = 0, sq = 0;
    int count = 0;
    gp_Pnt worst;
    BRepExtrema_DistShapeShape dist;
    dist.LoadS2(*sb);
    for (TopExp_Explorer x(*sa, TopAbs_FACE); x.More(); x.Next()) {
      TopoDS_Face f = TopoDS::Face(x.Current());
      double u0, u1, v0, v1;
      BRepTools::UVBounds(f, u0, u1, v0, v1);
      BRepAdaptor_Surface ad(f);
      BRepTopAdaptor_FClass2d cls(f, 1e-9);
      for (int i = 0; i <= n; i++)
        for (int j = 0; j <= n; j++) {
          double u = u0 + (u1 - u0) * i / n, v = v0 + (v1 - v0) * j / n;
          if (cls.Perform(gp_Pnt2d(u, v)) == TopAbs_OUT) continue;
          gp_Pnt q = ad.Value(u, v);
          dist.LoadS1(BRepBuilderAPI_MakeVertex(q).Vertex());
          dist.Perform();
          if (!dist.IsDone()) continue;
          double d = dist.Value();
          if (d > mx) mx = d, worst = q;
          sum += d, sq += d * d, count++;
        }
    }
    if (!count) throw Standard_Failure("nothing to measure");
    out[0] = mx, out[1] = sum / count, out[2] = std::sqrt(sq / count), out[3] = worst.X(), out[4] = worst.Y(), out[5] = worst.Z();
    return count;
  });
}

/// The picked edges of a body chained end to end into one wire (a curve to
/// sweep along, fill to, or blend from).
int occ_edge_wire(void* p, int h, const int* idx, int n) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    if (n < 1) throw Standard_Failure("no edges picked");
    Edges ed(*s);
    return push(a, chain(picked_edges(ed, idx, n)));
  });
}

/// The last surface call's numbers (see each call).
int occ_surf_data(void* p, const double** data, int* len) {
  Arena* a = static_cast<Arena*>(p);
  *data = a->surf_data.data();
  *len = (int)a->surf_data.size();
  return *len;
}

}  // extern "C"
