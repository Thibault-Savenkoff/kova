// Packages (PLAN Part 4 "Packages"): `import json from "github.com/ada/kova-json" version "1.2.0"`.
// A package is a git repository whose tag `v1.2.0` (or `1.2.0`) holds a `package.kova`. Kova
// downloads it with `git` the first time `run`, `build` or `test` meets it, into a cache shared by
// every program, and writes its commit and the fingerprint of its files into `kova.lock`, next to
// the program: from then on, a package whose files differ is refused. The editor never downloads.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::locale::t;

/// The file of a package that `import` reads.
pub const ENTRY: &str = "package.kova";
/// Set by `run`, `build` and `test`: only they download.
pub static DOWNLOAD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Next to the program: what each package was the first time.
pub const LOCK: &str = "kova.lock";

/// The folder holding `address` at `version`, downloaded if needed and allowed; else the message
/// saying why not. `program_dir` holds `kova.lock`.
pub fn locate(address: &str, version: &str, program_dir: &Path, download: bool) -> Result<PathBuf, String> {
    if !address_fits(address) {
        return Err(t!("package_bad_address", address = address));
    }
    if !version_fits(version) {
        return Err(t!("package_bad_version", version = version));
    }
    let dir = cache().join(address).join(version);
    let lock = program_dir.join(LOCK);
    let remembered = read_lock(&lock).into_iter().find(|e| e.0 == address && e.1 == version);
    let commit = if dir.is_dir() {
        None
    } else if !download {
        return Err(t!("package_not_downloaded", address = address, version = version));
    } else {
        Some(fetch(address, version, &dir)?)
    };
    if let (Some(c), Some((_, _, was, _))) = (&commit, &remembered)
        && c != was
    {
        return Err(t!("package_changed", address = address, version = version, lock = LOCK));
    }
    let print = fingerprint(&dir);
    match remembered {
        Some((.., was)) if was != print => Err(t!("package_changed", address = address, version = version, lock = LOCK)),
        Some(_) => Ok(dir),
        // First met: remembered from now on (never by the editor, which only reads).
        None if download => {
            let commit = commit.or_else(|| head(&dir)).unwrap_or_else(|| "-".into());
            write_lock(&lock, (address.into(), version.into(), commit, print));
            Ok(dir)
        }
        None => Ok(dir),
    }
}

/// `host.tld/owner/repo…`: plain characters only, so that `git` reads nothing but an address.
fn address_fits(a: &str) -> bool {
    let parts: Vec<&str> = a.split('/').collect();
    parts.len() >= 3
        && parts[0].contains('.')
        && parts.iter().all(|p| !p.is_empty() && !p.starts_with(['-', '.']) && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
}

fn version_fits(v: &str) -> bool {
    !v.is_empty() && !v.starts_with(['-', '.']) && !v.contains("..") && v.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
}

/// Where packages are kept: `KOVA_PACKAGES`, else the system's cache folder.
fn cache() -> PathBuf {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(p) = env("KOVA_PACKAGES") {
        return p;
    }
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Caches"))
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("kova").join("packages")
}

/// Clones the tag `v{version}` (else `{version}`) into `dir`, through a folder of this process
/// (two programs may download at once); the commit.
fn fetch(address: &str, version: &str, dir: &Path) -> Result<String, String> {
    if Command::new("git").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        return Err(t!("package_git_missing", address = address));
    }
    // `KOVA_PACKAGES_FROM`: another place to clone from (the tests use local repositories).
    let from = std::env::var("KOVA_PACKAGES_FROM").unwrap_or_else(|_| "https://".into());
    let url = format!("{from}{address}");
    let parent = dir.parent().expect("an address has parts");
    let _ = std::fs::create_dir_all(parent);
    let temp = parent.join(format!(".{version}-{}", std::process::id()));
    let mut said = String::new();
    for tag in [format!("v{version}"), version.to_string()] {
        let _ = std::fs::remove_dir_all(&temp);
        let mut git = Command::new("git");
        // The same bytes on every system (no CRLF on Windows), never a question in the terminal.
        git.args(["-c", "core.autocrlf=false", "clone", "--quiet", "--depth", "1", "--branch", &tag, "--"]).arg(&url).arg(&temp);
        git.env("GIT_TERMINAL_PROMPT", "0").env("GCM_INTERACTIVE", "never").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        match run(git, 300) {
            Some((true, _)) => {
                let commit = head(&temp).unwrap_or_else(|| "-".into());
                if std::fs::rename(&temp, dir).is_err() {
                    // Downloaded meanwhile by another program: theirs is checked like any other.
                    let _ = std::fs::remove_dir_all(&temp);
                }
                return Ok(commit);
            }
            Some((false, err)) => said = err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string(),
            None => said = t!("package_too_long"),
        }
    }
    let _ = std::fs::remove_dir_all(&temp);
    Err(t!("package_download_failed", address = address, version = version, reason = said))
}

