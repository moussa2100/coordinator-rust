//! The PATH an agent is started with. A process only sees the PATH it inherited, which can miss
//! folders added since it started, such as omp's install folder. On Windows the user and machine
//! PATH from the registry are appended after the inherited entries.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The inherited PATH with any registry entries it lacks appended.
/// Falls back to the inherited PATH alone if the combined value cannot be encoded.
pub fn agent_path() -> Option<OsString> {
    let mut entries: Vec<PathBuf> = env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect())
        .unwrap_or_default();
    for entry in registry_entries() {
        if !entries.iter().any(|e| same_dir(e, &entry)) {
            entries.push(entry);
        }
    }
    env::join_paths(entries).ok()
}

/// Compares folder paths ignoring case and trailing slashes, as Windows does.
fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

#[cfg(windows)]
fn registry_entries() -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    const MACHINE_ENV: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    let user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .and_then(|key| key.get_value::<String, _>("Path"));
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(MACHINE_ENV)
        .and_then(|key| key.get_value::<String, _>("Path"));
    [user, machine]
        .into_iter()
        .flatten()
        .flat_map(|path| {
            path.split(';')
                .map(|entry| expand_vars(entry.trim().trim_matches('"')))
                .collect::<Vec<String>>()
        })
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(not(windows))]
fn registry_entries() -> Vec<PathBuf> {
    Vec::new()
}

/// Replaces `%NAME%` with that environment variable's value. Unknown names stay as written,
/// since the registry PATH stores entries such as `%USERPROFILE%\AppData` unexpanded.
#[cfg(windows)]
fn expand_vars(entry: &str) -> String {
    let mut out = String::new();
    let mut rest = entry;
    while let Some(start) = rest.find('%') {
        let Some(len) = rest[start + 1..].find('%') else {
            break;
        };
        let name = &rest[start + 1..start + 1 + len];
        out.push_str(&rest[..start]);
        match env::var(name) {
            Ok(value) if !name.is_empty() => out.push_str(&value),
            _ => out.push_str(&rest[start..start + len + 2]),
        }
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}
