use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use url::Url;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

#[derive(Debug, Serialize, Deserialize)]
struct SavedToken {
    client_id: String,
    refresh_token: String,
    access_token: String,
    expires_at: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

pub struct Auth {
    client_id: String,
    client_secret: Option<String>,
    token_path: PathBuf,
    token: Option<SavedToken>,
}

impl Auth {
    pub fn from_environment() -> Result<Self, String> {
        let client_id = std::env::var("SYNCHUB_CLIENT_ID").map_err(|_| {
            "set SYNCHUB_CLIENT_ID to your Google Desktop OAuth client ID".to_string()
        })?;
        if client_id.trim().is_empty() {
            return Err("SYNCHUB_CLIENT_ID is empty".into());
        }
        let client_secret = std::env::var("SYNCHUB_CLIENT_SECRET")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let config_dir = dirs::config_dir()
            .ok_or_else(|| {
                "could not find a configuration directory for this operating system".to_string()
            })?
            .join("synchub");
        fs::create_dir_all(&config_dir)
            .map_err(|e| format!("cannot create token directory: {e}"))?;
        set_private_dir(&config_dir)?;
        let token_path = config_dir.join("google-token.json");
        let token = match fs::read(&token_path) {
            Ok(bytes) => serde_json::from_slice::<SavedToken>(&bytes).ok(),
            Err(_) => None,
        };
        Ok(Self {
            client_id,
            client_secret,
            token_path,
            token,
        })
    }

    pub fn access_token(&mut self) -> Result<String, String> {
        let now = now_secs()?;
        if let Some(token) = &self.token {
            if token.client_id == self.client_id && token.expires_at > now + 60 {
                return Ok(token.access_token.clone());
            }
        }
        if self
            .token
            .as_ref()
            .is_some_and(|token| token.client_id == self.client_id)
        {
            self.refresh()
        } else {
            self.authorize()
        }
    }

    fn refresh(&mut self) -> Result<String, String> {
        let saved = self
            .token
            .as_ref()
            .expect("refresh is called with a saved token");
        let mut form = vec![
            ("client_id", self.client_id.as_str()),
            ("refresh_token", saved.refresh_token.as_str()),
            ("grant_type", "refresh_token"),
        ];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret", secret));
        }
        let response = reqwest::blocking::Client::new()
            .post(TOKEN_URL)
            .form(&form)
            .send()
            .map_err(|e| format!("Google token refresh failed: {e}"))?;
        let body = response
            .text()
            .map_err(|e| format!("cannot read Google token response: {e}"))?;
        let token_response: TokenResponse = serde_json::from_str(&body)
            .map_err(|_| format!("Google token refresh failed: {body}"))?;
        let refresh_token = self.token.as_ref().unwrap().refresh_token.clone();
        self.save_token(token_response, refresh_token)
    }

    fn authorize(&mut self) -> Result<String, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|e| format!("cannot open the local OAuth callback port: {e}"))?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let redirect_uri = format!("http://127.0.0.1:{}/", address.port());
        let state = random_url_token(32);
        let verifier = random_url_token(64);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));

        let mut url = Url::parse(AUTH_URL).map_err(|e| e.to_string())?;
        url.query_pairs_mut()
            .append_pair("client_id", &self.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", SCOPE)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("state", &state)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");

        println!("Opening Google sign-in in your browser...");
        if webbrowser::open(url.as_str()).is_err() {
            println!("Open this URL to authorize synchub:\n{}", url);
        }
        println!("Waiting for Google sign-in to finish...");
        let (mut stream, _) = listener
            .accept()
            .map_err(|e| format!("OAuth callback failed: {e}"))?;
        let callback = read_callback(&mut stream)?;
        let mut code = None;
        let mut returned_state = None;
        let mut oauth_error = None;
        for (key, value) in callback.query_pairs() {
            match key.as_ref() {
                "code" => code = Some(value.into_owned()),
                "state" => returned_state = Some(value.into_owned()),
                "error" => oauth_error = Some(value.into_owned()),
                _ => {}
            }
        }
        write_callback_response(&mut stream);
        if returned_state.as_deref() != Some(state.as_str()) {
            return Err("OAuth state did not match; please retry sign-in".into());
        }
        if let Some(error) = oauth_error {
            return Err(format!("Google sign-in was not completed: {error}"));
        }
        let code = code.ok_or_else(|| "Google did not return an authorization code".to_string())?;
        let mut form = vec![
            ("client_id", self.client_id.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect_uri.as_str()),
        ];
        if let Some(secret) = &self.client_secret {
            form.push(("client_secret", secret));
        }
        let response = reqwest::blocking::Client::new()
            .post(TOKEN_URL)
            .form(&form)
            .send()
            .map_err(|e| format!("Google token exchange failed: {e}"))?;
        let body = response
            .text()
            .map_err(|e| format!("cannot read Google token response: {e}"))?;
        let token_response: TokenResponse = serde_json::from_str(&body)
            .map_err(|_| format!("Google token exchange failed: {body}"))?;
        let refresh_token = token_response.refresh_token.clone()
            .ok_or_else(|| "Google did not issue a refresh token; revoke synchub access in Google Account settings and try again".to_string())?;
        self.save_token(token_response, refresh_token)
    }

    fn save_token(
        &mut self,
        response: TokenResponse,
        refresh_token: String,
    ) -> Result<String, String> {
        let saved = SavedToken {
            client_id: self.client_id.clone(),
            refresh_token,
            access_token: response.access_token,
            expires_at: now_secs()?.saturating_add(response.expires_in),
        };
        let serialized = serde_json::to_vec(&saved).map_err(|e| e.to_string())?;
        let temp_path = self
            .token_path
            .with_extension(format!("tmp-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|e| format!("cannot write token file: {e}"))?;
        set_private_file(&file)?;
        file.write_all(&serialized)
            .map_err(|e| format!("cannot write token file: {e}"))?;
        file.sync_all()
            .map_err(|e| format!("cannot save token file: {e}"))?;
        #[cfg(windows)]
        if self.token_path.exists() {
            fs::remove_file(&self.token_path)
                .map_err(|e| format!("cannot replace the previous token file: {e}"))?;
        }
        fs::rename(&temp_path, &self.token_path)
            .map_err(|e| format!("cannot install token file: {e}"))?;
        self.token = Some(saved);
        Ok(self.token.as_ref().unwrap().access_token.clone())
    }
}

fn read_callback(stream: &mut TcpStream) -> Result<Url, String> {
    let mut request = [0u8; 8192];
    let count = stream
        .read(&mut request)
        .map_err(|e| format!("cannot read OAuth callback: {e}"))?;
    let request =
        std::str::from_utf8(&request[..count]).map_err(|_| "invalid OAuth callback".to_string())?;
    let target = request
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| "invalid OAuth callback request".to_string())?;
    Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|e| format!("invalid OAuth callback URL: {e}"))
}

fn write_callback_response(stream: &mut TcpStream) {
    let body =
        "<html><body><h2>Google sign-in complete</h2><p>Return to synchub.</p></body></html>";
    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
    let _ = stream.write_all(response.as_bytes());
}

fn random_url_token(bytes: usize) -> String {
    let mut value = vec![0u8; bytes];
    OsRng.fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}

fn now_secs() -> Result<u64, String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs())
}

#[cfg(unix)]
fn set_private_dir(path: &std::path::Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("cannot secure token directory: {e}"))
}

#[cfg(not(unix))]
fn set_private_dir(_: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(file: &File) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot secure token file: {e}"))
}

#[cfg(not(unix))]
fn set_private_file(_: &File) -> Result<(), String> {
    Ok(())
}
