#!/usr/bin/env python3
"""Profile-isolated Google AI Studio (GEMINI_API_KEY) & Google Workspace CLI helper for Tuner."""

import argparse
import base64
import datetime
import json
import os
import sys
import urllib.parse
from email.message import EmailMessage
from pathlib import Path

SCOPES = [
    "https://www.googleapis.com/auth/gmail.modify",
    "https://www.googleapis.com/auth/gmail.compose",
    "https://www.googleapis.com/auth/gmail.send",
    "https://www.googleapis.com/auth/drive",
    "https://www.googleapis.com/auth/calendar",
    "https://www.googleapis.com/auth/spreadsheets",
    "https://www.googleapis.com/auth/documents",
    "https://www.googleapis.com/auth/tasks",
    "https://www.googleapis.com/auth/contacts.readonly",
    "https://www.googleapis.com/auth/youtube.readonly",
]


def _resolve_tuner_paths() -> tuple[Path, Path]:
    tuner_home = Path(os.environ.get("TUNER_HOME") or (Path.home() / ".tuner"))
    cwd = Path.cwd().resolve()
    for p in [cwd, *cwd.parents]:
        if p.name == "workspace" and p.parent.parent.name == "profiles":
            return tuner_home, p.parent / "vault"
    profile = os.environ.get("TUNER_PROFILE", "default")
    return tuner_home, tuner_home / "profiles" / profile / "vault"


def _read_dotenv_key(env_file: Path, key: str) -> str:
    val = os.environ.get(key, "").strip()
    if val:
        return val
    if not env_file.is_file():
        return ""
    for line in env_file.read_text(encoding="utf-8", errors="replace").splitlines():
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        if s.startswith("export "):
            s = s[7:].strip()
        if "=" in s:
            k, v = s.split("=", 1)
            if k.strip() == key:
                return v.strip().strip('"').strip("'")
    return ""


def _load_workspace_creds(vault_dir: Path):
    from google.auth.transport.requests import Request
    from google.oauth2.credentials import Credentials

    token_path = vault_dir / "google_token.json"
    if not token_path.is_file():
        raise SystemExit(
            f"Google Workspace OAuth token not found at {token_path}.\n"
            "Run `google_cli.py auth status` or set up google_oauth_client.json first."
        )
    creds = Credentials.from_authorized_user_file(str(token_path), SCOPES)
    if creds and creds.expired and creds.refresh_token:
        creds.refresh(Request())
        data = json.loads(creds.to_json())
        data["type"] = "authorized_user"
        token_path.write_text(json.dumps(data, indent=2), encoding="utf-8")
        os.chmod(token_path, 0o600)
    return creds


def cmd_auth(args, tuner_home: Path, vault_dir: Path) -> None:
    client_path = vault_dir / "google_oauth_client.json"
    token_path = vault_dir / "google_token.json"
    gemini_key = _read_dotenv_key(tuner_home / ".env", "GEMINI_API_KEY")
    if args.sub == "status":
        masked = f"{gemini_key[:4]}...{gemini_key[-4:]}" if len(gemini_key) >= 12 else None
        print(json.dumps({
            "gemini_api_key_configured": bool(gemini_key),
            "gemini_api_key_masked": masked,
            "oauth_client_present": client_path.is_file(),
            "oauth_token_present": token_path.is_file(),
            "vault_dir": str(vault_dir),
        }, indent=2))
    elif args.sub == "url":
        from google_auth_oauthlib.flow import InstalledAppFlow
        if not client_path.is_file():
            raise SystemExit(f"Missing {client_path}. Place Desktop OAuth client JSON there first.")
        flow = InstalledAppFlow.from_client_secrets_file(str(client_path), SCOPES)
        flow.redirect_uri = "http://127.0.0.1:8085/"
        url, _ = flow.authorization_url(access_type="offline", prompt="consent")
        print(json.dumps({"oauth_url": url, "redirect_uri": flow.redirect_uri}, indent=2))
    elif args.sub == "exchange":
        from google_auth_oauthlib.flow import InstalledAppFlow
        vault_dir.mkdir(parents=True, exist_ok=True)
        flow = InstalledAppFlow.from_client_secrets_file(str(client_path), SCOPES)
        flow.redirect_uri = "http://127.0.0.1:8085/"
        raw = args.callback.strip()
        code = urllib.parse.parse_qs(urllib.parse.urlparse(raw).query).get("code", [raw])[0]
        flow.fetch_token(code=code)
        data = json.loads(flow.credentials.to_json())
        data["type"] = "authorized_user"
        token_path.write_text(json.dumps(data, indent=2), encoding="utf-8")
        os.chmod(token_path, 0o600)
        print(json.dumps({"ok": True, "token_path": str(token_path)}, indent=2))


