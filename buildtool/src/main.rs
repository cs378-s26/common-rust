#![feature(decl_macro)]
#![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
#![feature(gen_blocks)]

use anyhow::{Error, Result};
use cargo_metadata::{Message, MetadataCommand};
use clap::{Parser, Subcommand};
use debug::gen_debug_module;
use fatfs::{FatType, FileSystem, FormatVolumeOptions, FsOptions, format_volume};
use fscommon::StreamSlice;
use gptman::{GPT, GPTPartitionEntry};
use reqwest::blocking;
use std::env::{current_dir, current_exe};
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;
use uuid::Uuid;
use core::str;

mod debug;

const LIMINE_X86_URL: &str =
    "https://github.com/limine-bootloader/limine/raw/refs/heads/v10.x-binary/BOOTX64.EFI";
const LIMINE_AARCH64_URL: &str =
    "https://codeberg.org/Limine/Limine/raw/tag/v10.5.1-binary/BOOTAA64.EFI";
const OVMF_X86_URL: &str = "https://github.com/osdev0/edk2-ovmf-nightly/releases/download/nightly-20251126T024608Z/ovmf-code-x86_64.fd";
const OVMF_AARCH64_URL: &str = "https://github.com/osdev0/edk2-ovmf-nightly/releases/download/nightly-20251126T024608Z/ovmf-code-aarch64.fd";
const LIMINE_CONF: &str = "limine.conf";

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::ValueEnum, Clone, Debug, Copy)]
enum Target {
    X86_64,
    Aarch64,
}

impl Target {
    fn name(self) -> &'static str {
        match self {
            Target::X86_64 => "x86_64",
            Target::Aarch64 => "aarch64",
        }
    }

    fn limine_url(self) -> &'static str {
        match self {
            Target::X86_64 => LIMINE_X86_URL,
            Target::Aarch64 => LIMINE_AARCH64_URL,
        }
    }

    fn ovmf_url(self) -> &'static str {
        match self {
            Target::X86_64 => OVMF_X86_URL,
            Target::Aarch64 => OVMF_AARCH64_URL,
        }
    }

    fn target_triple(self) -> &'static str {
        match self {
            Target::X86_64 => "x86_64-unknown-none",
            Target::Aarch64 => "aarch64-unknown-none",
        }
    }

    fn strip_tool(self) -> &'static str {
        match self {
            Target::X86_64 => "strip",
            Target::Aarch64 => "aarch64-linux-gnu-strip",
        }
    }

    fn limine_efi_path(self) -> &'static str {
        match self {
            Target::X86_64 => "efi/boot/BOOTX64.EFI",
            Target::Aarch64 => "efi/boot/BOOTAA64.EFI",
        }
    }

    fn qemu_machine(self) -> &'static str {
        match self {
            Target::X86_64 => "pc",
            Target::Aarch64 => "virt,acpi=off",
        }
    }

    fn qemu_display_args(self) -> &'static [&'static str] {
        match self {
            Target::X86_64 => &["-vga", "std"],
            // "virt" machine on aarch64 does not support -vga; use a firmware framebuffer device.
            Target::Aarch64 => &["-device", "ramfb"],
        }
    }

    fn qemu_binary(self) -> &'static str {
        match self {
            Target::X86_64 => "qemu-system-x86_64",
            Target::Aarch64 => "qemu-system-aarch64",
        }
    }

    fn qemu_cpu_without_kvm(self) -> Option<&'static str> {
        match self {
            Target::X86_64 => None,
            // QEMU may default to a 32-bit ARM CPU on "virt"; force a stable AArch64 model.
            Target::Aarch64 => Some("cortex-a72"),
        }
    }

    fn requires_c_toolchain_config(self) -> bool {
        matches!(self, Target::Aarch64)
    }
}

fn require_tool(name: &str) -> Result<()> {
    let status = Command::new("which")
        .arg(name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| Error::msg("which command not available"))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::msg(format!("{} not found in PATH", name)))
    }
}

fn configure_c_toolchain(target: Target, cmd: &mut Command) -> Result<()> {
    if !target.requires_c_toolchain_config() {
        return Ok(());
    }

    let target_triple = target.target_triple();

    require_tool("clang")?;
    require_tool("ar")?;

    let cc_key = format!("CC_{}", target_triple.replace('-', "_"));
    let ar_key = format!("AR_{}", target_triple.replace('-', "_"));
    let cflags_key = format!("CFLAGS_{}", target_triple.replace('-', "_"));

    cmd.env(cc_key, "clang");
    cmd.env(ar_key, "ar");
    cmd.env(cflags_key, format!("--target={}", target_triple));
    Ok(())
}

