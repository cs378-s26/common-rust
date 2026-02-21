use std::path::PathBuf;

#[cfg(target_arch = "x86_64")]
fn build_flanterm() {
    cc::Build::new()
        .file("flanterm-c/src/flanterm.c")
        .file("flanterm-c/src/flanterm_backends/fb.c")
        .include("flanterm-c/src")
        .include("flanterm-c/src/backends")
        .flag("-ffreestanding")
        .flag("-fno-omit-frame-pointer")
        .flag("-mno-sse")
        .flag("-mno-sse2")
        .flag("-mno-mmx")
        .flag("-mno-80387")
        .flag("-fno-stack-protector")
        .flag("-fno-PIC")
        .flag("-mcmodel=kernel")
        .compile("flanterm");
}

#[cfg(target_arch = "aarch64")]
fn build_flanterm() {
    cc::Build::new()
        .file("flanterm-c/src/flanterm.c")
        .file("flanterm-c/src/flanterm_backends/fb.c")
        .include("flanterm-c/src")
        .include("flanterm-c/src/backends")
        .flag("-ffreestanding")
        .flag("-fno-omit-frame-pointer")
        .flag("-mgeneral-regs-only")
        .flag("-fno-stack-protector")
        .flag("-fno-pic")
        .compile("flanterm");
}

fn main() {
    build_flanterm();

    let bindings = bindgen::Builder::default()
        .use_core()
        .header("flanterm-c/src/flanterm_backends/fb.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_item(r"^flanterm_.+$")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from("src");
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
