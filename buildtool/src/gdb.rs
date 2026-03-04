use anyhow::Result;

use crate::{Target, build_kernel, exec, path_to_string};

pub fn run(kvm: bool, release: bool, target: Target) -> Result<()> {
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