/// Runs a command for at most `seconds`: whether it succeeded, and its error output; `None` if
/// it took too long (it is then stopped).
fn run(mut command: Command, seconds: u64) -> Option<(bool, String)> {
    let mut child = command.spawn().ok()?;
    let start = std::time::Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                let _ = std::io::Read::read_to_string(&mut e, &mut err);
            }
            return Some((status.success(), err));
        }
        if start.elapsed().as_secs() >= seconds {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// The commit a clone is at.
fn head(dir: &Path) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(["rev-parse", "HEAD"]).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// SHA-256 of a package's files (`.git` left out): each path (with `/`) then its bytes, in the
/// order of the paths, so every system gets the same.
pub fn fingerprint(dir: &Path) -> String {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            match e.file_type() {
                Ok(t) if t.is_dir() && name != ".git" => walk(&e.path(), &path, out),
                Ok(t) if t.is_file() => out.push((path, std::fs::read(e.path()).unwrap_or_default())),
                _ => {}
            }
        }
    }
    let mut files = vec![];
    walk(dir, "", &mut files);
    files.sort();
    let mut all = vec![];
    for (path, bytes) in files {
        all.extend(path.as_bytes());
        all.push(0);
        all.extend((bytes.len() as u64).to_le_bytes());
        all.extend(bytes);
    }
    crate::shared::hex(&crate::crypto::sha256(&all))
}

type Entry = (String, String, String, String);

/// `kova.lock`: one line per package, `address version commit sha256`.
fn read_lock(path: &Path) -> Vec<Entry> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    (text.lines())
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| match l.split_whitespace().collect::<Vec<_>>()[..] {
            [a, v, c, s] => Some((a.into(), v.into(), c.into(), s.trim_start_matches("sha256:").into())),
            _ => None,
        })
        .collect()
}

fn write_lock(path: &Path, entry: Entry) {
    let mut all = read_lock(path);
    all.retain(|e| !(e.0 == entry.0 && e.1 == entry.1));
    all.push(entry);
    all.sort();
    let mut text = String::from("# Written by Kova: the packages this program uses, as they were the first time.\n");
    for (a, v, c, s) in all {
        text += &format!("{a} {v} {c} sha256:{s}\n");
    }
    let _ = std::fs::write(path, text);
}

#[cfg(test)]
mod tests {
    #[test]
    fn addresses() {
        assert!(super::address_fits("github.com/ada/kova-json"));
        assert!(super::address_fits("codeberg.org/ada/tools/json"));
        for bad in ["--upload-pack=x/y/z", "github.com/ada", "github.com/../x", "github.com/ada/-x", "file:///tmp/x", "github.com/a b/c", "localhost/a/b"] {
            assert!(!super::address_fits(bad), "{bad}");
        }
        assert!(super::version_fits("1.2.0") && super::version_fits("2024-10-01"));
        assert!(!super::version_fits("--branch") && !super::version_fits("1..2") && !super::version_fits(""));
    }
}
