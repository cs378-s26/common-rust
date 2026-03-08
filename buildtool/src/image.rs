use anyhow::Result;

use crate::util::{Target, build_image, build_kernel};

pub fn run(release: bool, target: Target) -> Result<()> {
    build_image(&build_kernel(release, target)?, release, target)?;
    Ok(())
}
