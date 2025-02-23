mod sysinfo_ffi;
mod system_properties;

use crate::extra;
use crate::traits::*;
use itertools::Itertools;
use std::ffi::{CStr, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use sysinfo_ffi::sysinfo;
use system_properties::getprop;

impl From<std::str::Utf8Error> for ReadoutError {
    fn from(e: std::str::Utf8Error) -> Self {
        ReadoutError::Other(e.to_string())
    }
}
impl From<std::num::ParseFloatError> for ReadoutError {
    fn from(e: std::num::ParseFloatError) -> Self {
        ReadoutError::Other(e.to_string())
    }
}

pub struct AndroidBatteryReadout;

pub struct AndroidKernelReadout {
    utsname: Option<libc::utsname>,
}

pub struct AndroidGeneralReadout {
    sysinfo: sysinfo,
}

pub struct AndroidMemoryReadout {
    sysinfo: sysinfo,
}

pub struct AndroidProductReadout;

pub struct AndroidPackageReadout;

impl BatteryReadout for AndroidBatteryReadout {
    fn new() -> Self {
        AndroidBatteryReadout
    }

    fn percentage(&self) -> Result<u8, ReadoutError> {
        let bat_path = Path::new("/sys/class/power_supply/battery/capacity");
        let percentage_text = extra::pop_newline(fs::read_to_string(bat_path)?);
        let percentage_parsed = percentage_text.parse::<u8>();

        match percentage_parsed {
            Ok(p) => Ok(p),
            Err(e) => Err(ReadoutError::Other(format!(
                "Could not parse the value '{}' of {} into a \
            digit: {:?}",
                percentage_text,
                bat_path.to_str().unwrap_or_default(),
                e
            ))),
        }
    }

    fn status(&self) -> Result<BatteryState, ReadoutError> {
        let bat_path = Path::new("/sys/class/power_supply/battery/status");

        let status_text = extra::pop_newline(fs::read_to_string(bat_path)?).to_lowercase();
        match &status_text[..] {
            "charging" => Ok(BatteryState::Charging),
            "discharging" | "full" => Ok(BatteryState::Discharging),
            s => Err(ReadoutError::Other(format!(
                "Got unexpected value '{}' from {}.",
                s,
                bat_path.to_str().unwrap_or_default()
            ))),
        }
    }
}

impl KernelReadout for AndroidKernelReadout {
    fn new() -> Self {
        let mut __utsname: libc::utsname = unsafe { std::mem::zeroed() };
        let utsname: Option<libc::utsname>;
        if unsafe { libc::uname(&mut __utsname) } == -1 {
            utsname = None
        } else {
            utsname = Some(__utsname)
        }
        AndroidKernelReadout { utsname }
    }

    fn os_release(&self) -> Result<String, ReadoutError> {
        if let Some(utsname) = self.utsname {
            return Ok(unsafe { CStr::from_ptr(utsname.release.as_ptr()) }
                .to_str()
                .unwrap()
                .to_owned());
        } else {
            Err(ReadoutError::Other(String::from(
                "Failed to get os_release",
            )))
        }
    }

    fn os_type(&self) -> Result<String, ReadoutError> {
        if let Some(utsname) = self.utsname {
            return Ok(unsafe { CStr::from_ptr(utsname.sysname.as_ptr()) }
                .to_str()
                .unwrap()
                .to_owned());
        } else {
            Err(ReadoutError::Other(String::from("Failed to get os_type")))
        }
    }
}

impl GeneralReadout for AndroidGeneralReadout {
    fn new() -> Self {
        AndroidGeneralReadout {
            sysinfo: sysinfo::new(),
        }
    }

    fn machine(&self) -> Result<String, ReadoutError> {
        let product_readout = AndroidProductReadout::new();

        let vendor = product_readout.vendor()?;
        let family = product_readout.family()?;
        let product = product_readout.product()?;

        let product = format!("{} {} ({})", vendor, family, product);
        let new_product: Vec<_> = product.split_whitespace().into_iter().unique().collect();

        if product.is_empty() || product.len() <= 15 {
            return Ok(new_product.into_iter().join(" "));
        }

        Ok(product)
    }

    fn local_ip(&self, interface: Option<String>) -> Result<String, ReadoutError> {
        crate::shared::local_ip(interface)
    }

    fn username(&self) -> Result<String, ReadoutError> {
        crate::shared::username()
    }

    fn hostname(&self) -> Result<String, ReadoutError> {
        let __name: *mut std::os::raw::c_char = CString::new("").unwrap().into_raw();
        let __len: usize = libc::_SC_HOST_NAME_MAX as usize;
        let ret = unsafe { libc::gethostname(__name, __len) };
        if ret == -1 {
            Err(ReadoutError::Other(String::from("Failed to get hostname")))
        } else {
            Ok(unsafe { CStr::from_ptr(__name).to_string_lossy().into_owned() })
        }
    }

    fn shell(&self, format: ShellFormat, kind: ShellKind) -> Result<String, ReadoutError> {
        if let Some(shell) = std::env::var_os("SHELL") {
            if let Some(relative) = PathBuf::from(shell).file_name() {
                if let Some(str) = relative.to_str() {
                    return Ok(str.to_owned());
                }
            }
        }

        return crate::shared::shell(format, kind);
    }

    fn cpu_model_name(&self) -> Result<String, ReadoutError> {
        use std::io::{BufRead, BufReader};
        let file = fs::File::open("/proc/cpuinfo");
        let mut model: Option<String> = None;
        let mut hardware: Option<String> = None;
        let mut processor: Option<String> = None;

        if let Ok(content) = file {
            let reader = BufReader::new(content);
            for line in reader.lines().into_iter().flatten() {
                if line.starts_with("Hardware") {
                    hardware = Some(
                        line.replace("Hardware", "")
                            .replace(":", "")
                            .trim()
                            .to_string(),
                    );
                    break; // if we already got hardware then others are not needed
                } else if line.starts_with("Processor") {
                    processor = Some(
                        line.replace("Processor", "")
                            .replace(":", "")
                            .trim()
                            .to_string(),
                    );
                } else if line.starts_with("model name") && model.is_none() {
                    model = Some(
                        line.replace("model name", "")
                            .replace(":", "")
                            .trim()
                            .to_string(),
                    );
                }
            }
        }
        match (hardware, model, processor) {
            (Some(hardware), _, _) => Ok(hardware),
            (_, Some(model), _) => Ok(model),
            (_, _, Some(processor)) => Ok(processor),
            (_, _, _) => Err(ReadoutError::Other(String::from(
                "Failed to get processor model name",
            ))),
        }
    }

    fn cpu_physical_cores(&self) -> Result<usize, ReadoutError> {
        crate::shared::cpu_physical_cores()
    }

    fn cpu_cores(&self) -> Result<usize, ReadoutError> {
        crate::shared::cpu_cores()
    }

    fn cpu_usage(&self) -> Result<usize, ReadoutError> {
        let mut info = self.sysinfo;
        let info_ptr: *mut sysinfo = &mut info;
        let ret = unsafe { sysinfo(info_ptr) };
        if ret != -1 {
            let f_load = 1f64 / (1 << libc::SI_LOAD_SHIFT) as f64;
            let cpu_usage = info.loads[0] as f64 * f_load;
            let cpu_usage_u = (cpu_usage / num_cpus::get() as f64 * 100.0).round() as usize;
            if cpu_usage_u != 0 {
                return Ok(cpu_usage_u as usize);
            }
            Err(ReadoutError::Other("Processor usage is null.".to_string()))
        } else {
            Err(ReadoutError::Other(
                "Failed to get system statistics".to_string(),
            ))
        }
    }

    fn uptime(&self) -> Result<usize, ReadoutError> {
        let mut info = self.sysinfo;
        let info_ptr: *mut sysinfo = &mut info;
        let ret = unsafe { sysinfo(info_ptr) };
        if ret != -1 {
            Ok(info.uptime as usize)
        } else {
            Err(ReadoutError::Other(
                "Failed to get system statistics".to_string(),
            ))
        }
    }
}

impl MemoryReadout for AndroidMemoryReadout {
    fn new() -> Self {
        AndroidMemoryReadout {
            sysinfo: sysinfo::new(),
        }
    }

    fn total(&self) -> Result<u64, ReadoutError> {
        let mut info = self.sysinfo;
        let info_ptr: *mut sysinfo = &mut info;
        let ret = unsafe { sysinfo(info_ptr) };
        if ret != -1 {
            Ok(info.totalram * info.mem_unit as u64 / 1024)
        } else {
            Err(ReadoutError::Other(
                "Failed to get system statistics".to_string(),
            ))
        }
    }

    fn free(&self) -> Result<u64, ReadoutError> {
        let mut info = self.sysinfo;
        let info_ptr: *mut sysinfo = &mut info;
        let ret = unsafe { sysinfo(info_ptr) };
        if ret != -1 {
            Ok(info.freeram * info.mem_unit as u64 / 1024)
        } else {
            Err(ReadoutError::Other(
                "Failed to get system statistics".to_string(),
            ))
        }
    }

    fn buffers(&self) -> Result<u64, ReadoutError> {
        let mut info = self.sysinfo;
        let info_ptr: *mut sysinfo = &mut info;
        let ret = unsafe { sysinfo(info_ptr) };
        if ret != -1 {
            Ok(info.bufferram * info.mem_unit as u64 / 1024)
        } else {
            Err(ReadoutError::Other(
                "Failed to get system statistics".to_string(),
            ))
        }
    }

    fn cached(&self) -> Result<u64, ReadoutError> {
        Ok(crate::shared::get_meminfo_value("Cached"))
    }

    fn reclaimable(&self) -> Result<u64, ReadoutError> {
        Ok(crate::shared::get_meminfo_value("SReclaimable"))
    }

    fn used(&self) -> Result<u64, ReadoutError> {
        let total = self.total().unwrap();
        let free = self.free().unwrap();
        let cached = self.cached().unwrap();
        let reclaimable = self.reclaimable().unwrap();
        let buffers = self.buffers().unwrap();

        Ok(total - free - cached - reclaimable - buffers)
    }
}

impl ProductReadout for AndroidProductReadout {
    fn new() -> Self {
        AndroidProductReadout
    }

    fn family(&self) -> Result<String, ReadoutError> {
        getprop("ro.product.model").ok_or(ReadoutError::Other("getprop failed".to_string()))
        // ro.product.model
        // ro.product.odm.model
        // ro.product.product.model
        // ro.product.system.model
        // ro.product.system_ext.model
        // ro.product.vendor.model
        // Same in all cases ( needs more testing in other devices )
    }

    fn vendor(&self) -> Result<String, ReadoutError> {
        getprop("ro.product.brand").ok_or(ReadoutError::Other("getprop failed".to_string()))
        // ro.product.brand
        // ro.product.manufacturer
        // ro.product.odm.brand
        // ro.product.odm.manufacturer
        // ro.product.product.brand
        // ro.product.product.manufacturer
        // ro.product.system.brand
        // ro.product.system.manufacturer
        // ro.product.system_ext.brand
        // ro.product.system_ext.manufacturer
        // ro.product.vendor.brand
        // ro.product.vendor.manufacturer
        // Same in all cases ( needs more testing in other devices )
    }

    fn product(&self) -> Result<String, ReadoutError> {
        getprop("ro.build.product").ok_or(ReadoutError::Other("getprop failed".to_string()))
        // ro.build.product
        // ro.product.device
        // ro.product.odm.device
        // ro.product.product.device
        // ro.product.system.device
        // ro.product.system_ext.device
        // ro.product.vendor.device
        // Same in all cases ( needs more testing in other devices )
    }
}

impl PackageReadout for AndroidPackageReadout {
    fn new() -> Self {
        AndroidPackageReadout
    }

    /// Supports: pm, dpkg, cargo
    fn count_pkgs(&self) -> Vec<(PackageManager, usize)> {
        let mut packages = Vec::new();
        // Since the target is Android we can assume that pm is available
        if let Some(c) = AndroidPackageReadout::count_pm() {
            packages.push((PackageManager::Android, c));
        }

        if extra::which("dpkg") {
            if let Some(c) = AndroidPackageReadout::count_dpkg() {
                packages.push((PackageManager::Dpkg, c));
            }
        }

        if extra::which("cargo") {
            if let Some(c) = AndroidPackageReadout::count_cargo() {
                packages.push((PackageManager::Cargo, c));
            }
        }

        packages
    }
}

impl AndroidPackageReadout {
    /// Returns the number of installed apps for the system
    /// Includes all apps ( user + system )
    fn count_pm() -> Option<usize> {
        let pm_output = Command::new("pm")
            .args(&["list", "packages"])
            .stdout(Stdio::piped())
            .output()
            .unwrap();

        extra::count_lines(
            String::from_utf8(pm_output.stdout)
                .expect("ERROR: \"pm list packages\" output was not valid UTF-8"),
        )
    }
    /// Return the number of installed packages for systems
    /// that have `dpkg` installed.
    /// In android that's mainly termux.
    fn count_dpkg() -> Option<usize> {
        let prefix = match std::env::var_os("PREFIX") {
            None => return None,
            Some(prefix) => prefix,
        };

        let dpkg_dir = Path::new(&prefix).join("var/lib/dpkg/info");
        let dir_entries = extra::list_dir_entries(&dpkg_dir);

        if !dir_entries.is_empty() {
            return Some(
                dir_entries
                    .iter()
                    .filter(|x| {
                        if let Some(ext) = extra::path_extension(x) {
                            ext == "list"
                        } else {
                            false
                        }
                    })
                    .into_iter()
                    .count(),
            );
        }

        None
    }

    /// Returns the number of installed packages for systems
    /// that have `cargo` installed.
    fn count_cargo() -> Option<usize> {
        crate::shared::count_cargo()
    }
}
