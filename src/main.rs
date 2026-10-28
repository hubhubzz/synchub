use std::{
    env,
    ffi::OsString,
    path::PathBuf,
    process::{Command, ExitCode},
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("SyncHub: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(ExitCode::SUCCESS);
    }

    let dry_run = if args.first().is_some_and(|arg| arg == "--dry-run") {
        args.remove(0);
        true
    } else {
        false
    };

    if args.len() != 2 {
        print_help();
        return Err("expected a local folder and a configured rclone remote path".into());
    }

    let source = PathBuf::from(&args[0]);
    let source = source
        .canonicalize()
        .map_err(|e| format!("cannot access source folder: {e}"))?;
    if !source.is_dir() {
        return Err("source must be a folder".into());
    }

    let destination = &args[1];
    validate_remote(destination)?;

    println!("Copying folder contents to Google Drive via rclone.");
    println!("From: {}", source.display());
    println!("To:   {}", destination.to_string_lossy());
    if dry_run {
        println!("Dry run: no files will be uploaded.");
    } else {
        println!("Existing Drive files absent from the local folder will be kept.");
    }

    let mut command = Command::new("rclone");
    command.arg("copy").arg("--progress");
    if dry_run {
        command.arg("--dry-run");
    }
    let status = command
        .arg(source)
        .arg(destination)
        .status()
        .map_err(|e| format!("could not start rclone ({e}); install rclone and configure a Google Drive remote"))?;

    if status.success() {
        Ok(ExitCode::SUCCESS)
    } else {
        let code = status.code().unwrap_or(1).clamp(1, 255) as u8;
        Ok(ExitCode::from(code))
    }
}

fn validate_remote(destination: &OsString) -> Result<(), String> {
    let value = destination
        .to_str()
        .ok_or_else(|| "Drive remote path must be valid UTF-8".to_string())?;
    let (remote, _) = value
        .split_once(':')
        .ok_or_else(|| "destination must use rclone format remote:path, for example gdrive:Backups".to_string())?;
    if remote.is_empty() || remote.contains('/') || remote.chars().any(char::is_whitespace) {
        return Err("destination must start with a configured rclone remote name, for example gdrive:".into());
    }
    Ok(())
}

fn print_help() {
    println!("SyncHub — copy a local folder to Google Drive using rclone\n\nUsage:\n  synchub [--dry-run] <LOCAL_FOLDER> <REMOTE:path>\n\nExamples:\n  synchub --dry-run ~/Documents gdrive:Backups/Documents\n  synchub ~/Documents gdrive:Backups/Documents\n\nSyncHub copies new and changed files. It does not delete Drive files.\nRequires rclone installed and a Google Drive remote configured as described in README.md.");
}
