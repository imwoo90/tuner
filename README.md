# tuner (Wooductor)

[![Rust Compile & Test](https://github.com/imwoo90/tuner/actions/workflows/rust.yml/badge.svg)](https://github.com/imwoo90/tuner/actions)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

`tuner`는 Rust 기반의 **에이전트 슈퍼바이저 및 자동화 런타임(Agent Supervisor & Automation Runtime)**입니다. 기존 Python 기반의 `ductor_for_agy` 서비스를 대체하여 성능과 안전성, 엄격한 코드 크기 제한(compile-time checked logical limits)을 충족하도록 밑바닥부터 설계되었습니다.

Antigravity CLI(`agy`)의 실행을 감시하고 제어하며, 멀티플랫폼 메시징(Telegram, Matrix), 웹훅 수신, 백그라운드 태스크 제어 및 다국어 로컬라이징 세션을 제공합니다.

---

## 🔑 주요 기능

- **메시저 봇 연동**: 텔레그램(Telegram) 및 매트릭스(Matrix) 프로토콜을 백엔드로 연동하여 AI 에이전트와 실시간 대화 및 백그라운드 실행을 제어할 수 있습니다.
- **세션 및 대화 영속성**: JSON 파일 기반의 영속성 저장소를 통해 채팅방 및 토픽(Topic)별 AI 대화 상태와 가용 비용(USD), 토큰 소모량을 세션별로 정밀 기록합니다.
- **웹훅 & API 서버 (Axum)**: 에이전트 상태 및 결과 리포트를 외부 서비스와 동기화할 수 있도록 서명 검증(HMAC-SHA256), 토큰 인증, 처리율 제한(Rate Limiting)이 구현된 비동기 API 서버를 제공합니다.
- **백그라운드 태스크 관리자**: 장시간 수행되는 에이전트 태스크를 PTY 세션을 활용해 백그라운드에서 실행하고 실시간 스트리밍 관측 및 타임아웃을 강제합니다.
- **DAG 태스크 러너**: 작업공간 내의 복합 태스크 의존성을 분석하여 DAG 형태로 순차/병렬 스케줄링하고 상태를 추적합니다.
- **워크스페이스 규칙 및 스킬 동기화**: `CLAUDE.md`, `GEMINI.md`, `AGENTS.md` 등의 규칙 선정 스키마와 커스텀 스킬 목록을 동적으로 동기화하여 프로젝트 환경을 초기화합니다.
- **크론(Cron) 스케줄링**: 주기적인 모니터링, 체크인, 자동 빌드 스케줄을 처리하고 야간 무음 모드(Quiet Hours)를 존중하도록 설계되었습니다.
- **다국어(i18n) 지원**: 영어(en)를 기본값으로 하며, 한국어(ko)를 포함한 9개국 언어 설정을 세션별로 지원합니다. `/lang` 슬래시 커맨드를 통해 실시간으로 변경 가능합니다.

---

## 🛠️ 시스템 아키텍처

`tuner`는 강력한 모듈화 원칙에 입각하여 구성되어 있습니다.

- `src/cli/antigravity`: `agy` CLI 래퍼, 이벤트 파서 및 모듈 탐색.
- `src/session`: 세션 키 관리, 데이터 영속화 및 수명 주기 관리.
- `src/telegram`: 메시지 수신, 포맷팅(HTML/Markdown) 및 명령어 매칭 라우터.
- `src/background`: Tokio PTY 세션 래퍼, 비동기 CLI 러너 및 타임아웃 감시.
- `src/security`: 경로 바운더리 체크, 디렉터리 접근 제어 및 안전 필터링.
- `src/webhook` & `src/tasks`: Axum API 서버 및 호스트 기반 DAG 태스크 러너.
- `src/i18n`: TOML 로더 기반 다국어 현지화 모듈.
- `src/messenger/matrix`: Matrix 클라이언트 연동 및 이벤트 디스패처.

---

## 🚀 시작하기

### 요구 사항
- **Rust & Cargo**: Rust 1.75 버전 이상이 권장됩니다.
- **Antigravity CLI (`agy`)**: 시스템 경로(`$PATH`)에 `agy` 실행 파일이 위치해야 합니다.

### 컴파일 및 빌드
```bash
# 개발용 컴파일
cargo check

# 릴리즈용 빌드
cargo build --release
```

### 실행 및 백포 배포 (Systemd)
`tuner`는 개발자 편의를 위해 `systemd --user` 서비스 데몬 등록 자동화 스크립트를 지원합니다.
```bash
# 실행 파일의 systemd 서비스 유닛 등록
./target/release/tuner --install-systemd
```

등록 후 아래 명령어로 제어할 수 있습니다.
```bash
# 서비스 활성화 및 재시작
systemctl --user daemon-reload
systemctl --user enable tuner.service
systemctl --user restart tuner.service

# 로그 모니터링
journalctl --user -u tuner.service -f
```

---

## ⚙️ 설정 가이드 (`config.json`)

기본 설정 파일은 `~/.tuner/config.json` 경로에 위치하며 아래와 같은 필드를 지원합니다.

```json
{
  "telegram_token": "YOUR_TELEGRAM_BOT_TOKEN",
  "allowed_user_ids": [123456789],
  "allowed_group_ids": [-100123456789],
  "provider": "antigravity",
  "model": "gemini-3.5-flash",
  "language": "en",
  "timezone": "Asia/Seoul",
  "matrix": {
    "homeserver_url": "https://matrix.org",
    "username": "@tuner_bot:matrix.org",
    "password": "YOUR_MATRIX_PASSWORD",
    "room_whitelist": ["!room_id:matrix.org"]
  }
}
```

---

## 🤖 텔레그램 슬래시 커맨드

채팅창에서 `/` 키를 입력하거나 자동 완성 기능을 활용하여 아래 명령어들을 조작할 수 있습니다.

| 명령어 | 설명 |
|---|---|
| `/new` \| `/reset` | 대화 세션을 초기화하고 새로운 세션을 시작합니다. |
| `/status` | 에이전트 설치 여부, 세션 수, 활성 AI 모델 등의 상태 및 진단 보고서를 출력합니다. |
| `/model` | 세션에서 사용할 인공지능 모델을 인라인 버튼으로 선택 및 변경합니다. |
| `/lang` | 세션의 주 언어(한국어, 영어 등)를 변경할 수 있는 인라인 인터랙티브 버튼을 노출합니다. |
| `/memory` | 현재 작업공간의 `MAINMEMORY.md` 핵심 기억 파일 내용을 조회합니다. |
| `/stop` | 현재 토픽/채팅방에서 기동 중인 에이전트 CLI 프로세스를 모두 안전하게 중단시킵니다. |
| `/abort` | 전체 활성 워커 노드들의 태스크를 강제 중지시킵니다. |
| `/restart` | 봇의 `tuner` 프로세스를 재부팅합니다. |
| `/plan` | 실행 전에 단계별 실행 계획(Plan)을 선제적으로 생성할 것을 지시합니다. |
| `/grill_me` | 대화형 인터뷰 세션을 열어 작업 계획과 정합성을 사전에 정교화합니다. |
| `/goal` | 오랜 시간이 소요되는 철저한 장기 실행 태스크(Overnight) 모드로 동작시킵니다. |
| `/learn` | 봇 동작에 대한 피드백 및 행동 수정 사항을 영속 기록에 바인딩합니다. |
| `/teamwork_preview` | 협업 에이전트 시뮬레이션 환경을 실행합니다. |

---

## 🧪 테스트 코드 실행

Tuner의 기능 안전성은 460개가 넘는 단위(Unit) 및 통합(Integration) 테스트 코드로 완벽하게 검증되어 있습니다.

```bash
# 모든 단위 테스트 및 시나리오 검증 수행
cargo test
```

---

## 📄 라이선스
This project is licensed under the [MIT License](LICENSE).
