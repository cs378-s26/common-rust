use crate::util::{
    Target, build_image, build_kernel, download_ovmf, exec, path_to_string, run_dir,
};
use anyhow::Result;

pub fn run(
    kvm: bool,
    cores: u8,
    mem_g: u8,
    release: bool,
    target: Target,
    disk_path: String,
) -> Result<()> {
    let path = build_image(&build_kernel(release, target)?, release, target)?;

    let machine = target.qemu_machine();
    let disk_path_absolute = current_dir()?.join(disk_path);
    if !disk_path_absolute.exists() {
        return Err(anyhow!(
            "Disk path '{}' does not exist, try creating one with qemu-img create -f raw disk.img 64M",
            disk_path_absolute.display()
        ));
    }

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
            path_to_string(&disk_path_absolute)?
        ),
        "-device".into(),
        // use pci for x86, mmio/device tree for aarch64
        match target {
            Target::X86_64 => "virtio-blk,drive=disk0".into(),
            Target::Aarch64 => "virtio-blk-device,drive=disk0".into(),
        },
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
