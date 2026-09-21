# RouteTransfer Windows 검증 안내

## 준비할 파일

GitHub에 로그인한 뒤 [빌드 결과 페이지](https://github.com/DabangKim/RouteTransfer/actions/runs/35582150222)의 **Artifacts**에서 **RouteTransfer-Windows** 압축 파일을 내려받고 압축을 풉니다.

- `RouteTransfer_…_x64-setup.exe`: Windows x64 설치 파일
- `SHA256SUMS.txt`: 설치 파일 무결성 확인 값
- `build-info.json`: 버전과 소스 커밋
- 이 안내 문서

이 구성은 Windows x64 PC용입니다. ARM64 PC는 별도 지원 확인이 필요합니다. 회사 PC에는 Node.js, Rust, Python, Docker를 설치할 필요가 없습니다. WebView2 오프라인 설치 프로그램을 패키지에 포함하도록 구성했습니다. 회사의 프로그램 실행 정책에 따라 IT 승인이 필요할 수 있습니다. 테스트 패키지는 코드 서명 전 버전입니다.

Windows 빌드와 7개 Rust 단위 테스트를 통과했습니다. [Windows 검증 패키지 다운로드](https://github.com/DabangKim/RouteTransfer/actions/runs/35582150222/artifacts/10630932517) · 설치 파일: `RouteTransfer_0.1.0_x64-setup.exe`. 회사 PC에서 실제 설치 및 서버 연결 검증은 다음 절차로 진행합니다.

## 설치와 첫 실행

1. `x64-setup.exe`를 실행하여 현재 사용자용으로 설치합니다.
2. 시작 메뉴에서 RouteTransfer를 엽니다.
3. 연결 관리·파일 전송·작업 기록·설정 화면이 열리는지 확인합니다.
4. 연결 관리에서 실제 서버 주소·포트·계정을 입력하고 경유 순서를 지정합니다. 마지막 서버가 최종 전송 대상입니다.
5. 서버 키 지문을 확인한 뒤 서버별 비밀번호를 입력합니다. 처음에는 비밀번호 저장을 선택하지 않습니다.

## 회사 서버에서 검증할 순서

업무 원본 대신 별도 테스트 폴더와 복사본을 사용합니다.

| 순서 | 확인 사항 | 통과 기준 |
|---|---|---|
| 1 | 연결 테스트 | 모든 경유 서버 인증 및 최종 폴더 조회 성공 |
| 2 | 탐색 | Windows 로컬 폴더와 Ubuntu 원격 폴더 표시 |
| 3 | 업로드 | 사진 3개·한글 이름·중첩 폴더·빈 폴더 전송 및 구조 유지 |
| 4 | 다운로드 | 새 로컬 폴더로 내려받고 원본과 크기·SHA-256 일치 |
| 5 | 건너뛰기 | 같은 파일을 다시 보내면 기본값으로 건너뜀 |
| 6 | 덮어쓰기 | 테스트 복사본 수정 후 명시적으로 덮어쓰면 새 내용 반영 |
| 7 | 중단·재시도 | 여러 파일 중 전송을 취소하고 재시도. 완료 파일의 시도 횟수는 유지 |
| 8 | 재실행 | 앱 종료 후 기록 유지, 자동 연결·전송 없음 |
| 9 | 비밀번호 저장(선택) | 저장을 선택한 서버로 재연결 후 설정 화면에서 저장 정보 삭제 가능 |

내용 비교가 필요하면 PowerShell에서 원본과 내려받은 파일 각각에 실행합니다.

```powershell
Get-FileHash -Algorithm SHA256 -LiteralPath 'C:\테스트\사진 01.jpg'
```

결과를 공유할 때는 앱 버전, Windows 버전, 서버 수, 수행한 순서, 성공·실패와 화면 오류 문구를 남기면 됩니다. 비밀번호는 포함하지 않습니다. 100G 이상 전송은 이번 소량 검증과 별개이며 현재까지 실측하지 않았습니다.

## 개발 담당자: Windows 빌드

GitHub Actions에서 수동 실행하면 Windows runner가 테스트 후 설치 파일을 생성합니다. 다운로드 가능한 artifact 생성까지가 이 workflow의 범위이며 GitHub Release 게시는 별도입니다.

Windows 개발 환경에서 직접 빌드할 때:

```powershell
npm ci
npm test
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml
npm run tauri -- build --target x86_64-pc-windows-msvc --config src-tauri/tauri.windows.conf.json
```

개발 PC에는 Node.js 24, Rust MSVC 도구체인, Visual Studio C++ Build Tools가 필요합니다. 결과는 `src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/`에 생성됩니다.

WebView2 구성 근거: [Tauri 공식 Windows installer 문서](https://v2.tauri.app/distribute/windows-installer/#webview2-installation-options).
