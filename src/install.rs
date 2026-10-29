// `kova install counter.kova`: the program built into the user's own programs, where the system
// looks for them, so it shows in the applications menu with its icon (GNOME and wlroots desktops
// only take an icon from there). `kova uninstall` removes what `install` wrote. Only for this
// user: nothing needs administrator rights.
//
// - Linux: `~/.local/bin/<name>`; a program with windows also gets `~/.local/share/applications/
//   <name>.desktop` (its file name = the window's app id, how GNOME finds it) and its icon.
// - Mac: a program with windows becomes `~/Applications/<name>.app` (the Dock and the Finder show
//   its icon); the others go to `~/.local/bin`.
// - Windows: `%LOCALAPPDATA%\Programs\<name>\<name>.exe`; with windows, a shortcut in the Start menu.

use std::path::{Path, PathBuf};

use crate::codegen::Target;
use crate::kir::Program;

/// What `install` writes for the program `name`, made for this machine: (the executable, the
/// other files and folders).
fn places(name: &str, windowed: bool) -> Option<(PathBuf, Vec<PathBuf>)> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        let exe = env("LOCALAPPDATA")?.join("Programs").join(name).join(format!("{name}.exe"));
        let menu = env("APPDATA")?.join("Microsoft/Windows/Start Menu/Programs").join(format!("{name}.lnk"));
        return Some((exe, if windowed { vec![menu] } else { vec![] }));
    }
    let home = env("HOME")?;
    if cfg!(target_os = "macos") && windowed {
        let app = home.join("Applications").join(format!("{name}.app"));
        return Some((app.join("Contents/MacOS").join(name), vec![app]));
    }
    let exe = home.join(".local/bin").join(name);
    if !windowed || cfg!(target_os = "macos") {
        return Some((exe, vec![]));
    }
    let share = env("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
    Some((exe, vec![share.join("applications").join(format!("{name}.desktop")), share.join("icons/hicolor/512x512/apps").join(format!("{name}.png"))]))
}

/// Builds and installs; the places written.
pub fn install(p: &Program, files: &[String], lang: Option<usize>, name: &str, windowed: bool) -> Result<Vec<PathBuf>, String> {
    let (exe, others) = places(name, windowed).ok_or_else(|| crate::locale::t!("install_no_home"))?;
    let made = |e: std::io::Error| e.to_string();
    std::fs::create_dir_all(exe.parent().unwrap()).map_err(made)?;
    crate::codegen::build(p, files, &exe.to_string_lossy(), lang, Target::host())?;
    let icon = p.icon.as_ref();
    let mut written = vec![exe.clone()];
    for other in &others {
        std::fs::create_dir_all(other.parent().unwrap()).map_err(made)?;
        match other.extension().and_then(|e| e.to_str()) {
            Some("desktop") => {
                let icon_line = icon.map_or(String::new(), |_| format!("Icon={}\n", others[1].display()));
                let entry = format!("[Desktop Entry]\nType=Application\nName={name}\nExec=\"{}\"\n{icon_line}StartupWMClass={name}\nTerminal=false\n", exe.display());
                std::fs::write(other, entry).map_err(made)?;
            }
            Some("png") => match icon {
                Some(i) => std::fs::write(other, crate::icon::sized(&i.png, 512).unwrap_or_else(|| i.png.clone())).map_err(made)?,
                None => continue,
            },
            Some("app") => {
                let contents = other.join("Contents");
                let icon_key = icon.map_or(String::new(), |_| "    <key>CFBundleIconFile</key><string>icon.icns</string>\n".into());
                let id: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
                let plist = format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n    <key>CFBundleExecutable</key><string>{}</string>\n    <key>CFBundleName</key><string>{}</string>\n    <key>CFBundleIdentifier</key><string>org.kova.{id}</string>\n    <key>CFBundlePackageType</key><string>APPL</string>\n{icon_key}    <key>NSHighResolutionCapable</key><true/>\n</dict>\n</plist>\n",
                    xml(name),
                    xml(name)
                );
                std::fs::write(contents.join("Info.plist"), plist).map_err(made)?;
                if let Some(i) = icon {
                    std::fs::create_dir_all(contents.join("Resources")).map_err(made)?;
                    std::fs::write(contents.join("Resources/icon.icns"), crate::icon::icns_of(i)).map_err(made)?;
                }
            }
            Some("lnk") => std::fs::write(other, shortcut(&exe)).map_err(made)?,
            _ => {}
        }
        written.push(other.clone());
    }
    Ok(written)
}

