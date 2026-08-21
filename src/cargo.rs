use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cargo_metadata::{MetadataCommand, Package};
use walkdir::WalkDir;

#[derive(Clone, Debug)]
pub struct WorkspacePackage {
    pub name: String,
    pub root: PathBuf,
    pub source_roots: Vec<PathBuf>,
    pub is_verus: bool,
}

pub fn workspace_packages(root: &Path) -> Result<Vec<WorkspacePackage>> {
    let manifest = root.join("Cargo.toml");
    let mut command = MetadataCommand::new();
    command.current_dir(root);
    if manifest.is_file() {
        command.manifest_path(&manifest);
    }
    let metadata = command.exec().context("running cargo metadata")?;
    let workspace_members: BTreeSet<_> = metadata.workspace_members.iter().collect();
    let mut packages = Vec::new();
    for package in metadata
        .packages
        .iter()
        .filter(|p| workspace_members.contains(&p.id))
    {
        let mut roots = BTreeSet::new();
        for target in &package.targets {
            let source = target.src_path.as_std_path();
            if let Some(parent) = source.parent() {
                roots.insert(parent.to_path_buf());
            }
        }
        let roots: Vec<_> = roots.into_iter().collect();
        let is_verus = is_verified(package) || roots.iter().any(|root| source_uses_verus(root));
        let package_root = package
            .manifest_path
            .parent()
            .context("package manifest has no parent")?
            .as_std_path()
            .to_path_buf();
        packages.push(WorkspacePackage {
            name: package.name.to_string(),
            root: package_root,
            source_roots: roots,
            is_verus,
        });
    }
    Ok(packages)
}

fn source_uses_verus(root: &Path) -> bool {
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != "target")
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "rs")
        })
        .any(|entry| {
            std::fs::read_to_string(entry.path()).is_ok_and(|source| source.contains("verus!"))
        })
}

fn is_verified(package: &Package) -> bool {
    package
        .metadata
        .get("verus")
        .and_then(|value| value.get("verify"))
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::workspace_packages;
    use std::fs;

    #[test]
    fn discovers_verus_package_without_metadata() {
        let project = tempfile::tempdir().unwrap();
        fs::create_dir(project.path().join("src")).unwrap();
        fs::write(
            project.path().join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            project.path().join("src/lib.rs"),
            "verus! { fn checked(x: i32) -> bool { x > 0 } }\n",
        )
        .unwrap();

        let packages = workspace_packages(project.path()).unwrap();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "fixture");
        assert!(packages[0].is_verus);
    }
}
