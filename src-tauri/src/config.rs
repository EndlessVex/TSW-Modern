use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// CDN connection parameters extracted from LocalConfig.xml.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchConfig {
    /// Base URL for HTTP patches, e.g. `http://update.secretworld.com/tswupm`.
    pub http_patch_addr: String,
    /// Subfolder on the patch server, e.g. `TSWLiveSteam`.
    pub http_patch_folder: String,
    /// Version fingerprint from the local install.
    pub patch_version: String,
    /// Universe server address, e.g. `um.live.secretworld.com:7000`.
    pub universe_addr: String,
}

impl PatchConfig {
    /// Full patch base URL including the folder path.
    pub fn patch_base_url(&self) -> String {
        format!(
            "{}/{}",
            self.http_patch_addr.trim_end_matches('/'),
            self.http_patch_folder
        )
    }
}

/// Errors produced by the LocalConfig.xml parser.
#[derive(Debug)]
pub enum ConfigParseError {
    Io(io::Error),
    Xml(quick_xml::Error),
    /// A required field was not found in the XML.
    MissingField(&'static str),
}

impl fmt::Display for ConfigParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigParseError::Io(e) => write!(f, "I/O error: {e}"),
            ConfigParseError::Xml(e) => write!(f, "XML parse error: {e}"),
            ConfigParseError::MissingField(field) => {
                write!(f, "required field missing from LocalConfig.xml: {field}")
            }
        }
    }
}

impl std::error::Error for ConfigParseError {}

impl From<io::Error> for ConfigParseError {
    fn from(e: io::Error) -> Self {
        ConfigParseError::Io(e)
    }
}

impl From<quick_xml::Error> for ConfigParseError {
    fn from(e: quick_xml::Error) -> Self {
        ConfigParseError::Xml(e)
    }
}

/// Parse `LocalConfig.xml` and extract CDN patch parameters.
///
/// Expects the `Universe/Client` section containing `HttpPatchAddr`,
/// `HttpPatchFolder`, and `PatchVersion` elements.
pub fn parse_local_config(path: &Path) -> Result<PatchConfig, ConfigParseError> {
    let xml_str = fs::read_to_string(path).map_err(|e| {
        ConfigParseError::Io(io::Error::new(
            e.kind(),
            format!("{}: {e}", path.display()),
        ))
    })?;

    let mut reader = quick_xml::Reader::from_str(&xml_str);

    let mut http_patch_addr: Option<String> = None;
    let mut http_patch_folder: Option<String> = None;
    let mut patch_version: Option<String> = None;
    let mut universe_addr: Option<String> = None;

    // Track current element name so we can capture text content.
    let mut current_tag: Option<String> = None;
    // Track whether we're inside Universe/Client.
    let mut in_universe = false;
    let mut in_client = false;

    loop {
        match reader.read_event() {
            Ok(quick_xml::events::Event::Start(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "Universe" => in_universe = true,
                    "Client" if in_universe => in_client = true,
                    _ => {}
                }
                if in_client {
                    current_tag = Some(name);
                }
            }
            Ok(quick_xml::events::Event::End(ref e)) => {
                let name = String::from_utf8_lossy(e.name().as_ref()).to_string();
                match name.as_str() {
                    "Universe" => {
                        in_universe = false;
                        in_client = false;
                    }
                    "Client" if in_universe => in_client = false,
                    _ => {}
                }
                current_tag = None;
            }
            Ok(quick_xml::events::Event::Text(ref t)) => {
                if let Some(ref tag) = current_tag {
                    let text = t.unescape().map_err(quick_xml::Error::from)?.to_string();
                    match tag.as_str() {
                        "HttpPatchAddr" => http_patch_addr = Some(text),
                        "HttpPatchFolder" => http_patch_folder = Some(text),
                        "PatchVersion" => patch_version = Some(text),
                        "UniverseAddr" => universe_addr = Some(text),
                        _ => {}
                    }
                }
            }
            Ok(quick_xml::events::Event::Eof) => break,
            Err(e) => return Err(ConfigParseError::Xml(e)),
            _ => {}
        }
    }

    Ok(PatchConfig {
        http_patch_addr: http_patch_addr
            .ok_or(ConfigParseError::MissingField("HttpPatchAddr"))?,
        http_patch_folder: http_patch_folder
            .ok_or(ConfigParseError::MissingField("HttpPatchFolder"))?,
        patch_version: patch_version
            .ok_or(ConfigParseError::MissingField("PatchVersion"))?,
        universe_addr: universe_addr
            .ok_or(ConfigParseError::MissingField("UniverseAddr"))?,
    })
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tsw_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../The Secret World")
    }

    /// Skip test if TSW isn't installed locally (e.g. CI)
    macro_rules! require_tsw {
        () => {
            if !tsw_dir().exists() { return; }
        };
    }

    fn local_config_path() -> PathBuf {
        tsw_dir().join("LocalConfig.xml")
    }

    #[test]
    fn config_parses_real_file() {
        require_tsw!();
        let cfg = parse_local_config(&local_config_path()).expect("parse LocalConfig.xml");
        assert_eq!(cfg.http_patch_addr, "http://update.secretworld.com/tswupm");
        assert_eq!(cfg.http_patch_folder, "TSWLiveSteam");
        assert_eq!(cfg.patch_version, "xb36bba4f8606fe8fda4fec2a747703bf");
        assert_eq!(cfg.universe_addr, "um.live.secretworld.com:7000");
    }

    #[test]
    fn config_patch_base_url() {
        require_tsw!();
        let cfg = parse_local_config(&local_config_path()).expect("parse");
        assert_eq!(
            cfg.patch_base_url(),
            "http://update.secretworld.com/tswupm/TSWLiveSteam"
        );
    }

    #[test]
    fn config_missing_field() {
        let xml = r#"<Config><Universe><Client><HttpPatchAddr>http://x</HttpPatchAddr></Client></Universe></Config>"#;
        let tmp = std::env::temp_dir().join("missing_field_config.xml");
        fs::write(&tmp, xml).unwrap();
        let err = parse_local_config(&tmp).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("HttpPatchFolder"),
            "should report missing HttpPatchFolder, got: {msg}"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn config_file_not_found() {
        let err =
            parse_local_config(Path::new("/tmp/definitely_nonexistent_config.xml")).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("definitely_nonexistent_config.xml"),
            "error should contain path: {msg}"
        );
    }

    #[test]
    fn config_invalid_xml() {
        // quick-xml silently skips non-element junk — so malformed input
        // surfaces as MissingField rather than Xml. Either error is acceptable;
        // what matters is we don't panic or return Ok.
        let tmp = std::env::temp_dir().join("invalid_config.xml");
        fs::write(&tmp, "<<<not xml>>>").unwrap();
        let err = parse_local_config(&tmp);
        assert!(err.is_err(), "should error on invalid XML");
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn config_empty_xml() {
        let tmp = std::env::temp_dir().join("empty_config.xml");
        fs::write(&tmp, "<Config></Config>").unwrap();
        let err = parse_local_config(&tmp).unwrap_err();
        assert!(
            matches!(err, ConfigParseError::MissingField(_)),
            "expected MissingField, got: {err}"
        );
        let _ = fs::remove_file(&tmp);
    }
}