def cmd_gemini(args, tuner_home: Path) -> None:
    from google import genai
    api_key = _read_dotenv_key(tuner_home / ".env", "GEMINI_API_KEY")
    if not api_key:
        raise SystemExit("GEMINI_API_KEY is not set in ~/.tuner/.env. Configure it via Dashboard -> Engines.")
    client = genai.Client(api_key=api_key)
    if args.sub == "models":
        names = [m.name for m in client.models.list()]
        print(json.dumps({"count": len(names), "models": names}, indent=2))
    elif args.sub == "generate":
        resp = client.models.generate_content(model=args.model, contents=args.prompt)
        print(resp.text or "")


def cmd_workspace(args, vault_dir: Path) -> None:
    from googleapiclient.discovery import build
    creds = _load_workspace_creds(vault_dir)
    if args.group == "gmail":
        svc = build("gmail", "v1", credentials=creds)
        if args.sub == "list":
            msgs = svc.users().messages().list(userId="me", maxResults=args.max, q=args.query).execute().get("messages", [])
            out = []
            for m in msgs:
                d = svc.users().messages().get(userId="me", id=m["id"], format="metadata", metadataHeaders=["From", "Subject", "Date"]).execute()
                hdrs = {h["name"]: h["value"] for h in d.get("payload", {}).get("headers", [])}
                out.append({"id": m["id"], "snippet": d.get("snippet", ""), **hdrs})
            print(json.dumps(out, ensure_ascii=False, indent=2))
        elif args.sub == "read":
            d = svc.users().messages().get(userId="me", id=args.id, format="full").execute()
            print(json.dumps({"id": d["id"], "snippet": d.get("snippet", ""), "payload": d.get("payload", {})}, ensure_ascii=False, indent=2))
        elif args.sub in ("send", "draft"):
            msg = EmailMessage()
            msg["To"] = args.to
            msg["Subject"] = args.subject
            msg.set_content(args.body)
            raw = base64.urlsafe_b64encode(msg.as_bytes()).decode()
            if args.sub == "send":
                res = svc.users().messages().send(userId="me", body={"raw": raw}).execute()
            else:
                res = svc.users().drafts().create(userId="me", body={"message": {"raw": raw}}).execute()
            print(json.dumps(res, indent=2))
    elif args.group == "calendar":
        svc = build("calendar", "v3", credentials=creds)
        if args.sub == "list":
            now = datetime.datetime.now(datetime.timezone.utc)
            end = now + datetime.timedelta(days=args.days)
            ev = svc.events().list(calendarId="primary", timeMin=now.isoformat(), timeMax=end.isoformat(), singleEvents=True, orderBy="startTime").execute()
            print(json.dumps(ev.get("items", []), ensure_ascii=False, indent=2))
        elif args.sub == "add":
            body = {"summary": args.summary, "description": args.desc or "", "start": {"dateTime": args.start}, "end": {"dateTime": args.end}}
            print(json.dumps(svc.events().insert(calendarId="primary", body=body).execute(), ensure_ascii=False, indent=2))
    elif args.group == "drive":
        svc = build("drive", "v3", credentials=creds)
        if args.sub == "list":
            r = svc.files().list(pageSize=args.max, fields="files(id,name,mimeType,modifiedTime)").execute()
            print(json.dumps(r.get("files", []), ensure_ascii=False, indent=2))
        elif args.sub == "search":
            q = f"name contains '{args.keyword.replace(chr(39), '')}' and trashed = false"
            r = svc.files().list(q=q, pageSize=20, fields="files(id,name,mimeType,modifiedTime)").execute()
            print(json.dumps(r.get("files", []), ensure_ascii=False, indent=2))
    elif args.group == "tasks":
        svc = build("tasks", "v1", credentials=creds)
        if args.sub == "list":
            print(json.dumps(svc.tasks().list(tasklist="@default").execute().get("items", []), ensure_ascii=False, indent=2))
        elif args.sub == "add":
            body = {"title": args.title, **({"due": f"{args.due}T00:00:00.000Z"} if args.due else {})}
            print(json.dumps(svc.tasks().insert(tasklist="@default", body=body).execute(), ensure_ascii=False, indent=2))
    elif args.group == "sheets":
        svc = build("sheets", "v4", credentials=creds)
        if args.sub == "create":
            print(json.dumps(svc.spreadsheets().create(body={"properties": {"title": args.title}}).execute(), ensure_ascii=False, indent=2))
        elif args.sub == "read":
            print(json.dumps(svc.spreadsheets().values().get(spreadsheetId=args.id, range=args.range).execute(), ensure_ascii=False, indent=2))
    elif args.group == "docs":
        svc = build("docs", "v1", credentials=creds)
        if args.sub == "create":
            doc = svc.documents().create(body={"title": args.title}).execute()
            if args.text:
                svc.documents().batchUpdate(documentId=doc["documentId"], body={"requests": [{"insertText": {"location": {"index": 1}, "text": args.text}}]}).execute()
            print(json.dumps(doc, ensure_ascii=False, indent=2))
        elif args.sub == "read":
            print(json.dumps(svc.documents().get(documentId=args.id).execute(), ensure_ascii=False, indent=2))
    elif args.group == "contacts":
        svc = build("people", "v1", credentials=creds)
        r = svc.people().searchContacts(query=args.query, readMask="names,emailAddresses,phoneNumbers").execute()
        print(json.dumps(r.get("results", []), ensure_ascii=False, indent=2))
    elif args.group == "youtube":
        svc = build("youtube", "v3", credentials=creds)
        print(json.dumps(svc.channels().list(part="snippet,statistics", mine=True).execute(), ensure_ascii=False, indent=2))


