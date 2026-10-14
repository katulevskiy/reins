//! `reins secrets leases` and `reins secrets revoke`: what the daemon holds (never a value), and dropping it.

use crate::leases::{LeaseInfo, RevokeRequest};
use crate::secrets::check_name;

/// One line per lease: what, for which command, who, where, how long still.
#[must_use]
pub fn lines(leases: &[LeaseInfo], now: i64) -> Vec<String> {
    if leases.is_empty() {
        return vec!["No secrets held.".to_owned()];
    }
    leases
        .iter()
        .map(|l| {
            let vars = if l.all {
                "all".to_owned()
            } else {
                l.variables.join(" ")
            };
            format!(
                "{}  {}/{}  {}  {}  {}  {}  {} left",
                l.id,
                l.project,
                l.env,
                l.command,
                vars,
                l.runner,
                l.root,
                left(l.expires_at - now)
            )
        })
        .collect()
}

fn left(secs: i64) -> String {
    match secs.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3_600 => format!("{}m", s / 60),
        s => format!("{}h{:02}m", s / 3_600, (s % 3_600) / 60),
    }
}

/// What `revoke` drops: `--all`, `--id` (a lease), `PROJECT/ENV`, `ENV` of `project` (the project here), or with
/// nothing named, the project here.
pub fn request(
    what: Option<&str>,
    id: Option<&str>,
    all: bool,
    project: Option<&str>,
) -> Result<RevokeRequest, String> {
    match (what, id, all) {
        (None, None, true) => Ok(RevokeRequest {
            all: true,
            ..RevokeRequest::default()
        }),
        (None, Some(id), false) => Ok(RevokeRequest {
            id: Some(id.to_owned()),
            ..RevokeRequest::default()
        }),
        (None, None, false) => match project {
            Some(p) => Ok(RevokeRequest {
                project: Some(p.to_owned()),
                ..RevokeRequest::default()
            }),
            None => Err("name what to drop (ENV, PROJECT/ENV, --id ID), or --all".to_owned()),
        },
        (Some(w), None, false) => {
            let (p, e) = match w.split_once('/') {
                Some((p, e)) => (p, e),
                None => (project.ok_or_else(|| format!("{w}: of which project? Write PROJECT/{w}"))?, w),
            };
            check_name("project", p)?;
            check_name("environment", e)?;
            Ok(RevokeRequest {
                project: Some(p.to_owned()),
                env: Some(e.to_owned()),
                ..RevokeRequest::default()
            })
        }
        _ => Err("name one thing to drop: ENV, PROJECT/ENV, --id ID or --all".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leases_are_listed_without_values() {
        let l = LeaseInfo {
            id: "a1b2c3d4".into(),
            project: "acme".into(),
            env: "development".into(),
            runner: "agent:codex".into(),
            root: "/home/me/acme".into(),
            command: "/usr/bin/npm run *".into(),
            variables: vec!["API_KEY".into()],
            all: false,
            revision: "r".into(),
            created_at: 0,
            expires_at: 3_700,
        };
        assert_eq!(
            lines(&[l], 0),
            ["a1b2c3d4  acme/development  /usr/bin/npm run *  API_KEY  agent:codex  /home/me/acme  1h01m left"]
        );
        assert_eq!(lines(&[], 0), ["No secrets held."]);
    }

    #[test]
    fn revoke_names_what_it_drops() {
        assert!(request(None, None, true, None).unwrap().all);
        assert!(request(Some("x"), None, true, None).is_err());
        assert!(request(None, Some("id"), true, None).is_err());
        assert_eq!(request(None, None, false, Some("acme")).unwrap().project.as_deref(), Some("acme"));
        assert!(request(None, None, false, None).is_err());
        let r = request(Some("acme/production"), None, false, None).unwrap();
        assert_eq!((r.project.as_deref(), r.env.as_deref()), (Some("acme"), Some("production")));
        let r = request(Some("staging"), None, false, Some("acme")).unwrap();
        assert_eq!((r.project.as_deref(), r.env.as_deref()), (Some("acme"), Some("staging")));
        assert!(request(Some("staging"), None, false, None).is_err());
        assert!(request(Some("Acme/x"), None, false, None).is_err());
        assert_eq!(request(None, Some("Xy_9"), false, None).unwrap().id.as_deref(), Some("Xy_9"));
    }
}
