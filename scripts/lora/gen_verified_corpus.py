#!/usr/bin/env python3
"""Verified-synthetic hex-boundaries corpus generator (ADR-2606161300, offline tooling).

The data-scarcity problem: there isn't enough existing hex-rule-following source code to
train a boundary-idiom LoRA. The unlock: hex OWNS the verifier, so we MANUFACTURE
verified data instead of finding it — generate mini hexagonal projects with a strong
teacher, run each through `hex analyze`, and keep only the analyzer-passing ones. This
is rejection sampling against an objective oracle (the same independent-oracle move the
ADR is built on, turned around to make training data).

This script is a YIELD TEST first and a corpus generator second: it reports what
fraction of generated projects pass the analyzer clean (the number that decides whether
this path is viable) and writes the clean ones as a training corpus.

NOT runtime code — offline dev tooling (like train-lora.sh).
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

OLLAMA_URL = os.environ.get("OLLAMA_URL", "http://localhost:11434")

# The hex boundary rules the teacher must follow — stated so compliance is *possible*;
# the analyzer then measures whether it actually complied.
RULES = """\
hexagonal architecture (ports & adapters), hex's rules:
- Files live under src/domain, src/ports, src/usecases, src/adapters/primary,
  src/adapters/secondary, and src/composition-root.ts.
- domain/ imports only domain/.
- ports/ imports only domain/ (for value types).
- usecases/ imports domain/ and ports/ only.
- adapters (primary AND secondary) import ports/ only — an adapter MUST NOT import
  another adapter.
