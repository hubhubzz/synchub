mod auth;
mod drive;

use std::{env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("SyncHub: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args_os().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help();
        return Ok(());
    }
    let dry_run = if args.first().is_some_and(|arg| arg == "--dry-run") {
        args.remove(0);
        true
    } else {
        false
    };
    if args.is_empty() || args.len() > 2 {
        print_help();
        return Err("provide a local folder and optionally a Google Drive folder name".into());
    }

    let local_root = PathBuf::from(&args[0])
        .canonicalize()
        .map_err(|e| format!("cannot access local folder: {e}"))?;
    if !local_root.is_dir() {
        return Err("the local path must be a folder".into());
    }
    let default_name = local_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Local Folder");
    let drive_folder = args
        .get(1)
        .map(|name| {
            name.to_str()
                .ok_or_else(|| "Google Drive folder name must be valid UTF-8".to_string())
        })
        .transpose()?
        .unwrap_or(default_name);
    if drive_folder.is_empty()
        || drive_folder == "."
        || drive_folder == ".."
        || drive_folder.contains('/')
    {
        return Err("Google Drive folder name must be a single non-empty folder name".into());
    }

    if dry_run {
        return drive::print_dry_run(&local_root);
    }

    let auth = auth::Auth::from_environment()?;
    drive::Drive::new(auth)?.sync(&local_root, drive_folder)
}

fn print_help() {
    println!("SyncHub — sync a local folder to Google Drive\n\nUsage:\n  synchub [--dry-run] <LOCAL_FOLDER> [DRIVE_FOLDER]\n\nExamples:\n  synchub --dry-run ~/Documents\n  synchub ~/Documents\n  synchub ~/Documents MyDocumentsBackup\n\nSyncHub uploads new or changed files into Drive/SyncHub/<DRIVE_FOLDER>.\nIt keeps Drive files that are absent locally. Symlinks are skipped.\nSet SYNCHUB_CLIENT_ID for your Google Desktop OAuth client before the first sync.");
}
