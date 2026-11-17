import { escapeHtml } from "/dashboard/assets/markdown.js";
import { getLang, t } from "/dashboard/assets/i18n.js";

const KO_DESC = {
  new: "현재 맥락을 요약 인계하고 새 세션 시작",
  reset: "/new와 동일 — 세션 초기화",
  stop: "실행 중인 응답·도구 작업 즉시 중지",
  abort: "실행 중인 작업 강제 중지",
  usage: "프로바이더별 쿼터 및 토큰 사용량 열기",
  cron: "스케줄 작업(Cron) 목록 및 즉시 실행",
  vault: "암호화된 사이트 자격 증명(Vault) 열기",
  security: "PIN 비밀번호 설정 및 로그인 세션 관리",
  status: "현재 세션 및 백그라운드 에이전트 상태 보기",
  provider: "AI 엔진 변경 (예: /provider claude)",
  model: "모델 변경 (예: /model sonnet)",
  effort: "추론 강도 변경 (low · medium · high · xhigh)",
  memory: "워크스페이스 핵심 메모리(MAINMEMORY) 조회 요청",
  lang: "인터페이스 언어 전환 (한국어 / English)",
  help: "사용 가능한 슬래시 명령어 및 단축키 보기",
  compact: "긴 대화 컨텍스트 압축 및 요약",
  plan: "실행 전 단계별 설계·계획 수립 요청",
  grill_me: "요구사항 구체화를 위한 역질문 인터뷰 시작",
  goal: "목표 달성까지 멈추지 않는 심층 자율 실행",
  teamwork: "다중 에이전트 협업(Swarm/Teamwork) 실행",
  boost: "다각도 심층 추론 및 교차 검증 실행",
  learn: "피드백·교정 내용을 규칙 및 메모리에 학습",
  browser: "웹 브라우저 탐색·상호작용 워크플로우",
  schedule: "반복 스케줄 또는 일회성 타이머 예약",
  review: "변경된 코드 및 작업 내역 정밀 코드 리뷰",
  security_review: "보안 취약점 및 권한 경계 집중 감사",
  verify: "테스트·요구사항 대비 구현 결과 검증",
  debug: "오류 원인 추적 및 근본 수정",
  simplify: "복잡한 코드 단순화 및 리팩토링",
  deep_research: "다중 소스 심층 코드베이스·기술 리서치",
  code_review: "다중 패스 자동 코드 리뷰 실행",
  init: "프로젝트 에이전트 지침 문서 초기화"
};

const INSTANT_BUILTINS = new Set([
  "new", "reset", "stop", "abort", "usage", "cron", "vault", "security", "status", "help", "lang"
]);

export function describeCommand(cmdObj) {
  if (getLang() === "ko") {
    return KO_DESC[cmdObj.command] || cmdObj.description || "";
  }
  return cmdObj.description || KO_DESC[cmdObj.command] || "";
}

