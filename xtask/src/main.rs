//! Repository chores, run as `cargo xtask <command>` (see `.cargo/config.toml`).
//!
//! The workspace `Cargo.toml` is the only place the version lives. These
//! commands keep it that way and prepare release assets:
//!
//!   version                       print the version
//!   check-version [<tag>]         fail unless every crate and Cargo.lock follow the workspace version (and the tag, if given, is v<version>)
//!   bump <x.y.z>                  set the version in Cargo.toml and Cargo.lock
//!   stage --target T --from DIR --to DIR [--repo owner/name]
//!                                 rename cargo-packager's output to release asset names and write the updater fragment
//!   screenshots [<name>]          render docs/screenshots from the scripted mock, offscreen (no window opens)
//!   manifest --notes FILE --out latest.json FRAGMENT...
//!                                 merge fragments into the updater manifest

use semver::Version;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use toml_edit::{DocumentMut, Item};

type Result<T> = std::result::Result<T, String>;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<()> {
    let root = workspace_root();
    let (command, rest) = args.split_first().ok_or_else(|| "no command; see the top of xtask/src/main.rs".to_string())?;
    match command.as_str() {
        "version" => {
            println!("{}", workspace_version(&root)?);
            Ok(())
        }
        "check-version" => {
            let version = check_version(&root)?;
            if let Some(tag) = rest.first() {
                let expected = format!("v{version}");
                if *tag != expected {
                    return Err(format!("the tag {tag} does not match the workspace version {version} (expected {expected})"));
                }
            }
            println!("{version}");
            Ok(())
        }
        "bump" => {
            let next = rest.first().ok_or("usage: cargo xtask bump <x.y.z>")?;
            let version = bump(&root, next)?;
            println!("Version set to {version}. Commit, then: git tag v{version} && git push origin v{version}");
            Ok(())
        }
        "stage" => stage_command(&root, rest),
        "screenshots" => screenshots(&root, rest),
        "manifest" => manifest_command(rest),
        other => Err(format!("unknown command {other:?}")),
    }
}

/// The example draws with GPUI's headless renderer, so nothing appears on screen or takes focus.
fn screenshots(root: &Path, rest: &[String]) -> Result<()> {
    let out = root.join("docs/screenshots");
    let status = std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root)
        .args(["run", "--release", "--locked", "-p", "hearth-app", "--example", "screenshots", "--"])
        .arg(&out)
        .args(rest.iter().take(1))
        .status()
        .map_err(|e| format!("cargo: {e}"))?;
    status.success().then_some(()).ok_or_else(|| "the screenshots example failed".to_string())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives one level below the root").to_path_buf()
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn parse_semver(text: &str) -> Result<Version> {
    Version::parse(text.trim_start_matches('v')).map_err(|e| format!("{text:?} is not a semantic version: {e}"))
}

// ---- Versions ----

fn workspace_manifest(root: &Path) -> Result<DocumentMut> {
    let path = root.join("Cargo.toml");
    read(&path)?.parse::<DocumentMut>().map_err(|e| format!("{}: {e}", path.display()))
}

fn workspace_version(root: &Path) -> Result<Version> {
    let doc = workspace_manifest(root)?;
    let text = doc["workspace"]["package"]["version"].as_str().ok_or("Cargo.toml has no [workspace.package] version")?;
    parse_semver(text)
}

/// (manifest path, package name) of every workspace member.
fn members(root: &Path) -> Result<Vec<(PathBuf, String)>> {
    let doc = workspace_manifest(root)?;
    let list = doc["workspace"]["members"].as_array().ok_or("Cargo.toml has no [workspace] members")?;
    let mut out = Vec::new();
    for member in list.iter().filter_map(|m| m.as_str()) {
        let path = root.join(member).join("Cargo.toml");
        let manifest = read(&path)?.parse::<DocumentMut>().map_err(|e| format!("{}: {e}", path.display()))?;
        let name = manifest["package"]["name"].as_str().ok_or_else(|| format!("{} has no package name", path.display()))?.to_string();
        out.push((path, name));
    }
    Ok(out)
}

