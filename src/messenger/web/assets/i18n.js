const STORAGE_KEY = "tuner_lang";
const listeners = new Set();

const DICT = {
  ko: {
    newTopicBtn: "+ 새 대화",
    newTopicTitle: "새 웹 대화방 만들기",
    searchPlaceholder: "대화 검색...",
    topicNamePlaceholder: "대화 제목 (선택 사항)",
    cancel: "취소",
    create: "만들기",
    selectTopic: "대화를 선택하세요",
    newSession: "새 세션",
    newSessionTitle: "이전 맥락을 요약 인계하고 새 세션을 시작합니다 (/new)",
    toolsPanel: "도구 패널",
    toolsPanelTitle: "파일 미리보기 · 사용량 · 스케줄 · 보안 패널",
    closePanel: "패널 닫기",
    dropFiles: "파일을 놓아 첨부하기",
    emptyNoSession: "좌측 목록에서 대화를 선택하거나 + 새 대화를 눌러 시작하세요.",
    emptyNoMessages: "메시지를 입력하거나 / 를 눌러 명령어 목록을 열어보세요.",
    noMatchingSessions: "일치하는 대화가 없습니다.",
    webTopicFallback: "새 대화 #{id}",
    tgTopicFallback: "텔레그램 #{id}",
    tgMirroredTitle: "텔레그램 연동 토픽",
    deleteTopicTitle: "대화 삭제",
    streaming: "작성 중…",
    runningTitle: "실행 중",
    wsConnected: "실시간 연결됨",
    wsReconnecting: "재연결 중...",
    switchToLight: "라이트 모드로 전환",
    switchToDark: "다크 모드로 전환",
    langToggleTitle: "언어 전환 (한국어 / English)",
    askBadge: "확인 질문",
    askWriteinPlaceholder: "직접 답변 입력...",
    askSendBtn: "답변 전송",
    askPrevBtn: "← 이전",
    askSubmitBtn: "선택 완료",
    askSkipBtn: "건너뛰기",
    composerPlaceholder: "메시지를 입력하세요... ('/' 명령어 자동완성, '@' 모델 전환)",
    composerAskPlaceholder: "위 확인 질문에 대한 답변을 입력하거나 선택지를 클릭하세요...",
    attachTitle: "파일 첨부",
    slashBtnTitle: "슬래시 명령어 보기 (/)",
    providerTitle: "AI 프로바이더",
    modelTitle: "AI 모델",
    effortTitle: "추론 강도",
    stopBtn: "■ 중지",
    stopBtnTitle: "실행 중지 (/stop)",
    sendBtnTitle: "전송 (Enter)",
    copy: "복사",
    copied: "복사됨",
    cotSummary: "사고 과정 · 도구 실행",
    acBuiltinBadge: "바로 실행",
    acWorkflowBadge: "{prov} 워크플로우",
    acModelBadge: "모델 지정",
    acEffortsDesc: "지원 강도: {list}",
    acFixedEffortDesc: "추론 강도 고정 모델",
    helpHeader: "사용 가능한 명령어 ({prov})",
    helpTip: "**팁**: 입력창에서 `/` 를 입력하면 명령어 자동완성이, `@` 를 입력하면 모델 지정 목록이 열립니다.",
    langChangedNotice: "인터페이스 언어가 **한국어(KO)**로 변경되었습니다.",
    tabPreview: "미리보기",
    tabUsage: "사용량",
    tabCron: "스케줄",
    tabVault: "보안 금고",
    tabSecurity: "PIN·세션",
    tabTree: "에이전트",
    loadingPreview: "파일 불러오는 중…",
    loadingUsage: "프로바이더 사용량 조회 중…",
    loadingCron: "예약 스케줄 불러오는 중…",
    loadingVault: "보안 금고 불러오는 중…",
    loadingSecurity: "인증 정보 불러오는 중…",
    loadingTree: "에이전트 상태 확인 중…",
    refresh: "새로고침",
    open: "열기",
    download: "다운로드",
    viewRendered: "렌더링 보기",
    viewSource: "원본 보기",
    previewPlaceholder: "워크스페이스 파일 경로 입력...",
    previewEmptyHint: "대화 속 📄 파일 칩을 클릭하거나 위 입력창에 경로를 입력하면 여기서 바로 미리볼 수 있습니다.",
    previewDenied: "접근 거부됨 ({status})",
    previewBinary: "바이너리 파일 ({lang}).",
    previewTruncated: "512KB까지만 표시됨",
    warmPoolsTitle: "대기 중인 엔진 풀 (Warm Pools)",
    pidDetails: "세부 프로세스 PID",
    noActiveSubagents: "현재 실행 중인 서브에이전트나 백그라운드 작업이 없습니다.",
    subagentFallback: "서브에이전트",
    bgTaskPrefix: "백그라운드 작업",
    rootSession: "메인 세션",
    childSession: "하위 세션",
    treeLoadError: "에이전트 상태를 불러오지 못했습니다 ({status}).",
    usageTitle: "프로바이더 쿼터 및 토큰",
    usageLoadError: "사용량 조회 실패 ({status}).",
    usageReady: "연결됨",
    fiveHourLimit: "5시간 한도",
    weeklyLimit: "주간 한도",
    geminiGroup: "Gemini 모델군",
    claudeGptGroup: "Claude / GPT 모델군",
    claudeSubGroup: "Claude 구독 한도",
    codexGroup: "Codex 한도",
    turnsLabel: "턴",
    inTokensLabel: "입력",
    outTokensLabel: "출력",
    tokensLabel: "토큰",
    cronTitle: "예약된 스케줄 ({count})",
    cronLoadError: "스케줄 목록 조회 실패 ({status}).",
    cronEmpty: "등록된 예약 작업이 없습니다.",
    cronEnabled: "활성",
    cronPaused: "일시정지",
    cronNext: "다음 실행",
    cronLast: "최근 결과",
    cronNever: "없음",
    cronPauseBtn: "일시정지",
    cronEnableBtn: "활성화",
    cronRunNowBtn: "즉시 실행",
    cronTriggeredBtn: "실행 요청됨",
    vaultTitle: "자격 증명 금고 ({profile})",
    vaultLoadError: "보안 금고 오류 ({status}).",
    hitlActive: "브라우저 인증 진행 중",
    hitlIdle: "유휴",
    vaultZeroKnowledgeNote: "비밀번호는 암호화 저장되며 API로 절대 평문 반환되지 않습니다.",
    vaultFormTitle: "계정 등록 / 수정",
    vaultKeyPh: "식별 키 (예: github)",
    vaultNamePh: "사이트 이름 (예: GitHub)",
    vaultUserPh: "아이디 또는 이메일",
    vaultPassPh: "비밀번호 (수정 시 비워두면 기존 값 유지)",
    vaultSaveBtn: "암호화 저장",
    vaultDeleteBtn: "삭제",
    vaultAccountLabel: "계정",
    secStatusTitle: "로그인 및 세션 상태",
    secProfileLabel: "프로필",
    secPinConfigured: "고정 PIN 설정됨",
    yes: "예",
    no: "아니오",
    secPinFormTitle: "고정 PIN / 패스프레이즈 설정",
    secPinPh: "새 PIN 또는 패스프레이즈 (4자 이상)",
    secPinSaveBtn: "PIN 저장",
    secPinSavedMsg: "새 PIN이 안전하게 저장되었습니다.",
    secPinFailMsg: "PIN 저장에 실패했습니다.",
    secRevokeTitle: "브라우저 세션 관리",
    secLogoutBtn: "현재 브라우저 로그아웃",
    secRevokeAllBtn: "모든 기기 세션 만료",
    loginSubtitle: "개인 AI 에이전트 워크스페이스",
    loginPinLabel: "대시보드 PIN 또는 패스프레이즈",
    loginPinPh: "설정한 PIN 입력",
    loginSubmitBtn: "대시보드 열기",
    loginNoPinTitle: "아직 설정된 PIN이 없습니다.",
    loginNoPinBody: "텔레그램 채팅방에서 /dashboard 를 입력하거나 터미널에서 일회용 매직 링크를 발급해 주세요:",
    loginNoPinHint: "접속 후 우측 도구 패널 → PIN·세션에서 고정 PIN을 등록할 수 있습니다.",
    loginAuthReq: "인증이 필요합니다 ({err}).",
    loginFailed: "로그인 실패 ({status})",
    loginNetErr: "네트워크 오류로 PIN을 확인하지 못했습니다.",
    segWeb: "웹 대화",
    segTg: "텔레그램",
    newTgTopicBtn: "+ 새 토픽",
    newTgTopicTitle: "실제 텔레그램 앱에 새 토픽을 생성하고 연동합니다",
    tgTopicNamePh: "새 텔레그램 토픽 제목 (비워두면 자동 지정)",
    createTgTopicBtn: "토픽 생성",
    creatingBtn: "생성 중…",
    formModeWeb: "💻 웹 전용 대화 (텔레그램에 표시되지 않음)",
    formModeTg: "✈️ 실제 텔레그램 앱에 새 토픽이 생성됩니다",
    channelBadgeWeb: "웹 전용",
    channelBadgeTg: "✈️ 텔레그램 연동",
    tgTabHint: "새 토픽 생성·이름 변경·대화가 텔레그램과 실시간 양방향 동기화됩니다.",
    emptyNoWebSessions: "아직 웹 대화가 없습니다. 위 + 새 대화를 눌러 시작하세요.",
    emptyNoTgSessions: "아직 연동된 텔레그램 토픽이 없습니다. 위 + 새 토픽을 눌러 생성하세요.",
    grpPinned: "고정됨",
    grpToday: "오늘",
    grpYesterday: "어제",
    grpPrev7: "지난 7일",
    grpPrev30: "지난 30일",
    grpOlder: "이전",
    pinTopicTitle: "상단 고정",
    unpinTopicTitle: "고정 해제",
    renameTopicTitle: "이름 변경",
    deleteTgTopicTitle: "텔레그램 토픽 및 대시보드 기록 삭제 (한 번 더 누르면 삭제)",
    confirmDeleteTgBtn: "토픽 삭제",
    pruneBtn: "빈 대화·오래된 기록 정리",
    pruneBtnTitle: "현재 탭에서 비어 있거나 30일 이상 지난 미고정 대화를 정리합니다",
    pruneDoneToast: "{count}개 정리 완료",
    tabBrowser: "브라우저",
    loadingBrowser: "원격 브라우저 세션 확인 중…",
    launchingBrowser: "원격 Chrome 브라우저 기동 중…",
    hitlBannerText: "원격 브라우저 인증 대기 중 ({site})",
    hitlBannerOpenBtn: "패널에서 열기",
    browserLaunchTitle: "웹안웹(Web-in-Web) 원격 브라우저",
    browserLaunchHint: "직접 로그인·2FA·본인인증이 필요할 때 원격 Chrome 화면을 띄워 인증 세션을 금고에 저장합니다.",
    browserTargetPh: "금고 키(예: github) 또는 https://... 주소",
    browserLaunchBtn: "원격 브라우저 열기",
    browserNewTabBtn: "새 창",
    browserDoneNotice: "인증 세션({site}) 저장이 완료되었습니다.",
    browserLaunchErr: "원격 브라우저 실행 실패",
    vaultAuthBtn: "🌐 브라우저 인증"
  },
  en: {
    newTopicBtn: "+ New Chat",
    newTopicTitle: "Create a new web topic",
    searchPlaceholder: "Search chats...",
    topicNamePlaceholder: "Topic title (optional)",
    cancel: "Cancel",
    create: "Create",
    selectTopic: "Select a conversation",
    newSession: "New Session",
    newSessionTitle: "Start a fresh session with context handover (/new)",
    toolsPanel: "Inspector",
    toolsPanelTitle: "File Preview · Usage · Schedules · Security",
    closePanel: "Close panel",
    dropFiles: "Drop files to attach",
    emptyNoSession: "Select a conversation on the left or click + New Chat to begin.",
    emptyNoMessages: "Type a message or press / to browse slash commands.",
    noMatchingSessions: "No matching conversations.",
    webTopicFallback: "Web #{id}",
    tgTopicFallback: "Telegram #{id}",
    tgMirroredTitle: "Mirrored Telegram topic",
    deleteTopicTitle: "Delete topic",
    streaming: "Generating…",
    runningTitle: "Running",
    wsConnected: "Real-time connected",
    wsReconnecting: "Reconnecting...",
    switchToLight: "Switch to Light mode",
    switchToDark: "Switch to Dark mode",
    langToggleTitle: "Switch language (한국어 / English)",
    askBadge: "Question",
    askWriteinPlaceholder: "Type a custom answer...",
    askSendBtn: "Send Answer",
    askPrevBtn: "← Prev",
    askSubmitBtn: "Submit Selected",
    askSkipBtn: "Skip",
    composerPlaceholder: "Message Tuner... ('/' for commands, '@' to switch model)",
    composerAskPlaceholder: "Type an answer to the question above or click an option...",
    attachTitle: "Attach file",
    slashBtnTitle: "Browse slash commands (/)",
    providerTitle: "AI Provider",
    modelTitle: "AI Model",
    effortTitle: "Reasoning Effort",
    stopBtn: "■ Stop",
    stopBtnTitle: "Interrupt active run (/stop)",
    sendBtnTitle: "Send (Enter)",
    copy: "Copy",
    copied: "Copied",
    cotSummary: "Reasoning & Tool Calls",
    acBuiltinBadge: "Built-in",
    acWorkflowBadge: "{prov} workflow",
    acModelBadge: "Model",
    acEffortsDesc: "Efforts: {list}",
    acFixedEffortDesc: "Fixed reasoning effort",
    helpHeader: "Available Slash Commands ({prov})",
    helpTip: "**Tip**: Type `/` to open command autocomplete, or `@` to switch models inline.",
    langChangedNotice: "Interface language switched to **English (EN)**.",
    tabPreview: "Preview",
    tabUsage: "Usage",
    tabCron: "Cron",
    tabVault: "Vault",
    tabSecurity: "Security",
    tabTree: "Agents",
    loadingPreview: "Loading file preview…",
    loadingUsage: "Fetching provider usage…",
    loadingCron: "Loading scheduled jobs…",
    loadingVault: "Loading credential vault…",
    loadingSecurity: "Loading session security…",
    loadingTree: "Checking active agents…",
    refresh: "Refresh",
    open: "Open",
    download: "Download",
    viewRendered: "Rendered",
    viewSource: "Source",
    previewPlaceholder: "Enter workspace file path...",
    previewEmptyHint: "Click any 📄 file chip in chat or enter an allowed path above to preview.",
    previewDenied: "Access denied ({status})",
    previewBinary: "Binary file ({lang}).",
    previewTruncated: "Truncated at 512KB",
    warmPoolsTitle: "Warm Pools",
    pidDetails: "Process PIDs",
    noActiveSubagents: "No active subagents or background tasks on this session.",
    subagentFallback: "Subagent",
    bgTaskPrefix: "Task",
    rootSession: "Main Session",
    childSession: "Subagent Session",
    treeLoadError: "Failed to load agent status ({status}).",
    usageTitle: "Provider Quotas & Tokens",
    usageLoadError: "Failed to load usage ({status}).",
    usageReady: "Ready",
    fiveHourLimit: "5-Hour Limit",
    weeklyLimit: "Weekly Limit",
    geminiGroup: "Gemini Models",
    claudeGptGroup: "Claude / GPT Models",
    claudeSubGroup: "Claude Subscription",
    codexGroup: "Codex Rate Limits",
    turnsLabel: "turns",
    inTokensLabel: "in",
    outTokensLabel: "out",
    tokensLabel: "tokens",
    cronTitle: "Scheduled Jobs ({count})",
    cronLoadError: "Failed to load schedules ({status}).",
    cronEmpty: "No scheduled jobs configured.",
    cronEnabled: "Enabled",
    cronPaused: "Paused",
    cronNext: "Next",
    cronLast: "Last",
    cronNever: "never",
    cronPauseBtn: "Pause",
    cronEnableBtn: "Enable",
    cronRunNowBtn: "Run Now",
    cronTriggeredBtn: "Triggered",
    vaultTitle: "Credential Vault ({profile})",
    vaultLoadError: "Vault error ({status}).",
    hitlActive: "HITL Browser Active",
    hitlIdle: "Idle",
    vaultZeroKnowledgeNote: "Zero-knowledge store: passwords are encrypted at rest and never returned by API.",
    vaultFormTitle: "Add / Update Entry",
    vaultKeyPh: "site_key (e.g. github)",
    vaultNamePh: "Title (e.g. GitHub)",
    vaultUserPh: "Username or Email",
    vaultPassPh: "Password (leave blank on edit to keep existing)",
    vaultSaveBtn: "Save Encrypted",
    vaultDeleteBtn: "Delete",
    vaultAccountLabel: "Account",
    secStatusTitle: "Session & Security",
    secProfileLabel: "Profile",
    secPinConfigured: "Owner PIN set",
    yes: "Yes",
    no: "No",
    secPinFormTitle: "Set / Update Owner PIN",
    secPinPh: "New PIN or passphrase (min 4 chars)",
    secPinSaveBtn: "Save PIN",
    secPinSavedMsg: "Owner PIN saved securely.",
    secPinFailMsg: "Failed to update PIN.",
    secRevokeTitle: "Browser Sessions",
    secLogoutBtn: "Log Out This Browser",
    secRevokeAllBtn: "Revoke All Sessions",
    loginSubtitle: "Personal AI Agent Workspace",
    loginPinLabel: "Dashboard PIN or Passphrase",
    loginPinPh: "Enter your PIN",
    loginSubmitBtn: "Unlock Dashboard",
    loginNoPinTitle: "No owner PIN configured yet.",
    loginNoPinBody: "Send /dashboard in Telegram or mint a one-time Magic Link in terminal:",
    loginNoPinHint: "Once signed in, set a persistent PIN under Inspector → Security.",
    loginAuthReq: "Authentication required ({err}).",
    loginFailed: "Login failed ({status})",
    loginNetErr: "Network error while verifying PIN.",
    segWeb: "Web",
    segTg: "Telegram",
    newTgTopicBtn: "+ New Topic",
    newTgTopicTitle: "Create a real topic in Telegram and sync live",
    tgTopicNamePh: "New Telegram topic title (optional)",
    createTgTopicBtn: "Create Topic",
    creatingBtn: "Creating…",
    formModeWeb: "💻 Web-only chat (private to Dashboard)",
    formModeTg: "✈️ Creates a real synced topic in Telegram",
    channelBadgeWeb: "Web",
    channelBadgeTg: "✈️ Telegram Sync",
    tgTabHint: "Topic creation, rename, and messages sync live with Telegram.",
    emptyNoWebSessions: "No web chats yet. Click + New Chat above to start.",
    emptyNoTgSessions: "No Telegram topics yet. Click + New Topic above to create one.",
    grpPinned: "Pinned",
    grpToday: "Today",
    grpYesterday: "Yesterday",
    grpPrev7: "Previous 7 Days",
    grpPrev30: "Previous 30 Days",
    grpOlder: "Older",
    pinTopicTitle: "Pin chat",
    unpinTopicTitle: "Unpin chat",
    renameTopicTitle: "Rename chat",
    deleteTgTopicTitle: "Delete Telegram topic & dashboard history (click again to confirm)",
    confirmDeleteTgBtn: "Del Topic",
    pruneBtn: "Clean up empty & old",
    pruneBtnTitle: "Remove empty or >30d unpinned chats in this tab",
    pruneDoneToast: "Cleaned up {count}",
    tabBrowser: "Browser",
    loadingBrowser: "Checking remote browser sessions…",
    launchingBrowser: "Launching remote Chrome browser…",
    hitlBannerText: "Remote browser auth active ({site})",
    hitlBannerOpenBtn: "Open in Panel",
    browserLaunchTitle: "In-App Remote Browser (HITL)",
    browserLaunchHint: "Complete login, 2FA, or human verification in an isolated remote Chrome and save session state to Vault.",
    browserTargetPh: "Vault site_key (e.g. github) or https://... URL",
    browserLaunchBtn: "Launch Remote Browser",
    browserNewTabBtn: "Pop out",
    browserDoneNotice: "Saved authenticated session ({site}).",
    browserLaunchErr: "Failed to launch remote browser",
    vaultAuthBtn: "🌐 Browser Auth"
  }
};

