fn main() {
    // Windows: bake the tray icon into the binary as a PE resource (see
    // `icons/resources.rc`); a no-op on other targets.
    embed_resource::compile("icons/resources.rc", embed_resource::NONE);

    // Linux has no equivalent embedded-resource mechanism: the StatusNotifierItem
    // (ksni) tray wants raw ARGB pixels, so decode the icon PNG at build time.
    #[cfg(unix)]
    generate_tray_icon();

    slint_build::compile("ui/main.slint").expect("Slint build failed")
}

/// Decode `icons/app-icon.png` into the ARGB32 pixmap the Linux ksni tray needs
/// and emit it as a generated module (`tray_icon.rs`) the tray includes. Kept at
/// build time so no PNG decoding happens at startup and the bytes ship in the
/// binary, matching how Windows embeds its icon.
#[cfg(unix)]
fn generate_tray_icon() {
    use std::{env, fs, path::Path};

    const SRC: &str = "icons/app-icon.png";
    println!("cargo:rerun-if-changed={SRC}");

    let image = image::open(SRC)
        .unwrap_or_else(|e| panic!("Failed to read tray icon {SRC}: {e}"))
        .into_rgba8();
    let (width, height) = image.dimensions();

    // StatusNotifierItem pixmaps are ARGB32 in network byte order — i.e. bytes
    // [A, R, G, B] per pixel. `image` gives us RGBA, so reorder each pixel.
    let mut argb = Vec::with_capacity(image.as_raw().len());
    for pixel in image.pixels() {
        let [r, g, b, a] = pixel.0;
        argb.extend_from_slice(&[a, r, g, b]);
    }

    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_dir = Path::new(&out_dir);

    fs::write(out_dir.join("tray_icon.argb"), &argb).expect("Failed to write tray icon data");
    fs::write(
        out_dir.join("tray_icon.rs"),
        format!(
            "pub const WIDTH: i32 = {width};\n\
             pub const HEIGHT: i32 = {height};\n\
             pub static ARGB: &[u8] = include_bytes!(\"tray_icon.argb\");\n"
        ),
    )
    .expect("Failed to write tray icon module");
}
