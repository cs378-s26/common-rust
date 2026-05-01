use crate::print::kprintln;

#[derive(Debug, Clone, Copy)]
pub struct BuildInfo {
    pub version: &'static str,
    pub build_timestamp: &'static str,

    pub cargo_debug: bool,
    pub cargo_opt_level: &'static str,
    pub cargo_target_triple: &'static str,

    pub git_branch: &'static str,
    pub git_sha: &'static str,

    pub rustc_channel: &'static str,
    pub rustc_commit_date: &'static str,
    pub rustc_commit_hash: &'static str,
    pub rustc_host_triple: &'static str,
    pub rustc_llvm_version: &'static str,
    pub rustc_semver: &'static str,
}

pub const BUILD_INFO: BuildInfo = BuildInfo {
    version: env!("CARGO_PKG_VERSION"),
    build_timestamp: env!("VERGEN_BUILD_TIMESTAMP"),

    cargo_debug: matches!(env!("VERGEN_CARGO_DEBUG"), "true"),
    cargo_opt_level: env!("VERGEN_CARGO_OPT_LEVEL"),
    cargo_target_triple: env!("VERGEN_CARGO_TARGET_TRIPLE"),

    git_branch: env!("VERGEN_GIT_BRANCH"),
    git_sha: env!("VERGEN_GIT_SHA"),

    rustc_channel: env!("VERGEN_RUSTC_CHANNEL"),
    rustc_commit_date: env!("VERGEN_RUSTC_COMMIT_DATE"),
    rustc_commit_hash: env!("VERGEN_RUSTC_COMMIT_HASH"),
    rustc_host_triple: env!("VERGEN_RUSTC_HOST_TRIPLE"),
    rustc_llvm_version: env!("VERGEN_RUSTC_LLVM_VERSION"),
    rustc_semver: env!("VERGEN_RUSTC_SEMVER"),
};

impl BuildInfo {
    pub fn print(&self) {
        kprintln!("build info:");
        kprintln!(
            "  kernel v{}+git.{}.{} built on {}",
            self.version,
            self.git_branch,
            &self.git_sha[0..10],
            self.build_timestamp
        );
        kprintln!(
            "  --debug={}, -O{}, --target={}",
            self.cargo_debug,
            self.cargo_opt_level,
            self.cargo_target_triple
        );

        kprintln!(
            "  rustc {} {} ({} {}) host {} llvm {} ",
            self.rustc_channel,
            self.rustc_semver,
            &self.rustc_commit_hash[0..10],
            self.rustc_commit_date,
            self.rustc_host_triple,
            self.rustc_llvm_version
        );
    }
}
