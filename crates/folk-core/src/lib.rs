use std::env;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::PathBuf;

use rand::{Rng, RngCore};
use thiserror::Error;

const HTTP_BEARER_BYTES: usize = 32;
const HTTP_BEARER_HEX_LEN: usize = HTTP_BEARER_BYTES * 2;

pub fn terminal_time() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() % 86_400)
        .unwrap_or(0);
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    format!("{hour:02}:{minute:02}:{second:02}")
}

pub fn log_status(message: &str) {
    let now = terminal_time();
    eprintln!("[{now}] {message}");
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccessMode {
    Full,
    Limited,
    Sandbox,
}

impl AccessMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "full" => Some(Self::Full),
            "limited" => Some(Self::Limited),
            "sandbox" => Some(Self::Sandbox),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Limited => "limited",
            Self::Sandbox => "sandbox",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AppConfig {
    pub signal_url: Option<String>,
    pub room: Option<String>,
    pub http_port: Option<u16>,
    pub mode: Option<String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("HOME is not set")]
    HomeMissing,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("invalid HTTP bearer credential")]
    InvalidHttpBearer,
}

pub fn load_config() -> Result<AppConfig, ConfigError> {
    let path = config_path()?;
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(AppConfig::default()),
        Err(err) => return Err(err.into()),
    };

    let mut config = AppConfig::default();
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match key.trim() {
            "signal_url" => config.signal_url = Some(value.to_string()),
            "room" => config.room = Some(value.to_string()),
            "http_port" => config.http_port = value.parse::<u16>().ok(),
            "mode" => config.mode = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(config)
}

pub fn save_config(config: &AppConfig) -> Result<(), ConfigError> {
    let dir = config_dir()?;
    ensure_private_dir(&dir)?;
    let mut contents = String::new();
    if let Some(value) = &config.signal_url {
        contents.push_str("signal_url=");
        contents.push_str(value);
        contents.push('\n');
    }
    if let Some(value) = &config.room {
        contents.push_str("room=");
        contents.push_str(value);
        contents.push('\n');
    }
    if let Some(value) = config.http_port {
        contents.push_str("http_port=");
        contents.push_str(&value.to_string());
        contents.push('\n');
    }
    if let Some(value) = &config.mode {
        contents.push_str("mode=");
        contents.push_str(value);
        contents.push('\n');
    }
    let path = dir.join("config");
    fs::write(&path, contents)?;
    restrict_file_permissions(&path)?;
    Ok(())
}

pub fn load_or_create_http_bearer() -> Result<String, ConfigError> {
    let dir = config_dir()?;
    ensure_private_dir(&dir)?;
    let path = dir.join("http-token");

    match open_new_private_file(&path) {
        Ok(mut file) => {
            let token = generate_http_bearer();
            file.write_all(token.as_bytes())?;
            file.sync_all()?;
            sync_dir(&dir)?;
            Ok(token)
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.file_type().is_file() || metadata.len() != HTTP_BEARER_HEX_LEN as u64 {
                return Err(ConfigError::InvalidHttpBearer);
            }
            restrict_file_permissions(&path)?;
            let mut token = String::with_capacity(HTTP_BEARER_HEX_LEN);
            File::open(path)?
                .take((HTTP_BEARER_HEX_LEN + 1) as u64)
                .read_to_string(&mut token)?;
            if !valid_http_bearer(&token) {
                return Err(ConfigError::InvalidHttpBearer);
            }
            Ok(token)
        }
        Err(err) => Err(err.into()),
    }
}

fn generate_http_bearer() -> String {
    let mut bytes = [0_u8; HTTP_BEARER_BYTES];
    rand::rng().fill_bytes(&mut bytes);
    let mut token = String::with_capacity(HTTP_BEARER_HEX_LEN);
    for byte in bytes {
        write!(token, "{byte:02x}").expect("writing to a String cannot fail");
    }
    token
}

fn valid_http_bearer(token: &str) -> bool {
    token.len() == HTTP_BEARER_HEX_LEN
        && token
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn ensure_private_dir(path: &std::path::Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn open_new_private_file(path: &std::path::Path) -> Result<File, std::io::Error> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn restrict_file_permissions(path: &std::path::Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn sync_dir(path: &std::path::Path) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn generate_pairing_code() -> String {
    format!("{:016x}", rand::rng().random::<u64>())
}

fn config_dir() -> Result<PathBuf, ConfigError> {
    let home = env::var_os("HOME").ok_or(ConfigError::HomeMissing)?;
    Ok(PathBuf::from(home).join(".config").join("folk-around"))
}

fn config_path() -> Result<PathBuf, ConfigError> {
    Ok(config_dir()?.join("config"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_mode_should_parse_known_modes() {
        assert_eq!(AccessMode::parse("full"), Some(AccessMode::Full));
        assert_eq!(AccessMode::parse("limited"), Some(AccessMode::Limited));
        assert_eq!(AccessMode::parse("sandbox"), Some(AccessMode::Sandbox));
        assert_eq!(AccessMode::parse("bad"), None);
    }

    #[test]
    fn pairing_code_should_be_hex_16_chars() {
        let code = generate_pairing_code();
        assert_eq!(code.len(), 16);
        assert!(code.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn http_bearer_should_be_bounded_lowercase_hex() {
        let token = generate_http_bearer();
        assert!(valid_http_bearer(&token));
        assert!(!valid_http_bearer(&format!("{token}0")));
        assert!(!valid_http_bearer(&"A".repeat(HTTP_BEARER_HEX_LEN)));
    }

    #[cfg(unix)]
    #[test]
    fn credential_paths_should_be_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!(
            "folk-around-http-token-{:016x}",
            rand::rng().random::<u64>()
        ));
        ensure_private_dir(&dir).unwrap();
        let path = dir.join("token");
        let file = open_new_private_file(&path).unwrap();
        drop(file);

        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
