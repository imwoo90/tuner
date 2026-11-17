import { t } from "/dashboard/assets/i18n.js";

export function escapeHtml(str) {
  return String(str ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function cleanFilePath(rawUrl) {
  const withoutScheme = rawUrl.replace(/^file:\/\//i, "");
  const clean = withoutScheme.split("#")[0].split("?")[0];
  try {
    return decodeURIComponent(clean);
  } catch (_) {
    return clean;
  }
}

function utf8ToBase64(text) {
  const bytes = new TextEncoder().encode(text);
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x4000) {
    bin += String.fromCharCode(...bytes.subarray(i, i + 0x4000));
  }
  return btoa(bin);
}

export function base64ToUtf8(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new TextDecoder().decode(bytes);
}

function renderCodeBlock(lang, codeText) {
  const safeLang = escapeHtml(lang || "text");
  const lines = codeText.replace(/\n$/, "").split("\n");
  const numbered = lines
    .map((line, idx) => `<div class="code-line"><span class="ln">${idx + 1}</span><span>${escapeHtml(line)}</span></div>`)
    .join("");
  const b64 = utf8ToBase64(codeText);
  return `<div class="code-block"><div class="code-header"><span>${safeLang}</span><button type="button" class="btn btn-sm copy-code-btn" data-code-b64="${escapeHtml(b64)}">${escapeHtml(t("copy"))}</button></div><pre class="code-pre"><code>${numbered}</code></pre></div>`;
}

function renderInline(escapedLine, tokens) {
  let out = escapedLine;
  const stash = (html) => {
    const id = `@@TKN_${tokens.length}@@`;
    tokens.push(html);
    return id;
  };

  out = out.replace(/`([^`]+)`/g, (_, code) => stash(`<code>${code}</code>`));

  out = out.replace(/!?\[([^\]]+)\]\(([^)\s]+)\)/g, (full, label, rawHref) => {
    const rawDec = rawHref.replace(/&amp;/g, "&");
    if (/^file:\/\//i.test(rawDec) || (rawDec.startsWith("/") && !rawDec.startsWith("//"))) {
      const fpath = cleanFilePath(rawDec);
      return stash(`<button type="button" class="file-chip" data-file-path="${escapeHtml(fpath)}">📄 ${label}</button>`);
    }
    if (/^https?:\/\//i.test(rawDec)) {
      return stash(`<a href="${escapeHtml(rawDec)}" target="_blank" rel="noopener noreferrer">${label}</a>`);
    }
    return full;
  });

  out = out.replace(/(file:\/\/(?:(?!&lt;|&gt;|&quot;|&#39;|[\s()\[\]`,;]).)+)/gi, (match) => {
    const trimmed = match.replace(/[.,;]+$/, "");
    const trail = match.slice(trimmed.length);
    const rawDec = trimmed.replace(/&amp;/g, "&");
    const fpath = cleanFilePath(rawDec);
    const base = fpath.split("/").pop() || fpath;
    return stash(`<button type="button" class="file-chip" data-file-path="${escapeHtml(fpath)}">📄 ${escapeHtml(base)}</button>`) + trail;
  });

  out = out.replace(/(https?:\/\/(?:(?!&lt;|&gt;|&quot;|&#39;|[\s()\[\]`]).)+)/gi, (match) => {
    const trimmed = match.replace(/[.,;!?]+$/, "");
    const trail = match.slice(trimmed.length);
    const rawHref = trimmed.replace(/&amp;/g, "&");
    return stash(`<a href="${escapeHtml(rawHref)}" target="_blank" rel="noopener noreferrer">${trimmed}</a>`) + trail;
  });

  out = out.replace(/\*\*\*([^*]+)\*\*\*/g, "<strong><em>$1</em></strong>");
  out = out.replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>");
  out = out.replace(/(^|[^*])\*([^*\n]+)\*/g, "$1<em>$2</em>");
  return out;
}

function splitTableRow(line) {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|")) s = s.slice(0, -1);
  return s.split("|").map((c) => c.trim());
}

function isTableDivider(line) {
  const cells = splitTableRow(line);
  return cells.length > 0 && cells.every((c) => /^:?-{2,}:?$/.test(c));
}

export function renderMarkdown(raw) {
  if (!raw) return "";
  const codeBlocks = [];
  const withCodePlaceholders = String(raw).replace(/```([a-zA-Z0-9_-]*)\n([\s\S]*?)```/g, (_, lang, code) => {
    const id = `@@CODE_${codeBlocks.length}@@`;
    codeBlocks.push(renderCodeBlock(lang, code));
    return `\n${id}\n`;
  });

  const lines = withCodePlaceholders.split("\n");
  const out = [];
  const inlineTokens = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];
    const trimmed = line.trim();

    if (/^@@CODE_(\d+)@@$/.test(trimmed)) {
      const idx = Number(trimmed.match(/^@@CODE_(\d+)@@$/)[1]);
      out.push(codeBlocks[idx]);
      i++;
      continue;
    }

    if (!trimmed) {
      i++;
      continue;
    }

    if (trimmed.startsWith(">!") || ((trimmed.includes("Thinking Process") || trimmed.includes("Tool Calls")) && i + 1 < lines.length && lines[i + 1].trim().startsWith(">!"))) {
      let summary = t("cotSummary");
      if (!trimmed.startsWith(">!")) {
        summary = trimmed.replace(/\*\*/g, "");
        i++;
      }
      const bodyLines = [];
      while (i < lines.length && lines[i].trim().startsWith(">!")) {
        bodyLines.push(lines[i].trim().slice(2).trimStart());
        i++;
      }
      const bodyHtml = bodyLines.map((bl) => renderInline(escapeHtml(bl), inlineTokens)).join("<br>");
      out.push(`<details class="cot-block"><summary>${escapeHtml(summary)}</summary><div class="cot-body">${bodyHtml}</div></details>`);
      continue;
    }

    if (line.includes("|") && i + 1 < lines.length && isTableDivider(lines[i + 1])) {
      const headers = splitTableRow(line);
      i += 2;
      const rows = [];
      while (i < lines.length && lines[i].includes("|") && lines[i].trim() !== "") {
        rows.push(splitTableRow(lines[i]));
        i++;
      }
      const thead = `<thead><tr>${headers.map((h) => `<th>${renderInline(escapeHtml(h), inlineTokens)}</th>`).join("")}</tr></thead>`;
      const tbody = `<tbody>${rows.map((r) => `<tr>${r.map((c) => `<td>${renderInline(escapeHtml(c), inlineTokens)}</td>`).join("")}</tr>`).join("")}</tbody>`;
      out.push(`<div class="table-wrap"><table>${thead}${tbody}</table></div>`);
      continue;
    }

    const hMatch = trimmed.match(/^(#{1,3})\s+(.+)$/);
    if (hMatch) {
      const level = hMatch[1].length;
      out.push(`<h${level}>${renderInline(escapeHtml(hMatch[2]), inlineTokens)}</h${level}>`);
      i++;
      continue;
    }

    if (trimmed.startsWith("> ")) {
      const qLines = [];
      while (i < lines.length && lines[i].trim().startsWith("> ")) {
        qLines.push(renderInline(escapeHtml(lines[i].trim().slice(2)), inlineTokens));
        i++;
      }
      out.push(`<blockquote>${qLines.join("<br>")}</blockquote>`);
      continue;
    }

    if (/^[-*]\s+/.test(trimmed)) {
      const items = [];
      while (i < lines.length && /^[-*]\s+/.test(lines[i].trim())) {
        items.push(`<li>${renderInline(escapeHtml(lines[i].trim().replace(/^[-*]\s+/, "")), inlineTokens)}</li>`);
        i++;
      }
      out.push(`<ul>${items.join("")}</ul>`);
      continue;
    }

    const pLines = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !/^@@CODE_\d+@@$/.test(lines[i].trim()) &&
      !/^#{1,3}\s+/.test(lines[i].trim()) &&
      !lines[i].trim().startsWith("> ") &&
      !lines[i].trim().startsWith(">!") &&
      !/^[-*]\s+/.test(lines[i].trim())
    ) {
      pLines.push(renderInline(escapeHtml(lines[i]), inlineTokens));
      i++;
    }
    if (pLines.length) {
      out.push(`<p>${pLines.join("<br>")}</p>`);
    }
  }

  let finalHtml = out.join("");
  for (let t = 0; t < inlineTokens.length; t++) {
    finalHtml = finalHtml.replace(`@@TKN_${t}@@`, () => inlineTokens[t]);
  }
  return finalHtml;
}
