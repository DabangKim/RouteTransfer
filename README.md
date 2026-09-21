# RouteTransfer

여러 SSH 서버를 순서대로 거쳐 마지막 Ubuntu 서버와 파일·폴더를 주고받는 내부 팀용 데스크톱 앱입니다. React/TypeScript 화면과 Tauri/Rust 전송 엔진으로 구성합니다.

현재 v0.1 개발 MVP입니다. macOS Apple Silicon에서 앱 실행과 실제 제품 엔진의 Ubuntu 전송을 확인했습니다. Windows x64 설치 파일도 생성했고 Windows 환경의 단위 테스트 7개를 통과했습니다. [Windows 검증 패키지](https://github.com/DabangKim/RouteTransfer/actions/runs/35582150222/artifacts/10630932517)를 내려받아 설치할 수 있습니다. 회사 PC 설치·서버 연결 검증은 아직 남아 있습니다.

## 바로 실행

이 작업 공간에서 생성한 앱:

`src-tauri/target/release/bundle/macos/RouteTransfer.app`

ZIP 패키지는 `releases/RouteTransfer-0.1.0-macOS-arm64.zip`에 있습니다. 더블클릭하여 실행하거나 다음을 사용합니다.

```sh
open src-tauri/target/release/bundle/macos/RouteTransfer.app
```

별도 서버 프로그램을 설치할 필요는 없습니다. 경유 서버는 SSH TCP forwarding을 허용해야 하고 최종 서버는 SFTP를 제공해야 합니다. 비밀번호 인증을 지원하며 SSH 키·MFA 인증은 이번 버전에서 제공하지 않습니다.

## 사용 순서

1. **연결 관리**에서 프로필 이름과 서버별 호스트·포트·사용자를 입력합니다. 서버를 필요한 만큼 추가하고 순서를 정합니다. 마지막 서버가 전송 대상입니다.
2. **연결하기**를 누릅니다. 최초 서버 키 지문을 확인하고 서버별 비밀번호를 입력합니다. 비밀번호 저장 선택은 기본 해제입니다.
3. **파일 전송**에서 내 PC와 최종 서버의 폴더를 엽니다. 원본 파일·폴더를 선택하고 업로드 또는 다운로드합니다.
4. 전송 확인 창에서 대상 경로와 제외 항목, 충돌 정책을 확인합니다. 같은 이름은 기본적으로 건너뛰며 덮어쓰기는 해당 작업에만 적용합니다.
5. **작업 기록**에서 파일별 결과를 확인합니다. 중단된 작업은 원래 경로로 연결한 후 **미완료 재시도**를 실행합니다. 완료 파일은 유지하고 미완료 파일을 처음부터 전송합니다.

앱 재실행 시 자동 연결·전송하지 않습니다. 조사 미완료 작업은 새로 선택해 조사합니다. `확인 필요` 항목은 원본·대상을 확인한 뒤 새 작업을 만드세요. 임시 파일 경로는 상세 화면에 표시되며 소유권을 확인할 수 없는 파일은 자동 삭제하지 않습니다. 서버 키 변경은 연결을 차단합니다.

## 개발 실행

Node.js 24, Rust stable 및 OS별 Tauri 빌드 환경이 필요합니다. macOS에서는 Xcode Command Line Tools, Windows에서는 MSVC C++ Build Tools와 WebView2가 필요합니다.

```sh
npm ci
npm run tauri -- dev
```

이 작업 공간의 전용 Rust 도구체인을 사용할 때는 `./scripts/dev.sh`로 실행합니다. `npm run dev`만 실행하면 화면 확인용 브라우저가 열리며 실제 SSH 기능은 데스크톱 앱에서만 동작합니다.

```sh
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml
npm run tauri -- build --bundles app
```

Windows 패키지 빌드:

```powershell
npm run tauri -- build --config src-tauri/tauri.windows.conf.json
```

`.github/workflows/build.yml`은 수동 실행 시 macOS 앱 ZIP과 Windows NSIS 설치 파일을 Actions artifact로 생성합니다. 제품 소스와 Actions 구성은 [GitHub 저장소](https://github.com/DabangKim/RouteTransfer)에 업로드했습니다. Windows 빌드 결과는 Actions에서 확인합니다. GitHub Release 게시는 별도입니다. 생성한 로컬 앱은 배포용 Developer ID 서명·공증을 완료한 제품이 아닙니다.

## 검증 자료

- [구현 및 검증 결과](RouteTransfer_Implementation_Status.md)
- [실제 제품 엔진 통합 검사](tests/results/product-integration.json)
- [화면 동작 검사](tests/results/ui.json)
- [Windows 검증 안내](WINDOWS_TEST_GUIDE.md)

제품 데이터는 OS 앱 데이터 폴더의 `app.routetransfer.desktop`에 보관합니다. macOS는 `~/Library/Application Support/app.routetransfer.desktop`입니다. SQLite에는 프로필·전송 기록·신뢰한 서버 키를 저장하며, 저장을 선택한 비밀번호는 OS 보안 저장소에만 저장합니다. 기록은 자동 삭제하지 않습니다.
