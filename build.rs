// Embed the PAgent icon in the executable. GPUI's Windows window class loads
// icon resource 1 from this module, so the titlebar/taskbar/Alt-Tab all use it.
fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("assets/app.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("嵌入 PAgent 图标失败");
    }
}