fn check_version(root: &Path) -> Result<Version> {
    let version = workspace_version(root)?;
    let mut problems = Vec::new();
    let members = members(root)?;

    for (path, name) in &members {
        let manifest: DocumentMut = read(path)?.parse().map_err(|e| format!("{}: {e}", path.display()))?;
        let inherits = manifest["package"]["version"].get("workspace").and_then(|v| v.as_bool()) == Some(true);
        if !inherits {
            problems.push(format!("{name} sets its own version; use `version.workspace = true`"));
        }
    }

    let lock_path = root.join("Cargo.lock");
    if lock_path.exists() {
        let lock: DocumentMut = read(&lock_path)?.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
        for package in lock["package"].as_array_of_tables().into_iter().flatten() {
            let name = package.get("name").and_then(|n| n.as_str()).unwrap_or_default();
            if package.get("source").is_none() && members.iter().any(|(_, m)| m == name) {
                let locked = package.get("version").and_then(|v| v.as_str()).unwrap_or_default();
                if locked != version.to_string() {
                    problems.push(format!("Cargo.lock has {name} {locked}, expected {version}; run `cargo xtask bump {version}`"));
                }
            }
        }
    }

    if problems.is_empty() { Ok(version) } else { Err(problems.join("\n")) }
}

/// Replaces a string value but keeps the comment after it.
fn set_string(item: &mut Item, text: &str) {
    let mut new = toml_edit::Value::from(text);
    if let Some(old) = item.as_value() {
        *new.decor_mut() = old.decor().clone();
    }
    *item = Item::Value(new);
}

fn bump(root: &Path, next: &str) -> Result<Version> {
    let version = parse_semver(next)?;

    let path = root.join("Cargo.toml");
    let mut doc = workspace_manifest(root)?;
    set_string(&mut doc["workspace"]["package"]["version"], &version.to_string());
    fs::write(&path, doc.to_string()).map_err(|e| format!("{}: {e}", path.display()))?;

    // Keep the lockfile in step so `--locked` builds do not fail on a stale version.
    let lock_path = root.join("Cargo.lock");
    if lock_path.exists() {
        let names: Vec<String> = members(root)?.into_iter().map(|(_, name)| name).collect();
        let mut lock: DocumentMut = read(&lock_path)?.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
        if let Some(packages) = lock["package"].as_array_of_tables_mut() {
            for package in packages.iter_mut() {
                let ours = package.get("source").is_none() && package.get("name").and_then(|n| n.as_str()).is_some_and(|n| names.iter().any(|m| m == n));
                if ours {
                    set_string(&mut package["version"], &version.to_string());
                }
            }
        }
        fs::write(&lock_path, lock.to_string()).map_err(|e| format!("Cargo.lock: {e}"))?;
    }
    Ok(version)
}

// ---- Release assets ----

/// How an installer cargo-packager produced is published.
#[derive(Debug, PartialEq, Eq)]
struct AssetKind {
    /// Appended to `hearth-<version>-<target>`.
    suffix: &'static str,
    /// The `format` the updater uses for it; `None` if it is only a download.
    updater: Option<&'static str>,
}

const KINDS: [(&str, AssetKind); 5] = [
    (".dmg", AssetKind { suffix: ".dmg", updater: None }),
    (".app.tar.gz", AssetKind { suffix: ".app.tar.gz", updater: Some("app") }),
    ("-setup.exe", AssetKind { suffix: "-setup.exe", updater: Some("nsis") }),
    (".AppImage", AssetKind { suffix: ".AppImage", updater: Some("appimage") }),
    (".deb", AssetKind { suffix: ".deb", updater: None }),
];

fn classify(file_name: &str) -> Option<&'static AssetKind> {
    KINDS.iter().find(|(ending, _)| file_name.ends_with(ending)).map(|(_, kind)| kind)
}

