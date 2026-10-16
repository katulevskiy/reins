//! The joins that decide a conflict between two versions of a security record, whatever the clocks say: each result
//! is never more permissive than either side. The snapshot merge (`merge.rs`) and the op-log store (`oplog.rs`) use the
//! same functions, so a record merges alike whichever way it travelled.

use std::collections::BTreeSet;

use reins_policy::Grant;
use serde_json::Value as Json;

/// The tightest of some limits, where `None` is no limit.
fn tightest<T: Ord + Copy>(limits: impl IntoIterator<Item = Option<T>>) -> Option<T> {
    limits.into_iter().flatten().min()
}

/// Every version of one grant (the base's too, when there is one) joined: revoked in any version, revoked; a scope,
/// account or connection that differs between versions, revoked (neither can be shown to be the narrower); otherwise
/// the earliest expiry, the smallest use limit and every use counted (with a base, the uses each side added since it,
/// summed; without one, the larger count). The result keeps `newer`'s other fields.
pub(crate) fn join_grants(newer: &Grant, other: &Grant, base: Option<&Grant>) -> Grant {
    let all: Vec<&Grant> = [Some(newer), Some(other), base].into_iter().flatten().collect();
    let mut merged = newer.clone();
    merged.revoked = all.iter().any(|g| g.revoked)
        || all.iter().any(|g| {
            g.id != newer.id
                || g.connection_id != newer.connection_id
                || g.scope != newer.scope
                || g.account != newer.account
        });
    merged.expires_at = tightest(all.iter().map(|g| g.expires_at));
    merged.max_uses = tightest(all.iter().map(|g| g.max_uses));
    let base_uses = base.map_or(0, |b| b.uses);
    let added = if base.is_some() {
        other.uses.saturating_add(newer.uses).saturating_sub(base_uses)
    } else {
        other.uses.max(newer.uses)
    };
    merged.uses = added.max(other.uses).max(newer.uses).max(base_uses);
    merged
}

/// How much an Autopilot mode lets through without asking: lower is more restrictive. No mode is the default
/// (manual).
pub(crate) fn permissiveness(mode: Option<&str>) -> u8 {
    match mode {
        Some("lockdown") => 0,
        Some("auto") => 2,
        Some("overnight") => 3,
        Some("bypass") => 4,
        _ => 1,
    }
}

/// Two versions' bypass ends joined: stopped (`None`) on either side stays stopped, else the earlier end.
pub(crate) fn join_bypass_until(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        _ => None,
    }
}

/// A setting kept as JSON merged by its entries: an object key by key (changed on one side: that side's; on both, or
/// deleted on one: `newer`'s), a list by its items' `id` (or the whole item: `newer`'s items but those `other` deleted
/// since `base`, then the ones `other` added). With `asking_wins` (always-ask settings), `true` on either side wins.
/// `None` when the two are not both objects or both lists.
pub(crate) fn merge_json(base: Option<&Json>, other: &Json, newer: &Json, asking_wins: bool) -> Option<Json> {
    match (other, newer) {
        (Json::Object(l), Json::Object(s)) => {
            let empty = serde_json::Map::new();
            let b = match base {
                Some(Json::Object(b)) => b,
                _ => &empty,
            };
            let mut out = serde_json::Map::new();
            let keys: BTreeSet<&String> = l.keys().chain(s.keys()).collect();
            for k in keys {
                let (bv, lv, sv) = (b.get(k), l.get(k), s.get(k));
                let asks = asking_wins && (lv == Some(&Json::Bool(true)) || sv == Some(&Json::Bool(true)));
                let v = if asks {
                    Some(&Json::Bool(true))
                } else if lv == sv || bv == sv {
                    lv
                } else {
                    sv
                };
                if let Some(v) = v {
                    out.insert(k.clone(), v.clone());
                }
            }
            Some(Json::Object(out))
        }
        (Json::Array(l), Json::Array(s)) => {
            let item_id = |v: &Json| v.get("id").and_then(Json::as_str).map_or_else(|| v.to_string(), str::to_owned);
            let ids = |items: &[Json]| items.iter().map(item_id).collect::<BTreeSet<_>>();
            let b = match base {
                Some(Json::Array(b)) => ids(b),
                _ => BTreeSet::new(),
            };
            let (here, there) = (ids(l), ids(s));
            let mut out: Vec<Json> =
                s.iter().filter(|v| !b.contains(&item_id(v)) || here.contains(&item_id(v))).cloned().collect();
            out.extend(l.iter().filter(|v| !there.contains(&item_id(v)) && !b.contains(&item_id(v))).cloned());
            Some(Json::Array(out))
        }
        _ => None,
    }
}

/// The meta key whose JSON value is "ask every time" per item: asking wins any merge.
pub(crate) const ALWAYS_ASK_KEY: &str = "app.secrets.always-ask";

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> Grant {
        crate::store::grants::tests::read_grant("g", "c1", Some(1_000))
    }

    #[test]
    fn grants_join_to_the_tighter_version() {
        let mut a = grant();
        a.expires_at = Some(500);
        a.uses = 2;
        let mut b = grant();
        b.max_uses = Some(3);
        b.uses = 1;
        let j = join_grants(&a, &b, None);
        assert_eq!((j.expires_at, j.max_uses, j.uses, j.revoked), (Some(500), Some(3), 2, false));
        let mut revoked = grant();
        revoked.revoked = true;
        assert!(join_grants(&a, &revoked, None).revoked && join_grants(&revoked, &a, None).revoked);
        let mut wider = grant();
        wider.scope = reins_policy::Scope::Read(reins_policy::ReadScope::default());
        assert!(join_grants(&a, &wider, None).revoked, "a scope change revokes");
    }

    #[test]
    fn modes_and_bypasses_join_to_the_more_restrictive() {
        assert!(permissiveness(Some("lockdown")) < permissiveness(None));
        assert!(permissiveness(None) < permissiveness(Some("auto")));
        assert!(permissiveness(Some("auto")) < permissiveness(Some("overnight")));
        assert_eq!(join_bypass_until(Some(5), Some(9)), Some(5));
        assert_eq!(join_bypass_until(None, Some(9)), None, "stopped stays stopped");
    }

    #[test]
    fn json_settings_merge_by_entry_and_asking_wins() {
        let merged = merge_json(None, &serde_json::json!({"a": true}), &serde_json::json!({"a": false}), true);
        assert_eq!(merged, Some(serde_json::json!({"a": true})));
        let merged = merge_json(None, &serde_json::json!({"a": 1}), &serde_json::json!({"a": 2, "b": 3}), false);
        assert_eq!(merged, Some(serde_json::json!({"a": 2, "b": 3})));
        assert_eq!(merge_json(None, &serde_json::json!(1), &serde_json::json!({}), false), None);
    }
}
