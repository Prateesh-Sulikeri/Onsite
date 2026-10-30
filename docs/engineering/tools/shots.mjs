// Screenshots of the real UI (ui/index.html) for UI-DESIGN.md, with a fake backend (shots-mock.js)
// and sample history. Writes JPEGs to docs/engineering/img/. Usage: npm run shots
import { readFileSync, existsSync, readdirSync, mkdirSync } from "node:fs";
import { join, dirname } from "node:path";
import { homedir } from "node:os";
import { fileURLToPath, pathToFileURL } from "node:url";
import { chromium } from "playwright-core";

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, "../../..");
const out = join(here, "../img");
mkdirSync(out, { recursive: true });
const json = p => JSON.parse(readFileSync(join(repo, p), "utf8"));

// The same shapes the Rust commands return.
const panels = json("prompts/panels.json").panels.map(p => ({
  id: p.id, name: p.name, description: p.description, round_owner: {},
  interviewers: p.interviewers.map(i => ({ id: i.id, name: i.name, role: i.role, traits: i.personality.traits,
    style: i.personality.communication_style, strictness: i.personality.strictness, backstory: "", voice: "", rounds: [] })),
}));
const data = `window.__MOCK__=${JSON.stringify({ catalog: json("prompts/catalog.json"), dossier: json("dossiers/razorpay-sde-2-backend.json"), panels })};`;
const mock = readFileSync(join(here, "shots-mock.js"), "utf8");

function findChrome() {
  if (process.env.CHROME) return process.env.CHROME;
  const cache = join(homedir(), ".cache/ms-playwright");
  for (const d of existsSync(cache) ? readdirSync(cache).filter(d => d.startsWith("chromium-")).sort().reverse() : [])
    for (const sub of ["chrome-linux64/chrome", "chrome-linux/chrome", "chrome-win/chrome.exe"])
      if (existsSync(join(cache, d, sub))) return join(cache, d, sub);
  throw new Error("No Chromium found. Set CHROME=/path/to/chrome.");
}

const browser = await chromium.launch({ executablePath: findChrome() });

async function page(theme) {
  const p = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  p.on("pageerror", e => console.error("page error:", e.message));
  await p.addInitScript({ content: data + mock + (theme ? `localStorage.setItem("onsite.theme", ${JSON.stringify(JSON.stringify(theme))});` : "") });
  await p.goto(pathToFileURL(join(repo, "ui/index.html")).href);
  await p.waitForTimeout(1200);
  return p;
}

async function shoot(p, name, fn, arg) {
  if (fn) await p.evaluate(fn, arg);
  await p.waitForTimeout(900);
  await p.screenshot({ path: join(out, `${name}.jpg`), type: "jpeg", quality: 82 });
  console.log(`img/${name}.jpg`);
}

const go = ([view, sec]) => { if (sec) SET_SEC = sec; nav(view); };

let p = await page();
for (const [name, view, sec] of [
  ["dashboard", "dashboard"], ["interviews", "interviews"], ["resume", "resume"], ["dossiers", "dossiers"],
  ["feedback", "feedback"], ["settings-appearance", "settings", "appearance"], ["settings-models", "settings", "models"],
  ["settings-voice", "settings", "voice"], ["settings-privacy", "settings", "privacy"],
]) await shoot(p, name, go, [view, sec]);

// The call room, mid-round, with sample turns.
await shoot(p, "room", async () => {
  PANELS[0].round_owner = { coding: PANELS[0].interviewers[2].id };
  loop = { id: 1, count: 3, setup: { rounds: ["coding", "lld", "sysdesign"], company: "inpay", level: "sde2", lang: "Java", temperament: "realistic", panel: PANELS[0].id,
    dossier: { company: "Razorpay", process: [{ name: "Problem solving (DSA)", minutes: 45 }, { name: "Machine coding", minutes: 90 }, { name: "System design", minutes: 60 }] } } };
  await openRoom(0);
  $("#transcript").innerHTML = "";
  addMsg("iv", "Hi, I'm Karthik. Thanks for joining. The problem is on your screen. Take a minute to read it, then tell me how you'd approach it.");
  addMsg("me", "Okay. So I need two indices whose values add up to target. Brute force is checking every pair, O of n squared. I'd rather keep a hash map from value to index as I scan.");
  addMsg("iv", "Sounds good. What happens with duplicates, like the second example?");
  ED.set("class Solution {\n    public int[] twoSum(int[] nums, int target) {\n        Map<Integer, Integer> seen = new HashMap<>();\n        for (int i = 0; i < nums.length; i++) {\n            Integer j = seen.get(target - nums[i]);\n            if (j != null) return new int[] { j, i };\n            seen.put(nums[i], i);\n        }\n        return new int[0];\n    }\n}");
  setPhase("hearing"); setWave(0.7);
  $("#timerText").textContent = "31:12";
});
await p.close();

for (const [family, mode] of [["nord", "light"], ["gruvbox", "dark"], ["rosepine", "light"], ["onsite", "light"]]) {
  p = await page({ family, mode });
  await shoot(p, `theme-${family}-${mode}`, go, ["dashboard"]);
  await p.close();
}
await browser.close();
