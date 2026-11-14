// SPDX-License-Identifier: Apache-2.0
//! Build info (shown in Settings -> About phone, Dev tools and the UI server's boot log): `NEOUI_BUILD` =
//! "neoui <commit>[+dirty] <UTC time>, e32-rs <commit>", the e32-rs commit being the one the Symbian side is pinned
//! to (symbian/Cargo.lock). Also tells dependent crates where neoui's shared Slint files are (`DEP_NEOUI_UI`): an
//! app in another repo (NeoSymbian/phone) imports `@neoui/theme.slint`, the fonts and the components from them.
// shortcut: re-run when HEAD, the index or the lockfile change, so "+dirty" and the time can lag edits that touch
// neither; a clean commit always gets the right stamp.
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> Option<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = dir.join("../..");
    let rev = git(dir, &["rev-parse", "--short", "HEAD"])
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let rev = match rev {
        Some(r)
            if git(dir, &["diff", "--quiet", "HEAD", "--"]).is_some_and(|o| o.status.success()) =>
        {
            r
        }
        Some(r) => r + "+dirty",
        None => "unknown".into(),
    };
    let time = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%MZ"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    let lock = root.join("symbian/Cargo.lock");
    // source = "git+https://github.com/NeoSymbian/e32-rs?rev=<full>#<full>"
    let e32 = std::fs::read_to_string(&lock)
        .ok()
        .and_then(|l| {
            l.lines()
                .find(|s| s.contains("NeoSymbian/e32-rs?rev="))
                .and_then(|s| s.split('#').nth(1))
                .map(|h| h.trim_end_matches('"').chars().take(7).collect::<String>())
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=NEOUI_BUILD=neoui {rev} {time}, e32-rs {e32}");
    let ui = root.join("ui");
    println!("cargo:ui={}", ui.canonicalize().unwrap_or(ui).display());
    for f in [
        ".git/HEAD",
        ".git/index",
        ".git/logs/HEAD",
        "symbian/Cargo.lock",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(f).display());
    }
}