fn asset_name(version: &Version, target: &str, kind: &AssetKind) -> String {
    format!("hearth-{version}-{target}{}", kind.suffix)
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn stage_command(root: &Path, args: &[String]) -> Result<()> {
    let target = flag(args, "--target").ok_or("--target is required, e.g. macos-aarch64")?;
    let from = PathBuf::from(flag(args, "--from").ok_or("--from is required")?);
    let to = PathBuf::from(flag(args, "--to").ok_or("--to is required")?);
    let version = workspace_version(root)?;
    let repo = match flag(args, "--repo") {
        Some(repo) => repo.to_string(),
        None => repository_slug(&workspace_manifest(root)?)?,
    };
    let fragment = stage(&from, &to, target, &version, &repo)?;
    match fragment {
        Some(path) => println!("staged; updater fragment at {}", path.display()),
        None => println!("staged; no signed updater asset, so no updater fragment"),
    }
    Ok(())
}

/// `https://github.com/owner/name` to `owner/name`.
fn repository_slug(doc: &DocumentMut) -> Result<String> {
    let url = doc["workspace"]["package"]["repository"].as_str().ok_or("Cargo.toml has no [workspace.package] repository")?;
    url.trim_end_matches('/').strip_prefix("https://github.com/").map(str::to_string).ok_or_else(|| format!("{url} is not a GitHub repository"))
}

/// Copies the installers in `from` to `to` under the release names, with their
/// signatures, and returns the path of the updater fragment if one could be written.
fn stage(from: &Path, to: &Path, target: &str, version: &Version, repo: &str) -> Result<Option<PathBuf>> {
    fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let mut fragment: Option<(String, Value)> = None;

    let mut files: Vec<_> = fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.filter_map(|e| e.ok()).collect();
    files.sort_by_key(|e| e.file_name());
    for entry in files {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(kind) = classify(&name) else {
            continue;
        };
        if !path.is_file() {
            continue;
        }
        let asset = asset_name(version, target, kind);
        fs::copy(&path, to.join(&asset)).map_err(|e| format!("{}: {e}", path.display()))?;

        let Some(format) = kind.updater else { continue };
        let sig = PathBuf::from(format!("{}.sig", path.display()));
        if !sig.is_file() {
            eprintln!("warning: {name} has no signature, so it will not be offered as an update");
            continue;
        }
        let signature = read(&sig)?.trim().to_string();
        fs::write(to.join(format!("{asset}.sig")), &signature).map_err(|e| e.to_string())?;
        let url = format!("https://github.com/{repo}/releases/download/v{version}/{asset}");
        fragment = Some((updater_key(target), json!({ "url": url, "signature": signature, "format": format })));
    }

    let Some((key, entry)) = fragment else {
        return Ok(None);
    };
    let path = to.join(format!("update-{target}.json"));
    let body = json!({ key: entry });
    fs::write(&path, serde_json::to_string_pretty(&body).unwrap()).map_err(|e| e.to_string())?;
    Ok(Some(path))
}

/// The updater looks platforms up as `<os>-<arch>`; targets here are already named that way.
fn updater_key(target: &str) -> String {
    target.to_string()
}

fn manifest_command(args: &[String]) -> Result<()> {
    let notes = flag(args, "--notes").map(|p| read(Path::new(p))).transpose()?;
    let out = flag(args, "--out").ok_or("--out is required")?;
    let version = match flag(args, "--version") {
        Some(v) => parse_semver(v)?,
        None => workspace_version(&workspace_root())?,
    };
    // Everything after the flags (and their values) is a fragment file.
    let mut fragments = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
        } else if arg.starts_with("--") {
            skip = true;
        } else {
            fragments.push(PathBuf::from(arg));
        }
    }
    if fragments.is_empty() {
        return Err("no update fragments given".into());
    }
    let manifest = merge_manifest(&version, notes.as_deref(), &fragments)?;
    fs::write(out, serde_json::to_string_pretty(&manifest).unwrap()).map_err(|e| format!("{out}: {e}"))?;
    Ok(())
}

