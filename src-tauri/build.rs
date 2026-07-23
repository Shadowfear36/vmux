/// Tauri's build-time codegen validates that every `bundle.externalBin`
/// path in tauri.conf.json exists on disk — unconditionally, even for a
/// plain `cargo check`/`cargo build` of this crate, not just `tauri build`.
/// Real sidecar binaries (vmuxctl/vmuxd) only get built and placed here by
/// `scripts/build-sidecars.mjs`, which itself builds them via `cargo build
/// --bin vmuxctl --bin vmuxd` against *this same package* — a hard
/// chicken-and-egg loop without this: building vmuxctl/vmuxd would first
/// require this build script to succeed, which requires vmuxctl/vmuxd to
/// already exist. Self-heal by creating empty placeholders if missing, so
/// any ordinary cargo invocation works standalone; `build-sidecars.mjs`
/// overwrites them with the real release binaries before `tauri build`
/// actually bundles anything.
fn ensure_sidecar_placeholders() {
    let dir = std::path::Path::new("binaries");
    let _ = std::fs::create_dir_all(dir);
    for name in ["vmuxctl-x86_64-pc-windows-msvc.exe", "vmuxd-x86_64-pc-windows-msvc.exe"] {
        let path = dir.join(name);
        if !path.exists() {
            let _ = std::fs::write(&path, []);
        }
    }
}

fn main() {
    ensure_sidecar_placeholders();
    tauri_build::build()
}
