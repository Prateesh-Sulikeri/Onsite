// Screenshots for the website's setup guides (site/src/guides.html): the real UI (ui/index.html) with the fake
// backend in shots-mock.js, set up per shot to show one AI / voice state. Writes JPEGs to site/src/img/guides/.
// Usage: npm run guide-shots   (CHROME=/path/to/chrome if Playwright's Chromium isn't in ~/.cache/ms-playwright)
import { readFileSync, existsSync, readdirSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { homedir } from "node:os";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "playwright-core";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "../../..");
const out = join(repo, "site/src/img/guides");
mkdirSync(out, { recursive: true });
const json = p => JSON.parse(readFileSync(join(repo, p), "utf8"));
const only = process.argv.slice(2);

const panels = json("prompts/panels.json").panels.map(p => ({
  id: p.id, name: p.name, description: p.description, round_owner: {},
  interviewers: p.interviewers.map(i => ({ id: i.id, name: i.name, role: i.role, traits: i.personality.traits, gender: i.gender,
    style: i.personality.communication_style, strictness: i.personality.strictness, backstory: "", voice: i.voice || "", rounds: [] })),
}));
const base = { catalog: json("prompts/catalog.json"), dossier: json("dossiers/razorpay-sde-2-backend.json"), panels };
const mock = readFileSync(join(here, "shots-mock.js"), "utf8");

function findChrome() {
  if (process.env.CHROME) return process.env.CHROME;
  const cache = join(homedir(), ".cache/ms-playwright");
  for (const d of existsSync(cache) ? readdirSync(cache).filter(d => d.startsWith("chromium-")).sort().reverse() : [])
    for (const sub of ["chrome-linux64/chrome", "chrome-linux/chrome", "chrome-win/chrome.exe"])
      if (existsSync(join(cache, d, sub))) return join(cache, d, sub);
  throw new Error("No Chromium found. Set CHROME=/path/to/chrome.");
}

