use crate::model::*;
use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{path::Path, sync::Mutex};
pub struct Db {
    conn: Mutex<Connection>,
    _lock: std::fs::File,
}
impl Db {
    pub fn open(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("instance.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| anyhow::anyhow!("이 데이터 폴더를 사용하는 앱이 이미 실행 중입니다"))?;
        let c = Connection::open(dir.join("routetransfer.sqlite3"))?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(
            version <= 1,
            "더 새로운 앱에서 작성된 데이터입니다. 업데이트 후 실행하세요"
        );
        c.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
 BEGIN IMMEDIATE;
 CREATE TABLE IF NOT EXISTS profiles(id TEXT PRIMARY KEY, data TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS settings(id TEXT PRIMARY KEY,data TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS trusted(id TEXT PRIMARY KEY,data TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS credentials(id TEXT PRIMARY KEY,data TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS batches(id TEXT PRIMARY KEY,data TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS items(id TEXT PRIMARY KEY,batch_id TEXT NOT NULL REFERENCES batches(id),data TEXT NOT NULL);
 CREATE INDEX IF NOT EXISTS item_batch ON items(batch_id);
 CREATE TABLE IF NOT EXISTS attempts(id TEXT PRIMARY KEY,item_id TEXT NOT NULL REFERENCES items(id),data TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS audit(id TEXT PRIMARY KEY,at INTEGER NOT NULL,data TEXT NOT NULL);
 CREATE INDEX IF NOT EXISTS audit_time ON audit(at);
 PRAGMA user_version=1; COMMIT;")?;
        let db = Self {
            conn: Mutex::new(c),
            _lock: lock,
        };
        db.recover()?;
        Ok(db)
    }
    fn table(t: &str) -> Result<&str> {
        ensure!(
            ["profiles", "settings", "trusted", "credentials", "batches"].contains(&t),
            "잘못된 테이블"
        );
        Ok(t)
    }
    pub fn put<T: Serialize>(&self, t: &str, id: &str, v: &T) -> Result<()> {
        let sql = format!(
            "INSERT INTO {}(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            Self::table(t)?
        );
        self.conn
            .lock()
            .unwrap()
            .execute(&sql, params![id, serde_json::to_string(v)?])?;
        Ok(())
    }
    pub fn get<T: DeserializeOwned>(&self, t: &str, id: &str) -> Result<Option<T>> {
        let sql = format!("SELECT data FROM {} WHERE id=?1", Self::table(t)?);
        let s: Option<String> = self
            .conn
            .lock()
            .unwrap()
            .query_row(&sql, [id], |r| r.get(0))
            .optional()?;
        s.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    pub fn all<T: DeserializeOwned>(&self, t: &str) -> Result<Vec<T>> {
        let sql = format!(
            "SELECT data FROM {} ORDER BY rowid DESC LIMIT 500",
            Self::table(t)?
        );
        let c = self.conn.lock().unwrap();
        let mut q = c.prepare(&sql)?;
        let rows = q.query_map([], |r| r.get::<_, String>(0))?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn delete(&self, t: &str, id: &str) -> Result<()> {
        self.conn.lock().unwrap().execute(
            &format!("DELETE FROM {} WHERE id=?1", Self::table(t)?),
            [id],
        )?;
        Ok(())
    }
    pub fn save_profile(&self, p: &mut Profile) -> Result<()> {
        p.validate()?;
        let mut c = self.conn.lock().unwrap();
        let tx = c.transaction()?;
        let old: Option<String> = tx
            .query_row("SELECT data FROM profiles WHERE id=?1", [&p.id], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(s) = old {
            ensure!(
                serde_json::from_str::<Profile>(&s)?.revision == p.revision,
                "프로필이 변경되었습니다. 다시 불러오세요"
            );
        } else {
            ensure!(p.revision == 0, "프로필이 삭제되었습니다");
        }
        p.revision += 1;
        tx.execute("INSERT INTO profiles(id,data) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![p.id,serde_json::to_string(p)?])?;
        tx.commit()?;
        Ok(())
    }
    pub fn add_item(&self, i: &Item) -> Result<()> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO items(id,batch_id,data) VALUES(?1,?2,?3)",
            params![i.id, i.batch_id, serde_json::to_string(i)?],
        )?;
        Ok(())
    }
    pub fn item(&self, id: &str) -> Result<Item> {
        let s: String = self.conn.lock().unwrap().query_row(
            "SELECT data FROM items WHERE id=?1",
            [id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&s)?)
    }
    pub fn items(&self, batch: &str, offset: u64, limit: u64) -> Result<Vec<Item>> {
        let c = self.conn.lock().unwrap();
        let mut q = c.prepare(
            "SELECT data FROM items WHERE batch_id=?1 ORDER BY rowid LIMIT ?2 OFFSET ?3",
        )?;
        let rows = q.query_map(params![batch, limit.min(500), offset], |r| {
            r.get::<_, String>(0)
        })?;
        rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
    }
    pub fn save_item(&self, i: &Item) -> Result<()> {
        let mut c = self.conn.lock().unwrap();
        let tx = c.transaction()?;
        let s = serde_json::to_string(i)?;
        tx.execute("UPDATE items SET data=?2 WHERE id=?1", params![i.id, s])?;
        if i.attempt > 0 {
            tx.execute("INSERT INTO attempts(id,item_id,data) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![format!("{}:{}",i.id,i.attempt),i.id,s])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn summary(&self, b: &str) -> Result<Value> {
        let c = self.conn.lock().unwrap();
        let mut q=c.prepare("SELECT json_extract(data,'$.state'), COUNT(*),COALESCE(SUM(CASE WHEN json_extract(data,'$.meta.kind')='file' THEN json_extract(data,'$.meta.size') ELSE 0 END),0) FROM items WHERE batch_id=?1 GROUP BY json_extract(data,'$.state')")?;
        let mut map = serde_json::Map::new();
        for r in q.query_map([b], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })? {
            let (s, n, size) = r?;
            map.insert(s, json!({"count":n,"bytes":size.to_string()}));
        }
        Ok(Value::Object(map))
    }
    pub fn audit(&self, action: &str, target: &str, result: &str) -> Result<()> {
        let at = now();
        self.conn.lock().unwrap().execute("INSERT INTO audit VALUES(?1,?2,?3)",params![id(),at,json!({"at":at,"actor":std::env::var("USER").or_else(|_|std::env::var("USERNAME")).unwrap_or_default(),"action":action,"target":target,"result":result}).to_string()])?;
        Ok(())
    }
    pub fn audits(&self) -> Result<Vec<Value>> {
        let c = self.conn.lock().unwrap();
        let mut q = c.prepare("SELECT data FROM audit ORDER BY at DESC LIMIT 500")?;
        q.query_map([], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    pub fn attempts(&self, item: &str) -> Result<Vec<Value>> {
        let c = self.conn.lock().unwrap();
        let mut q = c.prepare("SELECT data FROM attempts WHERE item_id=?1 ORDER BY rowid")?;
        q.query_map([item], |r| r.get::<_, String>(0))?
            .map(|r| Ok(serde_json::from_str(&r?)?))
            .collect()
    }
    fn recover(&self) -> Result<()> {
        let c = self.conn.lock().unwrap();
        c.execute_batch("BEGIN IMMEDIATE; UPDATE items SET data=json_set(data,'$.state',CASE WHEN json_extract(data,'$.state')='committing' THEN 'needs_review' ELSE 'interrupted' END) WHERE json_extract(data,'$.state') IN ('transferring','verifying','committing'); UPDATE attempts SET data=json_set(data,'$.state',CASE WHEN json_extract(data,'$.state')='committing' THEN 'needs_review' ELSE 'interrupted' END) WHERE json_extract(data,'$.state') IN ('transferring','verifying','committing'); UPDATE batches SET data=json_set(data,'$.state',CASE WHEN json_extract(data,'$.state')='scanning' THEN 'plan_failed' ELSE 'interrupted' END) WHERE json_extract(data,'$.state') IN ('running','scanning','queued'); COMMIT;")?;
        Ok(())
    }
    pub fn cleanup_ids(&self, kind: &str, before: i64) -> Result<Vec<String>> {
        let c = self.conn.lock().unwrap();
        let sql = if kind == "audit" {
            "SELECT id FROM audit WHERE at<?1 ORDER BY id"
        } else {
            "SELECT id FROM batches WHERE json_extract(data,'$.created_at')<?1 AND json_extract(data,'$.state')='finished' AND NOT EXISTS(SELECT 1 FROM items WHERE batch_id=batches.id AND (json_extract(data,'$.state') NOT IN ('succeeded','skipped') OR json_extract(data,'$.temp') IS NOT NULL)) ORDER BY id"
        };
        let mut q = c.prepare(sql)?;
        let out = q
            .query_map([before], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(out)
    }
    pub fn cleanup(&self, kind: &str, ids: &[String]) -> Result<()> {
        let mut c = self.conn.lock().unwrap();
        let tx = c.transaction()?;
        for id in ids {
            if kind == "audit" {
                tx.execute("DELETE FROM audit WHERE id=?1", [id])?;
            } else {
                tx.execute(
                    "DELETE FROM attempts WHERE item_id IN(SELECT id FROM items WHERE batch_id=?1)",
                    [id],
                )?;
                tx.execute("DELETE FROM items WHERE batch_id=?1", [id])?;
                tx.execute("DELETE FROM batches WHERE id=?1", [id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn future_schema_rejected() {
        let d = tempfile::tempdir().unwrap();
        let c = Connection::open(d.path().join("routetransfer.sqlite3")).unwrap();
        c.execute_batch("PRAGMA user_version=999").unwrap();
        drop(c);
        assert!(Db::open(d.path()).is_err());
    }
    #[test]
    fn single_owner() {
        let d = tempfile::tempdir().unwrap();
        let _db = Db::open(d.path()).unwrap();
        assert!(Db::open(d.path()).is_err());
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    #[test]
    fn recovery_preserves_completed_and_flags_uncertain_commit() {
        let d = tempfile::tempdir().unwrap();
        let db = Db::open(d.path()).unwrap();
        db.put("batches", "b", &json!({"state":"running"})).unwrap();
        db.put("batches", "scan", &json!({"state":"scanning"}))
            .unwrap();
        for state in ["succeeded", "transferring", "committing"] {
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO items VALUES(?1,'b',?2)",
                    params![state, json!({"state":state}).to_string()],
                )
                .unwrap();
        }
        drop(db);
        let db = Db::open(d.path()).unwrap();
        let c = db.conn.lock().unwrap();
        for (old, new) in [
            ("succeeded", "succeeded"),
            ("transferring", "interrupted"),
            ("committing", "needs_review"),
        ] {
            let state: String = c
                .query_row(
                    "SELECT json_extract(data,'$.state') FROM items WHERE id=?1",
                    [old],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(state, new);
        }
        drop(c);
        assert_eq!(
            db.get::<Value>("batches", "scan").unwrap().unwrap()["state"],
            "plan_failed"
        );
    }
}
