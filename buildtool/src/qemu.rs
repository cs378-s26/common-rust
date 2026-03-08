use anyhow::Result;

use crate::util::{
    Target, build_image, build_kernel, download_ovmf, exec, path_to_string, run_dir,
};

pub fn run(kvm: bool, cores: u8, mem_g: u8, release: bool, target: Target) -> Result<()> {
    let path = build_image(&build_kernel(release, target)?, release, target)?;

    let machine = target.qemu_machine();

    let mut args = vec![
        "-machine".into(),
        machine.into(),
        "-bios".into(),
        path_to_string(&download_ovmf(target)?)?,
        "-drive".into(),
        format!("file={},format=raw", path_to_string(&path)?),
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
