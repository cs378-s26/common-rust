use anyhow::{Error, Result};
use std::env::current_dir;
use std::fs::{self, File};
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const LIMINE_X86_URL: &str =
    "https://github.com/limine-bootloader/limine/raw/refs/heads/v10.x-binary/BOOTX64.EFI";
const LIMINE_AARCH64_URL: &str =
    "https://codeberg.org/Limine/Limine/raw/tag/v10.5.1-binary/BOOTAA64.EFI";
const OVMF_X86_URL: &str = "https://github.com/osdev0/edk2-ovmf-nightly/releases/download/nightly-20251126T024608Z/ovmf-code-x86_64.fd";
const OVMF_AARCH64_URL: &str = "https://github.com/osdev0/edk2-ovmf-nightly/releases/download/nightly-20251126T024608Z/ovmf-code-aarch64.fd";
pub const LIMINE_CONF: &str = "limine.conf";

#[derive(clap::ValueEnum, Clone, Debug, Copy, PartialEq, Eq)]
pub enum Target {
    X86_64,
    Aarch64,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::X86_64 => "x86_64",
            Target::Aarch64 => "aarch64",
        }
    }

    pub fn limine_url(self) -> &'static str {
        match self {
            Target::X86_64 => LIMINE_X86_URL,
            Target::Aarch64 => LIMINE_AARCH64_URL,
        }
    }

    pub fn ovmf_url(self) -> &'static str {
        match self {
            Target::X86_64 => OVMF_X86_URL,
            Target::Aarch64 => OVMF_AARCH64_URL,
        }
    }

    pub fn target_triple(self) -> &'static str {
        match self {
            Target::X86_64 => "x86_64-unknown-none",
            Target::Aarch64 => "aarch64-unknown-none",
        }
    }

    pub fn strip_tool(self) -> &'static str {
        match self {
            Target::X86_64 => "strip",
            Target::Aarch64 => "aarch64-linux-gnu-strip",
        }
    }

    pub fn limine_efi_path(self) -> &'static str {
        match self {
            Target::X86_64 => "efi/boot/BOOTX64.EFI",
            Target::Aarch64 => "efi/boot/BOOTAA64.EFI",
        }
    }

    pub fn qemu_machine(self) -> &'static str {
        match self {
            Target::X86_64 => "pc",
            Target::Aarch64 => "virt",
        }
    }

    pub fn qemu_display_args(self) -> &'static [&'static str] {
        match self {
            Target::X86_64 => &["-vga", "std"],
            // "virt" machine on aarch64 does not support -vga; use a firmware framebuffer device.
            Target::Aarch64 => &["-device", "ramfb"],
        }
    }

    pub fn qemu_binary(self) -> &'static str {
        match self {
            Target::X86_64 => "qemu-system-x86_64",
            Target::Aarch64 => "qemu-system-aarch64",
        }
    }

    pub fn qemu_cpu_without_kvm(self) -> Option<&'static str> {
        match self {
            Target::X86_64 => None,
            // QEMU may default to a 32-bit ARM CPU on "virt"; force a stable AArch64 model.
            Target::Aarch64 => Some("cortex-a72"),
        }
    }

    pub fn requires_c_toolchain_config(self) -> bool {
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

pub fn configure_c_toolchain(target: Target, cmd: &mut Command) -> Result<()> {
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

pub fn cache_dir() -> Result<PathBuf> {
    let root = current_dir()?.join("buildtool-cache");
    fs::create_dir_all(&root)?;
    Ok(root)
}

pub fn resources_dir() -> Result<PathBuf> {
    Ok(current_dir()?.join("resources"))
}

pub fn run_dir() -> Result<PathBuf> {
    let root = current_dir()?.join("run");
    fs::create_dir_all(&root)?;
    Ok(root)
}

fn download_if_missing(url: &str, dest_path: &PathBuf) -> Result<()> {
    if dest_path.exists() {
        return Ok(());
    }

    let response = reqwest::blocking::get(url)?;
    let mut dest = File::create(dest_path)?;
    let content = response.bytes()?;
    io::copy(&mut content.as_ref(), &mut dest)?;
    Ok(())
}

pub fn download_limine(target: Target) -> Result<PathBuf> {
    let path = cache_dir()?.join(format!("limine-{}.efi", target.name()));
    download_if_missing(target.limine_url(), &path)?;
    Ok(path)
}

pub fn download_ovmf(target: Target) -> Result<PathBuf> {
    let path = cache_dir()?.join(format!("ovmf-{}.fd", target.name()));
    download_if_missing(target.ovmf_url(), &path)?;
    Ok(path)
}
