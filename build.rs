fn main() {
    let _ = embed_resource::compile("tray-example.rc", embed_resource::NONE);
    slint_build::compile("ui/percentage.slint").expect("Slint build failed")
}
