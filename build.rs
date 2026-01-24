fn main() {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    println!("cargo:rustc-link-arg=-Tresources/linker-{arch}.lds");
    println!("cargo:rerun-if-changed=resources/linker-{arch}.lds");
}
