use crate::{
    db::Db,
    model::*,
    network::{Route, scope},
    storage::{Store, basename},
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Mutex as AsyncMutex, oneshot},
};
use tokio_util::sync::CancellationToken;
pub struct Core {
    pub db: Db,
    view: Mutex<RuntimeView>,
    route: Mutex<Option<Arc<Route>>>,
    answer: Mutex<Option<(String, oneshot::Sender<Answer>)>>,
    cancel: Mutex<CancellationToken>,
    pub work: Arc<AsyncMutex<()>>,
    cancel_items: Mutex<HashSet<String>>,
    listings: Mutex<HashMap<String, (String, Vec<Entry>)>>,
    cleanup: Mutex<HashMap<String, (String, i64, Vec<String>)>>,
}
impl Core {
    pub fn open(path: &Path) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            db: Db::open(path)?,
            view: Mutex::new(RuntimeView {
                state: "disconnected".into(),
                ..Default::default()
            }),
            route: Mutex::new(None),
            answer: Mutex::new(None),
            cancel: Mutex::new(CancellationToken::new()),
            work: Arc::new(AsyncMutex::new(())),
            cancel_items: Mutex::new(HashSet::new()),
            listings: Mutex::new(HashMap::new()),
            cleanup: Mutex::new(HashMap::new()),
        }))
    }
    pub fn update(&self, f: impl FnOnce(&mut RuntimeView)) {
        f(&mut self.view.lock().unwrap())
    }
    pub fn settings(&self) -> Result<Settings> {
        Ok(self.db.get("settings", "app")?.unwrap_or_default())
    }
    fn token(&self) -> CancellationToken {
        let t = CancellationToken::new();
        *self.cancel.lock().unwrap() = t.clone();
        t
    }
    pub async fn ask(&self, c: Challenge, cancel: &CancellationToken) -> Result<Answer> {
        let (tx, rx) = oneshot::channel();
        *self.answer.lock().unwrap() = Some((c.id.clone(), tx));
        self.update(|v| v.challenge = Some(c));
        let r = tokio::select! {_=cancel.cancelled()=>Err(anyhow::anyhow!("취소됨")),r=rx=>r.map_err(|_|anyhow::anyhow!("인증 요청 만료"))};
        self.answer.lock().unwrap().take();
        self.update(|v| v.challenge = None);
        r
    }
    pub async fn network_wait<T>(
        &self,
        f: impl Future<Output = Result<T>>,
        seconds: u64,
        cancel: &CancellationToken,
    ) -> Result<T> {
        tokio::pin!(f);
        let mut ticks = 0;
        loop {
            tokio::select! {_=cancel.cancelled()=>bail!("취소됨"),r=&mut f=>return r,_=tokio::time::sleep(Duration::from_millis(100))=>{if self.view.lock().unwrap().challenge.is_none(){ticks+=1;}ensure!(ticks<seconds*10,"IO_STALLED · 응답 시간 초과");}}
        }
    }
    fn active(&self) -> Result<Arc<Route>> {
        let r = self
            .route
            .lock()
            .unwrap()
            .clone()
            .context("서버에 먼저 연결하세요")?;
        ensure!(!r.lost(), "CONNECTION_LOST · 재연결이 필요합니다");
        Ok(r)
    }
    pub fn snapshot(&self) -> Result<Value> {
        let idle_guard = self.work.try_lock();
        let busy = idle_guard.is_err();
        let mut view = self.view.lock().unwrap().clone();
        if self
            .route
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|r| r.lost())
        {
            view.state = "lost".into();
        }
        let batches = self.db.all::<Batch>("batches")?;
        let mut jobs = vec![];
        for b in batches {
            jobs.push(json!({"summary":self.db.summary(&b.id)?,"batch":b}));
        }
        Ok(
            json!({"runtime":view,"profiles":self.db.all::<Profile>("profiles")?,"jobs":jobs,"settings":self.settings()?,"credentials":self.db.all::<Value>("credentials")?,"busy":busy}),
        )
    }
    pub async fn dispatch(self: &Arc<Self>, action: &str, a: Value) -> Result<Value> {
        match action {
            "snapshot" => self.snapshot(),
            "save_profile" => {
                let mut p: Profile = serde_json::from_value(a)?;
                self.db.save_profile(&mut p)?;
                self.db.audit("프로필 저장", &p.name, "성공")?;
                Ok(json!(p))
            }
            "delete_profile" => {
                let id = strarg(&a, "id")?;
                self.db.delete("profiles", id)?;
                self.db.audit("프로필 삭제", id, "성공")?;
                Ok(Value::Null)
            }
            "save_settings" => {
                let s: Settings = serde_json::from_value(a)?;
                s.validate()?;
                self.db.put("settings", "app", &s)?;
                self.db.audit("설정 변경", "이 PC", "성공")?;
                Ok(Value::Null)
            }
            "answer" => {
                let id = strarg(&a, "id")?;
                let mut slot = self.answer.lock().unwrap();
                ensure!(
                    slot.as_ref().is_some_and(|(k, _)| k == id),
                    "만료된 인증 요청"
                );
                let (_, tx) = slot.take().unwrap();
                let answer: Answer = serde_json::from_value(a)?;
                tx.send(answer)
                    .map_err(|_| anyhow::anyhow!("인증 요청 만료"))?;
                Ok(Value::Null)
            }
            "connect" | "test_connection" => {
                let guard = self
                    .work
                    .clone()
                    .try_lock_owned()
                    .map_err(|_| anyhow::anyhow!("작업 종료 후 연결하세요"))?;
                let p: Profile = self
                    .db
                    .get("profiles", strarg(&a, "id")?)?
                    .context("프로필 없음")?;
                p.validate()?;
                let token = self.token();
                let test = action == "test_connection";
                let core = self.clone();
                self.update(|v| {
                    v.error = None;
                    v.state = "connecting".into();
                    v.profile = Some(p.clone());
                    v.operation = Some(id());
                });
                tokio::spawn(async move {
                    let _guard = guard;
                    let old = core.route.lock().unwrap().take();
                    if let Some(r) = old {
                        r.close().await;
                    }
                    let result = Route::connect(core.clone(), p.clone(), token).await;
                    match result {
                        Ok(r) => {
                            if test {
                                r.close().await;
                                core.update(|v| {
                                    v.state = "disconnected".into();
                                    v.error = Some("연결 테스트 성공 · 최종 폴더 조회 완료".into())
                                });
                            } else {
                                *core.route.lock().unwrap() = Some(Arc::new(r));
                                core.update(|v| v.state = "ready".into());
                            }
                            let _ = core.db.audit("연결", &p.name, "성공");
                        }
                        Err(e) => {
                            core.update(|v| {
                                v.state = "failed".into();
                                if v.error.is_none() {
                                    v.error = Some(format!("{e:#}"));
                                }
                            });
                            let _ = core.db.audit("연결", &p.name, "실패");
                        }
                    }
                    core.update(|v| {
                        v.operation = None;
                        v.challenge = None;
                    });
                });
                Ok(Value::Null)
            }
            "disconnect" => {
                let _guard = self
                    .work
                    .clone()
                    .try_lock_owned()
                    .map_err(|_| anyhow::anyhow!("먼저 작업을 취소하고 종료를 기다리세요"))?;
                let old = self.route.lock().unwrap().take();
                if let Some(r) = old {
                    r.close().await;
                }
                self.listings.lock().unwrap().clear();
                self.update(|v| {
                    v.state = "disconnected".into();
                    v.profile = None;
                });
                Ok(Value::Null)
            }
            "cancel" => {
                self.cancel.lock().unwrap().cancel();
                Ok(Value::Null)
            }
            "cancel_item" => {
                self.cancel_items
                    .lock()
                    .unwrap()
                    .insert(strarg(&a, "id")?.into());
                Ok(Value::Null)
            }
            "list" => {
                let remote = a["remote"].as_bool().unwrap_or(false);
                let p = strarg(&a, "path")?;
                let store = if remote {
                    Store::Remote(self.active()?)
                } else {
                    Store::Local
                };
                let entries = store.list(p).await?;
                ensure!(
                    entries.len() <= 100_000,
                    "폴더 항목이 100,000개를 초과합니다. 하위 폴더를 선택하세요"
                );
                let key = id();
                let identity = if remote {
                    scope(
                        &self.active()?.profile,
                        self.active()?.profile.hops.len() - 1,
                    )
                } else {
                    "local".into()
                };
                let mut lists = self.listings.lock().unwrap();
                lists.retain(|_, (s, _)| s != &identity);
                lists.insert(key.clone(), (identity, entries));
                Ok(json!({"listing_id":key}))
            }
            "page" => {
                let lists = self.listings.lock().unwrap();
                let (_, entries) = lists
                    .get(strarg(&a, "listing_id")?)
                    .context("목록이 만료되었습니다. 새로고침하세요")?;
                let filter = a["filter"].as_str().unwrap_or("").to_lowercase();
                let mut filtered: Vec<&Entry> = entries
                    .iter()
                    .filter(|e| e.name.to_lowercase().contains(&filter))
                    .collect();
                match a["sort"].as_str().unwrap_or("name") {
                    "size" => filtered.sort_by_key(|e| e.meta.size),
                    "modified" => filtered.sort_by_key(|e| e.meta.modified),
                    _ => {}
                }
                if a["descending"].as_bool().unwrap_or(false) {
                    filtered.reverse();
                }
                let offset = a["offset"].as_u64().unwrap_or(0) as usize;
                Ok(
                    json!({"total":filtered.len(),"entries":filtered.into_iter().skip(offset).take(200).collect::<Vec<_>>()}),
                )
            }
            "prepare" => {
                let guard = self
                    .work
                    .clone()
                    .try_lock_owned()
                    .map_err(|_| anyhow::anyhow!("현재 작업을 마친 뒤 조사하세요"))?;
                let route = self.active()?;
                let direction = strarg(&a, "direction")?.to_string();
                ensure!(
                    ["upload", "download"].contains(&direction.as_str()),
                    "잘못된 전송 방향"
                );
                let listing = strarg(&a, "listing_id")?;
                let selected: Vec<String> = serde_json::from_value(a["paths"].clone())?;
                ensure!(!selected.is_empty(), "파일을 선택하세요");
                {
                    let lists = self.listings.lock().unwrap();
                    let (identity, entries) =
                        lists.get(listing).context("선택 목록이 만료되었습니다")?;
                    let expected = if direction == "upload" {
                        "local".into()
                    } else {
                        scope(&route.profile, route.profile.hops.len() - 1)
                    };
                    ensure!(identity == &expected, "연결 대상이 변경되었습니다");
                    ensure!(
                        selected
                            .iter()
                            .all(|p| entries.iter().any(|e| &e.path == p)),
                        "선택 경로가 목록과 다릅니다"
                    );
                }
                let b = Batch {
                    id: id(),
                    profile: route.profile.clone(),
                    direction,
                    destination: strarg(&a, "destination")?.into(),
                    state: "scanning".into(),
                    policy: "skip".into(),
                    created_at: now(),
                    issue_count: 0,
                    revision: 1,
                };
                self.db.put("batches", &b.id, &b)?;
                let bid = b.id.clone();
                let token = self.token();
                let core = self.clone();
                self.update(|v| {
                    v.operation = Some(bid.clone());
                    v.error = None;
                });
                tokio::spawn(async move {
                    let _g = guard;
                    let mut b = b;
                    let r = core.scan(&mut b, selected, route, &token).await;
                    if let Err(e) = r {
                        b.state = "plan_failed".into();
                        core.update(|v| v.error = Some(format!("{e:#}")));
                    }
                    let _ = core.db.put("batches", &b.id, &b);
                    core.update(|v| v.operation = None);
                });
                Ok(json!({"id":bid}))
            }
            "items" => Ok(json!(self.db.items(
                strarg(&a, "id")?,
                a["offset"].as_u64().unwrap_or(0),
                200
            )?)),
            "attempts" => Ok(json!(self.db.attempts(strarg(&a, "id")?)?)),
            "confirm" | "retry" => {
                let guard = self
                    .work
                    .clone()
                    .try_lock_owned()
                    .map_err(|_| anyhow::anyhow!("전송 작업이 이미 진행 중입니다"))?;
                let mut b: Batch = self
                    .db
                    .get("batches", strarg(&a, "id")?)?
                    .context("작업 없음")?;
                let route = self.active()?;
                ensure!(
                    route.profile.hops == b.profile.hops,
                    "원래 전송과 연결 경로가 다릅니다. 원래 프로필로 연결하세요"
                );
                ensure!(
                    b.state != "scanning" && b.state != "running",
                    "작업 진행 중"
                );
                let retry = action == "retry";
                if !retry {
                    ensure!(
                        b.state == "awaiting_confirmation",
                        "확인 가능한 계획이 아닙니다"
                    );
                    ensure!(
                        b.issue_count == 0 || a["ack_issues"].as_bool() == Some(true),
                        "제외 항목을 확인하세요"
                    );
                } else {
                    ensure!(
                        ["finished", "interrupted", "recovery_required"]
                            .contains(&b.state.as_str()),
                        "재시도 가능한 상태가 아닙니다"
                    );
                }
                let policy = a["policy"].as_str().unwrap_or("skip");
                ensure!(["skip", "overwrite"].contains(&policy), "잘못된 충돌 정책");
                b.policy = policy.into();
                b.state = "running".into();
                self.db.put("batches", &b.id, &b)?;
                let core = self.clone();
                let token = self.token();
                self.update(|v| {
                    v.operation = Some(b.id.clone());
                    v.error = None;
                });
                tokio::spawn(async move {
                    let _g = guard;
                    let result = core.run_batch(&mut b, route, &token, retry).await;
                    if let Err(e) = result {
                        b.state = "recovery_required".into();
                        core.update(|v| v.error = Some(format!("{e:#}")));
                    }
                    let _ = core.db.put("batches", &b.id, &b);
                    let _ = core.db.audit(
                        if retry {
                            "재시도 종료"
                        } else {
                            "전송 종료"
                        },
                        &b.id,
                        &b.state,
                    );
                    core.update(|v| {
                        v.operation = None;
                        v.current_item = None;
                        v.bytes = "0".into();
                    });
                });
                Ok(Value::Null)
            }
            "audits" => Ok(json!(self.db.audits()?)),
            "delete_credential" => {
                let key = strarg(&a, "id")?.to_string();
                let k = key.clone();
                tokio::task::spawn_blocking(move || {
                    keyring::Entry::new("RouteTransfer", &k)?.delete_credential()
                })
                .await??;
                self.db.delete("credentials", &key)?;
                Ok(Value::Null)
            }
            "preview_cleanup" => {
                let kind = strarg(&a, "kind")?.to_string();
                ensure!(
                    ["audit", "history"].contains(&kind.as_str()),
                    "정리 유형 오류"
                );
                let before = a["before"].as_i64().context("기간 필요")?;
                let ids = self.db.cleanup_ids(&kind, before)?;
                let key = id();
                let n = ids.len();
                self.cleanup
                    .lock()
                    .unwrap()
                    .insert(key.clone(), (kind, before, ids));
                Ok(json!({"token":key,"count":n}))
            }
            "confirm_cleanup" => {
                let _g = self
                    .work
                    .clone()
                    .try_lock_owned()
                    .map_err(|_| anyhow::anyhow!("작업 종료 후 정리하세요"))?;
                let (k, before, ids) = self
                    .cleanup
                    .lock()
                    .unwrap()
                    .remove(strarg(&a, "token")?)
                    .context("정리 확인 만료")?;
                ensure!(
                    self.db.cleanup_ids(&k, before)? == ids,
                    "대상이 변경되었습니다. 다시 확인하세요"
                );
                self.db.cleanup(&k, &ids)?;
                Ok(Value::Null)
            }
            "cleanup_temp" => {
                let _g = self
                    .work
                    .clone()
                    .try_lock_owned()
                    .map_err(|_| anyhow::anyhow!("작업 종료 후 정리하세요"))?;
                let mut i = self.db.item(strarg(&a, "id")?)?;
                ensure!(
                    !["committing", "needs_review"].contains(&i.state.as_str()),
                    "결과 확인 전에는 정리할 수 없습니다"
                );
                let b: Batch = self.db.get("batches", &i.batch_id)?.context("작업 없음")?;
                let store = if b.direction == "upload" {
                    let r = self.active()?;
                    ensure!(r.profile.hops == b.profile.hops, "연결 대상 불일치");
                    Store::Remote(r)
                } else {
                    Store::Local
                };
                if let Some(p) = &i.temp {
                    ensure!(
                        i.temp_owned,
                        "임시 파일의 소유권을 확인할 수 없습니다. 수동 확인이 필요합니다"
                    );
                    store.remove_temp(p).await?;
                    i.temp = None;
                    self.db.save_item(&i)?;
                }
                Ok(Value::Null)
            }
            _ => bail!("알 수 없는 명령"),
        }
    }
    async fn scan(
        &self,
        b: &mut Batch,
        paths: Vec<String>,
        route: Arc<Route>,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let (source, dest) = stores(&b.direction, route);
        dest.validate_path(&b.destination)?;
        ensure!(
            dest.meta(&b.destination)
                .await?
                .is_some_and(|m| m.kind == "dir"),
            "대상 폴더를 확인하세요"
        );
        let mut stack = vec![];
        let mut targets = HashSet::new();
        let mut selected = paths;
        selected.sort();
        selected.dedup();
        let roots = selected.clone();
        for p in selected.into_iter().rev() {
            if roots
                .iter()
                .any(|q| p != *q && Path::new(&p).starts_with(q))
            {
                continue;
            }
            let t = dest.join(&b.destination, &basename(&p)?)?;
            stack.push((p, t));
        }
        while let Some((p, t)) = stack.pop() {
            ensure!(!cancel.is_cancelled(), "조사 취소됨");
            let key = if b.direction == "download"
                && cfg!(any(target_os = "windows", target_os = "macos"))
            {
                t.to_lowercase()
            } else {
                t.clone()
            };
            ensure!(
                targets.insert(key),
                "여러 항목이 같은 대상 경로로 연결됩니다: {}",
                t
            );
            let mut item = Item {
                id: id(),
                batch_id: b.id.clone(),
                source: p.clone(),
                target: t.clone(),
                meta: Meta {
                    kind: "unknown".into(),
                    size: 0,
                    modified: None,
                },
                target_meta: None,
                state: "pending".into(),
                error: String::new(),
                attempt: 0,
                temp: None,
                temp_owned: false,
                bytes: 0,
            };
            let inspect: Result<()> = async {
                item.meta = source.meta(&p).await?.context("원본 없음")?;
                item.target_meta = dest.meta(&t).await?;
                ensure!(
                    ["file", "dir"].contains(&item.meta.kind.as_str()),
                    "링크·특수 파일은 전송하지 않습니다"
                );
                dest.check_parents(&t).await?;
                if let Some(m) = &item.target_meta {
                    ensure!(m.kind == item.meta.kind, "파일·폴더 종류 충돌");
                }
                if item.meta.kind == "dir" {
                    let entries = source.list(&p).await?;
                    for e in entries.into_iter().rev() {
                        stack.push((e.path, dest.join(&t, &e.name)?));
                    }
                }
                Ok(())
            }
            .await;
            if let Err(e) = inspect {
                item.state = "blocked".into();
                item.error = format!("{e:#}");
                b.issue_count += 1;
            }
            self.db.add_item(&item)?;
            tokio::task::yield_now().await;
        }
        b.state = "awaiting_confirmation".into();
        Ok(())
    }
    async fn run_batch(
        &self,
        b: &mut Batch,
        route: Arc<Route>,
        cancel: &CancellationToken,
        retry: bool,
    ) -> Result<()> {
        let (source, dest) = stores(&b.direction, route.clone());
        let mut offset = 0;
        loop {
            let items = self.db.items(&b.id, offset, 200)?;
            if items.is_empty() {
                break;
            }
            offset += items.len() as u64;
            for mut i in items {
                if cancel.is_cancelled() {
                    b.state = "interrupted".into();
                    return Ok(());
                }
                let eligible = if retry {
                    ["failed", "interrupted", "pending"].contains(&i.state.as_str())
                } else {
                    i.state == "pending"
                };
                if !eligible {
                    continue;
                }
                if self.cancel_items.lock().unwrap().remove(&i.id) {
                    i.state = "cancelled".into();
                    self.db.save_item(&i)?;
                    continue;
                }
                ensure!(!route.lost(), "CONNECTION_LOST");
                i.attempt += 1;
                i.bytes = 0;
                i.error.clear();
                self.update(|v| {
                    v.current_item = Some(i.id.clone());
                    v.bytes = "0".into();
                });
                let r = self
                    .transfer_item(&mut i, &source, &dest, &b.policy, cancel)
                    .await;
                if let Err(e) = r {
                    i.error = format!("{e:#}");
                    i.state = if i.state == "committing" {
                        "needs_review"
                    } else if cancel.is_cancelled() {
                        "interrupted"
                    } else if i.error.contains("CHANGED") {
                        "needs_review"
                    } else if i.error == "파일 취소됨" {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    if i.state != "needs_review" && i.temp_owned {
                        if let Some(p) = i.temp.clone() {
                            if dest.remove_temp(&p).await.is_ok() {
                                i.temp = None;
                            }
                        }
                    }
                    self.db.save_item(&i)?;
                    if route.lost()
                        || i.error.contains("IO_STALLED")
                        || i.error.contains("No space")
                        || i.error.contains("os error 28")
                    {
                        bail!("공통 전송 장애 · {}", i.error)
                    }
                } else {
                    self.db.save_item(&i)?;
                }
            }
        }
        b.state = "finished".into();
        Ok(())
    }
    async fn transfer_item(
        &self,
        i: &mut Item,
        source: &Store,
        dest: &Store,
        policy: &str,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if let Some(p) = i.temp.clone() {
            ensure!(i.temp_owned, "이전 임시 파일을 수동 확인하세요");
            dest.remove_temp(&p).await?;
            i.temp = None;
            i.temp_owned = false;
            self.db.save_item(i)?;
        }
        source.check_parents(&i.source).await?;
        let original = source.meta(&i.source).await?.context("SOURCE_MISSING")?;
        ensure!(original == i.meta, "SOURCE_CHANGED");
        let target = dest.meta(&i.target).await?;
        if i.meta.kind == "dir" {
            dest.mkdir(&i.target).await?;
            i.state = "succeeded".into();
            return Ok(());
        }
        ensure!(target == i.target_meta, "DESTINATION_CHANGED");
        if target.is_some() && policy == "skip" {
            i.state = "skipped".into();
            return Ok(());
        }
        if let Some(m) = &target {
            ensure!(m.kind == "file", "PATH_TYPE_CONFLICT");
        }
        let parent = Path::new(&i.target)
            .parent()
            .and_then(|p| p.to_str())
            .context("잘못된 대상 경로")?;
        let mut input = source.read(&i.source).await?;
        let temp = dest.join(parent, &format!(".routetransfer-{}-{}.part", i.id, id()))?;
        i.temp = Some(temp.clone());
        i.temp_owned = false;
        i.state = "transferring".into();
        self.db.save_item(i)?;
        let settings = self.settings()?;
        let mut output = dest.create(&temp).await?;
        i.temp_owned = true;
        self.db.save_item(i)?;
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            ensure!(
                !self.cancel_items.lock().unwrap().remove(&i.id),
                "파일 취소됨"
            );
            let n = self
                .network_wait(
                    async { Ok(input.read(&mut buf).await?) },
                    settings.io_timeout,
                    cancel,
                )
                .await?;
            if n == 0 {
                break;
            }
            self.network_wait(
                async {
                    output.write_all(&buf[..n]).await?;
                    Ok(())
                },
                settings.io_timeout,
                cancel,
            )
            .await?;
            i.bytes += n as u64;
            self.update(|v| v.bytes = i.bytes.to_string());
        }
        self.network_wait(
            async {
                output.flush().await?;
                output.shutdown().await?;
                Ok(())
            },
            settings.io_timeout,
            cancel,
        )
        .await?;
        drop(output);
        drop(input);
        i.state = "verifying".into();
        self.db.save_item(i)?;
        ensure!(
            source.meta(&i.source).await? == Some(original),
            "SOURCE_CHANGED"
        );
        ensure!(
            i.bytes == i.meta.size
                && dest
                    .meta(&temp)
                    .await?
                    .is_some_and(|m| m.size == i.meta.size),
            "크기 확인 실패"
        );
        ensure!(dest.meta(&i.target).await? == target, "DESTINATION_CHANGED");
        ensure!(!cancel.is_cancelled(), "취소됨");
        ensure!(
            !self.cancel_items.lock().unwrap().remove(&i.id),
            "파일 취소됨"
        );
        i.state = "committing".into();
        self.db.save_item(i)?;
        // Cancellation after this durable intent must never turn an uncertain rename into a clean cancellation.
        tokio::time::timeout(
            Duration::from_secs(settings.io_timeout),
            dest.commit(&temp, &i.target, target.is_some()),
        )
        .await
        .context("COMMIT_UNCERTAIN")??;
        ensure!(
            dest.meta(&i.target)
                .await?
                .is_some_and(|m| m.kind == "file" && m.size == i.meta.size),
            "COMMIT_UNCERTAIN"
        );
        i.temp = None;
        i.state = "succeeded".into();
        Ok(())
    }
}
fn stores(direction: &str, route: Arc<Route>) -> (Store, Store) {
    if direction == "upload" {
        (Store::Local, Store::Remote(route))
    } else {
        (Store::Remote(route), Store::Local)
    }
}
fn strarg<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v[k].as_str().with_context(|| format!("필수 값: {k}"))
}
