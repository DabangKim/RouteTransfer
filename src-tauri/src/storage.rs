use crate::{model::*, network::Route};
use anyhow::{Context, Result, ensure};
use russh_sftp::{
    client::error::Error as SftpError,
    protocol::{OpenFlags, StatusCode},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};
use tokio::io::{AsyncRead, AsyncWrite};
pub trait ReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ReadWrite for T {}
pub type Stream = Box<dyn ReadWrite>;
#[derive(Clone)]
pub enum Store {
    Local,
    Remote(Arc<Route>),
}
pub fn local_meta(m: std::fs::Metadata) -> Meta {
    #[cfg(windows)]
    let link = {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let link = m.file_type().is_symlink();
    Meta {
        kind: if link {
            "link"
        } else if m.is_dir() {
            "dir"
        } else if m.is_file() {
            "file"
        } else {
            "other"
        }
        .into(),
        size: m.len(),
        modified: m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos().min(u64::MAX as u128) as u64),
    }
}
fn remote_meta(m: russh_sftp::protocol::FileAttributes) -> Meta {
    Meta {
        kind: if m.is_symlink() {
            "link"
        } else if m.is_dir() {
            "dir"
        } else if m.is_regular() {
            "file"
        } else {
            "other"
        }
        .into(),
        size: m.size.unwrap_or(0),
        modified: m.mtime.map(|s| s as u64),
    }
}
impl Store {
    pub fn validate_path(&self, p: &str) -> Result<()> {
        ensure!(!p.contains('\0'), "잘못된 경로");
        match self {
            Self::Local => ensure!(
                Path::new(p).is_absolute()
                    && !Path::new(p)
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir)),
                "상위 이동 없는 절대 경로를 입력하세요"
            ),
            Self::Remote(_) => ensure!(
                p.starts_with('/') && !p.split('/').any(|s| s == ".."),
                "상위 이동 없는 절대 경로를 입력하세요"
            ),
        };
        Ok(())
    }

    pub async fn meta(&self, p: &str) -> Result<Option<Meta>> {
        match self {
            Self::Local => match tokio::fs::symlink_metadata(p).await {
                Ok(m) => Ok(Some(local_meta(m))),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            },
            Self::Remote(r) => match r.sftp.symlink_metadata(p).await {
                Ok(m) => Ok(Some(remote_meta(m))),
                Err(SftpError::Status(s)) if s.status_code == StatusCode::NoSuchFile => Ok(None),
                Err(e) => Err(e.into()),
            },
        }
    }
    pub async fn list(&self, p: &str) -> Result<Vec<Entry>> {
        self.validate_path(p)?;
        self.check_parents(p).await?;
        ensure!(
            self.meta(p).await?.is_some_and(|m| m.kind == "dir"),
            "폴더가 아니거나 접근할 수 없습니다"
        );
        let mut out = vec![];
        match self {
            Self::Local => {
                let mut rd = tokio::fs::read_dir(p).await?;
                while let Some(e) = rd.next_entry().await? {
                    let name = e
                        .file_name()
                        .into_string()
                        .map_err(|_| anyhow::anyhow!("UNSUPPORTED_FILENAME_ENCODING"))?;
                    let path = e
                        .path()
                        .to_str()
                        .context("UNSUPPORTED_FILENAME_ENCODING")?
                        .to_string();
                    let meta = self.meta(&path).await?.context("목록 조회 중 파일 변경")?;
                    out.push(Entry { name, path, meta });
                }
            }
            Self::Remote(r) => {
                for e in r.sftp.read_dir(p).await? {
                    let name = e.file_name();
                    if name == "." || name == ".." {
                        continue;
                    }
                    ensure!(
                        !name.contains(['\0', '/']) && !name.contains('\u{fffd}'),
                        "UNSUPPORTED_FILENAME_ENCODING"
                    );
                    out.push(Entry {
                        path: self.join(p, &name)?,
                        name,
                        meta: remote_meta(e.metadata()),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }
    pub fn join(&self, base: &str, name: &str) -> Result<String> {
        ensure!(
            !name.is_empty() && name != "." && name != ".." && !name.contains(['\0', '/']),
            "INVALID_DESTINATION_NAME"
        );
        match self {
            Self::Remote(_) => Ok(format!("{}/{}", base.trim_end_matches('/'), name)),
            Self::Local => {
                validate_local_name(name)?;
                Ok(Path::new(base)
                    .join(name)
                    .to_str()
                    .context("잘못된 경로")?
                    .into())
            }
        }
    }
    pub async fn check_parents(&self, p: &str) -> Result<()> {
        self.validate_path(p)?;
        let mut current = match self {
            Self::Local => Path::new(p)
                .parent()
                .map(|x| x.to_string_lossy().to_string()),
            Self::Remote(_) => p
                .rsplit_once('/')
                .map(|(s, _)| if s.is_empty() { "/".into() } else { s.into() }),
        };
        while let Some(s) = current {
            if let Some(m) = self.meta(&s).await? {
                ensure!(
                    m.kind == "dir",
                    "PATH_TYPE_CONFLICT · 부모 폴더 또는 링크: {}",
                    s
                );
            }
            current = match self {
                Self::Local => Path::new(&s)
                    .parent()
                    .map(|x| x.to_string_lossy().to_string()),
                Self::Remote(_) => {
                    if s == "/" {
                        None
                    } else {
                        s.rsplit_once('/')
                            .map(|(a, _)| if a.is_empty() { "/".into() } else { a.into() })
                    }
                }
            };
        }
        Ok(())
    }
    pub async fn mkdir(&self, p: &str) -> Result<()> {
        self.check_parents(p).await?;
        if let Some(m) = self.meta(p).await? {
            ensure!(m.kind == "dir", "PATH_TYPE_CONFLICT");
            return Ok(());
        }
        match self {
            Self::Local => tokio::fs::create_dir(p).await?,
            Self::Remote(r) => r.sftp.create_dir(p).await?,
        };
        Ok(())
    }
    pub async fn read(&self, p: &str) -> Result<Stream> {
        match self {
            Self::Local => Ok(Box::new(tokio::fs::File::open(p).await?)),
            Self::Remote(r) => Ok(Box::new(r.sftp.open(p).await?)),
        }
    }
    pub async fn create(&self, p: &str) -> Result<Stream> {
        self.check_parents(p).await?;
        match self {
            Self::Local => Ok(Box::new(
                tokio::fs::OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .write(true)
                    .open(p)
                    .await?,
            )),
            Self::Remote(r) => Ok(Box::new(
                r.sftp
                    .open_with_flags(p, OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE)
                    .await?,
            )),
        }
    }
    pub async fn remove_temp(&self, p: &str) -> Result<()> {
        ensure!(
            Path::new(p)
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(".routetransfer-")),
            "임시 파일 경로 불일치"
        );
        self.check_parents(p).await?;
        if let Some(m) = self.meta(p).await? {
            ensure!(m.kind == "file", "임시 파일이 변경되었습니다");
            match self {
                Self::Local => tokio::fs::remove_file(p).await?,
                Self::Remote(r) => r.sftp.remove_file(p).await?,
            }
        }
        Ok(())
    }
    pub async fn commit(&self, temp: &str, target: &str, replace: bool) -> Result<()> {
        self.check_parents(target).await?;
        match self {
            Self::Remote(r) => {
                if replace {
                    r.replace(temp, target).await?
                } else {
                    r.sftp.rename(temp, target).await?
                }
            }
            Self::Local => {
                if replace {
                    local_replace(temp, target)?;
                } else {
                    tokio::fs::hard_link(temp, target).await?;
                    tokio::fs::remove_file(temp).await?;
                }
            }
        }
        Ok(())
    }
}
fn validate_local_name(name: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let stem = name.split('.').next().unwrap_or("").to_uppercase();
        ensure!(
            !name.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])
                && !name.ends_with([' ', '.'])
                && !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "INVALID_DESTINATION_NAME"
        );
    }
    ensure!(!name.contains('\0'), "INVALID_DESTINATION_NAME");
    Ok(())
}
#[cfg(not(windows))]
fn local_replace(temp: &str, target: &str) -> Result<()> {
    std::fs::rename(temp, target)?;
    Ok(())
}
#[cfg(windows)]
fn local_replace(temp: &str, target: &str) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    let a: Vec<u16> = std::ffi::OsStr::new(target)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let b: Vec<u16> = std::ffi::OsStr::new(temp)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        ReplaceFileW(
            a.as_ptr(),
            b.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    ensure!(
        result != 0,
        "COMMIT_UNCERTAIN · {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
pub fn basename(p: &str) -> Result<String> {
    Ok(PathBuf::from(p)
        .file_name()
        .and_then(|s| s.to_str())
        .context("루트 폴더는 직접 선택할 수 없습니다")?
        .into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn overwrite_replaces_target_and_removes_temp() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let temp = root.join("replacement");
        let target = root.join("existing.jpg");
        std::fs::write(&temp, b"complete new photo").unwrap();
        std::fs::write(&target, b"old photo").unwrap();
        Store::Local
            .commit(temp.to_str().unwrap(), target.to_str().unwrap(), true)
            .await
            .unwrap();
        assert_eq!(std::fs::read(target).unwrap(), b"complete new photo");
        assert!(!temp.exists());
    }
    #[cfg(windows)]
    #[test]
    fn rejects_windows_reserved_names() {
        for name in [
            "CON",
            "aux.jpg",
            "COM1.png",
            "name:stream",
            "trailing.",
            "trailing ",
            "a\\b",
        ] {
            assert!(validate_local_name(name).is_err(), "{name}");
        }
        assert!(validate_local_name("사진 01.jpg").is_ok());
    }
    #[tokio::test]
    async fn no_replace_preserves_existing() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().canonicalize().unwrap().join("temp");
        let b = d.path().canonicalize().unwrap().join("target");
        std::fs::write(&a, b"new").unwrap();
        std::fs::write(&b, b"old").unwrap();
        assert!(
            Store::Local
                .commit(a.to_str().unwrap(), b.to_str().unwrap(), false)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(b).unwrap(), b"old");
    }
    #[tokio::test]
    async fn exclusive_temp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().canonicalize().unwrap().join("same");
        let p = p.to_str().unwrap();
        let _f = Store::Local.create(p).await.unwrap();
        assert!(Store::Local.create(p).await.is_err());
    }
}
