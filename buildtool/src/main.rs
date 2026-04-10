#![feature(decl_macro)]
#![feature(coroutines, coroutine_trait, stmt_expr_attributes)]
#![feature(gen_blocks)]

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::fs;

mod debug;
mod gdb;
mod image;
mod qemu;
mod qemu_test;
mod test;
mod util;

use crate::util::{Target, cache_dir};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
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
        #[arg(short = 'f', long, default_value = "fs_path")]
        filesystem_path: String,
    },
    QemuTest {
        test_cfg_path: String,
        #[arg(short = 'r', long)]
        release: bool,
        #[arg(
            long,
            help = "Stream serial output live to stdout and bypass cached test results"
        )]
        stdout: bool,
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

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Image { target, release } => image::run(release, target)?,
        Commands::Qemu {
            target,
            kvm,
            cores,
            mem,
            release,
            filesystem_path,
        } => qemu::run(kvm, cores, mem, release, target, filesystem_path)?,
        Commands::Gdb {
            target,
            kvm,
            release,
        } => gdb::run(kvm, release, target)?,
        Commands::Test { release, target } => test::run_all(release, target)?,
        Commands::QemuTest {
            test_cfg_path,
            release,
            stdout,
        } => qemu_test::run(test_cfg_path, release, stdout)?,
        Commands::Clean => {
            fs::remove_dir_all(cache_dir()?)?;
            cache_dir()?;
        }
    }

    Ok(())
}
