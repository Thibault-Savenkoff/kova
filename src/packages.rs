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

fn git_present() -> bool {
    Command::new("git").arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok()
}

/// Clones `url` (at `tag`, else its main branch) into `into`; what went wrong if not.
fn clone(url: &str, into: &Path, tag: Option<&str>, seconds: u64) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(into);
    let mut git = Command::new("git");
    // The same bytes on every system (no CRLF on Windows), never a question in the terminal.
    git.args(["-c", "core.autocrlf=false", "clone", "--quiet", "--depth", "1"]);
    if let Some(tag) = tag {
        git.args(["--branch", tag]);
    }
    git.arg("--").arg(url).arg(into);
    match run(quiet(git), seconds) {
        Some((true, _)) => Ok(()),
        Some((false, err)) => Err(err.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()),
        None => Err(t!("package_too_long")),
    }
}

fn quiet(mut git: Command) -> Command {
    git.env("GIT_TERMINAL_PROMPT", "0").env("GCM_INTERACTIVE", "never").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    git
}

/// Clones the tag `v{version}` (else `{version}`) into `dir`, through a folder of this process
/// (two programs may download at once); the commit.
fn fetch(address: &str, version: &str, dir: &Path) -> Result<String, String> {
    if !git_present() {
        return Err(t!("package_git_missing", address = address));
    }
    let url = format!("{}{address}", from());
    let parent = dir.parent().expect("an address has parts");
    let _ = std::fs::create_dir_all(parent);
    let temp = parent.join(format!(".{version}-{}", std::process::id()));
    let mut said = String::new();
    for tag in [format!("v{version}"), version.to_string()] {
        match clone(&url, &temp, Some(&tag), 300) {
            Ok(()) => {
                let commit = head(&temp).unwrap_or_else(|| "-".into());
                if std::fs::rename(&temp, dir).is_err() {
                    // Downloaded meanwhile by another program: theirs is checked like any other.
                    let _ = std::fs::remove_dir_all(&temp);
                }
                return Ok(commit);
            }
            Err(e) => said = e,
        }
    }
    let _ = std::fs::remove_dir_all(&temp);
    Err(t!("package_download_failed", address = address, version = version, reason = said))
}

/// `KOVA_PACKAGES_FROM`: another place to clone from (the tests use local repositories).
fn from() -> String {
    std::env::var("KOVA_PACKAGES_FROM").unwrap_or_else(|_| "https://".into())
}

