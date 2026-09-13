//! Resolve blade-render's packaged WGSL (`code/`) so WASM can embed it into the VFS.
//! Native loads the same directory via `blade_render::shader_dir()` from disk.

fn main() {
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let cargo = std::env::var("CARGO").expect("CARGO");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let output = std::process::Command::new(cargo)
        .args(["metadata", "--format-version=1"])
        .current_dir(&manifest_dir)
        .output()
        .expect("cargo metadata failed to start");
    if !output.status.success() {
        panic!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let meta: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("parse cargo metadata");
    let packages = meta["packages"].as_array().expect("packages array");
    let blade_render = packages
        .iter()
        .find(|p| p["name"].as_str() == Some("blade-render"))
        .expect("blade-render not in cargo metadata");
    let manifest_path = blade_render["manifest_path"]
        .as_str()
        .expect("blade-render manifest_path");
    let code_dir = std::path::Path::new(manifest_path)
        .parent()
        .expect("blade-render parent")
        .join("code");
    assert!(
        code_dir.join("raster.wgsl").is_file(),
        "blade-render code/raster.wgsl missing at {}",
        code_dir.display()
    );
    println!(
        "cargo:rustc-env=REDLINE_BLADE_SHADER_DIR={}",
        code_dir.display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        code_dir.join("raster.wgsl").display()
    );
}
