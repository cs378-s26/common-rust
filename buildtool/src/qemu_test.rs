use std::{
    fs,
    io::BufReader,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::OnceLock,
    thread::sleep,
    time::{Duration, Instant, SystemTime},
};

use anyhow::{Context, Error, Result, anyhow};
use cargo_metadata::{Artifact, Message, TargetKind};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::util::{
    Target, build_ext2_filesystem_from_dir, build_image_with_tag, cache_dir, download_ovmf,
    path_to_string,
};

#[derive(Debug, Clone, Deserialize)]
pub struct TestConfig {
    pub is_unittest: bool,
    pub n_runs: u32,
    pub timeout_ms: u64,
    pub test_name: Option<String>,
    pub expected_output_path: String,
    pub filesystem_path: Option<String>,
    pub target: TestTarget,
    pub qemu_args: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum TestTarget {
    #[serde(rename = "x86_64")]
    X86_64,
    #[serde(rename = "aarch64")]
    Aarch64,
}

impl TestTarget {
    pub fn to_target(self) -> Target {
        match self {
            TestTarget::X86_64 => Target::X86_64,
            TestTarget::Aarch64 => Target::Aarch64,
        }
    }

    fn qemu_cmd(self) -> &'static str {
        match self {
            TestTarget::X86_64 => "qemu-system-x86_64",
            TestTarget::Aarch64 => "qemu-system-aarch64",
        }
    }

