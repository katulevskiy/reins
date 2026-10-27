//! TXT lookups for domain verification. Production asks the system resolver (hickory); the end-to-end tests, which
//! cannot publish DNS records, write the answers to the file `REINS_TEST_DNS_TXT_FILE` names (`name=value` lines,
//! read at every lookup), honoured only together with the loopback test switch (`outbound::ALLOW_LOOPBACK_ENV`),
//! which the server warns about at launch.

use std::sync::{Arc, LazyLock};

use hickory_resolver::{TokioResolver, net::runtime::TokioRuntimeProvider};

/// The TXT strings published at a name (each record's strings joined, as RFC 7208 reads them).
#[rocket::async_trait]
pub trait TxtResolver: Send + Sync {
    async fn txt(&self, name: &str) -> Result<Vec<String>, String>;
}

pub struct HickoryTxt(TokioResolver);

#[rocket::async_trait]
impl TxtResolver for HickoryTxt {
    async fn txt(&self, name: &str) -> Result<Vec<String>, String> {
        match self.0.txt_lookup(name).await {
            Ok(lookup) => Ok(lookup
                .answers()
                .iter()
                .filter_map(|record| match &record.data {
                    hickory_resolver::proto::rr::RData::TXT(txt) => {
                        Some(txt.txt_data.iter().map(|part| String::from_utf8_lossy(part)).collect::<String>())
                    }
                    _ => None,
                })
                .collect()),
            Err(e) if e.is_no_records_found() => Ok(Vec::new()),
            Err(e) => Err(format!("DNS lookup failed: {e}")),
        }
    }
}

/// Fixed answers (tests).
pub struct StaticTxt(pub Vec<(String, String)>);

impl StaticTxt {
    /// `name=value;name=value`.
    #[must_use]
    pub fn parse(spec: &str) -> Self {
        Self(
            spec.split(';')
                .filter_map(|pair| pair.split_once('='))
                .map(|(n, v)| (n.trim().trim_end_matches('.').to_ascii_lowercase(), v.trim().to_owned()))
                .collect(),
        )
    }
}

#[rocket::async_trait]
impl TxtResolver for StaticTxt {
    async fn txt(&self, name: &str) -> Result<Vec<String>, String> {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        Ok(self.0.iter().filter(|(n, _)| *n == name).map(|(_, v)| v.clone()).collect())
    }
}

/// The variable the end-to-end tests set (see the module docs).
pub const TEST_TXT_FILE_ENV: &str = "REINS_TEST_DNS_TXT_FILE";

/// TXT records from a file of `name=value` lines, read at every lookup (tests).
pub struct FileTxt(std::path::PathBuf);

#[rocket::async_trait]
impl TxtResolver for FileTxt {
    async fn txt(&self, name: &str) -> Result<Vec<String>, String> {
        let text = std::fs::read_to_string(&self.0).unwrap_or_default();
        StaticTxt::parse(&text.replace('\n', ";")).txt(name).await
    }
}

static RESOLVER: LazyLock<Arc<dyn TxtResolver>> = LazyLock::new(|| {
    if super::super::outbound::allow_loopback()
        && let Ok(path) = std::env::var(TEST_TXT_FILE_ENV)
    {
        warn!(
            "`{TEST_TXT_FILE_ENV}` is set: domain verification reads TXT records from a file. This is for tests only"
        );
        return Arc::new(FileTxt(path.into()));
    }
    match TokioResolver::builder(TokioRuntimeProvider::default()).and_then(|b| b.build()) {
        Ok(r) => Arc::new(HickoryTxt(r)),
        Err(e) => {
            error!("No DNS resolver for domain verification: {e:?}");
            Arc::new(StaticTxt(Vec::new()))
        }
    }
});

/// The resolver domain verification uses.
pub fn resolver() -> Arc<dyn TxtResolver> {
    Arc::clone(&RESOLVER)
}
