//! Stores only ciphertext. Reads are private to the authenticated user; writes require the approval device and CAS.
use super::device_api::{DeviceKey, PhoneResult, api_err, bad_request, read_body_limited, require_approval_device};
use crate::{CONFIG, auth::Headers, db::DbConn};
use reins_proto::account_state::{AccountState, AccountStateUpdate, MAX_CIPHERTEXT_BYTES};
use rocket::{Data, Route, http::Status, serde::json::Json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn routes() -> Vec<Route> {
    routes![get_state, put_state]
}
fn directory() -> PathBuf {
    PathBuf::from(CONFIG.data_folder()).join("reins-account-state")
}
fn path(directory: &Path, user: &str) -> PathBuf {
    // Only a server-owned, authenticated user id reaches this function; hash it to avoid path interpretation.
    directory.join(format!("{}.json", crate::crypto::sha256_hex(user.as_bytes())))
}
fn load(path: &Path) -> Result<AccountState, std::io::Error> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AccountState {
            revision: 0,
            ciphertext: None,
        }),
        Err(e) => Err(e),
    }
}
fn save(directory: &Path, user: &str, update: AccountStateUpdate) -> Result<AccountState, Status> {
    fs::create_dir_all(directory).map_err(|_| Status::InternalServerError)?;
    let path = path(directory, user);
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
        .map_err(|_| Status::InternalServerError)?;
    lock.lock().map_err(|_| Status::InternalServerError)?;
    let current = load(&path).map_err(|_| Status::InternalServerError)?;
    if current.revision != update.revision {
        return Err(Status::Conflict);
    }
    let revision = current.revision.checked_add(1).ok_or(Status::InternalServerError)?;
    let next = AccountState {
        revision,
        ciphertext: Some(update.ciphertext),
    };
    let bytes = serde_json::to_vec(&next).map_err(|_| Status::InternalServerError)?;
    let mut file = fs::File::create(path.with_extension("tmp")).map_err(|_| Status::InternalServerError)?;
    file.write_all(&bytes).and_then(|()| file.sync_all()).map_err(|_| Status::InternalServerError)?;
    fs::rename(path.with_extension("tmp"), &path).map_err(|_| Status::InternalServerError)?;
    fs::File::open(directory).and_then(|f| f.sync_all()).map_err(|_| Status::InternalServerError)?;
    Ok(next)
}
/// Deletes `user`'s stored state and its lock file (a deleted account); nothing stored is no error.
pub fn forget(user: &str) -> std::io::Result<()> {
    remove_state(&directory(), user)
}
fn remove_state(directory: &Path, user: &str) -> std::io::Result<()> {
    let path = path(directory, user);
    for file in [path.with_extension("tmp"), path.clone(), path.with_extension("lock")] {
        match fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}
#[get("/reins/api/account-state")]
async fn get_state(headers: Headers) -> PhoneResult<Json<AccountState>> {
    let path = path(&directory(), &headers.user.uuid.to_string());
    let state = tokio::task::spawn_blocking(move || load(&path))
        .await
        .map_err(|_| api_err(Status::InternalServerError, "state_unavailable", "Account state unavailable"))?
        .map_err(|_| api_err(Status::InternalServerError, "state_unavailable", "Account state unavailable"))?;
    Ok(Json(state))
}
#[put("/reins/api/account-state", data = "<data>")]
async fn put_state(headers: Headers, key: DeviceKey, data: Data<'_>, conn: DbConn) -> PhoneResult<Json<AccountState>> {
    require_approval_device(&headers, &key, &conn).await?;
    let bytes = read_body_limited(data, MAX_CIPHERTEXT_BYTES as u64 + 1024).await?;
    let update: AccountStateUpdate =
        serde_json::from_slice(&bytes).map_err(|_| bad_request("Invalid encrypted account state"))?;
    if update.ciphertext.len() > MAX_CIPHERTEXT_BYTES
        || update.ciphertext.is_empty()
        || data_encoding::BASE64URL_NOPAD.decode(update.ciphertext.as_bytes()).is_err()
    {
        return Err(bad_request("Invalid encrypted account state"));
    }
    let dir = directory();
    let user = headers.user.uuid.to_string();
    let state = tokio::task::spawn_blocking(move || save(&dir, &user, update))
        .await
        .map_err(|_| api_err(Status::InternalServerError, "state_unavailable", "Account state unavailable"))?
        .map_err(|status| {
            api_err(
                status,
                if status == Status::Conflict {
                    "state_conflict"
                } else {
                    "state_unavailable"
                },
                "Account state changed; refresh before retrying",
            )
        })?;
    Ok(Json(state))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_and_compare_and_swap() {
        let dir = std::env::temp_dir().join(format!("reins-account-state-test-{}", crate::util::get_uuid()));
        let a = save(
            &dir,
            "a",
            AccountStateUpdate {
                revision: 0,
                ciphertext: "opaque-a".to_owned(),
            },
        )
        .unwrap();
        assert_eq!(a.revision, 1);
        assert!(load(&path(&dir, "b")).unwrap().ciphertext.is_none());
        assert_eq!(
            save(
                &dir,
                "a",
                AccountStateUpdate {
                    revision: 0,
                    ciphertext: "stale".to_owned()
                }
            )
            .unwrap_err(),
            Status::Conflict
        );
        assert_eq!(load(&path(&dir, "a")).unwrap().ciphertext.as_deref(), Some("opaque-a"));
        remove_state(&dir, "a").unwrap();
        remove_state(&dir, "a").unwrap();
        assert!(load(&path(&dir, "a")).unwrap().ciphertext.is_none());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "the lock file goes too");
        fs::remove_dir_all(dir).unwrap();
    }
}
