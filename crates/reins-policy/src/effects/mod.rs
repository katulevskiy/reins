//! The consequence engine (`docs/design/decisions-v2/01-decision-engine.md` §2): a request becomes typed effects,
//! each with a level, and the request a level, a certainty and the always-ask floor (08 R4).
//!
//! Pure and IO-free, like the rest of this crate: the caller passes what it knows (the working directory, the
//! environment, the workspace). In Decisions v2 P1 nothing decides on an analysis yet: it is attached to requests and
//! shown in Activity, and `reins-replay` measures it. The guard's groups are kept as [`Analysis::labels`] only.
//!
//! Entry points: [`analyze_command`] (a shell command line), [`analyze_files`] (a file hook), [`analyze_push`] and
//! [`analyze_git_read`] (git through the desktop proxy), [`analyze_tool`] (a connector tool call), [`analyze_mcp`]
//! (another MCP server's tool) and [`analyze_question`] (`reins ask`).

mod connectors;
mod describe;
mod git;
mod infra;
mod inline;
mod level;
mod model;
pub mod nets;
mod programs;
mod shell;
mod workspace;

pub use connectors::{
    ToolAnnotations, analyze_files, analyze_mcp, analyze_question, analyze_tool, github_api, tool_repo,
};
pub use git::{analyze_git_read, analyze_push, is_default};
pub use infra::sql_change;
pub use level::{ACCOUNTS, AGENT, Analysis, MASS_MESSAGE, MAX_LINES, Summary, floor_of, level_of, local_host};
pub use model::{
    Area, Certainty, Channel, CodeSource, Effect, EffectKind, FileKind, Floor, InfraChange, Level, NetMethod, PathRef,
    SecretKind,
};
pub use programs::url_host;
pub use shell::{Context, analyze_command, file_kind};
pub use workspace::{Workspace, normalize};

#[cfg(test)]
mod hardening;
#[cfg(test)]
mod tests;
