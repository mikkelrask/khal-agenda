use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct Task {
    pub done: bool,
    pub title: String,
    pub priority: Option<char>,
    pub metadata: Vec<String>,
}

fn is_date(text: &str) -> bool {
    text.len() == 10
        && text.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

fn without_completion(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("x ") else {
        return text;
    };
    match rest.split_once(' ') {
        Some((date, body)) if is_date(date) => body,
        _ => rest,
    }
}

impl Task {
    pub fn parse(text: &str) -> Self {
        let body = without_completion(text.trim_end_matches(['\r', '\n']));
        let mut words = body.split_whitespace().peekable();
        let mut priority = None;
        if words.peek().is_some_and(|word| {
            let bytes = word.as_bytes();
            bytes.len() == 3
                && bytes[0] == b'('
                && bytes[1].is_ascii_uppercase()
                && bytes[2] == b')'
        }) {
            priority = words.next().map(|word| word.as_bytes()[1] as char);
        }
        let mut metadata = Vec::new();
        if words.peek().is_some_and(|word| is_date(word)) {
            metadata.push(format!("Created {}", words.next().unwrap()));
        }
        let mut title = Vec::new();
        for word in words {
            if word.len() > 1 && word.starts_with(['@', '+']) {
                metadata.push(word.to_owned());
            } else {
                title.push(word);
            }
        }
        let title = if title.is_empty() {
            priority = None;
            metadata.clear();
            body.to_owned()
        } else {
            title.join(" ")
        };
        Self {
            done: text.starts_with("x "),
            title,
            priority,
            metadata,
        }
    }
}

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
    pub fn set_completed(&mut self, index: usize, done: bool, date: &str) -> Result<()> {
        let text = self
            .lines
            .get(index)
            .context("Task no longer exists")?
            .trim_end_matches(['\r', '\n']);
        if text.trim().is_empty() {
            bail!("Task is empty");
        }
        if text.starts_with("x ") == done {
            return Ok(());
        }
        let updated = if done {
            if !is_date(date) {
                bail!("Could not determine today's date");
            }
            format!("x {date} {text}")
        } else {
            without_completion(text).to_owned()
        };
        self.save(index, &updated)
    }

    fn save(&mut self, index: usize, text: &str) -> Result<()> {
        if text.trim().is_empty() || text.contains(['\n', '\r']) {
            bail!("Task must be one nonempty line");
        }
        if fs::read_to_string(&self.path)? != self.original {
            bail!("The file changed elsewhere. Refresh before changing task completion.");
        }
        let mut lines = self.lines.clone();
        let old = lines.get_mut(index).context("Task no longer exists")?;
        let suffix = if old.ends_with("\r\n") {
            "\r\n"
        } else if old.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        *old = format!("{text}{suffix}");
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
        doc.set_completed(0, true, "2026-10-12").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "x 2026-10-12 (A) Keep @home +project\r\n\r\nx 2026-10-09 Done\r\n"
        );
        doc.set_completed(0, false, "").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "(A) Keep @home +project\r\n\r\nx 2026-10-09 Done\r\n"
        );
        assert!(doc.set_completed(1, true, "2026-10-12").is_err());
        assert!(doc.set_completed(0, true, "bad date").is_err());
        assert!(doc.set_completed(100, true, "2026-10-12").is_err());
        fs::write(&path, "Changed on another device\n").unwrap();
        assert!(doc.set_completed(0, true, "2026-10-12").is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "Changed on another device\n"
        );
        assert!(doc.save(0, "two\nlines").is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn parses_display_without_losing_task_metadata() {
        let task = Task::parse("(A) 2026-10-01 Buy café beans @town +shopping due:2026-10-15\r\n");
        assert!(!task.done);
        assert_eq!(task.title, "Buy café beans due:2026-10-15");
        assert_eq!(task.priority, Some('A'));
        assert_eq!(task.metadata, ["Created 2026-10-01", "@town", "+shopping"]);
        let task = Task::parse("x 2026-10-12 (B) 2026-10-01 Call Alice @phone");
        assert!(task.done);
        assert_eq!(task.title, "Call Alice");
        assert_eq!(task.priority, Some('B'));
        assert_eq!(task.metadata, ["Created 2026-10-01", "@phone"]);
        assert_eq!(
            Task::parse("x Finished without a date").title,
            "Finished without a date"
        );
        assert_eq!(Task::parse("@home +project").title, "@home +project");
        assert_eq!(
            Task::parse("(a) Keep lowercase priority").title,
            "(a) Keep lowercase priority"
        );
        assert_eq!(
            without_completion("x 2026-10-12 (A) 2026-10-01  Keep  spaces"),
            "(A) 2026-10-01  Keep  spaces"
        );
        assert_eq!(without_completion("x An undated task"), "An undated task");
    }

    #[test]
    fn completion_preserves_creation_dates_and_missing_final_newline() {
        let path =
            std::env::temp_dir().join(format!("agenda-todo-completion-{}", std::process::id()));
        let original = "(B) 2026-10-01  Read  @home +book";
        fs::write(&path, original).unwrap();
        let mut doc = Document::load(&path).unwrap();
        doc.set_completed(0, true, "2026-10-12").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!("x 2026-10-12 {original}")
        );
        doc.set_completed(0, false, "").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        fs::write(&path, "x Undated task\n").unwrap();
        let mut doc = Document::load(&path).unwrap();
        doc.set_completed(0, false, "").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "Undated task\n");
        fs::remove_file(path).unwrap();
    }
}
