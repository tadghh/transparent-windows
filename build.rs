fn main() {
    println!("cargo:rustc-link-lib=./icons/res");
    slint_build::compile("ui/main.slint").expect("Slint build failed")
}
