fn main() {
    managed_ffmpeg_cfg();
    tauri_build::build();
    link_apple_clang_runtime();
}

/// `cfg(managed_ffmpeg)`: targets with a pinned FFmpeg package in media_support.rs.
fn managed_ffmpeg_cfg() {
    println!("cargo:rustc-check-cfg=cfg(managed_ffmpeg)");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if matches!(
        (os.as_str(), arch.as_str()),
        ("windows", "x86_64") | ("macos", "aarch64")
    ) {
        println!("cargo:rustc-cfg=managed_ffmpeg");
    }
}

fn link_apple_clang_runtime() {
    // Build-script cfg describes the host, so use Cargo's target OS instead.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    for variable in ["DEVELOPER_DIR", "SDKROOT", "TOOLCHAINS"] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    // Metal's Objective-C availability checks need __isPlatformVersionAtLeast
    // when targeting older macOS versions. Rust links with -nodefaultlibs,
    // which omits this Apple Clang support archive. Ask the selected Xcode
    // toolchain for its archive instead of hard-coding an Xcode/Clang version.
    let output = std::process::Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "clang",
            "--print-file-name=libclang_rt.osx.a",
        ])
        .output()
        .expect("Cannot locate the Apple Clang runtime: xcrun failed to start.");
    assert!(
        output.status.success(),
        "Cannot locate the Apple Clang runtime: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path =
        String::from_utf8(output.stdout).expect("The Apple Clang runtime path is not valid UTF-8.");
    let archive = std::path::Path::new(path.trim());
    assert!(
        archive.is_absolute() && archive.is_file(),
        "The selected Apple Clang toolchain did not provide libclang_rt.osx.a."
    );
    println!("cargo:rerun-if-changed={}", archive.display());
    println!(
        "cargo:rustc-link-search=native={}",
        archive.parent().expect("Clang runtime directory").display()
    );
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}
