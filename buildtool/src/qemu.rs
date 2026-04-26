use std::{
    env::current_dir,
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::util::{
    Target, build_ext2_filesystem_from_dir, build_image, build_kernel, download_ovmf, exec,
    path_to_string, run_dir,
};

pub fn run(
    kvm: bool,
    cores: u8,
    mem_g: u8,
    release: bool,
    target: Target,
    filesystem_path: String,
    usb: bool,
) -> Result<()> {
    let path = build_image(&build_kernel(release, target)?, release, target)?;

    let machine = target.qemu_machine();
    let filesystem_source = resolve_path_from_repo_root(&filesystem_path)?;
    let filesystem_image = build_ext2_filesystem_from_dir(&filesystem_source, "fs_img")?;

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

    // Virtio-net: user-mode networking, no TAP setup needed
    args.push("-device".into());
    args.push(match target {
        Target::X86_64 => "virtio-net,netdev=net0".into(),
        Target::Aarch64 => "virtio-net-device,netdev=net0".into(),
    });
    args.push("-netdev".into());
    args.push("user,id=net0".into());

    if usb {
        args.push("-device".into());
        args.push("qemu-xhci".into());
        args.push("-device".into());
        args.push("usb-kbd".into());
        args.push("-device".into());
        args.push("usb-mouse".into());
    }

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