/// Runs a command for at most `seconds`: whether it succeeded, and what it printed (its error
/// output if it failed); `None` if it took too long (it is then stopped).
fn run(mut command: Command, seconds: u64) -> Option<(bool, String)> {
    let mut child = command.spawn().ok()?;
    // Read while it runs: a full pipe would stop it.
    let read = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_string(&mut text);
            }
            text
        })
    };
    let out = read(child.stdout.take().map(|p| Box::new(p) as _));
    let err = read(child.stderr.take().map(|p| Box::new(p) as _));
    let start = std::time::Instant::now();
    loop {
        if let Ok(Some(status)) = child.try_wait() {
            let said = if status.success() { out } else { err };
            return Some((status.success(), said.join().unwrap_or_default()));
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

/// The package index (PLAN Part 4 "Package index"): a public git repository whose `index.txt`
/// holds one line per package, `name address description`. It only helps to find a package:
/// `import` never reads it, so what a program runs still depends on its address and `kova.lock`.
const INDEX: &str = "github.com/Thibault-Savenkoff/kova-packages";
/// At most this many packages asked for their versions: each is one trip over the network.
const SHOWN: usize = 20;

/// `kova search word…`: the packages of the index holding every word; what stopped it if not.
pub fn search(words: &[&str]) -> Result<(), String> {
    if !git_present() {
        return Err(t!("index_git_missing"));
    }
    let url = std::env::var("KOVA_INDEX").unwrap_or_else(|_| format!("{}{INDEX}", from()));
    let dir = cache().join("index");
    let _ = std::fs::create_dir_all(cache());
    let temp = cache().join(format!(".index-{}", std::process::id()));
    match clone(&url, &temp, None, 30) {
        Ok(()) => {
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::rename(&temp, &dir);
        }
        // Offline: the last index still finds packages.
        Err(_) if dir.join("index.txt").is_file() => eprintln!("{}", t!("index_old")),
        Err(reason) => return Err(t!("index_unreachable", reason = reason)),
    }
    let _ = std::fs::remove_dir_all(&temp);
    let found = matching(&std::fs::read_to_string(dir.join("index.txt")).unwrap_or_default(), words);
    if found.is_empty() {
        println!("{}", if words.is_empty() { t!("index_empty") } else { t!("index_nothing", words = words.join(" ")) });
    }
    let shown = &found[..found.len().min(SHOWN)];
    let newest: Vec<Option<String>> = std::thread::scope(|s| {
        let asked: Vec<_> = shown.iter().map(|p| s.spawn(move || newest(&p.1))).collect();
        asked.into_iter().map(|t| t.join().unwrap_or(None)).collect()
    });
    for ((name, address, about), version) in shown.iter().zip(newest) {
        match version {
            Some(v) => println!("{name} {v}: {about}\n    import {name} from \"{address}\" version \"{v}\""),
            None => println!("{name}: {about}\n    {}", t!("index_no_version", address = address)),
        }
    }
    if found.len() > SHOWN {
        println!("{}", t!("index_more", count = found.len() - SHOWN));
    }
    Ok(())
}

/// The lines of `index.txt` holding every word, in any case. The file is written by strangers:
/// a line whose name or address is not plain is left out, and no control character is printed.
fn matching(index: &str, words: &[&str]) -> Vec<(String, String, String)> {
    let plain_name = |n: &str| !n.starts_with(|c: char| c.is_ascii_digit()) && !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    (index.lines())
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.trim().splitn(3, char::is_whitespace);
            let (name, address) = (parts.next()?, parts.next()?);
            let about: String = parts.next().unwrap_or("").trim().chars().filter(|c| !c.is_control()).collect();
            let text = l.to_lowercase();
            (plain_name(name) && address_fits(address) && words.iter().all(|w| text.contains(&w.to_lowercase())))
                .then(|| (name.to_string(), address.to_string(), about))
        })
        .collect()
}

/// The highest version a repository has tagged, read from its tags: the index never holds
/// versions, so it cannot be out of date. Only numbers count (`v1.2.0`, not `1.3.0-beta`).
fn newest(address: &str) -> Option<String> {
    let mut git = Command::new("git");
    git.args(["ls-remote", "--tags", "--refs", "--"]).arg(format!("{}{address}", from()));
    let (true, tags) = run(quiet(git), 20)? else { return None };
    tags.lines().filter_map(|l| release(l.rsplit("refs/tags/").next()?)).max().map(|(_, v)| v)
}

/// A tag's numbers, to compare them, and its version.
fn release(tag: &str) -> Option<(Vec<u64>, String)> {
    let version = tag.strip_prefix('v').unwrap_or(tag);
    let numbers: Option<Vec<u64>> = version.split('.').map(|n| n.parse().ok()).collect();
    Some((numbers?, version.to_string()))
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

    #[test]
    fn index_lines() {
        let index = "# comment\njson github.com/ada/json Reads JSON\n\nbad-name github.com/ada/x A\nx --upload-pack=y/z/w B\ncsv github.com/ada/csv Tables\x1b[2J\n";
        let names = |w: &[&str]| super::matching(index, w).into_iter().map(|p| p.0).collect::<Vec<_>>();
        assert_eq!(names(&[]), ["json", "csv"]);
        assert_eq!(names(&["READS", "ada"]), ["json"]);
        assert_eq!(names(&["reads", "tables"]), [] as [&str; 0]);
        assert_eq!(super::matching(index, &["csv"])[0].2, "Tables[2J");
        // 1.10 is after 1.9; a pre-release is no release.
        let best = ["v1.9.0", "1.10.0", "2.0.0-beta", "latest"].iter().filter_map(|t| super::release(t)).max().unwrap();
        assert_eq!(best.1, "1.10.0");
    }
}
