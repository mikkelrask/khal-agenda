use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Document {
    path: PathBuf,
    original: String,
    pub lines: Vec<String>,
}
impl Document {
    pub fn load(path: &Path) -> Result<Self> {
        let path =
            fs::canonicalize(path).context("Choose an existing todo.txt file in Settings")?;
        let original = fs::read_to_string(&path).context("Could not read todo.txt")?;
        let lines = original.split_inclusive('\n').map(str::to_owned).collect();
        Ok(Self {
            path,
            original,
            lines,
        })
    }
    pub fn save(&mut self, index: Option<usize>, text: &str) -> Result<()> {
        if text.trim().is_empty() || text.contains(['\n', '\r']) {
            bail!("Enter one nonempty task per line");
        }
        if fs::read_to_string(&self.path)? != self.original {
            bail!(
                "The file changed elsewhere. Refresh before saving; your edit has not been written."
            );
        }
        let mut lines = self.lines.clone();
        let ending = if self.original.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        if let Some(i) = index {
            let old = lines.get_mut(i).context("Task no longer exists")?;
            let suffix = if old.ends_with("\r\n") {
                "\r\n"
            } else if old.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            *old = format!("{text}{suffix}");
        } else {
            if let Some(last) = lines.last_mut()
                && !last.ends_with('\n')
            {
                last.push_str(ending);
            }
            lines.push(format!("{text}{ending}"));
        }
        let updated = lines.concat();
        let temporary = self.path.with_extension(format!(
            "khal-agenda-{}-{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let result = (|| -> Result<()> {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            fs::set_permissions(&temporary, fs::metadata(&self.path)?.permissions())?;
            file.write_all(updated.as_bytes())?;
            file.sync_all()?;
            if fs::read_to_string(&self.path)? != self.original {
                bail!("The file changed while saving. Refresh and try again.");
            }
            fs::rename(&temporary, &self.path)?;
            Ok(())
        })();
        if temporary.exists() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        self.original = updated;
        self.lines = lines;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_other_lines_and_rejects_external_changes() {
        let path = std::env::temp_dir().join(format!("agenda-todo-test-{}", std::process::id()));
        fs::write(
            &path,
            "(A) Keep @home +project\r\n\r\nx 2026-10-09 Done\r\n",
        )
        .unwrap();
        let mut doc = Document::load(&path).unwrap();
        doc.save(Some(0), "Edited @home +project").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "Edited @home +project\r\n\r\nx 2026-10-09 Done\r\n"
        );
        fs::write(&path, "Changed on another device\n").unwrap();
        assert!(doc.save(None, "New task").is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "Changed on another device\n"
        );
        assert!(doc.save(None, "two\nlines").is_err());
        fs::remove_file(path).unwrap();
    }
}
