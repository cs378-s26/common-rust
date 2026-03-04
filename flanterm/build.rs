use std::env;
use std::path::PathBuf;
use std::process::Command;

fn build_flanterm() {
    let target = env::var("TARGET").expect("TARGET not set");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").expect("CARGO_CFG_TARGET_ARCH not set");

    let clang = "clang";

    let clang_ok = Command::new(clang)
        .arg("--version")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if !clang_ok {
        panic!("clang not found in PATH");
    }

    let mut build = cc::Build::new();
    build.compiler(clang);
    build
        .file("flanterm-c/src/flanterm.c")
        .file("flanterm-c/src/flanterm_backends/fb.c")
        .include("flanterm-c/src")
        .include("flanterm-c/src/backends")
        .flag("-ffreestanding")
        .flag("-fno-omit-frame-pointer")
        .flag("-fno-stack-protector");

    build.flag(&format!("--target={target}"));

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

    let compiler = build.get_compiler();
    let mut cmd = compiler.to_command();
    cmd.arg("-dumpmachine");
    let output = cmd.output().expect("failed to run C compiler");
    if !output.status.success() {
        panic!(
            "C compiler {:?} did not accept -dumpmachine for target {}",
            compiler.path(), target
        );
    }

    let machine = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !machine.starts_with(&target_arch) {
        panic!(
            "C compiler triple '{}' does not match target arch '{}'; set CC_{} or CROSS_COMPILE",
            machine,
            target_arch,
            target.replace('-', "_"),
        );
    }

    build.compile("flanterm");
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
