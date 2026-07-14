use super::model::{DiffLineKind, PatchFile, PatchFileKind, PatchHunk, PatchLine};

pub fn parse_apply_patch(input: &str, max_files: usize) -> Result<Vec<PatchFile>, &'static str> {
    let normalized = input.replace("\r\n", "\n");
    let mut lines = normalized.lines();
    if lines.next() != Some("*** Begin Patch") {
        return Err("missing patch start");
    }

    let mut files = Vec::new();
    let mut current: Option<FileBuilder> = None;
    let mut ended = false;

    for line in lines {
        if line == "*** End Patch" {
            if ended {
                return Err("duplicate patch end");
            }
            if let Some(builder) = current.take() {
                files.push(builder.finish()?);
            }
            ended = true;
            continue;
        }
        if ended {
            if !line.is_empty() {
                return Err("trailing patch content");
            }
            continue;
        }

        if let Some(path) = line.strip_prefix("*** Add File: ") {
            push_previous(&mut current, &mut files)?;
            current = Some(FileBuilder::new(PatchFileKind::Add, path)?);
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            push_previous(&mut current, &mut files)?;
            current = Some(FileBuilder::new(PatchFileKind::Update, path)?);
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            push_previous(&mut current, &mut files)?;
            current = Some(FileBuilder::new(PatchFileKind::Delete, path)?);
        } else if let Some(path) = line.strip_prefix("*** Move to: ") {
            let builder = current.as_mut().ok_or("move without update")?;
            if builder.kind != PatchFileKind::Update || builder.moved_to.is_some() {
                return Err("invalid move");
            }
            validate_path(path)?;
            builder.moved_to = Some(path.to_string());
        } else if let Some(header) = line.strip_prefix("@@") {
            let builder = current.as_mut().ok_or("hunk without file")?;
            if builder.kind == PatchFileKind::Delete {
                return Err("delete file cannot contain hunks");
            }
            builder.start_hunk(header.trim().to_string());
        } else if line == "*** End of File" {
            let builder = current.as_mut().ok_or("end of file without file")?;
            builder.push_line(DiffLineKind::Meta, line.to_string())?;
        } else if let Some(builder) = current.as_mut() {
            let (kind, text) = if let Some(text) = line.strip_prefix('+') {
                (DiffLineKind::Add, text)
            } else if let Some(text) = line.strip_prefix('-') {
                (DiffLineKind::Delete, text)
            } else if let Some(text) = line.strip_prefix(' ') {
                (DiffLineKind::Context, text)
            } else if builder.kind == PatchFileKind::Add {
                return Err("add line must start with plus");
            } else {
                return Err("invalid patch line");
            };
            builder.push_line(kind, text.to_string())?;
        } else if !line.is_empty() {
            return Err("content outside file");
        }

        if files.len() + usize::from(current.is_some()) > max_files {
            return Err("too many files");
        }
    }

    if !ended || current.is_some() || files.is_empty() {
        return Err("unterminated patch");
    }
    Ok(files)
}

fn push_previous(
    current: &mut Option<FileBuilder>,
    files: &mut Vec<PatchFile>,
) -> Result<(), &'static str> {
    if let Some(builder) = current.take() {
        files.push(builder.finish()?);
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() || path.contains('\0') || path.chars().count() > 8192 {
        return Err("invalid path");
    }
    Ok(())
}

struct FileBuilder {
    kind: PatchFileKind,
    path: String,
    moved_to: Option<String>,
    hunks: Vec<PatchHunk>,
}

impl FileBuilder {
    fn new(kind: PatchFileKind, path: &str) -> Result<Self, &'static str> {
        validate_path(path)?;
        Ok(Self {
            kind,
            path: path.to_string(),
            moved_to: None,
            hunks: Vec::new(),
        })
    }

    fn start_hunk(&mut self, header: String) {
        self.hunks.push(PatchHunk {
            header,
            lines: Vec::new(),
        });
    }

    fn push_line(&mut self, kind: DiffLineKind, text: String) -> Result<(), &'static str> {
        if self.hunks.is_empty() {
            if self.kind == PatchFileKind::Add {
                self.start_hunk(String::new());
            } else {
                return Err("line without hunk");
            }
        }
        self.hunks
            .last_mut()
            .expect("hunk was created")
            .lines
            .push(PatchLine { kind, text });
        Ok(())
    }

    fn finish(self) -> Result<PatchFile, &'static str> {
        match self.kind {
            PatchFileKind::Add if self.hunks.is_empty() => return Err("empty add file"),
            PatchFileKind::Update if self.hunks.is_empty() => return Err("empty update file"),
            PatchFileKind::Delete if !self.hunks.is_empty() => {
                return Err("delete file cannot contain hunks")
            }
            _ => {}
        }
        let (kind, old_path, new_path) = if let Some(moved_to) = self.moved_to {
            (PatchFileKind::Move, Some(self.path), moved_to)
        } else {
            match self.kind {
                PatchFileKind::Add => (PatchFileKind::Add, None, self.path),
                PatchFileKind::Update => {
                    (PatchFileKind::Update, Some(self.path.clone()), self.path)
                }
                PatchFileKind::Delete => {
                    (PatchFileKind::Delete, Some(self.path.clone()), self.path)
                }
                PatchFileKind::Move => return Err("unexpected move kind"),
            }
        };
        Ok(PatchFile {
            kind,
            old_path,
            new_path,
            hunks: self.hunks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multi_file_patch_and_move() {
        let patch = "*** Begin Patch\n*** Add File: new.txt\n+hello\n*** Update File: old.txt\n*** Move to: moved.txt\n@@ section\n before\n-old\n+new\n*** Delete File: gone.txt\n*** End Patch\n";
        let files = parse_apply_patch(patch, 64).unwrap();
        assert_eq!(files.len(), 3);
        assert_eq!(files[0].kind, PatchFileKind::Add);
        assert_eq!(files[1].kind, PatchFileKind::Move);
        assert_eq!(files[1].old_path.as_deref(), Some("old.txt"));
        assert_eq!(files[1].new_path, "moved.txt");
        assert_eq!(files[2].kind, PatchFileKind::Delete);
    }

    #[test]
    fn rejects_partial_or_trailing_patch() {
        assert!(parse_apply_patch("*** Begin Patch\n*** Add File: x\n+x\n", 64).is_err());
        assert!(parse_apply_patch(
            "*** Begin Patch\n*** Add File: x\n+x\n*** End Patch\ntrailing",
            64
        )
        .is_err());
    }
}
