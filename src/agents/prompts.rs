//! Render the system + user conflict prompts.
//!
//! Templates ship embedded in the binary via `include_str!`. The system
//! prompt is static; the user prompt is rendered with [`minijinja`]
//! against the [`ConflictPrompt`] struct.

use minijinja::{context, Environment};

use crate::core::ports::ConflictPrompt;
use crate::error::{Error, Result};

const SYSTEM_PROMPT: &str = include_str!("prompts/conflict_system.md");
const USER_PROMPT_TEMPLATE: &str = include_str!("prompts/conflict_user.md.j2");

/// Fully-rendered prompt to hand to a `MergeAgent`.
#[derive(Debug, Clone)]
pub struct RenderedPrompt {
    pub system: String,
    pub user: String,
}

impl RenderedPrompt {
    /// Concatenate `system` and `user` into one block separated by a
    /// horizontal rule. Suitable for agents that don't have a system/user
    /// distinction at the CLI surface.
    pub fn combined(&self) -> String {
        format!("{}\n\n---\n\n{}", self.system.trim_end(), self.user)
    }
}

/// Render the embedded user-prompt template against `prompt`. The system
/// prompt is the static contents of `prompts/conflict_system.md`.
pub fn render(prompt: &ConflictPrompt) -> Result<RenderedPrompt> {
    let mut env = Environment::new();
    env.add_template("user", USER_PROMPT_TEMPLATE)
        .map_err(|e| Error::template(format!("loading user prompt template: {e}")))?;
    let tmpl = env
        .get_template("user")
        .map_err(|e| Error::template(format!("get user template: {e}")))?;
    let user = tmpl
        .render(context! {
            repo_name => &prompt.repo_name,
            source_branch => &prompt.source_branch,
            target_branch => &prompt.target_branch,
            conflicted_files => &prompt.conflicted_files,
            ci_command_hint => &prompt.ci_command_hint,
        })
        .map_err(|e| Error::template(format!("rendering user prompt: {e}")))?;
    Ok(RenderedPrompt {
        system: SYSTEM_PROMPT.to_string(),
        user,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ConflictPrompt {
        ConflictPrompt {
            repo_name: "mergequeue".into(),
            source_branch: "feat/auth".into(),
            target_branch: "main".into(),
            conflicted_files: vec!["src/lib.rs".into(), "Cargo.toml".into()],
            ci_command_hint: Some("cargo test".into()),
        }
    }

    #[test]
    fn renders_branches_and_files() {
        let r = render(&sample()).unwrap();
        assert!(r.user.contains("feat/auth"));
        assert!(r.user.contains("main"));
        assert!(r.user.contains("mergequeue"));
        assert!(r.user.contains("`src/lib.rs`"));
        assert!(r.user.contains("`Cargo.toml`"));
        assert!(r.user.contains("cargo test"));
    }

    #[test]
    fn renders_without_ci_hint() {
        let mut p = sample();
        p.ci_command_hint = None;
        let r = render(&p).unwrap();
        assert!(r.user.contains("run the project's tests"));
        assert!(!r.user.contains("```")); // No fenced block when no hint.
    }

    #[test]
    fn system_prompt_present() {
        let r = render(&sample()).unwrap();
        assert!(r.system.contains("Do not push"));
        assert!(r.system.contains("MERGEQUEUE: ready for retry"));
    }

    #[test]
    fn combined_separates_sections() {
        let r = render(&sample()).unwrap();
        assert!(r.combined().contains("\n---\n"));
    }
}
