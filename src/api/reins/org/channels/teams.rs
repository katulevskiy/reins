//! Microsoft Teams (Task 9).

use serde_json::Value;

use super::{ApprovalChannel, Message};

#[derive(Clone)]
pub struct TeamsConfig;

impl TeamsConfig {
    #[must_use]
    pub fn from_value(_v: &Value) -> Option<Self> {
        None
    }
}

pub struct Teams;

impl Teams {
    #[must_use]
    pub fn new(_cfg: TeamsConfig) -> Self {
        Self
    }
}

#[rocket::async_trait]
impl ApprovalChannel for Teams {
    async fn link(&self, _email: &str) -> Result<String, String> {
        Err("Teams isn't set up".to_owned())
    }

    async fn post(&self, _to: &str, _msg: &Message, _url: Option<&str>) -> Result<String, String> {
        Err("Teams isn't set up".to_owned())
    }

    async fn update(&self, _external_ref: &str, _outcome: &str) -> Result<(), String> {
        Err("Teams isn't set up".to_owned())
    }
}

pub fn routes() -> Vec<rocket::Route> {
    Vec::new()
}
