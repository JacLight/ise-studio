// Compiles the C++ layer over OpenCascade and links its libraries (LGPL-2.1,
// linked dynamically). OCCT is found at $OCCT_DIR, else Homebrew's prefix.
use std::path::PathBuf;

fn main() {
    let root = std::env::var("OCCT_DIR").map(PathBuf::from).unwrap_or_else(|_| {
        for p in ["/opt/homebrew/opt/opencascade", "/usr/local/opt/opencascade", "/usr"] {
            if PathBuf::from(p).join("include/opencascade/Standard.hxx").exists() {
                return PathBuf::from(p);
            }
        }
        panic!("OpenCascade not found: install it (brew install opencascade) or set OCCT_DIR");
    });
    let include = root.join("include/opencascade");
    let lib = root.join("lib");
    println!("cargo:rerun-if-changed=src/shim.cpp");
    // solids.cpp is #included by shim.cpp (one translation unit).
    println!("cargo:rerun-if-changed=src/solids.cpp");
    println!("cargo:rerun-if-changed=src/surfaces.cpp");
    println!("cargo:rerun-if-env-changed=OCCT_DIR");
    cc::Build::new()
        .cpp(true)
        .file("src/shim.cpp")
        .include(&include)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-Wno-deprecated-declarations")
        .flag_if_supported("-Wno-unused-parameter")
        .compile("de_occt_shim");
    println!("cargo:rustc-link-search=native={}", lib.display());
    // Runtime lookup of the shared libraries (development: Homebrew's lib folder).
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    for l in [
        "TKernel", "TKMath", "TKG2d", "TKG3d", "TKGeomBase", "TKBRep", "TKGeomAlgo", "TKTopAlgo", "TKPrim", "TKBO", "TKBool",
        "TKShHealing", "TKMesh", "TKOffset", "TKFillet", "TKFeat", "TKXSBase", "TKDE", "TKDESTEP", "TKCDF", "TKLCAF", "TKCAF",
        "TKXCAF", "TKVCAF", "TKService", "TKV3d", "TKHLR",
    ] {
        println!("cargo:rustc-link-lib=dylib={l}");
    }
    println!("cargo:rustc-link-lib=dylib=c++");
}