- composition-root.ts is the ONLY file allowed to import adapters; it wires them.
- ALL relative imports use explicit .js extensions (NodeNext).
"""

# Varied scenarios that exercise the full layering. The teacher implements each as a
# complete mini-project; the analyzer judges compliance.
SCENARIOS = [
    "a UserRepository port with a secondary PostgresUserRepository adapter and a primary HTTP UserController",
    "an EmailSender port with a secondary SmtpEmailSender adapter and a SendWelcomeEmail use case",
    "a PaymentGateway port, a secondary StripePaymentGateway adapter, and a ChargeOrder use case",
    "a Clock port, a secondary SystemClock adapter, and a use case that timestamps an event",
    "a FileStore port with a secondary S3FileStore adapter and a primary CLI command that uploads a file",
    "a Logger port, a secondary ConsoleLogger adapter, and a use case that logs an audit entry",
    "a Cache port with a secondary RedisCache adapter and a GetProfile use case that reads through the cache",
    "an EventBus port, a secondary InMemoryEventBus adapter, and a PublishOrderPlaced use case",
    "a TokenSigner port with a secondary JwtTokenSigner adapter and an IssueSession use case",
    "a WeatherProvider port, a secondary OpenWeatherProvider adapter, and a primary controller returning a forecast",
    "a NotificationSender port with secondary PushNotificationSender adapter and a NotifyUser use case",
    "a Translator port, a secondary DeeplTranslator adapter, and a TranslateDocument use case",
    "a RateLimiter port with a secondary TokenBucketRateLimiter adapter and a primary middleware using it",
    "a SearchIndex port, a secondary ElasticSearchIndex adapter, and an IndexProduct use case",
    "a GeoCoder port with a secondary GoogleGeoCoder adapter and a ResolveAddress use case",
    "a QueuePublisher port, a secondary KafkaQueuePublisher adapter, and an EnqueueJob use case",
    "a PasswordHasher port with a secondary Argon2PasswordHasher adapter and a RegisterUser use case",
    "a FeatureFlags port, a secondary LaunchDarklyFlags adapter, and a use case gated by a flag",
    "a MetricsSink port with a secondary PrometheusMetricsSink adapter and a RecordLatency use case",
    "an ImageResizer port, a secondary SharpImageResizer adapter, and a primary controller that resizes an upload",
    "a Secrets port with a secondary VaultSecrets adapter and a LoadConfig use case",
    "a PdfRenderer port, a secondary PuppeteerPdfRenderer adapter, and a GenerateInvoice use case",
    "a SmsSender port with a secondary TwilioSmsSender adapter and a SendOtp use case",
    "a Geolocation port, a secondary IpApiGeolocation adapter, and a primary controller returning a country",
    "a BlobStore port with a secondary GcsBlobStore adapter and an ArchiveReport use case",
    "a Scheduler port, a secondary CronScheduler adapter, and a ScheduleReminder use case",
    "a CurrencyRates port with a secondary FixerCurrencyRates adapter and a ConvertAmount use case",
    "a SessionStore port, a secondary CookieSessionStore adapter, and a primary auth controller",
    "an Antivirus port with a secondary ClamavAntivirus adapter and a ScanUpload use case",
    "a Webhook port, a secondary HttpWebhook adapter, and a DispatchWebhook use case",
]


def log(msg: str) -> None:
    print(f"[gen-verified] {msg}", flush=True)


def resolve_hex() -> str:
    for c in (shutil.which("hex"), os.path.expanduser("~/.local/bin/hex")):
        if c and Path(c).is_file():
            return c
    sys.exit("error: `hex` binary not found on PATH")


def ollama_chat(model: str, system: str, user: str, num_ctx: int = 8192) -> str:
    body = json.dumps({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "stream": False,
        "think": False,
        "format": "json",
        "options": {"temperature": 0.5, "num_predict": 3072, "num_ctx": num_ctx},
    }).encode()
    req = urllib.request.Request(f"{OLLAMA_URL}/api/chat", data=body,
                                headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=600) as r:
        return json.loads(r.read())["message"]["content"]


def parse_files(text: str):
    """Extract {"files":[{"path","content"}]} from the model JSON (tolerant)."""
    s, e = text.find("{"), text.rfind("}")
    if s < 0 or e <= s:
        return None
    try:
        obj = json.loads(text[s:e + 1])
    except json.JSONDecodeError:
        return None
    files = obj.get("files")
    if not isinstance(files, list):
        return None
    out = []
    for f in files:
        p, c = f.get("path", ""), f.get("content", "")
        # Keep paths inside src/, reject traversal.
        if p.startswith("src/") and ".." not in p and c.strip():
            out.append((p, c))
    return out or None


def analyze(hex_bin: str, project_dir: Path):
    """Run `hex analyze --json` and return (violation_count, score, violations)."""
    proc = subprocess.run(
        [hex_bin, "analyze", str(project_dir), "--violations-only", "--json"],
        capture_output=True, text=True, timeout=120,
    )
    out = proc.stdout
    s = out.find("{")
    if s < 0:
        return None
    try:
        d = json.loads(out[s:])
    except json.JSONDecodeError:
        return None
    return len(d.get("violations", [])), d.get("score", 0), d.get("violations", [])


def render_output(files) -> str:
    """Render a verified project as a single training target."""
    parts = []
    for path, content in files:
        parts.append(f"// FILE: {path}\n{content.rstrip()}")
    return "\n\n".join(parts)


def main() -> None:
    ap = argparse.ArgumentParser(description="Verified-synthetic hex corpus generator + yield test")
    ap.add_argument("--teacher", default="qwen2.5-coder:32b", help="Ollama model that generates projects")
    ap.add_argument("--n", type=int, default=len(SCENARIOS), help="How many scenarios to attempt")
    ap.add_argument("--out", default=".hex/corpus/hex-boundaries-verified/corpus.jsonl")
    args = ap.parse_args()

    hex_bin = resolve_hex()
    scenarios = SCENARIOS[: args.n]
    out_path = Path(args.out)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    system = "You output ONLY strict JSON. You are an expert in hexagonal (ports & adapters) TypeScript."
    clean, violating, failed = 0, 0, 0
    records = []
    violation_tally = {}

    for i, scenario in enumerate(scenarios, 1):
        user = (
            f"{RULES}\n"
            f"Implement: {scenario}.\n"
            "Return ONLY JSON: {\"files\":[{\"path\":\"src/...\",\"content\":\"<TypeScript>\"}]}. "
            "Include every layer the feature needs (domain, port, adapter(s), use case or "
            "primary adapter, and src/composition-root.ts wiring). Keep files small."
        )
        try:
            text = ollama_chat(args.teacher, system, user)
        except Exception as exc:  # noqa: BLE001
            log(f"[{i}/{len(scenarios)}] generation failed: {exc}")
            failed += 1
            continue
        files = parse_files(text)
        if not files:
            log(f"[{i}/{len(scenarios)}] unparseable / no files")
            failed += 1
            continue

        with tempfile.TemporaryDirectory(prefix="hexgen-") as td:
            root = Path(td)
            for path, content in files:
                fp = root / path
                fp.parent.mkdir(parents=True, exist_ok=True)
                fp.write_text(content)
            res = analyze(hex_bin, root)

        if res is None:
            log(f"[{i}/{len(scenarios)}] analyze failed")
            failed += 1
            continue
        vcount, score, violations = res
        if vcount == 0:
            clean += 1
            records.append({
                "instruction": f"Implement {scenario}, following hexagonal architecture (ports & adapters) with no cross-adapter imports and .js relative-import extensions. Provide all needed files.",
                "input": "",
                "output": render_output(files),
                "source_path": "verified-synthetic",
                "corpus_version": "",
            })
            log(f"[{i}/{len(scenarios)}] CLEAN (score {score}, {len(files)} files)")
        else:
            violating += 1
            for v in violations:
                violation_tally[v.get("rule", "?")] = violation_tally.get(v.get("rule", "?"), 0) + 1
            log(f"[{i}/{len(scenarios)}] {vcount} violation(s): {[v.get('rule') for v in violations]}")

    total = clean + violating
    with out_path.open("w") as fh:
        for r in records:
            fh.write(json.dumps(r) + "\n")

    log("=" * 60)
    log(f"YIELD: {clean}/{total} analyzer-clean"
        + (f" = {clean / total:.0%}" if total else "") + f"  (+{failed} gen/parse failures)")
    if violation_tally:
        log("Violations among rejected projects:")
        for rule, c in sorted(violation_tally.items(), key=lambda x: -x[1]):
            log(f"   {c:>3} × {rule}")
    log(f"Verified corpus written: {out_path} ({len(records)} records)")


if __name__ == "__main__":
    main()
