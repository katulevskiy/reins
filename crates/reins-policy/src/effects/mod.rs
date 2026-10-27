//! The consequence engine (`docs/design/decisions-v2/01-decision-engine.md` §2): a request becomes typed effects,
//! each with a level, and the request a level, a certainty and the always-ask floor (08 R4).
//!
//! Pure and IO-free, like the rest of this crate: the caller passes what it knows (the working directory, the
//! environment, the workspace). In Decisions v2 P1 nothing decides on an analysis yet: it is attached to requests and
//! shown in Activity, and `reins-replay` measures it. The guard's groups are kept as [`Analysis::labels`] only.

mod describe;
mod level;
mod model;
mod workspace;

pub use level::{ACCOUNTS, AGENT, Analysis, MASS_MESSAGE, MAX_LINES, Summary, floor_of, level_of, local_host};
pub use model::{
    Area, Certainty, Channel, CodeSource, Effect, FileKind, Floor, InfraChange, Level, NetMethod, PathRef, SecretKind,
};
pub use workspace::{Workspace, normalize};
