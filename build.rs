fn main() {
    embed_resource::compile("icons/resources.rc", embed_resource::NONE);

    slint_build::compile("ui/main.slint").expect("Slint build failed")
}
