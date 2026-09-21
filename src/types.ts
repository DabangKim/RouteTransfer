export type Hop = {
  id: string;
  alias: string;
  host: string;
  port: number;
  username: string;
};
export type Profile = {
  id: string;
  name: string;
  description: string;
  remote_path: string;
  revision: number;
  hops: Hop[];
};
export type Settings = {
  connect_timeout: number;
  auth_timeout: number;
  io_timeout: number;
  local_path: string;
};
export type Meta = { kind: string; size: number; modified: number | null };
export type Entry = { name: string; path: string; meta: Meta };
export type Batch = {
  id: string;
  profile: Profile;
  direction: string;
  destination: string;
  state: string;
  policy: string;
  created_at: number;
  issue_count: number;
  revision: number;
};
export type Item = {
  id: string;
  batch_id: string;
  source: string;
  target: string;
  meta: Meta;
  target_meta: Meta | null;
  state: string;
  error: string;
  attempt: number;
  temp: string | null;
  bytes: number;
};
export type Challenge = {
  id: string;
  kind: string;
  server: string;
  fingerprint: string | null;
  previous: string | null;
};
export type Runtime = {
  state: string;
  profile: Profile | null;
  hop: number | null;
  operation: string | null;
  error: string | null;
  challenge: Challenge | null;
  current_item: string | null;
  bytes: string;
};
export type Job = {
  batch: Batch;
  summary: Record<string, { count: number; bytes: string }>;
};
export type Snapshot = {
  runtime: Runtime;
  profiles: Profile[];
  jobs: Job[];
  settings: Settings;
  credentials: { id: string; server: string }[];
  busy: boolean;
};
export const labels: Record<string, string> = {
  disconnected: "미연결",
  connecting: "연결 중",
  authenticating: "인증 중",
  opening_sftp: "파일 연결 준비",
  ready: "연결됨",
  lost: "연결 끊김",
  failed: "실패",
  scanning: "파일 조사 중",
  plan_failed: "조사 미완료 · 새 전송 필요",
  awaiting_confirmation: "전송 확인 대기",
  running: "전송 중",
  finished: "작업 종료",
  interrupted: "중단됨",
  recovery_required: "복구 필요",
  pending: "대기",
  transferring: "전송 중",
  verifying: "확인 중",
  committing: "최종 반영 중",
  succeeded: "완료",
  skipped: "건너뜀",
  cancelled: "취소",
  blocked: "제외",
  needs_review: "확인 필요",
  file: "파일",
  dir: "폴더",
  link: "링크",
  other: "특수 파일",
};
export function size(n: number | string) {
  const v = Number(n);
  if (!Number.isFinite(v)) return "—";
  if (v < 1024) return `${v} B`;
  const p = Math.min(4, Math.floor(Math.log(v) / Math.log(1024)));
  return `${(v / 1024 ** p).toFixed(1)} ${["B", "KiB", "MiB", "GiB", "TiB"][p]}`;
}
export function parentPath(p: string) {
  const clean = p.replace(/[\\/]+$/, "");
  const index = Math.max(clean.lastIndexOf("/"), clean.lastIndexOf("\\"));
  if (index < 0) return p;
  if (index === 2 && clean[1] === ":") return clean.slice(0, 3);
  return clean.slice(0, index) || "/";
}
