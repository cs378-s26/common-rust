use anyhow::Result;

use crate::{Target, build_image, build_kernel};

pub fn run(release: bool, target: Target) -> Result<()> {
    build_image(&build_kernel(release, target)?, release, target)?;
    Ok(())
}
