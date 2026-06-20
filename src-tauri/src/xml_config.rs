use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum XmlConfigError {
    #[error("XML parse error: {0}")]
    Parse(String),
    #[error("config XML must have a <Robot> root element")]
    MissingRobotRoot,
    #[error("could not locate the Downloads directory")]
    DownloadsUnavailable,
    #[error("file error: {0}")]
    File(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct XmlValidation {
    pub ok: bool,
    pub warnings: Vec<String>,
}

pub fn validate_robot_xml(xml: &str) -> Result<XmlValidation, XmlConfigError> {
    let document =
        roxmltree::Document::parse(xml).map_err(|err| XmlConfigError::Parse(err.to_string()))?;
    let root = document.root_element();
    if root.tag_name().name() != "Robot" {
        return Err(XmlConfigError::MissingRobotRoot);
    }

    let mut seen = HashSet::new();
    let mut duplicates = HashSet::new();
    for node in document.descendants().filter(|node| node.is_element()) {
        if let Some(name) = node.attribute("name") {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            let normalized = trimmed.to_lowercase();
            if !seen.insert(normalized) {
                duplicates.insert(trimmed.to_string());
            }
        }
    }

    let mut warnings = Vec::new();
    if !duplicates.is_empty() {
        let mut names = duplicates.into_iter().collect::<Vec<_>>();
        names.sort();
        warnings.push(format!("Duplicate device names: {}", names.join(", ")));
    }

    Ok(XmlValidation { ok: true, warnings })
}

pub fn save_config_to_downloads(config_name: &str, xml: &str) -> Result<PathBuf, XmlConfigError> {
    let downloads = downloads_dir().ok_or(XmlConfigError::DownloadsUnavailable)?;
    fs::create_dir_all(&downloads)?;

    let base = sanitize_filename(config_name)
        .trim_end_matches(".xml")
        .trim_end_matches(".XML")
        .to_string();
    let base = if base.is_empty() {
        "robot-config".to_string()
    } else {
        base
    };

    let mut candidate = downloads.join(format!("{base}.xml"));
    let mut index = 2;
    while candidate.exists() {
        candidate = downloads.join(format!("{base}-{index}.xml"));
        index += 1;
    }

    fs::write(&candidate, xml)?;
    Ok(candidate)
}

fn downloads_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Downloads"))
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(xdg) = std::env::var_os("XDG_DOWNLOAD_DIR") {
            return Some(PathBuf::from(xdg));
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Downloads"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Downloads"))
    }
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
