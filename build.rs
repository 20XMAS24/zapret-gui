use embed_manifest::{manifest::ExecutionLevel, new_manifest};

mod build_resources;

fn main() {
    // winws.exe (WinDivert), sing-box (TUN) and the WFP kill switch all need admin rights,
    // so ask for elevation right away instead of relaunching ourselves.
    // ZAPRET_NO_MANIFEST=1 lets `cargo test` run the test binary without elevation.
    println!("cargo:rerun-if-env-changed=ZAPRET_NO_MANIFEST");
    println!("cargo:rerun-if-env-changed=ZAPRET_GUI_REPO");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let manifest = std::env::var_os("ZAPRET_NO_MANIFEST").is_none().then(||
            new_manifest("ZapretGUI").requested_execution_level(ExecutionLevel::RequireAdministrator).to_string());
        build_resources::embed(std::path::Path::new("installer/app.ico"), manifest);
    }
    println!("cargo:rerun-if-changed=installer/app.ico");
    println!("cargo:rerun-if-changed=build_resources.rs");

    // The self-contained MinGW of the windows-gnu toolchain ships no libshlwapi.a, which the
    // `webbrowser` crate (via egui-winit) links. GNU ld can link straight against a DLL, so put a
    // copy of the system shlwapi.dll on the library search path.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("syslibs");
        let _ = std::fs::create_dir_all(&out);
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let src = std::path::Path::new(&root).join("System32").join("shlwapi.dll");
        if std::fs::copy(&src, out.join("shlwapi.dll")).is_ok() {
            println!("cargo:rustc-link-search=native={}", out.display());
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