#[derive(Subcommand)]
enum Commands {
    Image {
        #[arg(short = 't', long, value_enum, default_value_t = Target::X86_64)]
        target: Target,
        #[arg(short = 'r', long)]
        release: bool,
    },
    Qemu {
        #[arg(short = 't', long, value_enum, default_value_t = Target::X86_64)]
        target: Target,
        #[arg(short = 'k', long)]
        kvm: bool,
        #[arg(short = 'j', long, default_value_t = 1)]
        cores: u8,
        #[arg(short = 'm', long, default_value_t = 4)]
        mem: u8,
        #[arg(short = 'r', long)]
        release: bool,
    },
    QemuTest {
        path_to_kernel: String,
        #[arg(short = 't', long, value_enum, default_value_t = Target::X86_64)]
        target: Target,
    },
    Gdb {
        #[arg(short = 't', long, value_enum, default_value_t = Target::X86_64)]
        target: Target,
        #[arg(short = 'k', long)]
        kvm: bool,
        #[arg(short = 'r', long)]
        release: bool,
    },
    Test {
        #[arg(short = 'r', long)]
        release: bool,
        #[arg(short = 't', long, value_enum, default_value_t = Target::X86_64)]
        target: Target,
    },
    Clean,
}

fn cache_dir() -> Result<PathBuf> {
    let root = current_dir()?.join("buildtool-cache");
    fs::create_dir_all(&root)?;
    Ok(root)
}

fn resources_dir() -> Result<PathBuf> {
    let root = current_dir()?.join("resources");
    Ok(root)
}

fn run_dir() -> Result<PathBuf> {
    let root = current_dir()?.join("run");
    fs::create_dir_all(&root)?;
    Ok(root)
}

fn download_limine(target: Target) -> Result<PathBuf> {
    let root = cache_dir()?;
    let limine_path = root.join(format!("limine-{}.efi", target.name()));

    if !limine_path.exists() {
        let url = target.limine_url();
        let response = blocking::get(url)?;
        let mut dest = File::create(&limine_path)?;
        let content = response.bytes()?;
        io::copy(&mut content.as_ref(), &mut dest)?;
    }

    Ok(limine_path)
}

fn download_ovmf(target: Target) -> Result<PathBuf> {
    let root = cache_dir()?;
    let ovmf_path = root.join(format!("ovmf-{}.fd", target.name()));

    if !ovmf_path.exists() {
        let url = target.ovmf_url();
        let response = blocking::get(url)?;
        let mut dest = File::create(&ovmf_path)?;
        let content = response.bytes()?;
        io::copy(&mut content.as_ref(), &mut dest)?;
    }

    Ok(ovmf_path)
}

fn build_kernel(release: bool, target: Target) -> Result<(PathBuf, Vec<(String, PathBuf)>)> {
    let mut args = vec![
        "build",
        "--message-format=json-render-diagnostics",
        "-Zbuild-std=core,alloc",
        "--target",
    ];

    let target_triple = target.target_triple();
    args.push(target_triple);

    if release {
        args.push("--release");
    }

    let mut crate_paths: Vec<(String, PathBuf)> = MetadataCommand::new()
        .exec()?
        .packages
        .iter()
        .filter_map(|pkg| {
            let path = pkg.manifest_path.parent()?;
            Some((format!("{}@{}", pkg.name, pkg.version), path.into()))
        })
        .collect();

    let sys_root = PathBuf::from(
        str::from_utf8(
            &Command::new("rustc")
                .arg("--print")
                .arg("sysroot")
                .output()?
                .stdout,
        )?
        .trim(),
    );

    crate_paths.push((
        "builtin::core".into(),
        sys_root.join("lib/rustlib/src/rust/library/core"),
    ));

    crate_paths.push((
        "builtin::alloc".into(),
        sys_root.join("lib/rustlib/src/rust/library/alloc"),
    ));

    crate_paths.push((
        "builtin::compiler-builtins".into(),
        sys_root.join("lib/rustlib/src/rust/library/compiler-builtins/compiler-builtins"),
    ));

    let mut cmd = Command::new("cargo");
    cmd.args(args)
        .env(
            "RUSTFLAGS",
            "-C relocation-model=static -C force-frame-pointers=yes",
        )
        .stdout(Stdio::piped());

    configure_c_toolchain(target, &mut cmd)?;

    let mut cmd = cmd.spawn()?;

    let stdout = cmd.stdout.take().expect("Failed to capture cargo stdout");
    let reader = BufReader::new(stdout);

    let mut res = None;

    for message in Message::parse_stream(reader) {
        // TODO: check package
        if let Message::CompilerArtifact(artifact) = message?
            && let Some(executable) = artifact.executable
        {
            res = Some(PathBuf::from(executable));
        }
    }

    cmd.wait()?;

    let executable = res.ok_or(Error::msg("failed to locate executable"))?;
    eprintln!("kernel binary path: {}", path_to_string(&executable)?);
    Ok((executable, crate_paths))
}

