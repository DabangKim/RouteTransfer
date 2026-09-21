import { useEffect, useRef, useState, type ReactNode } from "react";
import { call, desktop } from "./api";
import {
  labels,
  size,
  parentPath,
  type Snapshot,
  type Profile,
  type Hop,
  type Settings,
  type Entry,
  type Item,
  type Job,
  type Challenge,
} from "./types";
const uid = () => crypto.randomUUID();
const newHop = (): Hop => ({
  id: uid(),
  alias: "",
  host: "",
  port: 20022,
  username: "",
});
const newProfile = (): Profile => ({
  id: uid(),
  name: "",
  description: "",
  remote_path: "/",
  revision: 0,
  hops: [newHop()],
});
const empty: Snapshot = {
  runtime: {
    state: "disconnected",
    profile: null,
    hop: null,
    operation: null,
    error: null,
    challenge: null,
    current_item: null,
    bytes: "0",
  },
  profiles: [],
  jobs: [],
  settings: {
    connect_timeout: 30,
    auth_timeout: 30,
    io_timeout: 120,
    local_path: "/",
  },
  credentials: [],
  busy: false,
};
const pages = ["연결 관리", "파일 전송", "작업 기록", "설정"];
function Button({
  children,
  primary = false,
  danger = false,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & {
  primary?: boolean;
  danger?: boolean;
}) {
  return (
    <button
      {...props}
      className={`${primary ? "primary" : ""} ${danger ? "danger" : ""} ${props.className || ""}`}
    >
      {children}
    </button>
  );
}
function Field({
  label,
  ...props
}: React.InputHTMLAttributes<HTMLInputElement> & { label: string }) {
  return (
    <label className="field">
      <span>{label}</span>
      <input {...props} />
    </label>
  );
}
function Modal({
  title,
  children,
  onClose,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current;
    d?.showModal();
    return () => d?.close();
  }, []);
  return (
    <dialog
      ref={ref}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
    >
      <header>
        <h2>{title}</h2>
        <Button aria-label="닫기" onClick={onClose}>
          닫기
        </Button>
      </header>
      {children}
    </dialog>
  );
}
type Selection = { listing_id: string; paths: string[]; path: string };
export default function App() {
  const [snap, setSnap] = useState(empty),
    [page, setPage] = useState(0),
    [error, setError] = useState(""),
    [note, setNote] = useState(""),
    [profile, setProfile] = useState<Profile>(newProfile),
    [selectedHop, setSelectedHop] = useState(0),
    [search, setSearch] = useState(""),
    [pending, setPending] = useState(false),
    [localPath, setLocalPath] = useState(""),
    [remotePath, setRemotePath] = useState("/"),
    [local, setLocal] = useState<Selection>({
      listing_id: "",
      paths: [],
      path: "/",
    }),
    [remote, setRemote] = useState<Selection>({
      listing_id: "",
      paths: [],
      path: "/",
    }),
    [refresh, setRefresh] = useState(0),
    [detail, setDetail] = useState<string | null>(null),
    [confirm, setConfirm] = useState<string | null>(null),
    [retry, setRetry] = useState<string | null>(null),
    [policy, setPolicy] = useState("skip"),
    [ack, setAck] = useState(false),
    [cancelOpen, setCancelOpen] = useState(false),
    [settings, setSettings] = useState<Settings | null>(null),
    [cleanup, setCleanup] = useState<{ token: string; count: number } | null>(
      null,
    ),
    [cleanupKind, setCleanupKind] = useState("history"),
    [days, setDays] = useState(30),
    [folder, setFolder] = useState(false),
    [folderValue, setFolderValue] = useState("/");
  const snapRef = useRef(snap);
  snapRef.current = snap;
  const load = async () => {
    if (!desktop) return;
    try {
      const s = await call<Snapshot>("snapshot");
      setSnap(s);
      setLocalPath((old) => old || s.settings.local_path);
      return s;
    } catch (e) {
      setError(String(e));
    }
  };
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout>;
    async function tick() {
      if (alive) {
        await load();
        timer = setTimeout(tick, 750);
      }
    }
    void tick();
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, []);
  useEffect(() => {
    const fn = () => setCancelOpen(true);
    window.addEventListener("routetransfer-close-blocked", fn);
    return () => window.removeEventListener("routetransfer-close-blocked", fn);
  }, []);
  const lastConnection = useRef("");
  useEffect(() => {
    const r = snap.runtime;
    if (r.state === "ready" && r.profile) {
      const key = r.profile.id + ":" + r.profile.revision;
      if (lastConnection.current !== key) {
        setRemotePath(r.profile.remote_path);
        setRefresh((x) => x + 1);
        lastConnection.current = key;
        setPage(1);
      }
    }
    if (r.state === "disconnected") lastConnection.current = "";
  }, [
    snap.runtime.state,
    snap.runtime.profile?.id,
    snap.runtime.profile?.revision,
  ]);
  const act = async <T,>(fn: () => Promise<T>): Promise<T | undefined> => {
    setPending(true);
    setError("");
    try {
      return await fn();
    } catch (e) {
      setError(String(e));
      return undefined;
    } finally {
      setPending(false);
      void load();
    }
  };
  const save = async () => {
    const p = await call<Profile>("save_profile", profile);
    setProfile(p);
    setNote("프로필을 저장했습니다.");
    return p;
  };
  const connect = (test = false) =>
    void act(async () => {
      const p = await save();
      await call(test ? "test_connection" : "connect", { id: p.id });
    });
  const choose = (p: Profile) => {
    setProfile(structuredClone(p));
    setSelectedHop(0);
  };
  const changeHop = (delta: Partial<Hop>) =>
    setProfile((p) => ({
      ...p,
      hops: p.hops.map((h, i) => (i === selectedHop ? { ...h, ...delta } : h)),
    }));
  const moveHop = (delta: number) => {
    const next = selectedHop + delta;
    if (next < 0 || next >= profile.hops.length) return;
    setProfile((p) => {
      const h = [...p.hops];
      [h[next], h[selectedHop]] = [h[selectedHop], h[next]];
      return { ...p, hops: h };
    });
    setSelectedHop(next);
  };
  const prepare = (direction: string) =>
    void act(async () => {
      const selection = direction === "upload" ? local : remote;
      const result = await call<{ id: string }>("prepare", {
        direction,
        listing_id: selection.listing_id,
        paths: selection.paths,
        destination: direction === "upload" ? remote.path : local.path,
      });
      setConfirm(result.id);
      setAck(false);
      setPolicy("skip");
    });
  const job = snap.jobs.find((j) => j.batch.id === (confirm || retry));
  const connected = snap.runtime.state === "ready";
  const hop = profile.hops[selectedHop];
  return (
    <div className="app">
      <aside>
        <div className="brand">RouteTransfer</div>
        <small>TEAM WORKSPACE</small>
        <nav aria-label="주 메뉴">
          {pages.map((p, i) => (
            <button
              key={p}
              aria-current={page === i ? "page" : undefined}
              className={page === i ? "active" : ""}
              onClick={() => {
                setPage(i);
                if (i === 3) setSettings(snap.settings);
              }}
            >
              {p}
              {i === 1 && snap.busy ? <span className="dot" /> : null}
            </button>
          ))}
        </nav>
        <footer>
          내부 팀용 · v0.1
          <br />
          Windows / macOS
        </footer>
      </aside>
      <main>
        <header className="page-header">
          <div>
            <h1>{pages[page]}</h1>
            <p>
              {
                [
                  "서버를 원하는 순서로 연결하고, 마지막 서버로 파일을 전송합니다.",
                  "내 PC와 최종 서버 사이에서 폴더와 파일을 주고받습니다.",
                  "전송 결과와 재시도 내역을 확인합니다.",
                  "이 PC에서 사용할 인증 정보와 전송 기본값을 관리합니다.",
                ][page]
              }
            </p>
          </div>
          {page === 0 ? (
            <Button primary onClick={() => choose(newProfile())}>
              새 프로필
            </Button>
          ) : page === 1 ? (
            <Button
              disabled={!connected || snap.busy}
              onClick={() => void act(() => call("disconnect"))}
            >
              연결 해제
            </Button>
          ) : page === 2 ? (
            <Button
              onClick={() => {
                void load();
                setRefresh((x) => x + 1);
              }}
            >
              새로고침
            </Button>
          ) : (
            <Button
              primary
              disabled={pending}
              onClick={() =>
                void act(async () => {
                  await call("save_settings", settings || snap.settings);
                  setNote("설정을 저장했습니다.");
                })
              }
            >
              설정 저장
            </Button>
          )}
        </header>
        {!desktop && (
          <div className="banner">
            브라우저에서는 화면만 확인할 수 있습니다. 실제 연결과 파일 전송은
            데스크톱 앱에서 실행하세요.
          </div>
        )}
        {(error || snap.runtime.error) && (
          <div className="banner error" role="alert">
            {error || snap.runtime.error}
            <button onClick={() => setError("")} aria-label="알림 닫기">
              ×
            </button>
          </div>
        )}
        {note && (
          <div className="banner" role="status">
            {note}
            <button onClick={() => setNote("")} aria-label="안내 닫기">
              ×
            </button>
          </div>
        )}
        {snap.jobs.some((j) =>
          ["interrupted", "recovery_required"].includes(j.batch.state),
        ) && (
          <div className="banner">
            미완료 작업이 있습니다. 자동으로 전송하지 않습니다.{" "}
            <Button onClick={() => setPage(2)}>작업 기록에서 확인</Button>
          </div>
        )}
        {page === 0 && (
          <div className="connection-layout">
            <section className="card profiles">
              <h3>프로필 {snap.profiles.length}</h3>
              <input
                aria-label="프로필 검색"
                placeholder="이름으로 찾기"
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
              {snap.profiles
                .filter((p) => p.name.includes(search))
                .map((p) => (
                  <button
                    className={`profile ${profile.id === p.id ? "selected" : ""}`}
                    key={p.id}
                    onClick={() => choose(p)}
                  >
                    <b>{p.name}</b>
                    <small>
                      서버 {p.hops.length}개 ·{" "}
                      {p.hops.at(-1)?.alias || p.hops.at(-1)?.host}
                    </small>
                  </button>
                ))}
              {!snap.profiles.length && (
                <p className="empty">연결 경로를 등록해 시작하세요.</p>
              )}
            </section>
            <section className="card editor">
              <div className="grid2">
                <Field
                  label="프로필 이름"
                  value={profile.name}
                  onChange={(e) =>
                    setProfile({ ...profile, name: e.target.value })
                  }
                />
                <Field
                  label="기본 원격 경로"
                  value={profile.remote_path}
                  onChange={(e) =>
                    setProfile({ ...profile, remote_path: e.target.value })
                  }
                />
              </div>
              <Field
                label="설명"
                value={profile.description}
                onChange={(e) =>
                  setProfile({ ...profile, description: e.target.value })
                }
              />
              <h3>연결 경로 · 서버 {profile.hops.length}개</h3>
              <p className="muted">
                내 PC에서 순서대로 연결합니다. 마지막 서버가 파일 전송
                대상입니다.
              </p>
              <div className="server-list">
                {profile.hops.map((h, i) => (
                  <button
                    key={h.id}
                    className={`server ${selectedHop === i ? "selected" : ""}`}
                    onClick={() => setSelectedHop(i)}
                  >
                    <strong>{String(i + 1).padStart(2, "0")}</strong>
                    <span>{h.alias || "새 서버"}</span>
                    <span className="muted">
                      {h.host || "주소 미입력"}:{h.port}
                    </span>
                    <small>
                      {i === profile.hops.length - 1 ? "최종 대상" : "경유"}
                    </small>
                  </button>
                ))}
              </div>
              <Button
                onClick={() => {
                  setProfile({ ...profile, hops: [...profile.hops, newHop()] });
                  setSelectedHop(profile.hops.length);
                }}
              >
                ＋ 서버 추가
              </Button>
              {hop && (
                <div className="server-detail">
                  <div className="row spread">
                    <h3>
                      {selectedHop + 1}. {hop.alias || "서버 정보"}
                    </h3>
                    <div className="row">
                      <Button
                        disabled={selectedHop === 0}
                        onClick={() => moveHop(-1)}
                      >
                        ↑ 위로
                      </Button>
                      <Button
                        disabled={selectedHop === profile.hops.length - 1}
                        onClick={() => moveHop(1)}
                      >
                        ↓ 아래로
                      </Button>
                      <Button
                        disabled={profile.hops.length === 1}
                        onClick={() => {
                          setProfile({
                            ...profile,
                            hops: profile.hops.filter(
                              (_, i) => i !== selectedHop,
                            ),
                          });
                          setSelectedHop(Math.max(0, selectedHop - 1));
                        }}
                      >
                        삭제
                      </Button>
                    </div>
                  </div>
                  <div className="hop-fields">
                    <Field
                      label="서버 별칭"
                      value={hop.alias}
                      onChange={(e) => changeHop({ alias: e.target.value })}
                    />
                    <Field
                      label="호스트"
                      value={hop.host}
                      onChange={(e) => changeHop({ host: e.target.value })}
                    />
                    <Field
                      label="포트"
                      type="number"
                      min={1}
                      max={65535}
                      value={hop.port}
                      onChange={(e) =>
                        changeHop({ port: Number(e.target.value) })
                      }
                    />
                    <Field
                      label="사용자"
                      value={hop.username}
                      onChange={(e) => changeHop({ username: e.target.value })}
                    />
                  </div>
                  <p>Ubuntu Linux · SSH · 비밀번호 인증</p>
                  <small>
                    비밀번호는 연결할 때 입력합니다. 서버별 저장 선택은 기본
                    해제입니다.
                  </small>
                </div>
              )}
              <div className="row">
                <Button disabled={pending} onClick={() => void act(save)}>
                  프로필 저장
                </Button>
                <Button
                  disabled={pending || snap.busy}
                  onClick={() => connect(true)}
                >
                  연결 테스트
                </Button>
                <Button
                  primary
                  disabled={pending || snap.busy}
                  onClick={() => connect()}
                >
                  연결하기
                </Button>
                {profile.revision > 0 && (
                  <Button
                    danger
                    disabled={snap.busy}
                    onClick={() => {
                      if (
                        window.confirm(
                          "이 프로필을 삭제할까요? 기존 작업 기록은 유지됩니다.",
                        )
                      )
                        void act(async () => {
                          await call("delete_profile", { id: profile.id });
                          choose(newProfile());
                        });
                    }}
                  >
                    프로필 삭제
                  </Button>
                )}
              </div>
              {snap.busy && (
                <div className="banner">
                  {labels[snap.runtime.state] || snap.runtime.state}
                  {snap.runtime.hop !== null
                    ? ` · 서버 ${snap.runtime.hop + 1}`
                    : ""}
                  <Button onClick={() => void act(() => call("cancel"))}>
                    작업 취소
                  </Button>
                </div>
              )}
            </section>
          </div>
        )}
        {page === 1 && (
          <>
            <section className="card route">
              <b className={connected ? "accent" : ""}>
                {labels[snap.runtime.state]} ·{" "}
                {snap.runtime.profile?.name || "활성 프로필 없음"}
              </b>
              <p>
                내 PC{" "}
                {snap.runtime.profile?.hops
                  .map((h) => ` → ${h.alias || h.host}`)
                  .join("")}{" "}
                {connected ? "(최종 대상)" : ""}
              </p>
            </section>
            <div className="explorers">
              <Explorer
                title="내 PC"
                remote={false}
                initialPath={localPath || snap.settings.local_path}
                enabled={desktop}
                refresh={refresh}
                onSelection={setLocal}
                onError={setError}
              />
              <div className="directions">
                <Button
                  primary
                  disabled={!connected || snap.busy || !local.paths.length}
                  onClick={() => prepare("upload")}
                >
                  업로드 →
                </Button>
                <Button
                  disabled={!connected || snap.busy || !remote.paths.length}
                  onClick={() => prepare("download")}
                >
                  ← 다운로드
                </Button>
                <small>
                  각 영역에서
                  <br />
                  파일을 선택하세요
                </small>
              </div>
              <Explorer
                title={snap.runtime.profile?.hops.at(-1)?.alias || "최종 서버"}
                remote
                initialPath={remotePath}
                enabled={connected}
                refresh={refresh}
                onSelection={setRemote}
                onError={setError}
              />
            </div>
            <section className="card queue">
              <div className="row spread">
                <h3>전송 대기열 · 최근 작업</h3>
                {snap.busy && (
                  <Button onClick={() => setCancelOpen(true)}>작업 취소</Button>
                )}
              </div>
              {snap.jobs.length === 0 ? (
                <p className="empty">파일을 선택하고 전송을 시작하세요.</p>
              ) : (
                snap.jobs
                  .slice(0, 4)
                  .map((j) => (
                    <JobRow
                      key={j.batch.id}
                      job={j}
                      runtimeBytes={
                        j.batch.id === snap.runtime.operation
                          ? snap.runtime.bytes
                          : "0"
                      }
                      onOpen={() => setDetail(j.batch.id)}
                    />
                  ))
              )}
            </section>
          </>
        )}
        {page === 2 && (
          <History
            snap={snap}
            refresh={refresh}
            onDetail={setDetail}
            onRetry={(id) => {
              setRetry(id);
              setPolicy("skip");
            }}
            onConfirm={(id) => {
              setConfirm(id);
              setPolicy("skip");
              setAck(false);
            }}
            onError={setError}
          />
        )}
        {page === 3 && (
          <div className="settings-grid">
            <section className="card">
              <h3>인증 정보</h3>
              <p>비밀번호는 기본적으로 저장하지 않습니다.</p>
              <small>저장을 선택한 서버만 OS 보안 저장소를 사용합니다.</small>
              {snap.credentials.length === 0 ? (
                <p className="empty">저장된 인증 정보가 없습니다.</p>
              ) : (
                snap.credentials.map((c) => (
                  <div className="credential" key={c.id}>
                    <span>{c.server}</span>
                    <Button
                      onClick={() =>
                        void act(() => call("delete_credential", { id: c.id }))
                      }
                    >
                      저장 정보 삭제
                    </Button>
                  </div>
                ))
              )}
            </section>
            <section className="card">
              <h3>시간 제한</h3>
              {(
                [
                  ["connect_timeout", "연결·협상 응답 대기 (초)", 10, 300],
                  ["auth_timeout", "인증 응답 대기 (초)", 10, 300],
                  ["io_timeout", "전송 무응답 대기 (초)", 30, 600],
                ] as const
              ).map(([key, label, min, max]) => (
                <Field
                  key={key}
                  label={label}
                  type="number"
                  min={min}
                  max={max}
                  value={(settings || snap.settings)[key]}
                  onChange={(e) =>
                    setSettings({
                      ...(settings || snap.settings),
                      [key]: Number(e.target.value),
                    })
                  }
                />
              ))}
              <small>
                사람의 입력 대기나 전체 전송 시간을 제한하지 않습니다.
              </small>
            </section>
            <section className="card">
              <h3>전송 기본값</h3>
              <Field
                label="다운로드 기본 위치"
                value={(settings || snap.settings).local_path}
                onChange={(e) =>
                  setSettings({
                    ...(settings || snap.settings),
                    local_path: e.target.value,
                  })
                }
              />
              <Button
                onClick={() => {
                  setFolderValue((settings || snap.settings).local_path);
                  setFolder(true);
                }}
              >
                폴더 선택
              </Button>
              <p>
                같은 이름의 파일 · <b>건너뛰기</b>
              </p>
              <small>
                덮어쓰기는 해당 작업의 전송 확인에서 선택합니다.
                <br />
                파일은 하나씩 순서대로 전송합니다.
              </small>
            </section>
            <section className="card">
              <h3>기록 관리</h3>
              <p>기록을 자동으로 삭제하지 않습니다.</p>
              <small>미완료·확인 필요·임시 정리 대기 작업은 제외됩니다.</small>
              <label className="field">
                정리할 기록
                <select
                  value={cleanupKind}
                  onChange={(e) => setCleanupKind(e.target.value)}
                >
                  <option value="history">완료 전송 이력</option>
                  <option value="audit">감사 기록</option>
                </select>
              </label>
              <Field
                label="며칠 이전의 기록을 정리할까요?"
                type="number"
                min={1}
                value={days}
                onChange={(e) => setDays(Number(e.target.value))}
              />
              <Button
                onClick={() =>
                  void act(async () => {
                    if (days < 1) throw new Error("1일 이상을 입력하세요");
                    setCleanup(
                      await call("preview_cleanup", {
                        kind: cleanupKind,
                        before: Date.now() - days * 86400000,
                      }),
                    );
                  })
                }
              >
                정리 대상 확인
              </Button>
            </section>
            <section className="card">
              <h3>RouteTransfer 0.1.0</h3>
              <p>GitHub 설치 파일로 수동 업데이트합니다.</p>
              <small>배포 저장소 주소는 출시 준비 단계에서 지정합니다.</small>
            </section>
          </div>
        )}
      </main>
      {snap.runtime.challenge && (
        <Auth
          key={snap.runtime.challenge.id}
          challenge={snap.runtime.challenge}
          onAnswer={(a) =>
            void act(() =>
              call("answer", { id: snap.runtime.challenge!.id, ...a }),
            )
          }
        />
      )}
      {(confirm || retry) && (
        <Modal
          title={retry ? "파일 단위 재시도" : "전송 확인"}
          onClose={() => {
            setConfirm(null);
            setRetry(null);
          }}
        >
          {!job ? (
            <p>작업을 불러오는 중입니다.</p>
          ) : (
            <>
              <p>
                <b>
                  {job.batch.profile.name} ·{" "}
                  {job.batch.direction === "upload" ? "업로드" : "다운로드"}
                </b>
              </p>
              <div className="path-box">
                최종 경로
                <br />
                <strong>{job.batch.destination}</strong>
                <br />
                마지막 서버: {job.batch.profile.hops.at(-1)?.host}
              </div>
              <p>
                {retry
                  ? "완료·건너뜀·취소·확인 필요 항목을 제외하고 실패·중단·대기 파일을 처음부터 다시 전송합니다."
                  : "선택 폴더 자체가 목적지 아래에 생성됩니다."}
              </p>
              <p>
                상태: {labels[job.batch.state]} ·{" "}
                {Object.values(job.summary).reduce((n, s) => n + s.count, 0)}개
                항목
              </p>
              {job.batch.issue_count > 0 && (
                <label className="checkbox warning">
                  <input
                    type="checkbox"
                    checked={ack}
                    onChange={(e) => setAck(e.target.checked)}
                  />
                  {job.batch.issue_count}개 제외 항목을 확인하고 정상 항목만
                  전송합니다.{" "}
                  <button onClick={() => setDetail(job.batch.id)}>
                    목록 확인
                  </button>
                </label>
              )}
              <label className="checkbox">
                <input
                  type="radio"
                  name="policy"
                  checked={policy === "skip"}
                  onChange={() => setPolicy("skip")}
                />
                같은 이름의 파일 건너뛰기 (기본)
              </label>
              <label className="checkbox">
                <input
                  type="radio"
                  name="policy"
                  checked={policy === "overwrite"}
                  onChange={() => setPolicy("overwrite")}
                />
                덮어쓰기 · 현재 작업에만 적용
              </label>
              <footer className="row">
                <Button
                  onClick={() => {
                    setConfirm(null);
                    setRetry(null);
                  }}
                >
                  취소
                </Button>
                <Button
                  primary
                  disabled={
                    pending ||
                    snap.busy ||
                    !connected ||
                    (!retry && job.batch.state !== "awaiting_confirmation") ||
                    (!retry && job.batch.issue_count > 0 && !ack)
                  }
                  onClick={() =>
                    void act(async () => {
                      await call(retry ? "retry" : "confirm", {
                        id: job.batch.id,
                        policy,
                        ack_issues: ack,
                      });
                      setConfirm(null);
                      setRetry(null);
                      setPage(1);
                    })
                  }
                >
                  {connected
                    ? snap.busy
                      ? "조사·작업 중"
                      : "확인하고 전송"
                    : "먼저 원래 프로필로 연결하세요"}
                </Button>
              </footer>
            </>
          )}
        </Modal>
      )}
      {detail && (
        <Detail
          id={detail}
          busy={snap.busy}
          current={snap.runtime.current_item}
          refresh={snap.runtime.bytes}
          onClose={() => setDetail(null)}
          onError={setError}
          onCancel={(id) => void act(() => call("cancel_item", { id }))}
          onCleanup={(id) => void act(() => call("cleanup_temp", { id }))}
        />
      )}
      {cancelOpen && (
        <Modal
          title="진행 중인 작업을 취소할까요?"
          onClose={() => setCancelOpen(false)}
        >
          <p>
            완료 파일은 유지됩니다. 현재 파일과 남은 파일의 작업을 중단합니다.
          </p>
          <p>취소 처리가 끝난 뒤 앱을 닫을 수 있습니다.</p>
          <footer className="row">
            <Button onClick={() => setCancelOpen(false)}>계속 진행</Button>
            <Button
              danger
              onClick={() =>
                void act(async () => {
                  await call("cancel");
                  setCancelOpen(false);
                })
              }
            >
              작업 취소
            </Button>
          </footer>
        </Modal>
      )}
      {cleanup && (
        <Modal title="기록 정리 확인" onClose={() => setCleanup(null)}>
          <p>
            {days}일 이전의{" "}
            {cleanupKind === "history" ? "완료 전송 이력" : "감사 기록"}{" "}
            {cleanup.count}건을 삭제합니다.
          </p>
          <p>
            전송한 원본과 대상 파일은 삭제하지 않습니다. 삭제한 기록은 되돌릴 수
            없습니다.
          </p>
          <footer className="row">
            <Button onClick={() => setCleanup(null)}>취소</Button>
            <Button
              danger
              disabled={pending || snap.busy}
              onClick={() =>
                void act(async () => {
                  await call("confirm_cleanup", { token: cleanup.token });
                  setCleanup(null);
                  setNote("기록을 정리했습니다.");
                })
              }
            >
              해당 기록 삭제
            </Button>
          </footer>
        </Modal>
      )}
      {folder && (
        <Modal title="기본 다운로드 폴더 선택" onClose={() => setFolder(false)}>
          <Explorer
            title="내 PC"
            remote={false}
            initialPath={folderValue}
            enabled={desktop}
            refresh={0}
            onSelection={(s) => setFolderValue(s.path)}
            onError={setError}
          />
          <footer className="row">
            <Button
              primary
              onClick={() => {
                setSettings({
                  ...(settings || snap.settings),
                  local_path: folderValue,
                });
                setFolder(false);
              }}
            >
              현재 폴더 선택
            </Button>
          </footer>
        </Modal>
      )}
    </div>
  );
}
function Auth({
  challenge: c,
  onAnswer,
}: {
  challenge: Challenge;
  onAnswer: (a: { accepted: boolean; password: string; save: boolean }) => void;
}) {
  const [pw, setPw] = useState(""),
    [save, setSave] = useState(false);
  const submit = (accepted: boolean) => {
    onAnswer({ accepted, password: pw, save });
    setPw("");
  };
  return (
    <Modal
      title={c.kind === "host" ? "처음 연결하는 서버" : "서버 비밀번호 입력"}
      onClose={() => submit(false)}
    >
      <p>
        <b>{c.server}</b>
      </p>
      {c.kind === "host" ? (
        <>
          <p>담당자가 알려준 지문과 일치하는지 확인하세요.</p>
          <code className="fingerprint">{c.fingerprint}</code>
          <p>신뢰하면 이 PC에 서버 키를 저장합니다.</p>
        </>
      ) : (
        <>
          <Field
            label="비밀번호"
            type="password"
            autoFocus
            autoComplete="off"
            value={pw}
            onChange={(e) => setPw(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") submit(true);
            }}
          />
          <label className="checkbox">
            <input
              type="checkbox"
              checked={save}
              onChange={(e) => setSave(e.target.checked)}
            />
            이 서버 비밀번호 저장 · OS 보안 저장소
          </label>
        </>
      )}
      <footer className="row">
        <Button onClick={() => submit(false)}>취소</Button>
        <Button
          primary
          disabled={c.kind === "password" && !pw}
          onClick={() => submit(true)}
        >
          {c.kind === "host" ? "신뢰하고 연결" : "연결"}
        </Button>
      </footer>
    </Modal>
  );
}
function Explorer({
  title,
  remote,
  initialPath,
  enabled,
  refresh,
  onSelection,
  onError,
}: {
  title: string;
  remote: boolean;
  initialPath: string;
  enabled: boolean;
  refresh: number;
  onSelection: (s: Selection) => void;
  onError: (s: string) => void;
}) {
  const [path, setPath] = useState(initialPath),
    [draft, setDraft] = useState(initialPath),
    [listing, setListing] = useState(""),
    [entries, setEntries] = useState<Entry[]>([]),
    [selected, setSelected] = useState<string[]>([]),
    [filter, setFilter] = useState(""),
    [sort, setSort] = useState("name"),
    [offset, setOffset] = useState(0),
    [total, setTotal] = useState(0),
    [loading, setLoading] = useState(false),
    [failure, setFailure] = useState("");
  const gen = useRef(0),
    pageGen = useRef(0);
  const load = async (p: string) => {
    if (!enabled) return;
    const g = ++gen.current;
    setLoading(true);
    setSelected([]);
    setEntries([]);
    setListing("");
    setFailure("");
    onSelection({ listing_id: "", paths: [], path: p });
    try {
      const r = await call<{ listing_id: string }>("list", { path: p, remote });
      if (g !== gen.current) return;
      setPath(p);
      setDraft(p);
      setListing(r.listing_id);
      setOffset(0);
      setFilter("");
      onSelection({ listing_id: r.listing_id, paths: [], path: p });
    } catch (e) {
      if (g === gen.current) {
        setFailure(String(e));
        onError(String(e));
      }
    } finally {
      if (g === gen.current) setLoading(false);
    }
  };
  useEffect(() => {
    setDraft(initialPath);
    void load(initialPath);
    return () => {
      gen.current++;
    };
  }, [initialPath, enabled, refresh]);
  useEffect(() => {
    if (!listing || !enabled) return;
    const g = ++pageGen.current;
    void call<{ total: number; entries: Entry[] }>("page", {
      listing_id: listing,
      offset,
      filter,
      sort,
    })
      .then((r) => {
        if (g === pageGen.current) {
          setEntries(r.entries);
          setTotal(r.total);
        }
      })
      .catch((e) => setFailure(String(e)));
    return () => {
      pageGen.current++;
    };
  }, [listing, offset, filter, sort, enabled]);
  const toggle = (p: string) => {
    const next = selected.includes(p)
      ? selected.filter((s) => s !== p)
      : [...selected, p];
    setSelected(next);
    onSelection({ listing_id: listing, paths: next, path });
  };
  return (
    <section className="card explorer">
      <h3>{title}</h3>
      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          void load(draft);
        }}
      >
        <input
          aria-label={`${title} 현재 경로`}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          disabled={!enabled}
        />
        <Button disabled={!enabled || loading}>이동</Button>
      </form>
      <div className="row toolbar">
        <Button
          disabled={!enabled || loading}
          onClick={() => void load(parentPath(path))}
        >
          ↑ 상위
        </Button>
        <Button disabled={!enabled || loading} onClick={() => void load(path)}>
          새로고침
        </Button>
        <input
          aria-label={`${title} 이름 필터`}
          placeholder="이름으로 찾기"
          value={filter}
          onChange={(e) => {
            setFilter(e.target.value);
            setOffset(0);
          }}
        />
      </div>
      <div className="row">
        <label className="checkbox">
          <input
            type="checkbox"
            checked={
              entries.length > 0 &&
              entries.every((e) => selected.includes(e.path))
            }
            onChange={(e) => {
              const next = e.target.checked
                ? [...new Set([...selected, ...entries.map((e) => e.path)])]
                : selected.filter((p) => !entries.some((e) => e.path === p));
              setSelected(next);
              onSelection({ listing_id: listing, paths: next, path });
            }}
            disabled={!enabled || !entries.length}
          />
          현재 페이지 선택
        </label>
        <select
          aria-label={`${title} 정렬`}
          value={sort}
          onChange={(e) => {
            setSort(e.target.value);
            setOffset(0);
          }}
        >
          <option value="name">이름순</option>
          <option value="size">크기순</option>
          <option value="modified">수정일순</option>
        </select>
      </div>
      <div className="file-scroll">
        <table>
          <thead>
            <tr>
              <th>선택 / 이름</th>
              <th>유형</th>
              <th>크기</th>
              <th>수정일</th>
            </tr>
          </thead>
          <tbody>
            {enabled &&
              entries.map((e) => (
                <tr
                  key={e.path}
                  className={selected.includes(e.path) ? "selected" : ""}
                >
                  <td>
                    <label className="checkbox">
                      <input
                        aria-label={e.name + " 선택"}
                        type="checkbox"
                        checked={selected.includes(e.path)}
                        onChange={() => toggle(e.path)}
                      />
                      <button
                        className="filename"
                        title={e.path}
                        onClick={() =>
                          e.meta.kind === "dir"
                            ? void load(e.path)
                            : toggle(e.path)
                        }
                      >
                        {e.name}
                      </button>
                    </label>
                  </td>
                  <td>{labels[e.meta.kind]}</td>
                  <td>{e.meta.kind === "file" ? size(e.meta.size) : "—"}</td>
                  <td>
                    {e.meta.modified
                      ? new Date(
                          remote
                            ? e.meta.modified * 1000
                            : e.meta.modified / 1e6,
                        ).toLocaleDateString()
                      : "—"}
                  </td>
                </tr>
              ))}
          </tbody>
        </table>
        {!enabled ? (
          <p className="empty">
            {remote
              ? "서버에 연결하면 파일 목록이 표시됩니다."
              : "데스크톱 앱에서 폴더를 탐색하세요."}
          </p>
        ) : loading ? (
          <p className="empty" role="status">
            폴더를 불러오는 중…
          </p>
        ) : failure ? (
          <p className="empty error">{failure}</p>
        ) : !entries.length ? (
          <p className="empty">표시할 항목이 없습니다.</p>
        ) : null}
      </div>
      <div className="row spread">
        <small>
          {selected.length}개 선택 · {total}개 항목
        </small>
        <div className="row">
          <Button
            disabled={offset === 0}
            onClick={() => setOffset(Math.max(0, offset - 200))}
          >
            이전
          </Button>
          <Button
            disabled={offset + 200 >= total}
            onClick={() => setOffset(offset + 200)}
          >
            다음
          </Button>
        </div>
      </div>
    </section>
  );
}
function JobRow({
  job: j,
  runtimeBytes = "0",
  onOpen,
}: {
  job: Job;
  runtimeBytes?: string;
  onOpen: () => void;
}) {
  const sums = Object.values(j.summary);
  const total = sums.reduce((n, s) => n + Number(s.bytes), 0),
    done = Number(j.summary.succeeded?.bytes || 0),
    count = sums.reduce((n, s) => n + s.count, 0);
  const sample = useRef({
    at: performance.now(),
    bytes: done + Number(runtimeBytes),
  });
  const [speed, setSpeed] = useState(0);
  useEffect(() => {
    const at = performance.now(),
      bytes = done + Number(runtimeBytes),
      dt = (at - sample.current.at) / 1000;
    setSpeed(dt > 0 ? Math.max(0, (bytes - sample.current.bytes) / dt) : 0);
    sample.current = { at, bytes };
  }, [done, runtimeBytes]);
  return (
    <button className="job" onClick={onOpen}>
      <div className="row spread">
        <b>
          {j.batch.profile.name} ·{" "}
          {j.batch.direction === "upload" ? "업로드" : "다운로드"}
        </b>
        <span className="badge">{labels[j.batch.state]}</span>
      </div>
      <p>{j.batch.destination}</p>
      <progress value={done + Number(runtimeBytes)} max={total || 1} />
      <small>
        {j.batch.state === "running" && `${size(speed)}/s · `}완료{" "}
        {j.summary.succeeded?.count || 0} / {count}개 ·{" "}
        {size(done + Number(runtimeBytes))} / {size(total)} · 실패{" "}
        {j.summary.failed?.count || 0} · 건너뜀 {j.summary.skipped?.count || 0}{" "}
        · 확인 필요 {j.summary.needs_review?.count || 0}
      </small>
    </button>
  );
}
function History({
  snap,
  refresh,
  onDetail,
  onRetry,
  onConfirm,
  onError,
}: {
  snap: Snapshot;
  refresh: number;
  onDetail: (id: string) => void;
  onRetry: (id: string) => void;
  onConfirm: (id: string) => void;
  onError: (s: string) => void;
}) {
  const [tab, setTab] = useState(false),
    [audits, setAudits] = useState<Record<string, string>[]>([]),
    [filter, setFilter] = useState(""),
    [direction, setDirection] = useState(""),
    [result, setResult] = useState("");
  useEffect(() => {
    if (tab && desktop)
      void call<Record<string, string>[]>("audits")
        .then(setAudits)
        .catch((e) => onError(String(e)));
  }, [tab, refresh, snap.busy]);
  return (
    <>
      <div className="row tabs">
        <Button primary={!tab} onClick={() => setTab(false)}>
          전송 이력
        </Button>
        <Button primary={tab} onClick={() => setTab(true)}>
          감사 기록
        </Button>
      </div>
      {tab ? (
        <section className="card">
          <p>이 PC의 OS 사용자 기준 기록입니다. 최근 500건을 표시합니다.</p>
          <table>
            <thead>
              <tr>
                <th>시각</th>
                <th>OS 사용자</th>
                <th>행위</th>
                <th>대상</th>
                <th>결과</th>
              </tr>
            </thead>
            <tbody>
              {audits.map((a, i) => (
                <tr key={i}>
                  <td>{new Date(a.at).toLocaleString()}</td>
                  <td>{a.actor}</td>
                  <td>{a.action}</td>
                  <td>{a.target}</td>
                  <td>{a.result}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      ) : (
        <>
          <div className="card row">
            <input
              placeholder="프로필 이름으로 찾기"
              aria-label="이력 프로필 필터"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
            />
            <select
              aria-label="전송 방향 필터"
              value={direction}
              onChange={(e) => setDirection(e.target.value)}
            >
              <option value="">전체 방향</option>
              <option value="upload">업로드</option>
              <option value="download">다운로드</option>
            </select>
            <select
              aria-label="결과 필터"
              value={result}
              onChange={(e) => setResult(e.target.value)}
            >
              <option value="">전체 결과</option>
              {[
                "finished",
                "interrupted",
                "recovery_required",
                "awaiting_confirmation",
              ].map((s) => (
                <option key={s} value={s}>
                  {labels[s]}
                </option>
              ))}
            </select>
          </div>
          <section className="card">
            <table>
              <thead>
                <tr>
                  <th>시각</th>
                  <th>작업 / 프로필</th>
                  <th>방향</th>
                  <th>상태</th>
                  <th>동작</th>
                </tr>
              </thead>
              <tbody>
                {snap.jobs
                  .filter(
                    (j) =>
                      j.batch.profile.name.includes(filter) &&
                      (!direction || j.batch.direction === direction) &&
                      (!result || j.batch.state === result),
                  )
                  .map((j) => (
                    <tr key={j.batch.id}>
                      <td>{new Date(j.batch.created_at).toLocaleString()}</td>
                      <td>
                        <button
                          className="link"
                          onClick={() => onDetail(j.batch.id)}
                        >
                          {j.batch.profile.name}
                        </button>
                        <small className="block">{j.batch.destination}</small>
                      </td>
                      <td>
                        {j.batch.direction === "upload" ? "업로드" : "다운로드"}
                      </td>
                      <td>
                        {labels[j.batch.state]}
                        {j.summary.failed &&
                          ` · 실패 ${j.summary.failed.count}`}{" "}
                        {j.summary.needs_review &&
                          ` · 확인 필요 ${j.summary.needs_review.count}`}
                      </td>
                      <td>
                        <Button onClick={() => onDetail(j.batch.id)}>
                          상세
                        </Button>
                        {j.batch.state === "awaiting_confirmation" ? (
                          <Button
                            disabled={snap.busy}
                            onClick={() => onConfirm(j.batch.id)}
                          >
                            전송 확인
                          </Button>
                        ) : j.batch.state !== "plan_failed" &&
                          j.batch.state !== "scanning" &&
                          j.batch.state !== "running" &&
                          (j.summary.failed ||
                            j.summary.interrupted ||
                            j.summary.pending) ? (
                          <Button
                            disabled={snap.busy}
                            onClick={() => onRetry(j.batch.id)}
                          >
                            미완료 재시도
                          </Button>
                        ) : null}
                      </td>
                    </tr>
                  ))}
              </tbody>
            </table>
            {!snap.jobs.length && (
              <p className="empty">전송 기록이 없습니다.</p>
            )}
          </section>
        </>
      )}
    </>
  );
}
function Detail({
  id,
  busy,
  current,
  refresh,
  onClose,
  onError,
  onCancel,
  onCleanup,
}: {
  id: string;
  busy: boolean;
  current: string | null;
  refresh: string;
  onClose: () => void;
  onError: (s: string) => void;
  onCancel: (s: string) => void;
  onCleanup: (s: string) => void;
}) {
  const [items, setItems] = useState<Item[]>([]),
    [offset, setOffset] = useState(0),
    [attempts, setAttempts] = useState<Item[]>([]);
  useEffect(() => {
    let alive = true;
    void call<Item[]>("items", { id, offset })
      .then((r) => {
        if (alive) setItems(r);
      })
      .catch((e) => onError(String(e)));
    return () => {
      alive = false;
    };
  }, [id, offset, busy, refresh]);
  return (
    <Modal title="파일별 작업 상세" onClose={onClose}>
      <p>
        완료 파일은 다시 보내지 않습니다. ‘확인 필요’는 원본과 대상 상태를
        확인한 뒤 새 전송으로 진행하세요.
      </p>
      <div className="detail-scroll">
        <table>
          <thead>
            <tr>
              <th>파일 / 대상</th>
              <th>결과</th>
              <th>시도</th>
              <th>동작</th>
            </tr>
          </thead>
          <tbody>
            {items.map((i) => (
              <tr key={i.id}>
                <td className="wrap">
                  {i.source}
                  <small className="block">→ {i.target}</small>
                  {i.target_meta && (
                    <small className="block warning">
                      기존 대상 있음 · 충돌 정책 적용
                    </small>
                  )}
                  {i.error && <small className="block error">{i.error}</small>}
                  {i.temp && (
                    <small className="block">임시 파일: {i.temp}</small>
                  )}
                </td>
                <td>
                  {labels[i.state]}
                  {i.id === current && (
                    <>
                      <progress
                        aria-label="현재 파일 진행률"
                        value={Number(refresh)}
                        max={i.meta.size || 1}
                      />
                      <small>
                        {size(refresh)} / {size(i.meta.size)}
                      </small>
                    </>
                  )}
                </td>
                <td>
                  <button
                    className="link"
                    onClick={() =>
                      void call<Item[]>("attempts", { id: i.id })
                        .then(setAttempts)
                        .catch((e) => onError(String(e)))
                    }
                  >
                    {i.attempt}회
                  </button>
                </td>
                <td>
                  {busy && (current === i.id || i.state === "pending") && (
                    <Button onClick={() => onCancel(i.id)}>파일 취소</Button>
                  )}
                  {!busy && i.temp && i.state !== "needs_review" && (
                    <Button onClick={() => onCleanup(i.id)}>
                      임시 파일 정리
                    </Button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {attempts.length > 0 && (
        <div className="path-box">
          시도 기록
          {attempts.map((a) => (
            <p key={a.attempt}>
              {a.attempt}회 · {labels[a.state]} · {a.error || size(a.bytes)}
            </p>
          ))}
        </div>
      )}
      <footer className="row">
        <Button disabled={!offset} onClick={() => setOffset(offset - 200)}>
          이전
        </Button>
        <Button
          disabled={items.length < 200}
          onClick={() => setOffset(offset + 200)}
        >
          다음
        </Button>
        <Button onClick={onClose}>닫기</Button>
      </footer>
    </Modal>
  );
}
