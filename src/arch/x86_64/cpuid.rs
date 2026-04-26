use core::fmt::{self, Display};

use x86::cpuid::CpuId;

use crate::print::{ANSIFormatter, Color};

pub struct Features<'a>(pub &'a CpuId);

impl<'a> Display for Features<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn format(flag: bool) -> ANSIFormatter<'static, &'static str> {
            if flag {
                Color::GREEN.format(&"present")
            } else {
                Color::RED.format(&"not present")
            }
        }

        if let Some(features) = self.0.get_feature_info() {
            let _ = writeln!(f, "features:");
            let _ = writeln!(f, "avx = {}", format(features.has_avx()));
            let _ = writeln!(f, "sse4.2 = {}", format(features.has_sse42()));
            let _ = writeln!(f, "sse4.1 = {}", format(features.has_sse41()));
            let _ = writeln!(f, "ssse3 = {}", format(features.has_ssse3()));
            let _ = writeln!(f, "sse3 = {}", format(features.has_sse3()));
            let _ = writeln!(f, "xsave = {}", format(features.has_xsave()));
            let _ = writeln!(f, "oxsave = {}", format(features.has_oxsave()));
            let _ = writeln!(
                f,
                "monitor/mwait = {}",
                format(features.has_monitor_mwait())
            );
            let _ = writeln!(f, "vmx = {}", format(features.has_vmx()));
        } else {
            let _ = writeln!(f, "{}", Color::RED.format(&"features not detected"));
        }

        if let Some(features) = self.0.get_extended_feature_info() {
            let _ = writeln!(f, "extended features:");
            let _ = writeln!(f, "fsgsbase = {}", format(features.has_fsgsbase()));
        } else {
            let _ = writeln!(
                f,
                "{}",
                Color::RED.format(&"extended features not detected")
            );
        }

        if let Some(tsc) = self.0.get_tsc_info()
            && let Some(freq) = tsc.tsc_frequency()
        {
            let _ = writeln!(f, "nominal tsc freq: {} MHz", freq);
        }

        // TODO: svm, rdtscp

        Ok(())
    }
}
