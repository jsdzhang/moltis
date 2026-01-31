use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use {anyhow::Result, async_trait::async_trait};

use crate::types::Project;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Trait for persisting projects. Implementations can be TOML-file-backed,
/// SQLite, etc.
#[async_trait]
pub trait ProjectStore: Send + Sync {
    async fn list(&self) -> Result<Vec<Project>>;
    async fn get(&self, id: &str) -> Result<Option<Project>>;
    async fn upsert(&self, project: Project) -> Result<()>;
    async fn delete(&self, id: &str) -> Result<()>;
}

// ── TOML file-backed implementation ──────────────────────────────────

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
struct TomlFile {
    #[serde(default)]
    projects: Vec<Project>,
}

/// Stores projects in a TOML file at the given path.
pub struct TomlProjectStore {
    path: PathBuf,
}

impl TomlProjectStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn read_file(&self) -> Result<TomlFile> {
        if self.path.exists() {
            let data = fs::read_to_string(&self.path)?;
            Ok(toml::from_str(&data)?)
        } else {
            Ok(TomlFile::default())
        }
    }

    fn write_file(&self, file: &TomlFile) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = toml::to_string_pretty(file)?;
        fs::write(&self.path, data)?;
        Ok(())
    }
}

#[async_trait]
impl ProjectStore for TomlProjectStore {
    async fn list(&self) -> Result<Vec<Project>> {
        let mut projects = self.read_file()?.projects;
        projects.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(projects)
    }

    async fn get(&self, id: &str) -> Result<Option<Project>> {
        Ok(self.read_file()?.projects.into_iter().find(|p| p.id == id))
    }

    async fn upsert(&self, project: Project) -> Result<()> {
        let mut file = self.read_file()?;
        if let Some(existing) = file.projects.iter_mut().find(|p| p.id == project.id) {
            *existing = project;
        } else {
            file.projects.push(project);
        }
        self.write_file(&file)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let mut file = self.read_file()?;
        file.projects.retain(|p| p.id != id);
        self.write_file(&file)
    }
}

/// Create a new project with auto-derived fields.
pub fn new_project(id: String, label: String, directory: PathBuf) -> Project {
    let now = now_ms();
    Project {
        id,
        label,
        directory,
        system_prompt: None,
        auto_worktree: false,
        setup_command: None,
        detected: false,
        created_at: now,
        updated_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_toml_store_crud() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.toml");
        let store = TomlProjectStore::new(path);

        // Empty initially
        assert!(store.list().await.unwrap().is_empty());

        // Upsert
        let p = new_project("test".into(), "Test".into(), "/tmp/test".into());
        store.upsert(p).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);

        // Get
        let found = store.get("test").await.unwrap().unwrap();
        assert_eq!(found.label, "Test");

        // Update
        let mut updated = found;
        updated.label = "Updated".into();
        store.upsert(updated).await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        assert_eq!(store.get("test").await.unwrap().unwrap().label, "Updated");

        // Delete
        store.delete("test").await.unwrap();
        assert!(store.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_toml_store_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.toml");

        {
            let store = TomlProjectStore::new(path.clone());
            store
                .upsert(new_project("a".into(), "A".into(), "/a".into()))
                .await
                .unwrap();
        }

        // New store instance reads from disk
        let store = TomlProjectStore::new(path);
        let list = store.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "a");
    }
}
