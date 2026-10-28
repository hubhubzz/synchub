# SyncHub

SyncHub is a Rust command-line app that uploads a local folder directly to Google Drive with the Google Drive API.

SyncHub uploads new and changed files to `Drive/SyncHub/<folder-name>` and **does not delete files from Google Drive**. Use `--dry-run` first to list local files without connecting to Google.

## Requirements

- Rust and Cargo
- A Google Cloud project with the Google Drive API enabled
- A Google OAuth client of type **Desktop app**

Create a Desktop app OAuth client in Google Cloud Console and set its client ID before the first run:

```sh
export SYNCHUB_CLIENT_ID="your-client-id.apps.googleusercontent.com"
export SYNCHUB_CLIENT_SECRET="your-client-secret" # optional
```

The secret is optional for the Desktop OAuth flow. Do not commit it. On the first sync, SyncHub opens Google's sign-in page in your system browser and saves a refresh token in your operating system's user configuration directory. On Unix, the token directory and file are restricted to the current user.

The app requests the `drive.file` scope, which limits access to files created by SyncHub. OAuth branding and verification for production require a public home page and policy links on a domain you own and verify. The GitHub Pages default domain is not an owned domain for Google OAuth verification.

## Run

Preview the local files SyncHub would process:

```sh
cargo run -- --dry-run /path/to/local-folder
```

Copy the files:

```sh
cargo run -- /path/to/local-folder
```

By default, the Google Drive folder has the same name as the local folder. Pass a second argument to choose a different Drive folder name:

```sh
cargo run -- /path/to/local-folder MyDriveBackup
```

SyncHub creates folders as needed, uploads new files, updates files with changed contents, and skips unchanged files. It does not remove Drive files when they are deleted locally. Symbolic links are skipped.

## Build

```sh
cargo build --release
```

The binary is at `target/release/synchub`.

## Public pages

- Homepage: <https://hubhubzz.github.io/synchub/>
- Privacy: <https://hubhubzz.github.io/synchub/privacy.html>
- Terms: <https://hubhubzz.github.io/synchub/terms.html>

Enable GitHub Pages in the repository settings with **Deploy from a branch**, branch `main`, folder `/docs` to publish these pages.

## Third-party software

Google Drive authentication, file names, and file contents are sent directly from SyncHub to Google. SyncHub does not use a SyncHub-operated server or analytics service.