function normalizeLang(raw) {
  if (!raw) return null;
  const low = String(raw).trim().toLowerCase();
  if (low.startsWith("ko")) return "ko";
  if (low.startsWith("en")) return "en";
  return null;
}

let currentLang = (() => {
  try {
    const saved = normalizeLang(localStorage.getItem(STORAGE_KEY));
    if (saved) return saved;
  } catch (_) {}
  return normalizeLang(navigator.language) || "ko";
})();

export function getLang() {
  return currentLang;
}

export function initLang(serverDefault) {
  try {
    const saved = normalizeLang(localStorage.getItem(STORAGE_KEY));
    if (saved) {
      currentLang = saved;
    } else if (normalizeLang(serverDefault)) {
      currentLang = normalizeLang(serverDefault);
    }
  } catch (_) {}
  document.documentElement.lang = currentLang;
  return currentLang;
}

export function setLang(nextRaw) {
  const next = normalizeLang(nextRaw) || (currentLang === "ko" ? "en" : "ko");
  currentLang = next;
  document.documentElement.lang = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
  } catch (_) {}
  applyDomI18n();
  for (const fn of listeners) {
    try { fn(next); } catch (_) {}
  }
  return next;
}

export function toggleLang() {
  return setLang(currentLang === "ko" ? "en" : "ko");
}

export function onLangChange(fn) {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function t(key, vars = {}) {
  const table = DICT[currentLang] || DICT.ko;
  let str = table[key] ?? DICT.en[key] ?? DICT.ko[key] ?? key;
  for (const [k, v] of Object.entries(vars)) {
    str = str.replaceAll(`{${k}}`, String(v ?? ""));
  }
  return str;
}

export function applyDomI18n(root = document) {
  root.querySelectorAll("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });
  root.querySelectorAll("[data-i18n-placeholder]").forEach((el) => {
    el.placeholder = t(el.dataset.i18nPlaceholder);
  });
  root.querySelectorAll("[data-i18n-title]").forEach((el) => {
    el.title = t(el.dataset.i18nTitle);
  });
  root.querySelectorAll("[data-i18n-aria]").forEach((el) => {
    el.setAttribute("aria-label", t(el.dataset.i18nAria));
  });
  const langBtn = document.getElementById("btn-lang");
  if (langBtn) {
    langBtn.textContent = currentLang === "ko" ? "KO" : "EN";
    langBtn.title = t("langToggleTitle");
  }
}
