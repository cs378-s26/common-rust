use vergen_git2::{BuildBuilder, CargoBuilder, Emitter, Git2Builder, RustcBuilder};

fn main() {
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    println!("cargo:rustc-link-arg=-Tresources/linker-{arch}.lds");
    println!("cargo:rerun-if-changed=resources/linker-{arch}.lds");

    let build = BuildBuilder::all_build().unwrap();
    let git = Git2Builder::all_git().unwrap();
    let cargo = CargoBuilder::all_cargo().unwrap();
    let rustc = RustcBuilder::all_rustc().unwrap();

    Emitter::default()
        .add_instructions(&git)
        .unwrap()
        .add_instructions(&build)
        .unwrap()
        .add_instructions(&cargo)
        .unwrap()
        .add_instructions(&rustc)
        .unwrap()
        .emit()
        .unwrap();
}