// ---- backend states (the same shapes the Rust commands return) ----
const MODELS = [
  { id: "sherpa-onnx-pocket-tts-int8-2026-01-26", label: "Pocket TTS", purpose: "The interviewer's voice", bytes: 98336520 },
  { id: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8", label: "Parakeet TDT 0.6B v2", purpose: "Live captions and your answers", bytes: 482468385 },
];
const VOICES = [{ id: "ethan_brooks", name: "Ethan", accent: "American, male" }, { id: "priya_shah", name: "Charlotte", accent: "British, female" }];
const voiceBase = { enabled: true, listen_only: false, base_url: "http://127.0.0.1:3900", voice: "", builtin_voice: "ethan_brooks",
  steps: 8, speed: 1, pause_ms: 3000, input_device: "", output_device: "" };
const settings = (provider, engine, extra = {}) => ({
  provider, claude: provider === "claude_code" ? { kind: "windows", exe: "C:\\Users\\you\\.local\\bin\\claude.exe" } : null,
  interviewer_model: { claude_code: "sonnet", groq: "qwen/qwen3.8-27b", anthropic: "claude-sonnet-5-5", openai: "gpt-5.5", gemini: "gemini-flash-latest" }[provider],
  report_model: { claude_code: "opus", groq: "openai/gpt-oss-120b", anthropic: "claude-opus-5-5", openai: "gpt-5.5", gemini: "gemini-flash-latest" }[provider],
  voice: { ...voiceBase, engine, ...extra },
});
const builtin = installed => ({ up: installed, loaded: installed, device: "Built-in", profiles: [], installed: false, english: null,
  whisper: { state: "off" }, version: null, engine: "builtin", studio_up: false,
  models: MODELS.map(m => ({ ...m, installed })), voices: VOICES });
const studio = (up, device = "cuda") => ({ up, loaded: up, device: up ? device : "", installed: true, english: up ? "Ready" : null,
  whisper: up ? { state: "ready", model: "large-v3" } : { state: "off" }, version: "0.5.6", engine: "voicestudio", studio_up: up,
  profiles: up ? [{ id: "p1", name: "Onsite · Karthik Iyer" }, { id: "p2", name: "Onsite · Neha Kapoor" }, { id: "p3", name: "Narrator" }] : [],
  models: MODELS.map(m => ({ ...m, installed: false })), voices: VOICES });
const health = (model, location, ms) => ({ ok: true, reply: "OK", latency_ms: ms, model, location });
const GROQ_MODELS = ["qwen/qwen3.8-27b", "openai/gpt-oss-120b", "openai/gpt-oss-20b", "llama-3.3-70b-versatile"];

// ---- browser ----
const browser = await chromium.launch({ executablePath: findChrome() });

async function page(over, size = { width: 1280, height: 1700 }) {
  const p = await browser.newPage({ viewport: size, deviceScaleFactor: 1.25 });
  p.on("pageerror", e => console.error("page error:", e.message));
  await p.addInitScript({ content: `window.__MOCK__=${JSON.stringify({ ...base, over })};` + mock });
  await p.goto(pathToFileURL(join(repo, "ui/index.html")).href);
  await p.waitForTimeout(1000);
  return p;
}
async function snap(p, name, clip) {
  await p.waitForTimeout(700);
  const opts = { path: join(out, `${name}.jpg`), type: "jpeg", quality: 80 };
  if (clip) await p.locator(clip).first().screenshot(opts); else await p.screenshot(opts);
  // An element taller than the window renders blank below the fold, so tall shots use a panel or a taller window.
  console.log(`site/src/img/guides/${name}.jpg`);
}
const ENGINE = '.set-sec[data-sec="voice"] .panel >> nth=0';
const settingsAt = (p, sec) => p.evaluate(s => { SET_SEC = s; nav("settings"); }, sec);
const want = n => !only.length || only.includes(n);

const shots = {
  // AI model
  async "ai-claude-code"() {
    const p = await page({ get_settings: settings("claude_code", "builtin"), voice_status: builtin(true),
      health_check: health("claude-sonnet-5-5", "Windows: C:\\Users\\you\\.local\\bin\\claude.exe", 6400) });
    await settingsAt(p, "models"); await p.waitForTimeout(500); await p.click("#clHealth");
    await snap(p, "ai-claude-code", ".set-layout");
  },
  async "ai-claude-code-wsl"() {
    const s = settings("claude_code", "builtin"); s.claude = { kind: "wsl" };
    const p = await page({ get_settings: s, voice_status: builtin(true), health_check: health("claude-sonnet-5-5", "WSL: claude", 7100) });
    await settingsAt(p, "models"); await p.waitForTimeout(500); await p.click("#clHealth");
    await snap(p, "ai-claude-code-wsl", ".set-layout");
  },
  async "ai-groq-key"() {
    const p = await page({ get_settings: settings("groq", "builtin"), ai_keys: {}, voice_status: builtin(true) });
    await settingsAt(p, "models"); await p.waitForTimeout(500);
    await p.fill("#aiKeyInput", "gsk_••••••••••••••••••••••••••••••••");
    await snap(p, "ai-groq-key", ".set-layout");
  },
  async "ai-groq-ready"() {
    const p = await page({ get_settings: settings("groq", "builtin"), ai_keys: { groq: true }, list_models: GROQ_MODELS,
      voice_status: builtin(true), health_check: health("qwen/qwen3.8-27b", "Groq API", 450) });
    await settingsAt(p, "models"); await p.waitForTimeout(600); await p.click("#clHealth");
    await snap(p, "ai-groq-ready", ".set-layout");
  },
  async "ai-api-key"() {
    const p = await page({ get_settings: settings("gemini", "builtin"), ai_keys: { gemini: true }, list_models: ["gemini-flash-latest", "gemini-2.5-flash", "gemini-2.5-flash-lite"],
      voice_status: builtin(true), health_check: health("gemini-flash-latest", "Gemini API", 900) });
    await settingsAt(p, "models"); await p.waitForTimeout(600); await p.click("#clHealth");
    await snap(p, "ai-api-key", ".set-layout");
  },
  // Voice
  async "voice-builtin-download"() {
    const p = await page({ get_settings: settings("groq", "builtin"), ai_keys: { groq: true }, voice_status: builtin(false) });
    await settingsAt(p, "voice");
    await snap(p, "voice-builtin-download", ENGINE);
  },
  async "voice-builtin-ready"() {
    const p = await page({ get_settings: settings("groq", "builtin"), ai_keys: { groq: true }, voice_status: builtin(true) });
    await settingsAt(p, "voice");
    await snap(p, "voice-builtin-ready", ENGINE);
  },
  async "voice-studio-ready"() {
    const p = await page({ get_settings: settings("claude_code", "voicestudio", { voice: "p3" }), voice_status: studio(true) });
    await settingsAt(p, "voice");
    await snap(p, "voice-studio-ready", ENGINE);
  },
  async "voice-typed"() {
    const p = await page({ get_settings: settings("groq", "builtin", { enabled: false }), ai_keys: { groq: true }, voice_status: builtin(false) });
    await settingsAt(p, "voice");
    await snap(p, "voice-typed", '.set-sec[data-sec="voice"] .panel >> nth=1');
  },
  // Using it
  async "use-setup"() {
    const p = await page({ get_settings: settings("groq", "builtin"), ai_keys: { groq: true }, voice_status: builtin(true) }, { width: 1280, height: 1000 });
    await p.evaluate(() => nav("interviews"));
    await snap(p, "use-setup");
  },
  async "use-ready"() {
    const p = await page({ get_settings: settings("groq", "builtin"), ai_keys: { groq: true }, voice_status: builtin(true) }, { width: 1280, height: 860 });
    await p.evaluate(async () => {
      loop = { id: 1, count: 3, setup: { rounds: ["coding", "lld", "sysdesign"], company: "inpay", level: "sde2", lang: "Java", temperament: "realistic", panel: PANELS[0].id } };
      await openReady(0);
    });
    await snap(p, "use-ready");
  },
  async "use-report"() {
    const p = await page({ get_settings: settings("groq", "builtin"), ai_keys: { groq: true }, voice_status: builtin(true) }, { width: 1280, height: 1100 });
    await p.evaluate(async () => {
      const h = await api.invoke("history");
      const r = h[3];
      window.__TAURI__.core.invoke = (orig => async (cmd, args) => cmd === "get_round"
        ? { ...r, idx: 0, persona: "Rahul", transcript: [{ who: "iv", text: "Walk me through your approach." }, ...Array.from({ length: 9 }, () => ({ who: "me", text: "I would keep a hash map from value to index as I scan the array once, so each lookup is constant time and the whole pass is linear." }))] }
        : cmd === "get_report" ? r.report : orig(cmd, args))(window.__TAURI__.core.invoke);
      await openReport(r.round_id);
    });
    await snap(p, "use-report");
  },
  async "settings-feedback"() {
    const p = await page({ get_settings: settings("groq", "builtin"), ai_keys: { groq: true }, voice_status: builtin(true) });
    await settingsAt(p, "share");
    await snap(p, "settings-feedback", '.set-sec[data-sec="share"] .panel');
  },
};

for (const [name, fn] of Object.entries(shots)) if (want(name)) await fn();
await browser.close();
