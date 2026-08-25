use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use walkdir::WalkDir;

use crate::model::Mutant;

pub fn copy_project(source: &Path, destination: &Path) -> Result<()> {
    for entry in WalkDir::new(source)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !excluded(source, entry.path()))
    {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_symlink() {
            let link = fs::read_link(entry.path())?;
            #[cfg(unix)]
            std::os::unix::fs::symlink(link, target)?;
            #[cfg(not(unix))]
            anyhow::bail!("symlinked project inputs are not supported on this platform");
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

fn excluded(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        relative.components().any(|part| {
            let part = part.as_os_str();
            part == ".git" || part == "target" || part == ".verus-mutants"
        })
    })
}

pub fn apply(root: &Path, mutant: &Mutant) -> Result<()> {
    let path = root.join(&mutant.file);
    let mut source = fs::read_to_string(&path)
        .with_context(|| format!("reading mutant target {}", path.display()))?;
    if mutant.expected_occurrences > 1 {
        let count = source.matches(&mutant.original).count();
        anyhow::ensure!(
            count == mutant.expected_occurrences,
            "mutant {} expected {} occurrences, found {}",
            mutant.id,
            mutant.expected_occurrences,
            count
        );
        source = source.replace(&mutant.original, &mutant.replacement);
        return fs::write(&path, source)
            .with_context(|| format!("writing mutant target {}", path.display()));
    }
    anyhow::ensure!(
        mutant.end <= source.len(),
        "mutant {} span is outside its source",
        mutant.id
    );
    anyhow::ensure!(
        source[mutant.start..mutant.end] == mutant.original,
        "mutant {} context changed",
        mutant.id
    );
    source.replace_range(mutant.start..mutant.end, &mutant.replacement);
    fs::write(&path, source).with_context(|| format!("writing mutant target {}", path.display()))
}