export function createSlashController({
  inputEl,
  menuEl,
  slashBtnEl,
  getProvider,
  getCommands,
  getModels,
  onRunBuiltin
}) {
  let items = [];
  let activeIndex = 0;
  let mode = "";

  function closeMenu() {
    items = [];
    activeIndex = 0;
    mode = "";
    menuEl.classList.add("hidden");
    menuEl.innerHTML = "";
  }

  function renderMenu() {
    if (!items.length) {
      closeMenu();
      return;
    }
    menuEl.classList.remove("hidden");
    menuEl.innerHTML = items
      .map((it, idx) => {
        const activeCls = idx === activeIndex ? " active" : "";
        return `<button type="button" class="ac-item${activeCls}" data-idx="${idx}">
          <div class="ac-main">
            <span class="ac-token">${escapeHtml(it.display)}</span>
            <span class="ac-desc">${escapeHtml(it.desc)}</span>
          </div>
          <span class="ac-badge">${escapeHtml(it.badge)}</span>
        </button>`;
      })
      .join("");

    menuEl.querySelectorAll(".ac-item").forEach((btn) => {
      btn.addEventListener("mousedown", (e) => {
        e.preventDefault();
        const idx = Number(btn.dataset.idx || 0);
        applySelection(idx, "click");
      });
    });
    const activeEl = menuEl.querySelector(".ac-item.active");
    activeEl?.scrollIntoView({ block: "nearest" });
  }

  function computeMatches(forceAllSlash = false) {
    const val = inputEl.value;
    if (forceAllSlash && !val.trim()) {
      inputEl.value = "/";
      inputEl.focus();
    }
    const raw = inputEl.value;
    if (raw.includes("\n")) {
      closeMenu();
      return;
    }
    const trimmedStart = raw.trimStart();
    const firstSpace = trimmedStart.indexOf(" ");
    if (firstSpace !== -1) {
      closeMenu();
      return;
    }

    if (trimmedStart.startsWith("/")) {
      const q = trimmedStart.slice(1).toLowerCase();
      const cmds = getCommands() || [];
      const prov = getProvider();
      mode = "slash";
      items = cmds
        .filter((c) => {
          if (!q) return true;
          const ko = (KO_DESC[c.command] || "").toLowerCase();
          const en = (c.description || "").toLowerCase();
          return c.command.toLowerCase().includes(q) || ko.includes(q) || en.includes(q);
        })
        .slice(0, 18)
        .map((c) => ({
          type: "slash",
          command: c.command,
          kind: c.kind,
          display: `/${c.command}`,
          desc: describeCommand(c),
          badge: c.kind === "builtin" ? t("acBuiltinBadge") : t("acWorkflowBadge", { prov })
        }));
      activeIndex = 0;
      renderMenu();
      return;
    }

    if (trimmedStart.startsWith("@")) {
      const q = trimmedStart.slice(1).toLowerCase();
      const models = getModels() || [];
      mode = "model";
      items = models
        .filter((m) => !q || m.model.toLowerCase().includes(q))
        .slice(0, 14)
        .map((m) => ({
          type: "model",
          model: m.model,
          display: `@${m.model}`,
          desc: m.efforts?.length
            ? t("acEffortsDesc", { list: m.efforts.join(" · ") })
            : t("acFixedEffortDesc"),
          badge: t("acModelBadge")
        }));
      activeIndex = 0;
      renderMenu();
      return;
    }

    closeMenu();
  }

  function applySelection(idx, trigger) {
    const item = items[idx];
    if (!item) return false;

    if (item.type === "model") {
      inputEl.value = `@${item.model} `;
      closeMenu();
      inputEl.focus();
      return true;
    }

    if (item.type === "slash") {
      if (trigger !== "tab" && INSTANT_BUILTINS.has(item.command)) {
        inputEl.value = "";
        inputEl.style.height = "auto";
        closeMenu();
        onRunBuiltin(item.command, "");
        return true;
      }
      const exactNow = inputEl.value.trim() === `/${item.command}`;
      if (trigger === "enter" && exactNow && item.kind === "workflow") {
        closeMenu();
        return false;
      }
      inputEl.value = `/${item.command} `;
      closeMenu();
      inputEl.focus();
      return true;
    }
    return false;
  }

  function handleKeyDown(e) {
    if (menuEl.classList.contains("hidden") || !items.length) {
      return false;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      activeIndex = (activeIndex + 1) % items.length;
      renderMenu();
      return true;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      activeIndex = (activeIndex - 1 + items.length) % items.length;
      renderMenu();
      return true;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      closeMenu();
      return true;
    }
    if (e.key === "Tab") {
      e.preventDefault();
      applySelection(activeIndex, "tab");
      return true;
    }
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
      const handled = applySelection(activeIndex, "enter");
      if (handled) {
        e.preventDefault();
        return true;
      }
    }
    return false;
  }

  inputEl.addEventListener("input", () => computeMatches(false));
  inputEl.addEventListener("blur", () => setTimeout(closeMenu, 140));
  slashBtnEl?.addEventListener("click", () => {
    if (!menuEl.classList.contains("hidden") && mode === "slash") {
      closeMenu();
      return;
    }
    computeMatches(true);
  });

  return { handleKeyDown, closeMenu, refresh: () => computeMatches(false) };
}

export function parseBuiltinSlashInput(rawText) {
  const trimmed = rawText.trim();
  if (!trimmed.startsWith("/")) return null;
  const spaceIdx = trimmed.indexOf(" ");
  const token = (spaceIdx === -1 ? trimmed.slice(1) : trimmed.slice(1, spaceIdx)).split("@")[0].toLowerCase();
  const args = spaceIdx === -1 ? "" : trimmed.slice(spaceIdx + 1).trim();
  const builtins = new Set([
    "new", "reset", "stop", "abort", "usage", "cron", "vault", "security",
    "status", "help", "provider", "model", "effort", "lang"
  ]);
  if (!builtins.has(token)) return null;
  return { command: token, args };
}
