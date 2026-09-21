use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Hop {
    pub id: String,
    pub alias: String,
    pub host: String,
    pub port: u16,
    pub username: String,
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub description: String,
    pub remote_path: String,
    pub revision: u64,
    pub hops: Vec<Hop>,
}
impl Profile {
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.trim().is_empty(), "프로필 이름이 필요합니다");
        ensure!(!self.hops.is_empty(), "서버를 한 개 이상 등록하세요");
        let mut ids = std::collections::HashSet::new();
        for h in &self.hops {
            ensure!(ids.insert(&h.id), "서버 ID 중복");
            ensure!(
                !h.host.trim().is_empty() && !h.username.trim().is_empty() && h.port > 0,
                "서버 주소·사용자·포트를 확인하세요"
            );
            ensure!(
                !h.host.contains(['\0', '\n', '/']) && !h.username.contains('\0'),
                "잘못된 서버 정보"
            );
        }
        ensure!(
            self.remote_path.starts_with('/') && !self.remote_path.contains('\0'),
            "원격 경로는 절대 경로여야 합니다"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub connect_timeout: u64,
    pub auth_timeout: u64,
    pub io_timeout: u64,
    pub local_path: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            connect_timeout: 30,
            auth_timeout: 30,
            io_timeout: 120,
            local_path: std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_else(|_| "/".into()),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (10..=300).contains(&self.connect_timeout)
                && (10..=300).contains(&self.auth_timeout)
                && (30..=600).contains(&self.io_timeout),
            "시간 제한 범위를 확인하세요"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Meta {
    pub kind: String,
    pub size: u64,
    pub modified: Option<u64>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub meta: Meta,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Batch {
    pub id: String,
    pub profile: Profile,
    pub direction: String,
    pub destination: String,
    pub state: String,
    pub policy: String,
    pub created_at: i64,
    pub issue_count: u64,
    pub revision: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub batch_id: String,
    pub source: String,
    pub target: String,
    pub meta: Meta,
    pub target_meta: Option<Meta>,
    pub state: String,
    pub error: String,
    pub attempt: u64,
    pub temp: Option<String>,
    #[serde(default)]
    pub temp_owned: bool,
    pub bytes: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Challenge {
    pub id: String,
    pub kind: String,
    pub server: String,
    pub fingerprint: Option<String>,
    pub previous: Option<String>,
}
#[derive(Deserialize)]
pub struct Answer {
    pub accepted: bool,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub save: bool,
}
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct RuntimeView {
    pub state: String,
    pub profile: Option<Profile>,
    pub hop: Option<usize>,
    pub operation: Option<String>,
    pub error: Option<String>,
    pub challenge: Option<Challenge>,
    pub current_item: Option<String>,
    pub bytes: String,
}
