use crate::{core::Core, model::*};
use anyhow::{Context, Result, ensure};
use russh::{
    Disconnect, client,
    keys::{HashAlg, PublicKeyOrCertificate},
};
use russh_sftp::{
    client::{RawSftpSession, SftpSession},
    protocol::{Packet, StatusCode},
};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Weak};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;
pub fn scope(p: &Profile, index: usize) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(&p.id, &p.hops[..=index])).unwrap())
    )
}
pub struct Verify {
    core: Weak<Core>,
    scope: String,
    server: String,
    cancel: CancellationToken,
}
impl client::Handler for Verify {
    type Error = anyhow::Error;
    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool> {
        ensure!(
            key.certificate().is_none(),
            "SSH 인증서는 현재 지원하지 않습니다"
        );
        let core = self.core.upgrade().context("앱 종료")?;
        let fp = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let old = core.db.get::<String>("trusted", &self.scope)?;
        if old.as_ref() == Some(&fp) {
            return Ok(true);
        }
        if old.is_some() {
            core.update(|v| {
                v.error = Some(format!(
                    "HOST_KEY_CHANGED · {} · 저장: {} · 현재: {}",
                    self.server,
                    old.unwrap(),
                    fp
                ))
            });
            return Ok(false);
        }
        let answer = core
            .ask(
                Challenge {
                    id: id(),
                    kind: "host".into(),
                    server: self.server.clone(),
                    fingerprint: Some(fp.clone()),
                    previous: None,
                },
                &self.cancel,
            )
            .await?;
        if answer.accepted {
            core.db.put("trusted", &self.scope, &fp)?;
            core.db.audit("서버 키 신뢰", &self.server, "허용")?;
        }
        Ok(answer.accepted)
    }
}
pub struct Route {
    pub profile: Profile,
    pub sessions: Vec<client::Handle<Verify>>,
    pub sftp: Arc<SftpSession>,
}
impl Route {
    pub async fn connect(core: Arc<Core>, p: Profile, cancel: CancellationToken) -> Result<Self> {
        let settings = core.settings()?;
        let mut sessions: Vec<client::Handle<Verify>> = vec![];
        for (index, hop) in p.hops.iter().enumerate() {
            core.update(|v| {
                v.hop = Some(index);
                v.state = "connecting".into()
            });
            let server = format!("{} · {}@{}:{}", hop.alias, hop.username, hop.host, hop.port);
            let sk = scope(&p, index);
            let handler = Verify {
                core: Arc::downgrade(&core),
                scope: sk.clone(),
                server: server.clone(),
                cancel: cancel.clone(),
            };
            let config = Arc::new(client::Config::default());
            let connect = async {
                if let Some(prev) = sessions.last() {
                    let ch = prev
                        .channel_open_direct_tcpip(
                            hop.host.clone(),
                            hop.port as u32,
                            "127.0.0.1",
                            0,
                        )
                        .await
                        .context("FORWARD_DENIED")?;
                    Ok::<_, anyhow::Error>(
                        client::connect_stream(config, ch.into_stream(), handler).await?,
                    )
                } else {
                    Ok(client::connect(config, (hop.host.as_str(), hop.port), handler).await?)
                }
            };
            let mut session = match core
                .network_wait(connect, settings.connect_timeout, &cancel)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    for s in sessions.iter().rev() {
                        let _ = s.disconnect(Disconnect::ByApplication, "failed", "").await;
                    }
                    return Err(e.context(format!("서버 {} 연결 실패", index + 1)));
                }
            };
            let saved = if core
                .db
                .get::<serde_json::Value>("credentials", &sk)?
                .is_some()
            {
                let k = sk.clone();
                tokio::task::spawn_blocking(move || {
                    keyring::Entry::new("RouteTransfer", &k).and_then(|e| e.get_password())
                })
                .await?
                .ok()
            } else {
                None
            };
            let (password, save) = if let Some(pw) = saved {
                (Zeroizing::new(pw), false)
            } else {
                let a = core
                    .ask(
                        Challenge {
                            id: id(),
                            kind: "password".into(),
                            server: server.clone(),
                            fingerprint: None,
                            previous: None,
                        },
                        &cancel,
                    )
                    .await?;
                ensure!(a.accepted, "취소됨");
                (Zeroizing::new(a.password), a.save)
            };
            core.update(|v| v.state = "authenticating".into());
            let auth = core
                .network_wait(
                    async {
                        Ok(session
                            .authenticate_password(hop.username.clone(), password.as_str())
                            .await?
                            .success())
                    },
                    settings.auth_timeout,
                    &cancel,
                )
                .await?;
            ensure!(auth, "AUTH_REJECTED · 서버 {}", index + 1);
            if save {
                let k = sk.clone();
                let pw = Zeroizing::new(password.to_string());
                let result = tokio::task::spawn_blocking(move || {
                    keyring::Entry::new("RouteTransfer", &k).and_then(|e| e.set_password(&pw))
                })
                .await?;
                match result {
                    Ok(()) => core.db.put(
                        "credentials",
                        &sk,
                        &serde_json::json!({"id":sk,"server":server}),
                    )?,
                    Err(_) => core.update(|v| {
                        v.error = Some("비밀번호 저장 실패 · 이번 연결에만 사용합니다".into())
                    }),
                };
            }
            sessions.push(session);
        }
        core.update(|v| v.state = "opening_sftp".into());
        let sftp = core
            .network_wait(
                async {
                    let ch = sessions.last().unwrap().channel_open_session().await?;
                    ch.request_subsystem(true, "sftp").await?;
                    Ok(SftpSession::new(ch.into_stream()).await?)
                },
                settings.connect_timeout,
                &cancel,
            )
            .await?;
        sftp.set_timeout(settings.io_timeout);
        sftp.read_dir(p.remote_path.clone())
            .await
            .context("기본 원격 폴더 조회 실패")?;
        Ok(Self {
            profile: p,
            sessions,
            sftp: Arc::new(sftp),
        })
    }
    pub async fn close(&self) {
        let _ = self.sftp.close().await;
        for s in self.sessions.iter().rev() {
            let _ = s.disconnect(Disconnect::ByApplication, "closed", "").await;
        }
    }
    pub async fn replace(&self, source: &str, target: &str) -> Result<()> {
        let ch = self
            .sessions
            .last()
            .context("연결 없음")?
            .channel_open_session()
            .await?;
        ch.request_subsystem(true, "sftp").await?;
        let raw = RawSftpSession::new(ch.into_stream());
        let version = raw.init().await?;
        ensure!(
            version
                .extensions
                .get("posix-rename@openssh.com")
                .map(String::as_str)
                == Some("1"),
            "SAFE_REPLACE_UNSUPPORTED"
        );
        let mut data = vec![];
        for p in [source, target] {
            let b = p.as_bytes();
            data.extend_from_slice(&(u32::try_from(b.len())?).to_be_bytes());
            data.extend_from_slice(b);
        }
        let r = raw.extended("posix-rename@openssh.com", data).await?;
        let _ = raw.close_session();
        ensure!(
            matches!(r,Packet::Status(ref s)if s.status_code==StatusCode::Ok),
            "COMMIT_UNCERTAIN"
        );
        Ok(())
    }
    pub fn lost(&self) -> bool {
        self.sessions.iter().any(|s| s.is_closed())
    }
}
