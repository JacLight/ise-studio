// Exact geometry from scans: B-spline faces from fitted control nets, flat
// faces merged on solids sewn from meshes, and tessellation with angular
// control (mesh from shape).
//
// Included at the end of shim.cpp (one translation unit, so it shares the
// arena and its helpers); kept apart so the files grow without colliding.

#include <Geom_BSplineSurface.hxx>
#include <TColgp_Array2OfPnt.hxx>
#include <TColStd_Array1OfInteger.hxx>
#include <TColStd_Array1OfReal.hxx>

extern "C" {

/// A face on the B-spline surface with these poles (nu rows of nv, row-major),
/// distinct knots and multiplicities, and degrees.
int occ_bspline_face(void* p, const double* poles, int nu, int nv, const double* uk, const int* um, int nuk, const double* vk, const int* vm, int nvk, int udeg, int vdeg) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    TColgp_Array2OfPnt pts(1, nu, 1, nv);
    for (int i = 0; i < nu; i++)
      for (int j = 0; j < nv; j++) {
        const double* q = poles + 3 * (i * nv + j);
        pts.SetValue(i + 1, j + 1, gp_Pnt(q[0], q[1], q[2]));
      }
    TColStd_Array1OfReal uks(1, nuk), vks(1, nvk);
    TColStd_Array1OfInteger ums(1, nuk), vms(1, nvk);
    for (int i = 0; i < nuk; i++) {
      uks.SetValue(i + 1, uk[i]);
      ums.SetValue(i + 1, um[i]);
    }
    for (int i = 0; i < nvk; i++) {
      vks.SetValue(i + 1, vk[i]);
      vms.SetValue(i + 1, vm[i]);
    }
    Handle(Geom_BSplineSurface) s = new Geom_BSplineSurface(pts, uks, vks, ums, vms, udeg, vdeg);
    BRepBuilderAPI_MakeFace mf(s, 1e-7);
    if (!mf.IsDone()) throw Standard_Failure("the B-spline surface does not make a face");
    return push(a, mf.Face());
  });
}

/// The shape with faces lying on one surface merged (a sewn mesh's coplanar
/// triangles become one planar face each).
int occ_unify(void* p, int h) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    ShapeUpgrade_UnifySameDomain u(*s, true, true, true);
    u.SetLinearTolerance(1e-6);
    u.SetAngularTolerance(1e-6);
    u.Build();
    return push(a, u.Shape());
  });
}

/// Triangles within `deflection` mm and `angle` radians of the true surfaces
/// (smaller angles add triangles round small curves).
int occ_mesh_angular(void* p, int h, double deflection, double angle, const double** positions, int* npos, const int** triangles, int* ntri) {
  Arena* a = static_cast<Arena*>(p);
  return guard(a, [&] {
    const TopoDS_Shape* s = get(a, h);
    if (!s) return -1;
    BRepTools::Clean(*s);
    BRepMesh_IncrementalMesh mesher(*s, deflection, false, angle, true);
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

}  // extern "C"
