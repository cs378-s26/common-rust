use std::env;
use std::path::PathBuf;

fn build_flanterm() {
    let target = env::var("TARGET").expect("TARGET not set");
    let target_arch =
        env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH not set");

    let mut build = cc::Build::new();
    build
        .file("flanterm-c/src/flanterm.c")
        .file("flanterm-c/src/flanterm_backends/fb.c")
        .include("flanterm-c/src")
        .include("flanterm-c/src/backends")
        .flag("-ffreestanding")
        .flag("-fno-omit-frame-pointer")
        .flag("-fno-stack-protector");

    match target_arch.as_str() {
        "x86_64" => {
            build
                .flag("-mno-sse")
                .flag("-mno-sse2")
                .flag("-mno-mmx")
                .flag("-mno-80387")
                .flag("-fno-PIC")
                .flag("-mcmodel=kernel");
        }
        "aarch64" => {
            build.flag("-mgeneral-regs-only").flag("-fno-pic");
        }
        other => {
            panic!("unsupported target arch for flanterm: {other}");
        }
    }

    build.target(&target).compile("flanterm");
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