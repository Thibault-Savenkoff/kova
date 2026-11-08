//! An iOS app for Xcode's simulator (PLAN Part 4 "iOS"): a folder `Name.app` holding the
//! executable (a Mach-O file whose load commands say "iOS simulator", src/link.rs) and the
//! `Info.plist` the system reads before it starts the program. The signature is the ad-hoc one the
//! linker writes, which the simulator accepts; a real iPhone needs Apple's certificate.

use crate::t;
use std::path::Path;

/// `output` becomes the `.app` folder (an old one is removed first).
pub fn app(output: &str, stem: &str, title: &str, exe: &[u8], device: bool) -> Result<(), String> {
    let made = |e: std::io::Error| e.to_string();
    let dir = Path::new(output);
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(made)?;
    }
    std::fs::create_dir_all(dir).map_err(made)?;
    let path = dir.join(stem);
    std::fs::write(&path, exe).map_err(made)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(made)?;
    }
    std::fs::write(dir.join("Info.plist"), plist(stem, title, device)).map_err(made)?;
    if device {
        crate::sign::bundle(dir, stem, &identifier(stem))?;
    }
    Ok(())
}

/// The identifier of the app: what `simctl launch` is given.
pub fn identifier(stem: &str) -> String {
    let word: String = stem.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_lowercase();
    format!("dev.kova.{}", if word.is_empty() { "app" } else { &word })
}

fn plist(stem: &str, title: &str, device: bool) -> String {
    let platform = if device { "iPhoneOS" } else { "iPhoneSimulator" };
    let escaped = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\
    <key>CFBundleExecutable</key><string>{stem}</string>\n\
    <key>CFBundleIdentifier</key><string>{id}</string>\n\
    <key>CFBundleName</key><string>{name}</string>\n\
    <key>CFBundlePackageType</key><string>APPL</string>\n\
    <key>CFBundleVersion</key><string>1</string>\n\
    <key>CFBundleShortVersionString</key><string>1.0</string>\n\
    <key>CFBundleSupportedPlatforms</key><array><string>{platform}</string></array>\n\
    <key>MinimumOSVersion</key><string>14.0</string>\n\
    <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>\n\
    <key>UILaunchScreen</key><dict/>\n\
    <key>UISupportedInterfaceOrientations</key><array><string>UIInterfaceOrientationPortrait</string><string>UIInterfaceOrientationLandscapeLeft</string><string>UIInterfaceOrientationLandscapeRight</string></array>\n\
</dict>\n</plist>\n",
        id = identifier(stem),
        name = escaped(title),
    )
}

/// `--run`: puts the app on the connected iPhone (`device`) or in the booted simulator, starts it
/// and shows what it prints until it ends. Needs Xcode's tools, so a Mac.
pub fn run(app: &str, id: &str, device: bool) -> Result<i32, String> {
    use std::process::Command;
    let failed = |what: &str, detail: String| t!("ios_failed", what = what, detail = detail);
    let check = |c: &mut Command, what: &str| -> Result<(), String> {
        let out = c.output().map_err(|e| failed(what, e.to_string()))?;
        if out.status.success() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
        Err(failed(what, text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string()))
    };
    let status = if device {
        let list = std::env::temp_dir().join(format!("kova-devices-{}.json", std::process::id()));
        check(Command::new("xcrun").args(["devicectl", "list", "devices", "--json-output"]).arg(&list), "devicectl")?;
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&list).map_err(|e| failed("devicectl", e.to_string()))?).map_err(|e| failed("devicectl", e.to_string()))?;
        let _ = std::fs::remove_file(&list);
        let phone = json["result"]["devices"]
            .as_array()
            .and_then(|all| all.iter().find(|d| d["hardwareProperties"]["reality"] == "physical" && d["connectionProperties"]["tunnelState"] == "connected"))
            .and_then(|d| d["identifier"].as_str().map(str::to_string))
            .ok_or_else(|| t!("ios_no_device"))?;
        check(Command::new("xcrun").args(["devicectl", "device", "install", "app", "--device", &phone, app]), "install")?;
        Command::new("xcrun").args(["devicectl", "device", "process", "launch", "--console", "--device", &phone, id]).status()
    } else {
        check(Command::new("xcrun").args(["simctl", "install", "booted", app]), "install")?;
        Command::new("xcrun").args(["simctl", "launch", "--console-pty", "booted", id]).status()
    };
    status.map(|s| s.code().unwrap_or(1)).map_err(|e| failed("launch", e.to_string()))
}
