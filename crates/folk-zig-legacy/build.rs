use std::path::PathBuf;
use std::process::Command;

use equilibrium_ffi::{Language, find_compiler};

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let repo = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root");
    let source = repo.join("src").join("legacy_bridge.zig");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let obj = out_dir.join("folk_zig_legacy.o");
    let cache = out_dir.join("zig-cache");
    let target = std::env::var("TARGET").unwrap_or_default();

    println!("cargo:rerun-if-changed={}", source.display());
    for file in [
        "src/tools.zig",
        "src/shell.zig",
        "src/mcp.zig",
        "src/config.zig",
        "src/http.zig",
        "src/p2p.zig",
    ] {
        println!("cargo:rerun-if-changed={}", repo.join(file).display());
    }

    let zig = find_compiler(Language::Zig)
        .and_then(|info| info.compiler)
        .unwrap_or_else(|| "zig".to_string());
    let status = Command::new(&zig)
        .env("ZIG_LOCAL_CACHE_DIR", &cache)
        .env("ZIG_GLOBAL_CACHE_DIR", &cache)
        .arg("build-obj")
        .arg("-fPIC")
        .arg("-OReleaseFast")
        .args(zig_target_args(&target))
        .arg(format!("-femit-bin={}", obj.display()))
        .arg(&source)
        .status()
        .expect("run zig");
    assert!(status.success(), "zig legacy bridge failed");
    cc::Build::new().object(&obj).compile("folk_zig_legacy");
}

fn zig_target_args(target: &str) -> Vec<String> {
    let Some(zig_target) = (match target {
        "aarch64-apple-darwin" => Some("aarch64-macos"),
        "x86_64-apple-darwin" => Some("x86_64-macos"),
        "aarch64-unknown-linux-gnu" => Some("aarch64-linux-gnu"),
        "x86_64-unknown-linux-gnu" => Some("x86_64-linux-gnu"),
        _ => None,
    }) else {
        return Vec::new();
    };
    vec!["-target".to_string(), zig_target.to_string()]
}
