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
        if let Err(err) = qemu_test::run(config_path.display().to_string(), release) {
            failed += 1;
            eprintln!("{}: {}", config_path.display(), err);
        }
    }

    if failed > 0 {
        return Err(anyhow!("{} out of {} tests failed", failed, total));
    }

    Ok(())
}

pub fn run_single(release: bool, target: Target, pattern: &str) -> Result<()> {
    let all_tests = load_test_config_paths(target)?;
    
    // Filter tests matching the pattern
    let matching_tests: Vec<_> = all_tests
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains(pattern))
        })
        .collect();

    if matching_tests.is_empty() {
        eprintln!("No tests matching '{}' found.", pattern);
        eprintln!("Available tests for target {:?}:", target);
        for test in all_tests {
            if let Some(name) = test.file_name().and_then(|n| n.to_str()) {
                eprintln!("  - {}", name);
            }
        }
        return Err(anyhow!("No matching tests found"));
    }

    if matching_tests.len() > 1 {
        eprintln!("Multiple tests matching '{}' found:", pattern);
        for test in &matching_tests {
            if let Some(name) = test.file_name().and_then(|n| n.to_str()) {
                eprintln!("  - {}", name);
            }
        }
        eprintln!("\nRunning all {} matching tests...\n", matching_tests.len());
    }

    let mut total = 0_u32;
    let mut failed = 0_u32;

    for config_path in matching_tests {
        total += 1;
        if let Err(err) = qemu_test::run(config_path.display().to_string(), release) {
            failed += 1;
            eprintln!("{}: {}", config_path.display(), err);
        }
    }

    if failed > 0 {
        return Err(anyhow!("{} out of {} tests failed", failed, total));
    }

    Ok(())
}

pub fn list_tests(target: Target) -> Result<()> {
    let all_tests = load_test_config_paths(target)?;
    
    if all_tests.is_empty() {
        eprintln!("No tests found for target {:?}", target);
        return Ok(());
    }

    println!("Available tests for target {:?}:", target);
    for test_path in all_tests {
        let test_cfg = qemu_test::load_test_config(&test_path)?;
        let test_name = if test_cfg.is_unittest {
            "unittest".to_string()
        } else {
            test_cfg.test_name.clone().unwrap_or_else(|| "unnamed".to_string())
        };
        
        if let Some(file_name) = test_path.file_name().and_then(|n| n.to_str()) {
            println!("  {} ({})", file_name, test_name);
        }
    }
    
    Ok(())
}
