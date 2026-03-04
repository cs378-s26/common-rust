use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;

use crate::{Target, qemu_test};

fn load_test_config_paths() -> Result<Vec<PathBuf>> {
    let mut config_paths = glob::glob("test_cfgs/**/*_test.json")
        .context("failed to read test config glob pattern")?
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed while expanding test config paths")?;
    config_paths.sort();
    Ok(config_paths)
}

pub fn run_all(release: bool, target: Target) -> Result<()> {
    let mut total = 0_u32;
    let mut failed = 0_u32;

    for config_path in load_test_config_paths()? {
        match qemu_test::run_for_target(&config_path, release, target) {
            Ok(Some(summary)) => {
                total += 1;
                if !summary.passed {
                    failed += 1;
                }
            }
            Ok(None) => {}
            Err(err) => {
                failed += 1;
                total += 1;
                eprintln!("{}... FAILED - 0/0", config_path.display());
                eprintln!("{}: {}", config_path.display(), err);
            }
        }
    }

    if failed > 0 {
        return Err(anyhow!("{} out of {} tests failed", failed, total));
    }

    Ok(())
}
