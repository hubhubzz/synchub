# SyncHub

SyncHub is a small Rust command-line app that copies a local folder to Google Drive. It uses your existing [rclone](https://rclone.org/) installation and Google Drive remote configuration, so OAuth tokens stay in rclone's local config.

SyncHub runs `rclone copy`: it uploads new or changed files and **does not delete files from Google Drive**. Use `--dry-run` first to preview transfers.

Google sign-in is handled by rclone's OAuth client. The Google consent screen for this MVP will identify rclone; SyncHub does not yet have its own OAuth client or branded Google consent screen. A branded OAuth flow requires a separate Google OAuth implementation and an owned, verified web domain.

## Requirements

- Rust and Cargo
- [rclone](https://rclone.org/install/)
- A Google Drive remote configured in rclone

Configure a Drive remote by running `rclone config`, choosing `n` for a new remote, naming it `gdrive`, and selecting Google Drive (`drive`). At the client ID and secret prompts, use your own Google OAuth credentials; rclone's shared Google Drive client ID is being retired during 2026. Enable the Google Drive API in your Google Cloud project and complete OAuth app setup before entering those credentials. Keep the client secret out of this repository. Follow rclone's browser sign-in flow, then confirm the remote with:

```sh
rclone lsd gdrive:
```

Google requires OAuth app home and policy links to use a domain you own and verify. The GitHub repository and default `github.io` address are useful for public project pages, but they are not an owned domain for Google OAuth verification. Add your custom domain to GitHub Pages and verify its root domain in Google Search Console before using the links in a production OAuth consent screen.

## Run

Preview what would be copied:

```sh
cargo run -- --dry-run /path/to/local-folder gdrive:Backups/MyFolder
```

Copy the files:

```sh
cargo run -- /path/to/local-folder gdrive:Backups/MyFolder
```

The source folder's contents are copied into the destination path. SyncHub does not implement two-way synchronization, scheduling, or deletion propagation.

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

SyncHub invokes rclone as a separate installed program. rclone is not bundled with this repository; its license and notices are available from the [rclone project](https://github.com/rclone/rclone).
