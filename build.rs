fn main() {
    let _ = embed_resource::compile("tray-icon.rc", embed_resource::NONE);
    slint_build::compile("ui/main.slint").expect("Slint build failed")
}