    fn name(self) -> &'static str {
        match self {
            TestTarget::X86_64 => "x86_64",
            TestTarget::Aarch64 => "aarch64",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TestRunSummary {
    pub name: String,
    pub successful_runs: u32,
    pub total_runs: u32,
    pub passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedTestResult {
    name: String,
    config_path: String,
    target: TestTarget,
    successful_runs: u32,
    total_runs: u32,
    passed: bool,
    failed_at_run: Option<u32>,
    failure_reason: Option<String>,
}

impl CachedTestResult {
    fn to_summary(&self) -> TestRunSummary {
        TestRunSummary {
            name: self.name.clone(),
            successful_runs: self.successful_runs,
            total_runs: self.total_runs,
            passed: self.passed,
        }
    }
}

struct CachePaths {
    root_dir: PathBuf,
    serial_output: PathBuf,
    report: PathBuf,
}

pub fn run(config_path: String, release: bool, stream_serial_stdout: bool) -> Result<()> {
    let config_path = resolve_repo_root_path(Path::new(&config_path))?;
    let test_cfg = load_test_config(&config_path)?;
    let summary = match run_with_config(&config_path, &test_cfg, release, stream_serial_stdout) {
        Ok(summary) => summary,
        Err(err) => {
            let summary = write_failure_artifacts(&config_path, &test_cfg, &err.to_string())
                .unwrap_or_else(|_| fallback_summary(&config_path, &test_cfg));
            print_summary(&summary);
            return Err(err);
        }
    };

    print_summary(&summary);
    if summary.passed {
        Ok(())
    } else {
        Err(anyhow!(
            "{} failed - {}/{}",
            summary.name,
            summary.successful_runs,
            summary.total_runs
        ))
    }
}

pub fn load_test_config(config_path: &Path) -> Result<TestConfig> {
    let config_path = resolve_repo_root_path(config_path)?;
    let raw = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read test config: {}", config_path.display()))?;
    serde_json::from_str::<TestConfig>(&raw)
        .with_context(|| format!("failed to parse test config: {}", config_path.display()))
}

fn run_with_config(
    config_path: &Path,
    test_cfg: &TestConfig,
    release: bool,
    stream_serial_stdout: bool,
) -> Result<TestRunSummary> {
    if test_cfg.n_runs == 0 {
        return Err(anyhow!("n_runs must be >= 1 in {}", config_path.display()));
    }
    if test_cfg.timeout_ms == 0 {
        return Err(anyhow!(
            "timeout_ms must be >= 1 in {}",
            config_path.display()
        ));
    }

    if test_cfg.qemu_args.iter().any(|arg| arg == "-serial") {
        return Err(anyhow!(
            "qemu_args in {} must not include -serial; it is managed by qemu_test",
            config_path.display()
        ));
    }
    let target = test_cfg.target.to_target();
    let display_name = test_display_name(config_path, test_cfg);
    let cache_paths = cache_paths(config_path, test_cfg.target)?;
    let kernel_path = build_test_binary(test_cfg, release)?;
    let img_path = build_image_with_tag(
        &(kernel_path.clone(), vec![]),
        release,
        target,
        Some(&display_name),
    )?;

    let filesystem_image = if let Some(filesystem_path) = test_cfg.filesystem_path.as_deref() {
        let source_path = resolve_repo_root_path(Path::new(filesystem_path))?;
        Some(build_ext2_filesystem_from_dir(
            &source_path,
            &format!("{}-filesystem", display_name),
        )?)
    } else {
        None
    };

    let path_to_efi = path_to_string(&download_ovmf(target)?)?;
    let path_to_img = path_to_string(&img_path)?;

    let expected_output_path = resolve_repo_root_path(Path::new(&test_cfg.expected_output_path))?;
    let expected_output = fs::read_to_string(&expected_output_path).with_context(|| {
        format!(
            "failed to read expected output file: {}",
            expected_output_path.display()
        )
    })?;

    let mut deps = vec![
        config_path,
        expected_output_path.as_path(),
        kernel_path.as_path(),
        img_path.as_path(),
    ];
    if let Some(filesystem_image) = filesystem_image.as_deref() {
        deps.push(filesystem_image);
    }
    if !stream_serial_stdout
        && !cache_is_stale(&cache_paths.serial_output, &cache_paths.report, &deps)?
        && let Some(cached_report) = load_cached_report(&cache_paths.report)
    {
        return Ok(cached_report.to_summary());
    }

    let mut successful_runs = 0_u32;
    let mut failed_at_run = None;
    let mut failure_reason = None;

    for run_idx in 0..test_cfg.n_runs {
        if cache_paths.serial_output.exists() {
            fs::remove_file(&cache_paths.serial_output).with_context(|| {
                format!(
                    "failed to remove previous serial output: {}",
                    cache_paths.serial_output.display()
                )
            })?;
        }

        let mut qemu_args: Vec<String> = test_cfg
            .qemu_args
            .iter()
            .map(|arg| {
                arg.replace("{PATH_TO_EFI}", &path_to_efi)
                    .replace("{PATH_TO_IMG}", &path_to_img)
                    .replace("{PATH_TO_DISK}", &path_to_disk)
            })
            .collect();
        if let Some(filesystem_image) = filesystem_image.as_deref() {
            let filesystem_image = path_to_string(filesystem_image)?;
            qemu_args.push("-drive".into());
            qemu_args.push(format!(
                "if=none,id=testfs0,file={filesystem_image},format=raw"
            ));
            qemu_args.push("-device".into());
            qemu_args.push(format!("{},drive=testfs0", target.qemu_virtio_blk_device()));
        }
        if stream_serial_stdout {
            qemu_args.push("-chardev".into());
            qemu_args.push(format!(
                "stdio,id=serial0,signal=off,logfile={}",
                cache_paths.serial_output.display()
            ));
            qemu_args.push("-serial".into());
            qemu_args.push("chardev:serial0".into());
        } else {
            qemu_args.push("-serial".into());
            qemu_args.push(format!("file:{}", cache_paths.serial_output.display()));
        }

        let qemu_cmd = test_cfg.target.qemu_cmd();
        eprintln!(
            "running {}/{}: {} {:?}",
            run_idx + 1,
            test_cfg.n_runs,
            qemu_cmd,
            qemu_args
        );
        let status = match run_qemu_with_timeout(
            qemu_cmd,
            &qemu_args,
            &cache_paths.root_dir,
            test_cfg.timeout_ms,
        ) {
            Ok(status) => status,
            Err(err) => {
                failed_at_run = Some(run_idx + 1);
                failure_reason = Some(err.to_string());
                break;
            }
        };

        if !qemu_status_ok(&status) {
            failed_at_run = Some(run_idx + 1);
            failure_reason = Some(format!("{} failed with status {}", qemu_cmd, status));
            break;
        }

        let actual_output = match fs::read_to_string(&cache_paths.serial_output) {
            Ok(output) => output,
            Err(err) => {
                failed_at_run = Some(run_idx + 1);
                failure_reason = Some(format!(
                    "failed to read serial output after run {}: {}",
                    run_idx + 1,
                    err
                ));
                break;
            }
        };
        let actual_output = strip_ansi_escape_codes(&actual_output);
        fs::write(&cache_paths.serial_output, &actual_output).with_context(|| {
            format!(
                "failed to write stripped serial output after run {}: {}",
                run_idx + 1,
                cache_paths.serial_output.display()
            )
        })?;

        if let Err(err) = assert_output_match(
            &expected_output,
            &actual_output,
            run_idx + 1,
            &expected_output_path,
            &cache_paths.serial_output,
        ) {
            failed_at_run = Some(run_idx + 1);
            failure_reason = Some(err.to_string());
            break;
        }

        successful_runs += 1;
    }

    if !cache_paths.serial_output.exists() {
        fs::write(&cache_paths.serial_output, "").with_context(|| {
            format!(
                "failed to create serial output file: {}",
                cache_paths.serial_output.display()
            )
        })?;
    }

    let summary = TestRunSummary {
        name: display_name.clone(),
        successful_runs,
        total_runs: test_cfg.n_runs,
        passed: successful_runs == test_cfg.n_runs,
    };

    let cached_result = CachedTestResult {
        name: display_name,
        config_path: config_path.display().to_string(),
        target: test_cfg.target,
        successful_runs: summary.successful_runs,
        total_runs: summary.total_runs,
        passed: summary.passed,
        failed_at_run,
        failure_reason,
    };
    write_cached_report(&cache_paths.report, &cached_result)?;

    Ok(summary)
}

fn fallback_summary(config_path: &Path, test_cfg: &TestConfig) -> TestRunSummary {
    TestRunSummary {
        name: test_display_name(config_path, test_cfg),
        successful_runs: 0,
        total_runs: test_cfg.n_runs,
        passed: false,
    }
}

fn write_failure_artifacts(
    config_path: &Path,
    test_cfg: &TestConfig,
    failure_reason: &str,
) -> Result<TestRunSummary> {
    let summary = fallback_summary(config_path, test_cfg);
    let cache_paths = cache_paths(config_path, test_cfg.target)?;
    fs::write(
        &cache_paths.serial_output,
        format!("test setup failed: {}\n", failure_reason),
    )
    .with_context(|| {
        format!(
            "failed to write serial output file: {}",
            cache_paths.serial_output.display()
        )
    })?;

    let cached_result = CachedTestResult {
        name: summary.name.clone(),
        config_path: config_path.display().to_string(),
        target: test_cfg.target,
        successful_runs: 0,
        total_runs: summary.total_runs,
        passed: false,
        failed_at_run: if summary.total_runs > 0 {
            Some(summary.total_runs)
        } else {
            None
        },
        failure_reason: Some(failure_reason.to_string()),
    };
    write_cached_report(&cache_paths.report, &cached_result)?;

    Ok(summary)
}

fn cache_paths(config_path: &Path, target: TestTarget) -> Result<CachePaths> {
    let root_dir = cache_dir()?.join("testing");
    fs::create_dir_all(&root_dir).with_context(|| {
        format!(
            "failed to create test cache directory: {}",
            root_dir.display()
        )
    })?;

    let config_name = config_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("test");
    let safe_name = sanitize_name(config_name);
    let cache_name = format!("{}-{}", safe_name, target.name());

    Ok(CachePaths {
        serial_output: root_dir.join(format!("{cache_name}.serial.txt")),
        report: root_dir.join(format!("{cache_name}.report.json")),
        root_dir,
    })
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn cache_is_stale(serial_output: &Path, report: &Path, deps: &[&Path]) -> Result<bool> {
    if !serial_output.exists() || !report.exists() {
        return Ok(true);
    }

    let serial_time = modified_time(serial_output)?;
    let report_time = modified_time(report)?;
    let oldest_output = if serial_time < report_time {
        serial_time
    } else {
        report_time
    };

    for dep in deps {
        if modified_time(dep)? > oldest_output {
            return Ok(true);
        }
    }

    Ok(false)
}

fn modified_time(path: &Path) -> Result<SystemTime> {
    fs::metadata(path)
        .with_context(|| format!("failed to read metadata: {}", path.display()))?
        .modified()
        .with_context(|| format!("failed to read mtime: {}", path.display()))
}

fn load_cached_report(report_path: &Path) -> Option<CachedTestResult> {
    let raw = fs::read_to_string(report_path).ok()?;
    serde_json::from_str::<CachedTestResult>(&raw).ok()
}

fn write_cached_report(report_path: &Path, report: &CachedTestResult) -> Result<()> {
    let json = serde_json::to_string_pretty(report).context("failed to serialize test report")?;
    fs::write(report_path, json)
        .with_context(|| format!("failed to write test report: {}", report_path.display()))
}

fn print_summary(summary: &TestRunSummary) {
    println!(
        "{}... {} - {}/{}",
        summary.name,
        if summary.passed { "PASSED" } else { "FAILED" },
        summary.successful_runs,
        summary.total_runs
    );
}

fn test_display_name(config_path: &Path, test_cfg: &TestConfig) -> String {
    if !test_cfg.is_unittest
        && let Some(name) = test_cfg.test_name.as_deref()
        && !name.is_empty()
    {
        return name.to_string();
    }

    config_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("test")
        .to_string()
}

fn resolve_repo_root_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    Ok(std::env::current_dir()
        .context("failed to determine repository root from current directory")?
        .join(path))
}

fn assert_output_match(
    expected: &str,
    actual: &str,
    run_idx: u32,
    expected_path: &Path,
    actual_path: &Path,
) -> Result<()> {
    let expected = normalize_output(expected);
    let actual = normalize_output(actual);
    if expected == actual {
        return Ok(());
    }

    let mut mismatch_preview = String::new();
    for (line_idx, (exp_line, act_line)) in expected
        .lines()
        .zip(actual.lines())
        .enumerate()
        .filter(|(_, (exp, act))| exp != act)
        .take(1)
    {
        mismatch_preview = format!(
            "first mismatch at line {}:\nexpected: {:?}\nactual:   {:?}",
            line_idx + 1,
            exp_line,
            act_line
        );
    }
    if mismatch_preview.is_empty() {
        mismatch_preview = format!(
            "line count mismatch (expected {} lines, got {} lines)",
            expected.lines().count(),
            actual.lines().count()
        );
    }

    Err(anyhow!(
        "serial output mismatch on run {}.\nexpected: {}\nactual:   {}\n{}",
        run_idx,
        expected_path.display(),
        actual_path.display(),
        mismatch_preview
    ))
}

fn normalize_output(output: &str) -> String {
    let truncated = &output[output.find("Starting Testing Code...").unwrap()..];
    truncated.replace("\r\n", "\n")
}

fn strip_ansi_escape_codes(output: &str) -> String {
    // Crude but effective regex stripper for CSI/OSC/single-byte ESC sequences.
    ansi_escape_regex().replace_all(output, "").into_owned()
}

fn ansi_escape_regex() -> &'static Regex {
    static ANSI_ESCAPE_RE: OnceLock<Regex> = OnceLock::new();
    ANSI_ESCAPE_RE.get_or_init(|| {
        Regex::new(r"\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~]|\][^\x1B\x07]*(?:\x07|\x1B\\))")
            .expect("failed to compile ANSI escape regex")
    })
}

fn qemu_status_ok(status: &ExitStatus) -> bool {
    status.success() || status.code() == Some(1)
}

fn run_qemu_with_timeout(
    qemu_cmd: &str,
    qemu_args: &[String],
    current_dir: &Path,
    timeout_ms: u64,
) -> Result<ExitStatus> {
    let mut child = Command::new(qemu_cmd)
        .args(qemu_args)
        .current_dir(current_dir)
        .spawn()
        .with_context(|| format!("failed to launch {}", qemu_cmd))?;

    let timeout = Duration::from_millis(timeout_ms);
    let start = Instant::now();

    loop {
        if let Some(status) = child
            .try_wait()
            .with_context(|| format!("failed to wait on {}", qemu_cmd))?
        {
            return Ok(status);
        }

        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(anyhow!("{} timed out after {} ms", qemu_cmd, timeout_ms));
        }

        sleep(Duration::from_millis(10));
    }
}

fn artifact_matches_requested_test(
    artifact: &Artifact,
    test_cfg: &TestConfig,
    integration_test_name: Option<&str>,
) -> bool {
    if test_cfg.is_unittest {
        return artifact
            .target
            .kind
            .iter()
            .any(|kind| *kind == TargetKind::Lib);
    }

    artifact
        .target
        .kind
        .iter()
        .any(|kind| *kind == TargetKind::Test)
        && integration_test_name.is_some_and(|name| artifact.target.name == name)
}

fn build_test_binary(test_cfg: &TestConfig, release: bool) -> Result<PathBuf> {
    let target = test_cfg.target.to_target();
    let mut args = vec![
        "test",
        "-Zbuild-std=core,alloc",
        "-Zpanic-abort-tests",
        "--message-format=json-render-diagnostics",
        "--no-run",
        "--target",
    ];
    let integration_test_name = if test_cfg.is_unittest {
        None
    } else {
        Some(
            test_cfg
                .test_name
                .as_deref()
                .context("test_name is required when is_unittest is false")?,
        )
    };

    args.push(target.target_triple());

    if test_cfg.is_unittest {
        args.push("--lib");
    } else if let Some(test_name) = integration_test_name {
        args.push("--test");
        args.push(test_name);
    }

    if release {
        args.push("--release");
    }

    let mut cmd = Command::new("cargo");
    cmd.args(args)
        .env(
            "RUSTFLAGS",
            "-C relocation-model=static -C force-frame-pointers=yes",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let mut cmd = cmd.spawn()?;
    let stdout = cmd
        .stdout
        .take()
        .context("failed to capture cargo stdout")?;
    let reader = BufReader::new(stdout);

    let mut executable = None;
    let mut fallback_executable = None;
    for message in Message::parse_stream(reader) {
        if let Message::CompilerArtifact(artifact) = message? {
            let matches_requested =
                artifact_matches_requested_test(&artifact, test_cfg, integration_test_name);

            if let Some(path) = artifact.executable {
                let path = PathBuf::from(path);
                if fallback_executable.is_none() {
                    fallback_executable = Some(path.clone());
                }

                if matches_requested {
                    executable = Some(path);
                }
            }
        }
    }

    let status = cmd.wait()?;
    if !status.success() {
        return Err(Error::msg(format!(
            "cargo test failed with status {status}"
        )));
    }

    executable
        .or(fallback_executable)
        .ok_or(Error::msg("failed to locate test executable"))
}
