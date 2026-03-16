fn main() {
    println!("cargo::rustc-check-cfg=cfg(rust_analyzer)");
    println!(
        "cargo:rustc-env=WABITY_APP_VERSION={}",
        env!("CARGO_PKG_VERSION")
    );
    println!(
        "cargo:rustc-env=WABITY_BUILD_DATE={}",
        build_date_utc().unwrap_or_else(|| "unknown".to_string())
    );
    tauri_build::try_build(
        tauri_build::Attributes::new().codegen(tauri_build::CodegenContext::new()),
    )
    .expect("failed to run tauri build script");
}

fn build_date_utc() -> Option<String> {
    if let Ok(source_date_epoch) = std::env::var("SOURCE_DATE_EPOCH") {
        return Some(source_date_epoch);
    }

    let output = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    Some(value.trim().to_string())
}