/// Removes what `install` wrote; the places removed (none: it was not installed).
pub fn uninstall(name: &str, windowed: bool) -> Result<Vec<PathBuf>, String> {
    let (exe, others) = places(name, windowed).ok_or_else(|| crate::locale::t!("install_no_home"))?;
    let mut removed = vec![];
    // Windows: the program's own folder; a Mac app: its bundle (in `others`).
    let exe_place = if cfg!(windows) { exe.parent().unwrap().to_path_buf() } else { exe };
    for place in std::iter::once(exe_place).chain(others) {
        let gone = if place.is_dir() { std::fs::remove_dir_all(&place) } else { std::fs::remove_file(&place) };
        match gone {
            Ok(()) => removed.push(place),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(removed)
}

/// Whether `~/.local/bin` (where `exe` is) is in `PATH`, so the program can be typed by its name.
pub fn reachable(exe: &Path) -> bool {
    let folder = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d == folder))
}

fn xml(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A Windows shortcut (`.lnk`, Microsoft's "Shell Link" format) to `exe`: a header, then where
/// the file is (its volume, then its path, in ANSI and in UTF-16), then the folder to start in.
fn shortcut(exe: &Path) -> Vec<u8> {
    let path = exe.to_string_lossy().into_owned();
    let folder = exe.parent().map_or(String::new(), |d| d.to_string_lossy().into_owned());
    let wide = |t: &str| t.encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
    let ansi: Vec<u8> = path.chars().map(|c| if c.is_ascii() { c as u8 } else { b'?' }).collect();
    let mut l = vec![];
    l.extend(0x4Cu32.to_le_bytes());
    l.extend([0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    l.extend(0x92u32.to_le_bytes()); // a link info, a working folder, texts in UTF-16
    l.extend(0x20u32.to_le_bytes()); // a file
    l.extend([0; 24 + 4 + 4]); // times, size, icon number
    l.extend(1u32.to_le_bytes()); // a normal window
    l.extend([0; 12]);
    // The link info: its header (0x24 bytes, with the UTF-16 path's place), the volume, the paths.
    let volume = [&17u32.to_le_bytes()[..], &3u32.to_le_bytes(), &0u32.to_le_bytes(), &16u32.to_le_bytes(), &[0]].concat();
    let base = 0x24 + volume.len();
    let suffix = base + ansi.len() + 1;
    let base_wide = suffix + 1;
    let suffix_wide = base_wide + 2 * path.encode_utf16().count() + 2;
    let size = suffix_wide + 2;
    for v in [size, 0x24, 1, 0x24, base, 0, suffix, base_wide, suffix_wide] {
        l.extend((v as u32).to_le_bytes());
    }
    l.extend(volume);
    l.extend(&ansi);
    l.extend([0, 0]);
    l.extend(wide(&path));
    l.extend([0, 0, 0, 0]);
    l.extend((folder.encode_utf16().count() as u16).to_le_bytes());
    l.extend(wide(&folder));
    l.extend([0; 4]); // the end
    l
}

#[cfg(test)]
mod tests {
    /// The shortcut's parts are where its header says.
    #[test]
    fn shortcut() {
        let l = super::shortcut(std::path::Path::new("C:/Users/Ada/Programs/Tally/Tally.exe"));
        let le32 = |at: usize| u32::from_le_bytes(l[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!((le32(0), le32(20)), (0x4C, 0x92));
        let info = 0x4C;
        let base = info + le32(info + 16);
        assert!(l[base..].starts_with(b"C:/Users/Ada"));
        if let Ok(out) = std::env::var("KOVA_LNK_OUT") {
            std::fs::write(out, &l).unwrap();
        }
    }
}