fn path_to_string(path: &Path) -> Result<String> {
    Ok(path
        .canonicalize()?
        .to_str()
        .ok_or(Error::msg("bad path"))?
        .to_string())
}

fn split_debug_info(elf: &Path, target: Target) -> Result<Vec<u8>> {
    let cache = cache_dir()?;
    let tmp_stripped = NamedTempFile::new_in(&cache)?;

    let strip_tool = target.strip_tool();

    let status = Command::new(strip_tool)
        .args([
            path_to_string(elf)?,
            "-o".into(),
            path_to_string(tmp_stripped.path())?,
        ])
        .status();

    if !matches!(status, Ok(s) if s.success()) {
        eprintln!("warning: {} failed; using unstripped kernel", strip_tool);
        return Ok(fs::read(elf)?);
    }

    Ok(fs::read(tmp_stripped)?)
}

fn build_image(
    build_res: &(PathBuf, Vec<(String, PathBuf)>),
    release: bool,
    target: Target,
) -> Result<PathBuf> {
    let (kernel_elf, package_data) = build_res;

    let cache_dir = cache_dir()?;
    let limine_efi = download_limine(target)?;
    let limine_cfg = resources_dir()?.join(LIMINE_CONF);
    let output_img = cache_dir.join(format!(
        "kernel-{}-{}.img",
        target.name(),
        if release { "release" } else { "debug" }
    ));
    let debug_mod = cache_dir.join(format!(
        "kernel-{}-debug_info-{}.mod",
        target.name(),
        if release { "release" } else { "debug" }
    ));

    if !fs::exists(&output_img)?
        || fs::metadata(kernel_elf)?.modified()? > fs::metadata(&output_img)?.modified()?
        || fs::metadata(&limine_efi)?.modified()? > fs::metadata(&output_img)?.modified()?
        || fs::metadata(&limine_cfg)?.modified()? > fs::metadata(&output_img)?.modified()?
        || fs::metadata(&current_exe()?)?.modified()? > fs::metadata(&output_img)?.modified()?
    {
        eprintln!(
            "rebuilding image: {}",
            output_img
                .to_str()
                .ok_or(Error::msg("could not convert image file"))?
        );

        let temp_img_out = NamedTempFile::new_in(cache_dir)?;
        let mut output_file = temp_img_out.as_file();

        output_file.set_len(64 * 1024 * 1024)?;

        let disk_guid = *Uuid::new_v4().as_bytes();
        let sector_size = 512;
        GPT::write_protective_mbr_into(&mut output_file, sector_size)?;
        let mut gpt = GPT::new_from(&mut output_file, sector_size, disk_guid)?;
        let start_lba = gpt.header.first_usable_lba;
        let end_lba = gpt.header.last_usable_lba;

        gpt[1] = GPTPartitionEntry {
            partition_type_guid: *Uuid::parse_str("c12a7328-f81f-11d2-ba4b-00a0c93ec93b")?
                .as_bytes(),
            unique_partition_guid: *Uuid::new_v4().as_bytes(),
            starting_lba: start_lba,
            ending_lba: end_lba,
            attribute_bits: 0,
            partition_name: "EFI System".into(),
        };

        gpt.write_into(&mut output_file)?;

        let mut slice = StreamSlice::new(
            &mut output_file,
            start_lba * sector_size,
            end_lba * sector_size,
        )?;

        format_volume(
            &mut slice,
            FormatVolumeOptions::new().fat_type(FatType::Fat32),
        )?;

        let fs = FileSystem::new(&mut slice, FsOptions::new())?;

        fs.root_dir().create_dir("efi")?;
        fs.root_dir().create_dir("efi/boot")?;

        let limine_efi_path = target.limine_efi_path();
        io::copy(
            &mut File::open(limine_efi)?,
            &mut fs.root_dir().create_file(limine_efi_path)?,
        )?;
        io::copy(
            &mut File::open(limine_cfg)?,
            &mut fs.root_dir().create_file(LIMINE_CONF)?,
        )?;

        let elf_data = split_debug_info(kernel_elf, target)?;
        let debug_data = gen_debug_module(fs::read(kernel_elf)?, package_data)?;

        fs.root_dir()
            .create_file("kernel_symbols.mod")?
            .write_all(&debug_data)?;

        fs::write(debug_mod, &debug_data)?;

        eprintln!("kernel.elf is {} bytes", elf_data.len());

        fs.root_dir()
            .create_file("kernel.elf")?
            .write_all(&elf_data)?;

        fs.unmount()?;

        output_file.flush()?;

        fs::rename(temp_img_out.path(), &output_img)?;
    }

    Ok(output_img)
}

