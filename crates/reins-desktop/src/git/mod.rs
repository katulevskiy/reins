//! Git: pkt-lines, packs and objects, and the description of a push the user approves.

pub mod analyze;
pub mod object;
pub mod pack;
pub mod pktline;
pub mod remote;

pub use analyze::{KeptPatch, analyze_push, analyze_push_keeping, with_kept};
pub use pktline::{Command, ReceiveHead, parse_receive_head};
pub use remote::{GitHubRemote, NoRemote, Remote};
