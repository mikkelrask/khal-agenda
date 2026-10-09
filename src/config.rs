use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub days_ahead: u32,
    pub month_view: bool,
    pub theme: String,
    pub excluded: Vec<String>,
    pub khal_config: Option<String>,
    pub python: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            days_ahead: 7,
            month_view: false,
            theme: "system".into(),
            excluded: vec![],
            khal_config: None,
            python: "python3".into(),
        }
    }
}
impl Config {
    pub fn path() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            })
            .join("khal-agenda/config.toml")
    }
    pub fn validate(&self) -> Result<()> {
        if self.days_ahead > 90 {
            bail!("Days ahead must be between 0 and 90");
        }
        if !["system", "light", "dark"].contains(&self.theme.as_str()) {
            bail!("Theme must be system, light, or dark");
        }
        if self.python.is_empty() {
            bail!("Python executable must not be empty");
        }
        Ok(())
    }
    pub fn load() -> Result<Self> {
        let path = Self::path();
        let c: Self = if path.exists() {
            toml::from_str(&std::fs::read_to_string(path)?)?
        } else {
            Self::default()
        };
        c.validate()?;
        Ok(c)
    }
    pub fn save(&self) -> Result<()> {
        self.validate()?;
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml::to_string_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_invalid_preferences() {
        assert!(
            Config {
                days_ahead: 91,
                ..Config::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            Config {
                theme: "typo".into(),
                ..Config::default()
            }
            .validate()
            .is_err()
        );
        assert!(toml::from_str::<Config>("days_ahhead = 2").is_err());
        assert!(
            Config {
                days_ahead: 0,
                ..Config::default()
            }
            .validate()
            .is_ok()
        );
    }
}
