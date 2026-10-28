use crate::auth::Auth;
use md5::{Digest, Md5};
use rand::{rngs::OsRng, RngCore};
use reqwest::blocking::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::Duration,
};

const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD_API: &str = "https://www.googleapis.com/upload/drive/v3/files";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const CHUNK_SIZE: usize = 8 * 1024 * 1024;
const MULTIPART_LIMIT: u64 = 5 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DriveFile {
    id: String,
    name: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
    #[serde(rename = "md5Checksum")]
    md5_checksum: Option<String>,
}

#[derive(Deserialize)]
struct FileList {
    files: Vec<DriveFile>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

pub struct Drive {
    http: Client,
    auth: Auth,
    uploaded: usize,
    skipped: usize,
}

impl Drive {
    pub fn new(auth: Auth) -> Result<Self, String> {
        let http = Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|e| format!("cannot initialize HTTP client: {e}"))?;
        Ok(Self {
            http,
            auth,
            uploaded: 0,
            skipped: 0,
        })
    }

    pub fn sync(&mut self, local_root: &Path, drive_folder_name: &str) -> Result<(), String> {
        let root_id = self.find_or_create_folder(None, "SyncHub")?;
        let target_id = self.find_or_create_folder(Some(&root_id), drive_folder_name)?;
        self.sync_directory(local_root, &target_id)?;
        println!(
            "Done: {} uploaded or updated, {} unchanged.",
            self.uploaded, self.skipped
        );
        Ok(())
    }

    fn sync_directory(&mut self, local_dir: &Path, drive_parent_id: &str) -> Result<(), String> {
        let remote_children = self.list_children(Some(drive_parent_id))?;
        for entry in fs::read_dir(local_dir)
            .map_err(|e| format!("cannot read {}: {e}", local_dir.display()))?
        {
            let entry = entry.map_err(|e| format!("cannot read a directory entry: {e}"))?;
            let kind = entry
                .file_type()
                .map_err(|e| format!("cannot inspect {}: {e}", entry.path().display()))?;
            if kind.is_symlink() {
                println!("Skipping symbolic link: {}", entry.path().display());
                continue;
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| format!("path is not valid UTF-8: {}", entry.path().display()))?;
            if kind.is_dir() {
                let child_id = match remote_children
                    .iter()
                    .find(|file| file.name == name && file.mime_type == FOLDER_MIME)
                {
                    Some(file) => file.id.clone(),
                    None => self.create_folder(Some(drive_parent_id), &name)?,
                };
                self.sync_directory(&entry.path(), &child_id)?;
            } else if kind.is_file() {
                let path = entry.path();
                let local_md5 = file_md5(&path)?;
                let existing = remote_children
                    .iter()
                    .find(|file| file.name == name && file.mime_type != FOLDER_MIME);
                if existing.and_then(|file| file.md5_checksum.as_deref())
                    == Some(local_md5.as_str())
                {
                    self.skipped += 1;
                    continue;
                }
                let id = existing.map(|file| file.id.as_str());
                self.upload_file(&path, &name, drive_parent_id, id)?;
                self.uploaded += 1;
                println!("Uploaded: {}", path.display());
            }
        }
        Ok(())
    }

    fn find_or_create_folder(
        &mut self,
        parent: Option<&str>,
        name: &str,
    ) -> Result<String, String> {
        let children = self.list_children(parent)?;
        if let Some(file) = children
            .iter()
            .find(|file| file.name == name && file.mime_type == FOLDER_MIME)
        {
            return Ok(file.id.clone());
        }
        self.create_folder(parent, name)
    }

    fn list_children(&mut self, parent: Option<&str>) -> Result<Vec<DriveFile>, String> {
        let parent_query = match parent {
            Some(id) => format!("'{}' in parents", query_escape(id)),
            None => "'root' in parents".to_string(),
        };
        let query = format!("{parent_query} and trashed = false");
        let mut page_token: Option<String> = None;
        let mut files = Vec::new();
        loop {
            let mut request = self
                .http
                .get(format!("{API}/files"))
                .bearer_auth(self.auth.access_token()?)
                .query(&[
                    ("q", query.as_str()),
                    ("pageSize", "1000"),
                    (
                        "fields",
                        "nextPageToken,files(id,name,mimeType,md5Checksum)",
                    ),
                ]);
            if let Some(token) = page_token.as_deref() {
                request = request.query(&[("pageToken", token)]);
            }
            let response = request
                .send()
                .map_err(|e| format!("Google Drive list request failed: {e}"))?;
            let result: FileList = parse_response(response)?;
            files.extend(result.files);
            page_token = result.next_page_token;
            if page_token.is_none() {
                return Ok(files);
            }
        }
    }

    fn create_folder(&mut self, parent: Option<&str>, name: &str) -> Result<String, String> {
        let mut metadata = json!({"name": name, "mimeType": FOLDER_MIME});
        if let Some(parent) = parent {
            metadata["parents"] = json!([parent]);
        }
        let response = self
            .http
            .post(format!("{API}/files?fields=id,name,mimeType"))
            .bearer_auth(self.auth.access_token()?)
            .json(&metadata)
            .send()
            .map_err(|e| format!("cannot create Drive folder {name}: {e}"))?;
        let file: DriveFile = parse_response(response)?;
        Ok(file.id)
    }

    fn upload_file(
        &mut self,
        path: &Path,
        name: &str,
        parent_id: &str,
        existing_id: Option<&str>,
    ) -> Result<(), String> {
        let size = fs::metadata(path)
            .map_err(|e| format!("cannot inspect {}: {e}", path.display()))?
            .len();
        let metadata = if existing_id.is_some() {
            json!({"name": name})
        } else {
            json!({"name": name, "parents": [parent_id]})
        };
        if size <= MULTIPART_LIMIT {
            return self.multipart_upload(path, metadata, existing_id);
        }
        self.resumable_upload(path, size, metadata, existing_id)
    }

