use crate::util::{
    Target, build_ext2_filesystem_from_dir, build_image, build_kernel, download_ovmf, exec,
    path_to_string, run_dir,
};
use anyhow::Result;
use std::env::current_dir;
use std::path::{Path, PathBuf};

pub fn run(
    kvm: bool,
    cores: u8,
    mem_g: u8,
    release: bool,
    target: Target,
    filesystem_path: String,
) -> Result<()> {
    let path = build_image(&build_kernel(release, target)?, release, target)?;

    let machine = target.qemu_machine();
    let filesystem_source = resolve_path_from_repo_root(&filesystem_path)?;
    let filesystem_image = build_ext2_filesystem_from_dir(&filesystem_source, "qemu-filesystem")?;

    let mut args = vec![
        "-machine".into(),
        machine.into(),
        "-bios".into(),
        path_to_string(&download_ovmf(target)?)?,
        "-drive".into(),
        format!("file={},format=raw", path_to_string(&path)?),
        "-drive".into(),
        format!(
            "if=none,file={},format=raw,id=disk0", //if=none means don't automatically attach the drive to a bus, this is done by virtio-blk
            path_to_string(&filesystem_image)?
        ),
        "-device".into(),
        format!("{},drive=disk0", target.qemu_virtio_blk_device()),
        "-no-reboot".into(),
        "-monitor".into(),
        "stdio".into(),
        "-d".into(),
        "int,cpu_reset".into(),
        "-D".into(),
        "qemu.log".into(),
        "-no-shutdown".into(),
        "-s".into(),
        // "-S".into(),
        // "-M".into(),
        // "smm=off".into(),
        "-m".into(),
        format!("{}G", mem_g),
        "-smp".into(),
        format!("{cores}"),
        "-serial".into(),
        format!("file:{}/serial.txt", path_to_string(&run_dir()?)?),
    ];

    args.extend(
        target
            .qemu_display_args()
            .iter()
            .map(|arg| (*arg).to_string()),
    );

    if kvm {
        args.push("-enable-kvm".into());
        args.push("-cpu".into());
        args.push("host".into());
    } else if let Some(cpu) = target.qemu_cpu_without_kvm() {
        args.push("-cpu".into());
        args.push(cpu.into());
    }

    let qemu_binary = target.qemu_binary();
    exec(qemu_binary, args)
}

fn resolve_path_from_repo_root(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(current_dir()?.join(path))
    }
}
