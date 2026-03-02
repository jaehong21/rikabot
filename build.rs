use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

fn main() {
    println!("cargo:rerun-if-changed=web/src");
    println!("cargo:rerun-if-changed=web/index.html");
    println!("cargo:rerun-if-changed=web/package.json");
    println!("cargo:rerun-if-changed=web/rsbuild.config.ts");
    println!("cargo:rerun-if-changed=web/tailwind.config.ts");
    println!("cargo:rerun-if-changed=web/postcss.config.mjs");

    if std::env::var_os("RIKA_SKIP_WEB_BUILD").is_some() {
        println!("cargo:warning=Skipping web build because RIKA_SKIP_WEB_BUILD is set");
        return;
    }

    if let Err(err) = ensure_web_dist() {
        panic!("failed to prepare web assets: {err}");
    }
}

fn ensure_web_dist() -> Result<(), String> {
    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR")
            .map_err(|e| format!("CARGO_MANIFEST_DIR is not set: {e}"))?,
    );
    let web_dir = manifest_dir.join("web");
    let dist_index = web_dir.join("dist").join("index.html");

    let needs_build = if dist_index.exists() {
        web_inputs_newer_than(&web_dir, &dist_index)?
    } else {
        true
    };

    if !needs_build {
        return Ok(());
    }

    let mut attempted: Vec<&'static str> = Vec::new();
    for (bin, args) in [
        ("bun", vec!["run", "build"]),
        ("npm", vec!["run", "build"]),
        ("pnpm", vec!["build"]),
        ("yarn", vec!["build"]),
    ] {
        if !command_exists(bin) {
            continue;
        }
        attempted.push(bin);

        let status = Command::new(bin)
            .args(args)
            .current_dir(&web_dir)
            .status()
            .map_err(|e| format!("failed to run `{bin}` web build: {e}"))?;

        if !status.success() {
            return Err(format!("`{bin}` web build exited with {status}"));
        }

        if dist_index.exists() {
            return Ok(());
        }

        return Err(format!(
            "`{bin}` reported success but {} was not generated",
            dist_index.display()
        ));
    }

    if dist_index.exists() {
        return Ok(());
    }

    let attempted_desc = if attempted.is_empty() {
        "none (bun/npm/pnpm/yarn not found)".to_string()
    } else {
        attempted.join(", ")
    };

    Err(format!(
        "missing {} and could not build web assets; attempted builders: {}",
        dist_index.display(),
        attempted_desc
    ))
}

fn command_exists(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn web_inputs_newer_than(web_dir: &Path, dist_index: &Path) -> Result<bool, String> {
    let dist_mtime = file_modified(dist_index)?;

    for rel in [
        "src",
        "index.html",
        "package.json",
        "rsbuild.config.ts",
        "tailwind.config.ts",
        "postcss.config.mjs",
        "tsconfig.json",
        "tsconfig.app.json",
    ] {
        let path = web_dir.join(rel);
        if !path.exists() {
            continue;
        }
        let latest = latest_modified(&path)?;
        if latest > dist_mtime {
            return Ok(true);
        }
    }

    Ok(false)
}

fn latest_modified(path: &Path) -> Result<SystemTime, String> {
    if path.is_file() {
        return file_modified(path);
    }

    let mut latest = SystemTime::UNIX_EPOCH;
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|e| format!("failed to read directory {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry
                .map_err(|e| format!("failed to read directory entry in {}: {e}", dir.display()))?;
            let entry_path = entry.path();
            if entry_path
                .file_name()
                .is_some_and(|name| name == OsStr::new("node_modules"))
            {
                continue;
            }

            if entry_path.is_dir() {
                stack.push(entry_path);
                continue;
            }

            let modified = file_modified(&entry_path)?;
            if modified > latest {
                latest = modified;
            }
        }
    }

    Ok(latest)
}

fn file_modified(path: &Path) -> Result<SystemTime, String> {
    let metadata =
        fs::metadata(path).map_err(|e| format!("failed to stat {}: {e}", path.display()))?;
    metadata
        .modified()
        .map_err(|e| format!("failed to get mtime for {}: {e}", path.display()))
}