fn merge_manifest(version: &Version, notes: Option<&str>, fragments: &[PathBuf]) -> Result<Value> {
    let mut platforms: BTreeMap<String, Value> = BTreeMap::new();
    for path in fragments {
        let fragment: Map<String, Value> = serde_json::from_str(&read(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
        platforms.extend(fragment);
    }
    let mut manifest = json!({ "version": version.to_string(), "platforms": platforms });
    if let Some(notes) = notes.map(str::trim).filter(|n| !n.is_empty()) {
        manifest["notes"] = json!(notes);
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hearth-xtask-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A miniature workspace: root manifest, two members and a lockfile.
    fn workspace(dir: &Path, version: &str, lock_version: &str, member_version: &str) {
        fs::write(
            dir.join("Cargo.toml"),
            format!("[workspace]\nmembers = [\"a\", \"b\"]\n\n[workspace.package]\nversion = \"{version}\" # keep\nedition = \"2021\"\n"),
        )
        .unwrap();
        for (name, version_line) in [("a", "version.workspace = true".to_string()), ("b", member_version.to_string())] {
            fs::create_dir_all(dir.join(name)).unwrap();
            fs::write(dir.join(name).join("Cargo.toml"), format!("[package]\nname = \"{name}\"\n{version_line}\n")).unwrap();
        }
        fs::write(
            dir.join("Cargo.lock"),
            format!(
                "version = 4\n\n[[package]]\nname = \"a\"\nversion = \"{lock_version}\"\n\n[[package]]\nname = \"b\"\nversion = \"{lock_version}\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn a_consistent_workspace_passes() {
        let dir = scratch("ok");
        workspace(&dir, "0.1.0", "0.1.0", "version.workspace = true");
        assert_eq!(check_version(&dir).unwrap(), Version::new(0, 1, 0));
    }

    #[test]
    fn a_member_with_its_own_version_or_a_stale_lock_fails() {
        let dir = scratch("bad");
        workspace(&dir, "0.2.0", "0.1.0", "version = \"0.1.0\"");
        let err = check_version(&dir).unwrap_err();
        assert!(err.contains("b sets its own version"), "{err}");
        assert!(err.contains("Cargo.lock has a 0.1.0, expected 0.2.0"), "{err}");
        assert!(!err.contains("serde"), "{err}");
    }

    #[test]
    fn bump_updates_the_manifest_and_lock_but_nothing_else() {
        let dir = scratch("bump");
        workspace(&dir, "0.1.0", "0.1.0", "version.workspace = true");
        assert_eq!(bump(&dir, "v0.2.0-rc.1").unwrap().to_string(), "0.2.0-rc.1");
        assert_eq!(check_version(&dir).unwrap().to_string(), "0.2.0-rc.1");
        let manifest = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
        assert!(manifest.contains("version = \"0.2.0-rc.1\" # keep"), "{manifest}");
        let lock = fs::read_to_string(dir.join("Cargo.lock")).unwrap();
        assert!(lock.contains("name = \"serde\"\nversion = \"1.0.0\""), "{lock}");
        assert!(bump(&dir, "not-a-version").is_err());
    }

    #[test]
    fn installers_get_release_names() {
        let v = Version::new(1, 2, 3);
        let name = |file: &str, target: &str| asset_name(&v, target, classify(file).unwrap());
        assert_eq!(name("Hearth_1.2.3_aarch64.dmg", "macos-aarch64"), "hearth-1.2.3-macos-aarch64.dmg");
        assert_eq!(name("Hearth.app.tar.gz", "macos-x86_64"), "hearth-1.2.3-macos-x86_64.app.tar.gz");
        assert_eq!(name("Hearth_1.2.3_x64-setup.exe", "windows-x86_64"), "hearth-1.2.3-windows-x86_64-setup.exe");
        assert_eq!(name("Hearth_1.2.3_x86_64.AppImage", "linux-x86_64"), "hearth-1.2.3-linux-x86_64.AppImage");
        assert_eq!(name("hearth_1.2.3_amd64.deb", "linux-x86_64"), "hearth-1.2.3-linux-x86_64.deb");
        assert!(classify("hearth").is_none());
        assert!(classify("Hearth_1.2.3_aarch64.dmg.sig").is_none());
        // The app bundle directory is skipped by the is_file check, not by name.
        assert!(classify("Hearth.app").is_none());
    }

    #[test]
    fn staging_renames_installers_and_writes_the_updater_fragment() {
        let (from, to) = (scratch("from"), scratch("to"));
        fs::write(from.join("Hearth_1.2.3_aarch64.dmg"), "dmg").unwrap();
        fs::write(from.join("Hearth.app.tar.gz"), "tar").unwrap();
        fs::write(from.join("Hearth.app.tar.gz.sig"), "c2lnbmF0dXJl\n").unwrap();
        fs::write(from.join("hearth"), "binary").unwrap();
        let fragment = stage(&from, &to, "macos-aarch64", &Version::new(1, 2, 3), "o/r").unwrap().unwrap();

        assert!(to.join("hearth-1.2.3-macos-aarch64.dmg").is_file());
        assert!(to.join("hearth-1.2.3-macos-aarch64.app.tar.gz.sig").is_file());
        assert!(!to.join("hearth").exists());
        let body: Value = serde_json::from_str(&fs::read_to_string(fragment).unwrap()).unwrap();
        assert_eq!(
            body,
            json!({ "macos-aarch64": {
                "url": "https://github.com/o/r/releases/download/v1.2.3/hearth-1.2.3-macos-aarch64.app.tar.gz",
                "signature": "c2lnbmF0dXJl",
                "format": "app",
            }})
        );
    }

    #[test]
    fn unsigned_builds_write_no_fragment() {
        let (from, to) = (scratch("from-unsigned"), scratch("to-unsigned"));
        fs::write(from.join("Hearth.app.tar.gz"), "tar").unwrap();
        assert_eq!(stage(&from, &to, "macos-aarch64", &Version::new(1, 2, 3), "o/r").unwrap(), None);
        assert!(to.join("hearth-1.2.3-macos-aarch64.app.tar.gz").is_file());
    }

    #[test]
    fn fragments_merge_into_one_manifest() {
        let dir = scratch("merge");
        fs::write(dir.join("a.json"), r#"{"macos-aarch64": {"url": "u1", "signature": "s1", "format": "app"}}"#).unwrap();
        fs::write(dir.join("b.json"), r#"{"linux-x86_64": {"url": "u2", "signature": "s2", "format": "appimage"}}"#).unwrap();
        let manifest = merge_manifest(&Version::new(1, 2, 3), Some("  - a fix \n"), &[dir.join("a.json"), dir.join("b.json")]).unwrap();
        assert_eq!(manifest["version"], "1.2.3");
        assert_eq!(manifest["notes"], "- a fix");
        assert_eq!(manifest["platforms"]["linux-x86_64"]["url"], "u2");
        assert_eq!(manifest["platforms"].as_object().unwrap().len(), 2);
        assert!(merge_manifest(&Version::new(1, 2, 3), None, &[dir.join("a.json")]).unwrap().get("notes").is_none());
    }

    #[test]
    fn the_real_workspace_is_consistent() {
        // The guard CI relies on: run it as a test too, so a stale Cargo.lock fails locally.
        check_version(&workspace_root()).unwrap();
    }

    #[test]
    fn repository_urls_become_slugs() {
        let doc: DocumentMut = "[workspace.package]\nrepository = \"https://github.com/madrazaldi/hearth\"\n".parse().unwrap();
        assert_eq!(repository_slug(&doc).unwrap(), "madrazaldi/hearth");
    }
}
