use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A project represents a codebase directory that moltis can work with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub label: String,
    pub directory: PathBuf,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub auto_worktree: bool,
    #[serde(default)]
    pub setup_command: Option<String>,
    #[serde(default)]
    pub detected: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

/// A context file loaded from a project directory hierarchy.
#[derive(Debug, Clone)]
pub struct ContextFile {
    pub path: PathBuf,
    pub content: String,
}

/// Aggregated context for a project: the project itself plus all loaded context files.
#[derive(Debug, Clone)]
pub struct ProjectContext {
    pub project: Project,
    /// Context files ordered from outermost (root) to innermost (project dir).
    pub context_files: Vec<ContextFile>,
    /// Active worktree directory, if one exists for this session.
    pub worktree_dir: Option<PathBuf>,
}

impl ProjectContext {
    /// Build the combined context string suitable for system prompt injection.
    pub fn to_prompt_section(&self) -> String {
        let mut out = format!(
            "# Project: {}\nDirectory: {}\n\n",
            self.project.label,
            self.project.directory.display()
        );
        if let Some(ref prompt) = self.project.system_prompt {
            out.push_str(prompt);
            out.push_str("\n\n");
        }
        for cf in &self.context_files {
            let name = cf.path.file_name().unwrap_or_default().to_string_lossy();
            out.push_str(&format!("## {}\n\n{}\n\n", name, cf.content));
        }
        out
    }
}