def main() -> None:
    p = argparse.ArgumentParser(description="Tuner Google AI & Workspace CLI")
    sub = p.add_subparsers(dest="group", required=True)

    pa = sub.add_parser("auth")
    pas = pa.add_subparsers(dest="sub", required=True)
    pas.add_parser("status")
    pas.add_parser("url")
    pae = pas.add_parser("exchange")
    pae.add_argument("callback")

    pg = sub.add_parser("gemini")
    pgs = pg.add_subparsers(dest="sub", required=True)
    pgs.add_parser("models")
    pgg = pgs.add_parser("generate")
    pgg.add_argument("prompt")
    pgg.add_argument("--model", default="gemini-2.5-flash")

    pm = sub.add_parser("gmail")
    pms = pm.add_subparsers(dest="sub", required=True)
    pml = pms.add_parser("list")
    pml.add_argument("--max", type=int, default=5)
    pml.add_argument("--query", default="")
    pmr = pms.add_parser("read")
    pmr.add_argument("id")
    for act in ("send", "draft"):
        px = pms.add_parser(act)
        px.add_argument("--to", required=True)
        px.add_argument("--subject", required=True)
        px.add_argument("--body", required=True)

    pc = sub.add_parser("calendar")
    pcs = pc.add_subparsers(dest="sub", required=True)
    pcl = pcs.add_parser("list")
    pcl.add_argument("--days", type=int, default=14)
    pca = pcs.add_parser("add")
    pca.add_argument("--summary", required=True)
    pca.add_argument("--start", required=True)
    pca.add_argument("--end", required=True)
    pca.add_argument("--desc", default="")

    pd = sub.add_parser("drive")
    pds = pd.add_subparsers(dest="sub", required=True)
    pdl = pds.add_parser("list")
    pdl.add_argument("--max", type=int, default=10)
    pdsr = pds.add_parser("search")
    pdsr.add_argument("keyword")

    pt = sub.add_parser("tasks")
    pts = pt.add_subparsers(dest="sub", required=True)
    pts.add_parser("list")
    pta = pts.add_parser("add")
    pta.add_argument("--title", required=True)
    pta.add_argument("--due", default="")

    ps = sub.add_parser("sheets")
    pss = ps.add_subparsers(dest="sub", required=True)
    psc = pss.add_parser("create")
    psc.add_argument("title")
    psr = pss.add_parser("read")
    psr.add_argument("id")
    psr.add_argument("range")

    pdo = sub.add_parser("docs")
    pdos = pdo.add_subparsers(dest="sub", required=True)
    pdoc = pdos.add_parser("create")
    pdoc.add_argument("title")
    pdoc.add_argument("--text", default="")
    pdor = pdos.add_parser("read")
    pdor.add_argument("id")

    pco = sub.add_parser("contacts")
    pcos = pco.add_subparsers(dest="sub", required=True)
    pcosr = pcos.add_parser("search")
    pcosr.add_argument("query")

    py = sub.add_parser("youtube")
    pys = py.add_subparsers(dest="sub", required=True)
    pys.add_parser("my-channel")

    args = p.parse_args()
    tuner_home, vault_dir = _resolve_tuner_paths()
    if args.group == "auth":
        cmd_auth(args, tuner_home, vault_dir)
    elif args.group == "gemini":
        cmd_gemini(args, tuner_home)
    else:
        cmd_workspace(args, vault_dir)


if __name__ == "__main__":
    main()
