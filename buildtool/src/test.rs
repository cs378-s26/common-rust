use crate::util::Target;
use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;

use crate::qemu_test;

fn load_test_config_paths(target: Target) -> Result<Vec<PathBuf>> {
    let mut config_paths = glob::glob("test_cfgs/**/*_test.json")
        .context("failed to read test config glob pattern")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed while expanding test config paths")?;
    config_paths.sort();

    let mut filtered = Vec::new();
    for config_path in config_paths {
        let test_cfg = qemu_test::load_test_config(&config_path)?;
        if test_cfg.target.to_target() == target {
            filtered.push(config_path);
        }
    }

    Ok(filtered)
}

pub fn run_all(release: bool, target: Target) -> Result<()> {
    let mut total = 0_u32;
    let mut failed = 0_u32;

    for config_path in load_test_config_paths(target)? {
        total += 1;
        if let Err(err) = qemu_test::run(config_path.display().to_string(), release, false) {
            failed += 1;
            eprintln!("{}: {}", config_path.display(), err);
        }
    }

    if failed > 0 {
        return Err(anyhow!("{} out of {} tests failed", failed, total));
    }

    Ok(())
}
