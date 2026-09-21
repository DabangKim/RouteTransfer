import { invoke, isTauri } from "@tauri-apps/api/core";
export const desktop = isTauri();
export async function call<T = unknown>(
  action: string,
  args: unknown = {},
): Promise<T> {
  if (!desktop)
    throw new Error(
      "실제 연결·전송은 RouteTransfer 데스크톱 앱에서 사용할 수 있습니다.",
    );
  return invoke<T>("dispatch", { action, args });
}