fn exec<T: std::fmt::Debug + AsRef<std::ffi::OsStr>>(command: &str, args: Vec<T>) -> Result<()> {
    eprintln!("running: {} {:?}", command, args);
    let err = Command::new(command)
        .args(args)
        .current_dir(run_dir()?)
        .exec();
    Err(err.into())
}

fn qemu(kvm: bool, cores: u8, mem_g: u8, release: bool, target: Target) -> Result<()> {
    let path = build_image(&build_kernel(release, target)?, release, target)?;
    let block_path = current_dir()?.join("disk.img");
    if !block_path.exists() {
        File::create(&block_path)?;
    }

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
        "-drive".into(),
        format!("file={},format=raw,if=none,id=hd", path_to_string(&block_path)?),
        "-device".into(),
        // Use the MMIO transport on aarch64 so the device shows up in the FDT.
        // On x86_64 we still want the PCI variant ("virtio-blk"/virtio-blk-pci).
        match target {
            Target::X86_64 => "virtio-blk,drive=hd".into(),
            Target::Aarch64 => "virtio-blk-device,drive=hd".into(),
        },
        // "-S".into(),
        // "-M".into(),
        // "smm=off".into(),
        "-m".into(),
        format!("{}G", mem_g),
        "-smp".into(),
        format!("{}", cores),
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

fn gdb(kvm: bool, release: bool, target: Target) -> Result<()> {
    let (kernel_elf, _) = build_kernel(release, target)?;

    let gdb_args = if kvm {
        vec!["target remote localhost:1234", "hbreak system_main", "c"]
    } else {
        vec!["target remote localhost:1234", "b system_main", "c"]
    };

    let mut args = vec![path_to_string(&kernel_elf)?];

    for ent in gdb_args {
        args.push("-ex".into());
        args.push(ent.into());
    }

    exec("rust-gdb", args)
}

// TODO: currently not doing ricky debug magic
fn test_kernel(release: bool, target: Target) -> Result<()> {
    let mut args = vec![
        "test",
        "--message-format=json-render-diagnostics",
        // "-Zbuild-std=core,alloc",
        "--lib",
        "--target",
    ];

    let target_triple = target.target_triple();
    args.push(target_triple);

    if release {
        args.push("--release");
    }

    let mut cmd = Command::new("cargo");
    cmd.args(args)
        .env(
            "RUSTFLAGS",
            "-C relocation-model=static -C force-frame-pointers=yes",
        )
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    configure_c_toolchain(target, &mut cmd)?;

    let status = cmd.status()?;
    if !status.success() {
        return Err(Error::msg(format!(
            "cargo test failed with status {status}"
        )));
    }

    Ok(())
}

fn qemu_test(path_to_kernel: String, target: Target) -> Result<()> {
    let img_path = build_image(&(PathBuf::from(path_to_kernel), vec![]), false, target)?;
    let args = vec![
        path_to_string(&download_ovmf(target)?)?,
        format!("{}", path_to_string(&img_path)?),
        format!("file:{}/serial.txt", path_to_string(&run_dir()?)?),
    ];
    let qemu_str = format!("../run_qemu_{}.sh", target.name());
    eprintln!("running: {} {:?}", qemu_str, args);
    let res = Command::new(qemu_str)
        .args(args)
        .current_dir(run_dir()?)
        .status()?;
    println!("{:#?}", res);
    match res.code() {
        Some(code) => {
            if code == 1 {
                Ok(())
            } else {
                Err(anyhow::anyhow!("bad"))
            }
        }
        None => Err(anyhow::anyhow!("bad")),
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Image { target, release } => {
            build_image(&build_kernel(release, target)?, release, target)?;
        }
        Commands::Qemu {
            target,
            kvm,
            cores,
            mem,
            release,
        } => qemu(kvm, cores, mem, release, target)?,
        Commands::Gdb {
            target,
            kvm,
            release,
        } => gdb(kvm, release, target)?,
        Commands::Test { release, target } => test_kernel(release, target)?,
        Commands::QemuTest {
            path_to_kernel,
            target,
        } => qemu_test(path_to_kernel, target)?,
        Commands::Clean => {
            fs::remove_dir_all(cache_dir()?)?;
            cache_dir()?;
        }
    }

    Ok(())
}
