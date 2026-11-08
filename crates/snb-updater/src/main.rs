#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Args {
    installer: PathBuf,
    app_exe: PathBuf,
    app_dir: PathBuf,
    data_dir: PathBuf,
    backup_dir: PathBuf,
    run_id: String,
    expected_version: String,
    log_path: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut values = std::env::args().skip(1);
    let mut map = std::collections::HashMap::new();
    while let Some(flag) = values.next() {
        let key = flag.trim_start_matches("--").to_string();
        let Some(value) = values.next() else {
            return Err(format!("missing value for {flag}"));
        };
        map.insert(key, value);
    }
    let required = |key: &str| {
        map.get(key)
            .map(PathBuf::from)
            .ok_or_else(|| format!("missing --{key}"))
    };
    Ok(Args {
        installer: required("installer")?,
        app_exe: required("app-exe")?,
        app_dir: required("app-dir")?,
        data_dir: required("data-dir")?,
        backup_dir: required("backup-dir")?,
        run_id: map.get("run-id").cloned().ok_or("missing --run-id")?,
        expected_version: map
            .get("expected-version")
            .cloned()
            .ok_or("missing --expected-version")?,
        log_path: required("log")?,
    })
}

fn log(path: &Path, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        use std::io::Write;
        let _ = writeln!(file, "[{stamp}] {message}");
    }
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let source = entry.path();
        let target = dst.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target)?;
        } else if source.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

fn create_backup(args: &Args) -> Result<PathBuf, String> {
    let backup = args.backup_dir.join(format!(
        "app-{}-{}",
        args.expected_version,
        now_secs()
    ));
    if backup.exists() {
        let _ = fs::remove_dir_all(&backup);
    }
    copy_tree(&args.app_dir, &backup)
        .map_err(|error| format!("failed to back up current launcher: {error}"))?;
    log(
        &args.log_path,
        &format!("previous launcher backed up to {}", backup.display()),
    );
    Ok(backup)
}

fn run_installer(args: &Args) -> Result<(), String> {
    let mut last_error = String::new();
    for attempt in 0..8 {
        let mut command = Command::new(&args.installer);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
            command.arg("/S");
        }
        #[cfg(not(windows))]
        {
            command.arg("--silent");
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        match command.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => last_error = format!("installer exited with {status}"),
            Err(error) => last_error = format!("installer could not start: {error}"),
        }
        thread::sleep(Duration::from_millis(500 * (attempt + 1)));
    }
    Err(last_error)
}

fn health_ok(data_dir: &Path, run_id: &str, expected_version: &str) -> bool {
    let path = data_dir.join("update-health.json");
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    text.contains(run_id) && text.contains(expected_version)
}

fn launch_app(args: &Args) -> Result<(), String> {
    let _ = fs::remove_file(args.data_dir.join("update-health.json"));
    Command::new(&args.app_exe)
        .arg(format!("--snb-update-run={}", args.run_id))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("new launcher could not start: {error}"))
}

fn restore_backup(backup: &Path, app_dir: &Path) -> Result<(), String> {
    for entry in fs::read_dir(app_dir)
        .map_err(|error| format!("cannot read app dir: {error}"))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path == app_dir.join("snb-updater.exe") || path == app_dir.join("snb-updater") {
            continue;
        }
        if path.is_dir() {
            fs::remove_dir_all(&path).map_err(|error| error.to_string())?;
        } else {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
        }
    }
    copy_tree(backup, app_dir).map_err(|error| format!("rollback copy failed: {error}"))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn relocate_out_of_install_dir() {
    if std::env::var("SNB_UPDATER_RELOCATED").is_ok() {
        return;
    }
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let temp_dir = std::env::temp_dir().join(format!(
        "snb-updater-{}-{}",
        std::process::id(),
        now_secs()
    ));
    let name = if cfg!(windows) { "snb-updater.exe" } else { "snb-updater" };
    let relocated = temp_dir.join(name);
    if fs::create_dir_all(&temp_dir).is_err() || fs::copy(&current, &relocated).is_err() {
        return;
    }
    let child = Command::new(&relocated)
        .args(std::env::args().skip(1))
        .env("SNB_UPDATER_RELOCATED", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if child.is_ok() {
        std::process::exit(0);
    }
}

fn main() {
    relocate_out_of_install_dir();
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("snb-updater: {error}");
            std::process::exit(2);
        }
    };
    log(&args.log_path, "updater helper started");
    thread::sleep(Duration::from_secs(2));
    let backup = match create_backup(&args) {
        Ok(backup) => backup,
        Err(error) => {
            log(&args.log_path, &error);
            std::process::exit(3);
        }
    };
    if let Err(error) = run_installer(&args) {
        log(&args.log_path, &format!("install failed: {error}; rolling back"));
        let _ = restore_backup(&backup, &args.app_dir);
        let _ = Command::new(&args.app_exe).spawn();
        std::process::exit(4);
    }
    if let Err(error) = launch_app(&args) {
        log(&args.log_path, &error);
        let _ = restore_backup(&backup, &args.app_dir);
        let _ = Command::new(&args.app_exe).spawn();
        std::process::exit(5);
    }
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(120) {
        if health_ok(&args.data_dir, &args.run_id, &args.expected_version) {
            log(
                &args.log_path,
                &format!("new launcher {} passed its health check", args.expected_version),
            );
            // The installer is no longer needed. Keep the previous-version
            // backup for emergency owner rollback.
            let _ = fs::remove_file(&args.installer);
            std::process::exit(0);
        }
        thread::sleep(Duration::from_millis(500));
    }
    log(
        &args.log_path,
        "new launcher did not report UI health in time; rolling back",
    );
    let _ = restore_backup(&backup, &args.app_dir);
    let _ = Command::new(&args.app_exe).spawn();
    std::process::exit(6);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_a_directory_tree() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/b/file.txt"), b"hello").unwrap();
        copy_tree(&dir.path().join("a"), &dir.path().join("copy")).unwrap();
        assert_eq!(fs::read(dir.path().join("copy/b/file.txt")).unwrap(), b"hello");
    }

    #[test]
    fn validates_health_marker() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("update-health.json"),
            br#"{"runId":"abc","version":"0.2.0"}"#,
        )
        .unwrap();
        assert!(health_ok(dir.path(), "abc", "0.2.0"));
        assert!(!health_ok(dir.path(), "different", "0.2.0"));
        assert!(!health_ok(dir.path(), "abc", "9.9.9"));
    }
}
