fn main() {
    // gpui's Windows backend loads icon resource id 1 out of the executable and
    // uses it as the window class icon, so embedding one here covers both the
    // window and what Explorer shows for the .exe.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=gittles.rc");
        println!("cargo:rerun-if-changed=assets/gittles.ico");
        embed_resource::compile("gittles.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("failed to embed the application icon");
    }
}
