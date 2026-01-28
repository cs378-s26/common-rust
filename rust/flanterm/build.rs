use std::path::PathBuf;

fn main() {
    cc::Build::new()
        .file("flanterm/src/flanterm.c")
        .file("flanterm/src/flanterm_backends/fb.c")
        .include("flanterm/src")
        .include("flanterm/src/backends")
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

    let bindings = bindgen::Builder::default()
        .use_core()
        .header("flanterm/src/flanterm_backends/fb.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .allowlist_item(r"^flanterm_.+$")
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from("src");
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