    fn multipart_upload(
        &mut self,
        path: &Path,
        metadata: Value,
        existing_id: Option<&str>,
    ) -> Result<(), String> {
        let media = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut random = [0u8; 24];
        OsRng.fill_bytes(&mut random);
        let boundary = format!(
            "synchub_{}",
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        let metadata_bytes = serde_json::to_vec(&metadata).map_err(|e| e.to_string())?;
        let mut body = Vec::with_capacity(metadata_bytes.len() + media.len() + 256);
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(&metadata_bytes);
        body.extend_from_slice(
            format!("\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(&media);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let url = upload_url(existing_id, "multipart")?;
        let token = self.auth.access_token()?;
        let request = self
            .http
            .request(
                if existing_id.is_some() {
                    reqwest::Method::PATCH
                } else {
                    reqwest::Method::POST
                },
                url,
            )
            .bearer_auth(token)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/related; boundary={boundary}"),
            )
            .body(body);
        let response = request
            .send()
            .map_err(|e| format!("upload failed for {}: {e}", path.display()))?;
        parse_response::<DriveFile>(response).map(|_| ())
    }

    fn resumable_upload(
        &mut self,
        path: &Path,
        size: u64,
        metadata: Value,
        existing_id: Option<&str>,
    ) -> Result<(), String> {
        let url = upload_url(existing_id, "resumable")?;
        let token = self.auth.access_token()?;
        let response = self
            .http
            .request(
                if existing_id.is_some() {
                    reqwest::Method::PATCH
                } else {
                    reqwest::Method::POST
                },
                url,
            )
            .bearer_auth(token)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/json; charset=UTF-8",
            )
            .header("X-Upload-Content-Type", "application/octet-stream")
            .header("X-Upload-Content-Length", size)
            .json(&metadata)
            .send()
            .map_err(|e| format!("cannot start upload for {}: {e}", path.display()))?;
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .ok_or_else(|| {
                format!(
                    "Google did not return an upload session for {}",
                    path.display()
                )
            })?
            .to_str()
            .map_err(|e| e.to_string())?
            .to_string();
        if !response.status().is_success() {
            return Err(response_error(response));
        }

        let mut file =
            File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let mut start = 0u64;
        while start < size {
            file.seek(SeekFrom::Start(start))
                .map_err(|e| format!("cannot seek {}: {e}", path.display()))?;
            let chunk_len = ((size - start).min(CHUNK_SIZE as u64)) as usize;
            let mut chunk = vec![0u8; chunk_len];
            file.read_exact(&mut chunk)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let end = start + chunk_len as u64 - 1;
            let response = self
                .http
                .put(&location)
                .bearer_auth(self.auth.access_token()?)
                .header(reqwest::header::CONTENT_LENGTH, chunk_len)
                .header(
                    reqwest::header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{size}"),
                )
                .body(chunk)
                .send()
                .map_err(|e| {
                    format!(
                        "upload interrupted for {}; rerun SyncHub to retry: {e}",
                        path.display()
                    )
                })?;
            if response.status() == reqwest::StatusCode::PERMANENT_REDIRECT {
                start = match response
                    .headers()
                    .get(reqwest::header::RANGE)
                    .and_then(|value| value.to_str().ok())
                {
                    Some(range) => range
                        .rsplit('-')
                        .next()
                        .and_then(|last| last.parse::<u64>().ok())
                        .map(|last| last + 1)
                        .ok_or_else(|| "Google returned an invalid upload range".to_string())?,
                    None => 0,
                };
                if start > size {
                    return Err("Google returned an invalid upload position".into());
                }
                continue;
            }
            if response.status().is_success() {
                return Ok(());
            }
            return Err(response_error(response));
        }
        Err("Google ended the upload without confirming completion".into())
    }
}

pub fn print_dry_run(local_root: &Path) -> Result<(), String> {
    fn walk(path: &Path) -> Result<(), String> {
        for entry in
            fs::read_dir(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?
        {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                println!("Would skip symbolic link: {}", entry.path().display());
            } else if kind.is_dir() {
                walk(&entry.path())?;
            } else if kind.is_file() {
                println!("Would upload or update: {}", entry.path().display());
            }
        }
        Ok(())
    }
    println!("Dry run only lists local files; no Google connection or changes are made.");
    walk(local_root)
}

fn file_md5(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut digest = Md5::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| format!("cannot hash {}: {e}", path.display()))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn upload_url(file_id: Option<&str>, upload_type: &str) -> Result<reqwest::Url, String> {
    let base = match file_id {
        Some(id) => format!(
            "{UPLOAD_API}/{}",
            url::form_urlencoded::byte_serialize(id.as_bytes()).collect::<String>()
        ),
        None => UPLOAD_API.to_string(),
    };
    let mut url = reqwest::Url::parse(&base).map_err(|e| e.to_string())?;
    url.query_pairs_mut()
        .append_pair("uploadType", upload_type)
        .append_pair("fields", "id,name,mimeType,md5Checksum");
    Ok(url)
}

fn query_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn parse_response<T: for<'de> Deserialize<'de>>(response: Response) -> Result<T, String> {
    if !response.status().is_success() {
        return Err(response_error(response));
    }
    response
        .json()
        .map_err(|e| format!("cannot parse Google Drive response: {e}"))
}

fn response_error(response: Response) -> String {
    let status = response.status();
    let body = response
        .text()
        .unwrap_or_else(|_| "could not read error response".into());
    format!("Google Drive returned {status}: {body}")
}
