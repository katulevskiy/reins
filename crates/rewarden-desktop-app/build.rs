//! On Windows, puts the Reins icon into `Reins.exe` (resource ID 1, which GPUI uses for the window and taskbar).

fn main() {
    println!("cargo:rerun-if-changed=packaging/windows/Reins.rc");
    println!("cargo:rerun-if-changed=packaging/windows/Reins.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && let Err(e) = embed_resource::compile_for("packaging/windows/Reins.rc", ["reins-app"], embed_resource::NONE)
            .manifest_optional()
    {
        panic!("the Windows icon resource did not compile: {e}");
    }
}
