use std::env;
use std::fs;
use std::path::PathBuf;

use rand::Rng;
use thiserror::Error;

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
    fs::create_dir_all(&dir)?;
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
    fs::write(dir.join("config"), contents)?;
    Ok(())
}

pub fn generate_pairing_code() -> String {
    const WORDS: [&str; 16] = [
        "amber", "cedar", "copper", "delta", "ember", "harbor", "indigo", "juno", "maple", "nova",
        "orbit", "pixel", "quartz", "river", "signal", "violet",
    ];
    let number = rand::rng().random::<u64>();
    format!(
        "{}-{:04}",
        WORDS[number as usize % WORDS.len()],
        number % 10000
    )
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
    fn pairing_code_should_have_word_and_four_digits() {
        let code = generate_pairing_code();
        let Some((word, number)) = code.split_once('-') else {
            panic!("pairing code missing hyphen");
        };
        assert!(word.len() >= 4);
        assert_eq!(number.len(), 4);
        assert!(number.chars().all(|ch| ch.is_ascii_digit()));
    }
}
