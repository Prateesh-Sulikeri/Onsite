"use strict";
/* Onsite UI. Model calls, storage and audio all live in Rust (window.api). */
const F = "```";
const $ = s => document.querySelector(s);
const $$ = s => [...document.querySelectorAll(s)];
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
const words = s => (String(s||"").trim().match(/\S+/g)||[]).length;
const errText = e => typeof e === "string" ? e : (e && e.message) || "Something went wrong.";
const icon = (id, cls = "") => `<svg class="${cls}"><use href="#i-${id}"/></svg>`;
const clock = () => new Date().toLocaleTimeString([], {hour:"numeric", minute:"2-digit"});
const LS = {
  get(k, d){ try{ const v = localStorage.getItem(k); return v ? JSON.parse(v) : d; }catch(e){ return d; } },
  set(k, v){ try{ localStorage.setItem(k, JSON.stringify(v)); }catch(e){} },
};

let CAT = null;          // catalog from prompts/catalog.json
let SETTINGS = null;     // Rust Settings
let VS = {up:false, loaded:false, profiles:[]};
let AIKEYS = {};         // provider id → a key is saved
let loop = null;         // {id, setup, count}
let session = null;      // the live round
let timerI = null;
let starting = false;    // a start_round call is in flight
const deadRounds = new Set();  // rounds we left; their late events are ignored

/* ---------------- round guidance (UI only) ---------------- */
const PLANS = {
  coding:        [["Introduction",.07],["Problem and clarifying questions",.13],["Approach and complexity",.17],["Code it",.38],["Dry run, edge cases, follow-up",.18],["Your questions",.07]],
  comprehension: [["Introduction",.06],["Read the code",.14],["Trace a scenario",.16],["Find the problems",.28],["Fix and test",.26],["Your questions",.1]],
  sysdesign:     [["Introduction",.05],["Requirements and scope",.15],["Estimates and APIs",.12],["High-level design",.28],["Deep dives",.28],["Your questions",.12]],
  lld:           [["Introduction",.05],["Requirements",.12],["Entities and classes",.25],["Code the core",.35],["Extend it",.15],["Your questions",.08]],
  resume:        [["Introduction",.1],["Your background",.25],["Line by line",.4],["Motivation and fit",.15],["Your questions",.1]],
  project:       [["Introduction",.06],["The problem",.12],["Architecture",.25],["Decisions and trade-offs",.3],["Failures and scale",.17],["Your questions",.1]],
  fundamentals:  [["Introduction",.05],["Language internals",.3],["Databases and caching",.25],["Distributed systems",.3],["Your questions",.1]],
  behavioral:    [["Introduction",.06],["Story one",.28],["Story two",.28],["Story three",.28],["Your questions",.1]],
  hm:            [["Introduction",.06],["Career story",.24],["Judgment scenarios",.3],["What you want next",.2],["Your questions",.2]],
};
const TIPS = {
  coding: ["Ask about input size and edge cases before coding.", "Say the complexity before you write a line.", "Dry-run your code on the example out loud."],
  comprehension: ["Summarise what the code does before hunting bugs.", "Rank the issues by impact, not by order found.", "Say how you would test each fix."],
  sysdesign: ["Drive it: requirements, estimates, then the design.", "Name the trade-off every time you pick a component.", "Talk about failure modes before you're asked."],
  lld: ["Pin down requirements before drawing classes.", "Keep interfaces small; show where it extends.", "Call out thread safety in shared state."],
  resume: ["Lead with impact and numbers.", "Separate what you did from what the team did.", "Have a crisp answer for why this role, why now."],
  project: ["Sketch the architecture on the whiteboard.", "Explain rejected alternatives and why.", "Know your scale numbers: QPS, data size, latency."],
  fundamentals: ["Answer, then go one level deeper on your own.", "Use a concrete scenario to show you've applied it.", "Say \"I'm not sure\" rather than guess."],
  behavioral: ["Use STAR, but keep the situation short.", "Say \"I\", and give the result in numbers.", "Pick stories with real conflict or failure."],
  hm: ["Show how you prioritise under pressure.", "Be specific about what you want next.", "Bring two sharp questions about the team."],
};

const ROUND_ICONS = {resume:"doc-check", project:"folder", fundamentals:"gear", coding:"code", comprehension:"search",
  lld:"layers", sysdesign:"network", behavioral:"user", hm:"chart"};

/* ---------------- shell ---------------- */
function nav(view){
  $$(".view").forEach(v => v.hidden = v.id !== "view-" + view);
  const section = view === "report" ? "feedback" : ["room", "ready", "prep"].includes(view) ? "interviews" : view;
  $$(".nav-item").forEach(b => b.dataset.nav === section ? b.setAttribute("aria-current", "page") : b.removeAttribute("aria-current"));
  $("#app").classList.toggle("in-call", view === "room");
  $(".main").scrollTop = 0;
  if (view === "dashboard") renderDashboard();
  if (view === "interviews") renderAgenda();
  if (view !== "ready") stopMonitor();
  if (view === "feedback") renderHistory();
  if (view === "settings"){ renderSettings(); setSection(SET_SEC); }
  if (view === "resume") openAts();
  if (view === "dossiers") renderDossiersView();
}
document.addEventListener("click", async e => {
  const t = e.target.closest("[data-nav]");
  if (!t) return;
  if (session){
    // Still loading: nothing to lose, so no question.
    if (session.roundId !== null && !await ask("Leave the call?", "The interviewer stops. The round keeps its transcript.", "Leave")) return;
    leaveRound().then(kept => {
      if (kept !== false) return;
      toast("You didn’t answer anything, so the round wasn’t saved.");
      // The list may have rendered before Rust dropped the round.
      if (!$("#view-feedback").hidden) renderHistory();
      if (!$("#view-dashboard").hidden) renderDashboard();
    });
  }
  if (t.dataset.sec) SET_SEC = t.dataset.sec;
  nav(t.dataset.nav);
});

/* In-app yes/no dialog. Resolves true on the OK button; Esc or Cancel resolve false. */
function ask(title, body, ok, danger = false){
  const d = $("#askDlg"), b = $("#askOk");
  $("#askTitle").textContent = title; $("#askBody").textContent = body;
  b.textContent = ok; b.classList.toggle("danger-fill", danger);
  d.returnValue = ""; d.showModal();
  (danger ? $("#askNo") : b).focus();
  return new Promise(r => d.addEventListener("close", () => r(d.returnValue === "ok"), {once: true}));
}

const toastSeen = new Map();
function toast(msg, kind = ""){
  const now = Date.now();
  if (toastSeen.get(msg) > now - 10000) return;
  toastSeen.set(msg, now);
  const el = document.createElement("div");
  el.className = "toast " + kind; el.textContent = msg;
  if (kind === "err") el.setAttribute("role", "alert");
  $("#toasts").appendChild(el);
  setTimeout(() => el.remove(), 6000);
}
/* Screen-reader-only announcements (the transcript itself is not live: streaming would flood it). */
function announce(text){ const el = $("#srLive"); el.textContent = ""; setTimeout(() => el.textContent = text, 30); }

/* What runs the interviewer. Model ids for the APIs come from the provider's live list. */
const PROVIDERS = {
  claude_code: {name: "Claude Code"},
  anthropic: {name: "Claude API", key: "Claude API key", hint: "Create one at console.anthropic.com, under API keys. It starts with sk-ant-.",
    models: {"claude-sonnet-5-5": "Sonnet 5.5 — fast replies", "claude-opus-5-5": "Opus 5.5 — best judgment", "claude-haiku-5-5": "Haiku 5.5 — fastest, cheapest"}},
  openai: {name: "OpenAI", key: "OpenAI API key", hint: "Create one at platform.openai.com, under API keys. ChatGPT Plus doesn’t cover API use, so add a little credit there."},
  gemini: {name: "Gemini", key: "Gemini API key", hint: "Get a free key at aistudio.google.com (Get API key). The free tier is enough for practice, with daily limits.",
    models: {"gemini-flash-latest": "Gemini Flash (latest) — free tier"}},
};
const provider = () => (SETTINGS && SETTINGS.provider) || "claude_code";
function aiReady(){ const p = provider(); return p === "claude_code" ? !!(SETTINGS && SETTINGS.claude) : !!AIKEYS[p]; }

async function refreshStatus(){
  try{ SETTINGS = await api.invoke("get_settings"); }catch(e){}
  try{ AIKEYS = await api.invoke("ai_keys"); }catch(e){}
  try{ VS = await api.invoke("voice_status"); }catch(e){ VS = {up:false, loaded:false, profiles:[]}; }
  const cl = $("#sysClaude"), name = PROVIDERS[provider()].name;
  cl.className = "sys " + (aiReady() ? "ok" : "bad");
  $("#sysClaudeName").textContent = name;
  cl.title = aiReady() ? `${name} connected` : `${name} isn’t set up. Open Settings.`;
  const vs = $("#sysVoice"); vs.className = "sys " + (!VS.up ? "bad" : VS.loaded ? "ok" : "warn");
  vs.title = !VS.up ? "VoiceStudio is not running" : VS.loaded ? "VoiceStudio ready" : "VoiceStudio is loading its voice model";
}
setInterval(() => { if (!session) refreshStatus(); }, 15000);

/* ---------------- dashboard ---------------- */
/* Everything here is computed from saved rounds and their feedback JSON. Dimension scores are 1–4. */
const DAY = 864e5;
const dayKey = d => { const x = new Date(d); return `${x.getFullYear()}-${x.getMonth()}-${x.getDate()}`; };
const weekStart = (d = new Date()) => { const x = new Date(d); x.setHours(0, 0, 0, 0); x.setDate(x.getDate() - (x.getDay() + 6) % 7); return x; };
const roundName = k => (CAT && CAT.ROUNDS[k] && CAT.ROUNDS[k].name) || k;
/* Round i of the current loop. A company loop keeps the company's own round names and lengths. */
function loopRound(i){
  const k = loop.setup.rounds[i];
  const d = (loop.setup.dossierRounds || [])[i] || ((loop.setup.dossier || {}).process || [])[i];
  return {key: k, name: d ? d.name : roundName(k), minutes: Number(loop.setup.minutes) || (d ? d.minutes : CAT.ROUNDS[k].min)};
}
const avg = a => a.reduce((x, y) => x + y, 0) / a.length;
function ago(iso){
  const days = Math.round((new Date().setHours(0, 0, 0, 0) - new Date(iso).setHours(0, 0, 0, 0)) / DAY);
  return days <= 0 ? "today" : days === 1 ? "yesterday" : `${days} days ago`;
}
const tookMinutes = x => x.ended_at ? Math.max(1, Math.round((new Date(x.ended_at) - new Date(x.started_at)) / 6e4)) : null;
/* Assessed dimensions of one reviewed round: [{name, score, evidence}]. */
const dimsOf = x => ((x.report && x.report.dimensions) || []).filter(d => d && d.name && Number.isFinite(d.score));

/* Per-skill history across rounds, oldest first. */
function skillStats(reviewed){
  const by = new Map();
  reviewed.slice().reverse().forEach(x => dimsOf(x).forEach(d => {
    let s = by.get(d.name);
    if (!s) by.set(d.name, s = {name: d.name, pts: [], kinds: new Map()});
    s.pts.push({score: d.score, evidence: d.evidence || "", round: x});
    s.kinds.set(x.round_key, (s.kinds.get(x.round_key) || 0) + 1);
  }));
  return [...by.values()].map(s => {
    const scores = s.pts.map(p => p.score);
    const recent = scores.slice(-3), before = scores.slice(0, -3);
    return {...s, n: scores.length, avg: avg(scores),
      trend: before.length ? avg(recent) - avg(before) : null,
      low: s.pts.filter(p => p.score <= 2), high: s.pts.filter(p => p.score >= 3),
      kind: [...s.kinds.entries()].sort((a, b) => b[1] - a[1])[0][0]};
  });
}

let DASH = {cal: null, goal: null, h: []};
async function renderDashboard(){
  const hr = new Date().getHours();
  $("#dashHello").textContent = hr < 12 ? "Good morning" : hr < 17 ? "Good afternoon" : "Good evening";
  let h = [], goal = {};
  try{ h = await api.invoke("history"); }catch(e){}
  try{ goal = await api.invoke("get_goal"); }catch(e){}
  DASH.h = h; DASH.goal = goal;
  if (!DASH.cal){ const n = new Date(); DASH.cal = new Date(n.getFullYear(), n.getMonth(), 1); }
  const reviewed = h.filter(x => x.overall != null);
  const skills = skillStats(reviewed);
  renderContinue(h);
  renderProgress(h, goal);
  renderSkills(skills);
  renderTrend(reviewed);
  renderNext(skills, h, goal);
  renderRecent(h);
  renderGoal(h, goal);
  renderCal();
  renderHighlights(skills);
  refreshStatus();
}

function renderContinue(h){
  const b = $("#dashContinue"), last = h[0];
  if (!last){ b.dataset.nav = "resume"; b.onclick = null; return; }
  delete b.dataset.nav;
  const reviewed = last.overall != null;
  b.querySelector(".action-ic").innerHTML = icon(ROUND_ICONS[last.round_key] || "chat");
  b.querySelector(".action-tx").innerHTML = `<b>${reviewed ? "Review your last round" : "Get feedback on your last round"}</b>
    <small>${esc(last.name)}${last.company ? " · " + esc(shortName(last.company)) : ""} · ${esc(ago(last.started_at))}${reviewed ? ` · ${last.overall}` : ""}</small>`;
  b.onclick = () => openInterview(last.loop_id, {focus: last.round_id});
}

function weekCounts(h){
  const from = weekStart().getTime(), c = {};
  h.forEach(x => { if (new Date(x.started_at).getTime() >= from) c[x.round_key] = (c[x.round_key] || 0) + 1; });
  return c;
}
function renderProgress(h, goal){
  const done = weekCounts(h);
  const ws = weekStart(), we = new Date(ws.getTime() + 6 * DAY);
  const f = d => d.toLocaleDateString([], {day: "numeric", month: "short"});
  $("#dashWeekRange").textContent = `${f(ws)} – ${f(we)}`;
  const keys = [...new Set([...Object.keys(goal), ...Object.keys(done)])];
  const target = Object.values(goal).reduce((a, n) => a + n, 0);
  const total = Object.values(done).reduce((a, n) => a + n, 0);
  const met = Object.entries(goal).reduce((a, [k, n]) => a + Math.min(n, done[k] || 0), 0);
  const pct = target ? Math.round(met / target * 100) : 0;
  $("#dashProgress").innerHTML = `
    <div class="prog">
      <div class="ring sm" style="--v:${pct}" role="img" aria-label="${met} of ${target} goal rounds done"><div><div><b>${total}</b><small>${target ? `of ${target}` : "rounds"}</small></div></div></div>
      <ul class="prog-list">${keys.length ? keys.map(k => {
        const n = done[k] || 0, t = goal[k];
        return `<li class="${t && n >= t ? "met" : ""}">${icon(ROUND_ICONS[k] || "chat")}<span>${esc(roundName(k))}</span><b>${n}${t ? `<small> / ${t}</small>` : ""}</b></li>`;
      }).join("") : `<li class="none">No rounds yet this week.</li>`}</ul>
    </div>`;
}

function trendHtml(t){
  if (t == null) return `<span class="trend na" title="Needs 4 scored rounds">—</span>`;
  if (Math.abs(t) < 0.25) return `<span class="trend flat" title="Last 3 vs earlier: steady">→</span>`;
  return `<span class="trend ${t > 0 ? "up" : "down"}" title="Last 3 rounds vs earlier: ${t > 0 ? "+" : ""}${t.toFixed(1)}">${t > 0 ? "↑" : "↓"} ${Math.abs(t).toFixed(1)}</span>`;
}
function renderSkills(skills){
  const el = $("#dashSkills");
  if (!skills.length){ el.innerHTML = `<p class="muted">Scores per skill appear after your first round with feedback. Each round grades 4–5 skills from its rubric.</p>`; return; }
  const top = skills.slice().sort((a, b) => b.n - a.n || a.avg - b.avg).slice(0, 6);
  el.innerHTML = `<ul class="skills">${top.map(s => `
    <li title="${esc(`Mostly from ${roundName(s.kind)} rounds`)}">
      <span class="sk-name">${esc(s.name)}</span>
      <span class="sk-bar"><i class="${s.avg >= 3 ? "hi" : s.avg >= 2.5 ? "mid" : "lo"}" style="width:${s.avg / 4 * 100}%"></i></span>
      <b>${s.avg.toFixed(1)}</b>${trendHtml(s.trend)}<span class="sk-n">${s.n}</span>
    </li>`).join("")}</ul>`;
}

function spark(vals){
  const w = 220, hgt = 56, pad = 4, n = vals.length;
  const x = i => n === 1 ? w / 2 : pad + i * (w - 2 * pad) / (n - 1);
  const y = v => hgt - pad - v / 100 * (hgt - 2 * pad);
  const pts = vals.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
  return `<svg class="spark" viewBox="0 0 ${w} ${hgt}" preserveAspectRatio="none" aria-hidden="true">
    <line x1="0" x2="${w}" y1="${y(70)}" y2="${y(70)}" class="spark-bar"/>
    <polyline points="${pts}"/>${vals.map((v, i) => `<circle cx="${x(i)}" cy="${y(v)}" r="${i === n - 1 ? 3.5 : 2.2}"/>`).join("")}</svg>`;
}
function renderTrend(reviewed){
  const vals = reviewed.slice(0, 10).reverse().map(x => x.overall);
  const el = $("#dashTrend");
  if (!vals.length){ el.innerHTML = `<h2>Score trend</h2><p class="muted">Your overall score per round, plotted once you have feedback.</p>`; return; }
  const last = vals.slice(-5), prev = vals.slice(-10, -5);
  const a = Math.round(avg(last));
  let line;
  if (prev.length >= 2){
    const d = a - Math.round(avg(prev));
    line = `Last ${last.length}: average <b>${a}</b>, ${d === 0 ? "level with" : `${d > 0 ? "up" : "down"} ${Math.abs(d)} on`} the ${prev.length} before.`;
  } else line = `Average <b>${a}</b> over ${vals.length} round${vals.length > 1 ? "s" : ""}. ${Math.max(0, 7 - vals.length)} more to compare against.`;
  el.innerHTML = `<h2>Score trend</h2>${spark(vals)}<p class="muted small">${line} Dashed line: 70, the usual hire bar.</p>`;
}

/* Evidence-based pick: the weakest skill seen in 2+ rounds, else the goal's biggest gap, else the latest report's advice. */
function renderNext(skills, h, goal){
  const el = $("#dashNext");
  const weak = skills.filter(s => s.n >= 2 && s.low.length >= 2).sort((a, b) => a.avg - b.avg)[0];
  let key = null, why = "", quote = "";
  if (weak){
    key = weak.kind;
    why = `<b>${esc(weak.name)}</b> scored 2/4 or lower in ${weak.low.length} of your ${weak.n} rounds that graded it.`;
    quote = weak.low[weak.low.length - 1].evidence;
  } else {
    const done = weekCounts(h);
    const gap = Object.entries(goal).map(([k, n]) => [k, n - (done[k] || 0)]).filter(([, r]) => r > 0).sort((a, b) => b[1] - a[1])[0];
    const latest = h.find(x => x.report && (x.report.practice_next || []).length);
    if (gap){ key = gap[0]; why = `${gap[1]} ${esc(roundName(gap[0]))} round${gap[1] > 1 ? "s" : ""} left on this week’s goal. No skill is weak in 2+ rounds yet.`; }
    else if (latest){ key = latest.round_key; why = `From your last feedback (${esc(latest.name)}):`; quote = latest.report.practice_next[0]; }
  }
  if (!key){ el.innerHTML = `<h2>Practise next</h2><p class="muted">After two rounds with feedback, this points at the skill that keeps scoring low and the round that trains it.</p>`; return; }
  el.innerHTML = `<h2>Practise next</h2>
    <div class="next-pick"><span class="row-ic">${icon(ROUND_ICONS[key] || "chat")}</span><div><b>${esc(roundName(key))}</b><p>${why}</p>${quote ? `<q>${esc(quote)}</q>` : ""}</div></div>
    <button class="btn primary sm" id="dashGo">Set up this round${icon("arrow")}</button>`;
  $("#dashGo").onclick = () => { selectRounds([key]); saveCfg(); nav("interviews"); };
}

function renderRecent(h){
  const el = $("#dashRecent");
  if (!h.length){ el.innerHTML = `<div class="none">No rounds yet. Your first one takes about a minute to set up.</div>`; return; }
  el.innerHTML = h.slice(0, 6).map(x => {
    const weakest = dimsOf(x).sort((a, b) => a.score - b.score)[0];
    const mins = tookMinutes(x);
    return `<button class="row-item recent" data-round-id="${x.round_id}" data-loop-id="${x.loop_id}">
      <span class="row-ic">${icon(ROUND_ICONS[x.round_key] || "chat")}</span>
      <span class="row-tx"><b>${esc(x.name)}</b><span>${esc(shortName(x.company) || "General")} · ${mins ? `${mins} of ${x.minutes} min` : `${x.minutes} min`}${weakest && weakest.score <= 2 ? ` · <em class="weak">Weakest: ${esc(weakest.name)} ${weakest.score}/4</em>` : ""}</span></span>
      <span class="recent-date">${esc(new Date(x.started_at).toLocaleDateString([], {day: "numeric", month: "short"}))}</span>
      ${x.overall != null ? `<span class="pill ${scoreClass(x.overall) === "hi" ? "ok" : scoreClass(x.overall) === "mid" ? "warn" : "bad"}">${x.overall}</span>` : `<span class="pill">No feedback</span>`}
    </button>`;
  }).join("");
  bindRows("#dashRecent");
}

function renderGoal(h, goal, editing = false){
  const el = $("#dashGoal"), done = weekCounts(h);
  const btn = $("#goalEdit");
  btn.textContent = editing ? "Cancel" : "Edit";
  btn.onclick = () => renderGoal(h, DASH.goal, !editing);
  if (editing){
    el.innerHTML = `<ul class="goal-edit">${Object.keys(CAT.ROUNDS).map(k => `
      <li><span>${esc(roundName(k))}</span><span class="stepper"><button class="btn ghost sm" data-k="${k}" data-d="-1" aria-label="Fewer">−</button><b data-v="${k}">${goal[k] || 0}</b><button class="btn ghost sm" data-k="${k}" data-d="1" aria-label="More">+</button></span></li>`).join("")}</ul>
      <div class="btn-row"><button class="btn primary sm" id="goalSave">Save goal</button></div>`;
    const draft = {...goal};
    el.querySelectorAll("[data-d]").forEach(b => b.onclick = () => {
      const k = b.dataset.k; draft[k] = Math.max(0, Math.min(14, (draft[k] || 0) + Number(b.dataset.d)));
      el.querySelector(`[data-v="${k}"]`).textContent = draft[k];
    });
    $("#goalSave").onclick = async () => {
      try{ await api.invoke("set_goal", {goal: draft}); DASH.goal = await api.invoke("get_goal"); toast("Goal saved."); renderDashboard(); }
      catch(e){ toast(errText(e), "err"); }
    };
    return;
  }
  const items = Object.entries(goal);
  if (!items.length){ el.innerHTML = `<p class="muted">No goal set. <button class="linkbtn" id="goalSet">Set rounds per week</button></p>`; $("#goalSet").onclick = () => renderGoal(h, goal, true); return; }
  const target = items.reduce((a, [, n]) => a + n, 0);
  const met = items.reduce((a, [k, n]) => a + Math.min(n, done[k] || 0), 0);
  el.innerHTML = `
    <p class="goal-line">${target} rounds this week<span>${met} / ${target}</span></p>
    <div class="bar"><i style="width:${met / target * 100}%"></i></div>
    <ul class="goal-list">${items.map(([k, n]) => {
      const d = Math.min(n, done[k] || 0);
      return `<li class="${d >= n ? "done" : ""}"><i class="tick">${d >= n ? icon("check") : ""}</i>${n} ${esc(roundName(k))}${n > 1 ? ` <small>${d} of ${n}</small>` : ""}</li>`;
    }).join("")}</ul>
    <p class="muted small">${met >= target ? "Goal met. Nice work." : `${target - met} to go. Resets Monday.`}</p>`;
}

function renderCal(){
  const m = DASH.cal, h = DASH.h;
  const perDay = new Map();
  h.forEach(x => { const k = dayKey(x.started_at); perDay.set(k, (perDay.get(k) || 0) + 1); });
  // Streak: consecutive days with a round, ending today (or yesterday, if today is still open).
  let streak = 0, d = new Date(); d.setHours(0, 0, 0, 0);
  if (!perDay.has(dayKey(d))) d = new Date(d.getTime() - DAY);
  while (perDay.has(dayKey(d))){ streak++; d = new Date(d.getTime() - DAY); }
  $("#calStreak").textContent = streak ? `${streak}-day streak` : "";
  const first = (m.getDay() + 6) % 7, days = new Date(m.getFullYear(), m.getMonth() + 1, 0).getDate();
  const today = dayKey(new Date());
  let cells = "";
  for (let i = 0; i < first; i++) cells += `<span></span>`;
  let monthCount = 0;
  for (let i = 1; i <= days; i++){
    const k = dayKey(new Date(m.getFullYear(), m.getMonth(), i)), n = perDay.get(k) || 0;
    monthCount += n;
    cells += `<span class="day${k === today ? " today" : ""}${n ? " has" : ""}" title="${n ? `${n} round${n > 1 ? "s" : ""}` : ""}">${i}${n ? `<i></i>` : ""}</span>`;
  }
  $("#dashCal").innerHTML = `
    <div class="cal-head"><button class="btn ghost sm" id="calPrev" aria-label="Previous month">‹</button><b>${m.toLocaleDateString([], {month: "long", year: "numeric"})}</b><button class="btn ghost sm" id="calNext" aria-label="Next month">›</button></div>
    <div class="cal">${["Mon","Tue","Wed","Thu","Fri","Sat","Sun"].map(w => `<span class="wd">${w}</span>`).join("")}${cells}</div>
    <p class="muted small">${monthCount ? `${monthCount} round${monthCount > 1 ? "s" : ""} this month.` : "No rounds this month."} A dot marks a day you practised.</p>`;
  $("#calPrev").onclick = () => { DASH.cal = new Date(m.getFullYear(), m.getMonth() - 1, 1); renderCal(); };
  $("#calNext").onclick = () => { DASH.cal = new Date(m.getFullYear(), m.getMonth() + 1, 1); renderCal(); };
}

/* Strengths and mistakes that repeat across rounds, each with the latest evidence from the feedback. */
function renderHighlights(skills){
  const el = $("#dashHighlights");
  const weak = skills.filter(s => s.low.length >= 2).sort((a, b) => b.low.length - a.low.length || a.avg - b.avg).slice(0, 3);
  const strong = skills.filter(s => s.high.length >= 2 && s.avg >= 3).sort((a, b) => b.high.length - a.high.length).slice(0, 2);
  if (!weak.length && !strong.length){
    el.innerHTML = `<p class="muted">${skills.length ? "Nothing repeats yet. A skill shows here once it scores low (or high) in two rounds." : "Once you have feedback on two rounds, repeated strengths and mistakes show here, with the interviewer’s own words."}</p>`;
    return;
  }
  const item = (s, good) => {
    const p = (good ? s.high : s.low).at(-1);
    return `<li class="${good ? "good" : "bad"}"><span class="hl-ic">${icon(good ? "check" : "target")}</span><div>
      <b>${esc(s.name)}</b><small>${good ? "Strong" : "Low"} in ${(good ? s.high : s.low).length} of ${s.n} rounds</small>
      ${p.evidence ? `<q>${esc(p.evidence)}</q>` : ""}</div></li>`;
  };
  el.innerHTML = `<ul class="highlights">${weak.map(s => item(s, false)).join("")}${strong.map(s => item(s, true)).join("")}</ul>`;
}

/* ---------------- setup ---------------- */
const cfgFields = ["company","panel","level","lang","temper","resume","jd","extra","roundLen"];
let PANELS = [];
let MODE = "general";    // "general" or "company"
let DOSSIERS = [];       // researched dossiers, newest first
const DEFAULT_ROUNDS = ["resume", "coding", "sysdesign", "behavioral"];
function fillSelect(sel, obj, label){ sel.innerHTML = Object.entries(obj).map(([k,v]) => `<option value="${esc(k)}">${esc(label(v))}</option>`).join(""); }

function initSetup(){
  fillSelect($("#company"), CAT.COMPANIES, v => v.name);
  fillSelect($("#level"), CAT.LEVELS, v => v);
  fillSelect($("#rqLevel"), CAT.LEVELS, v => v);
  $("#panel").innerHTML += PANELS.map(p => `<option value="${esc(p.id)}">${esc(p.name)} — the same five people every time</option>`).join("");
  const c = LS.get("onsite.cfg", {company:"auto", level:"sde2", lang:"Java", temper:"realistic", roundLen:"", rounds:DEFAULT_ROUNDS});
  cfgFields.forEach(f => {
    const el = $("#"+f); if (c[f] == null) return;
    // A saved value the catalog no longer has would blank the select; keep the default instead.
    if (el.tagName === "SELECT" && ![...el.options].some(o => o.value === String(c[f]))) return;
    el.value = c[f];
  });
  $("#rqLevel").value = $("#level").value;
  setMode(c.mode === "company" ? "company" : "general", false);
  loadDossiers();
  renderPanelPeople();
  $("#panel").addEventListener("change", renderPanelPeople);
  $("#roundList").onclick = e => {
    const b = e.target.closest(".chip"); if (!b) return;
    b.setAttribute("aria-pressed", b.getAttribute("aria-pressed") !== "true");
    saveCfg(); renderAgenda();
  };
  document.addEventListener("change", e => { if (e.target.closest("#view-interviews")){ saveCfg(); renderAgenda(); } });
  let t; document.addEventListener("input", e => { if (e.target.closest("#view-interviews")){ clearTimeout(t); t = setTimeout(saveCfg, 400); } });
  hookDrop("#dropResume", "#resumeFile", "#resume", "#resumeMsg", "Resume");
  hookDrop("#dropJd", "#jdFile", "#jd", "#jdMsg", "Job description");
}
function selectRounds(keys){ $$("#roundList .chip").forEach(b => b.setAttribute("aria-pressed", keys.includes(b.dataset.round))); }
const pickedTiles = () => $$('#roundList .chip[aria-pressed="true"]');
function selectedRounds(){ return pickedTiles().map(b => b.dataset.round); }
function saveCfg(){
  const old = LS.get("onsite.cfg", {});
  const c = {}; cfgFields.forEach(f => c[f] = $("#"+f).value);
  c.mode = MODE;
  c.dossier = $("#dossier").value || old.dossier || "";
  // Company tiles are the dossier's rounds; keep the general picks for when they switch back.
  c.rounds = MODE === "general" ? selectedRounds() : (old.rounds || []);
  LS.set("onsite.cfg", c);
}

/* ---- General vs Company-specific ---- */
function setMode(m, save = true){
  MODE = m;
  $("#modeGeneral").setAttribute("aria-checked", m === "general");
  $("#modeCompany").setAttribute("aria-checked", m === "company");
  $("#companyField").hidden = m === "company";
  $("#dossierField").hidden = m !== "company";
  $("#roundsTitle").textContent = m === "company" ? "Rounds in this loop" : "Choose your rounds";
  $("#roundsLede").textContent = m === "company" ? "As candidates reported them, in order. Untick any you want to skip." : "Pick the rounds you want to practise. They run in this order.";
  renderTiles();
  if (save) saveCfg();
  renderAgenda();
}
$("#modeGeneral").onclick = () => setMode("general");
$("#modeCompany").onclick = () => setMode("company");
function currentDossier(){ return MODE === "company" ? DOSSIERS.find(d => d.id === $("#dossier").value) || null : null; }

const tileHtml = (key, name, min, attrs) =>
  `<button type="button" class="chip tile" data-round="${esc(key)}" data-name="${esc(name)}" data-min="${Number(min) || 45}" ${attrs} aria-pressed="false"><span class="tile-ic">${icon(ROUND_ICONS[key] || "chat")}</span><span class="tile-tx"><b>${esc(name)}</b><span class="min">${Number(min) || 45} min</span></span><span class="tile-check">${icon("check")}</span></button>`;
function renderTiles(){
  if (MODE === "general"){
    $("#roundList").innerHTML = Object.entries(CAT.ROUNDS).map(([k,r]) => tileHtml(k, r.name, r.min, `title="${esc(r.blurb)}"`)).join("");
    selectRounds(LS.get("onsite.cfg", {rounds: DEFAULT_ROUNDS}).rounds || []);
    return;
  }
  const d = currentDossier();
  $("#roundList").innerHTML = d
    ? d.process.map((r, i) => tileHtml(r.type, r.name, r.minutes, `data-di="${i}" title="${esc(r.what || "")}"`)).join("")
    : `<p class="hint">No dossier yet. <button type="button" class="linkbtn" data-nav="dossiers">Research a company</button> to get its real loop.</p>`;
  $$("#roundList .chip").forEach(b => b.setAttribute("aria-pressed", "true"));
}
async function loadDossiers(){
  try{ DOSSIERS = await api.invoke("list_dossiers"); }catch(e){ DOSSIERS = []; }
  const sel = $("#dossier"), keep = sel.value || LS.get("onsite.cfg", {}).dossier;
  sel.innerHTML = DOSSIERS.length
    ? DOSSIERS.map(d => `<option value="${esc(d.id)}">${esc(d.company)} — ${esc(d.role)}</option>`).join("")
    : `<option value="">No dossiers yet</option>`;
  if (DOSSIERS.some(d => d.id === keep)) sel.value = keep;
  sel.disabled = !DOSSIERS.length;
  renderDossierHint();
  if (MODE === "company"){ renderTiles(); renderAgenda(); }
}
function renderDossierHint(){
  const d = DOSSIERS.find(x => x.id === $("#dossier").value);
  $("#dossierHint").innerHTML = d
    ? `${d.process.length} rounds · ${(d.sources || []).length} sources · researched ${esc(String(d.preparedAt || "").slice(0, 10))} · <button type="button" class="linkbtn" data-nav="dossiers">See it</button>`
    : `<button type="button" class="linkbtn" data-nav="dossiers">Research a company</button> to make one.`;
}
$("#dossier").addEventListener("change", () => {
  const d = currentDossier();
  if (d && CAT.LEVELS[d.level]) $("#level").value = d.level;
  renderDossierHint(); renderTiles();
});
const TEXT_FILE = /\.(txt|md|markdown|text)$/i;
function hookDrop(dropSel, inputSel, textSel, msgSel, label){
  const drop = $(dropSel), input = $(inputSel), ta = $(textSel), msgEl = $(msgSel);
  const initial = msgEl.innerHTML;
  const lower = label.toLowerCase();
  const show = () => {
    const n = words(ta.value);
    drop.classList.toggle("filled", n > 0);
    msgEl.innerHTML = n
      ? `${n} words loaded. <button type="button" class="linkbtn" data-edit>${ta.hidden ? "Edit" : "Hide"}</button> · <button type="button" class="linkbtn" data-clear>Remove</button>`
      : ta.hidden ? initial : `Type or paste the ${lower} below, or <button type="button" class="linkbtn" data-choose>choose a file</button>`;
  };
  const load = async f => {
    input.value = "";
    if (TEXT_FILE.test(f.name) || (f.type || "").startsWith("text/")){
      try{ ta.value = (await f.text()).trim(); ta.hidden = true; saveCfg(); show(); }
      catch(e){ toast(`Couldn’t read that file. Paste the ${lower} instead.`, "err"); }
      if (label === "Resume") rememberResumeFile(null);
      return;
    }
    // PDF, Word, RTF: Rust reads the text out of it.
    msgEl.textContent = `Reading ${f.name}…`;
    try{
      const file = await fileToB64(f);
      const doc = await api.invoke("read_document", {name: file.name, data: file.data});
      ta.value = doc.text; ta.hidden = true; saveCfg(); show();
      if (label === "Resume") rememberResumeFile(file);
    }catch(e){ show(); toast(errText(e), "err"); }
  };
  drop.onclick = e => {
    const b = e.target.closest("[data-paste],[data-edit],[data-clear],[data-choose]");
    if (b && b.matches("[data-paste]")){ ta.hidden = false; show(); ta.focus(); return; }
    if (b && b.matches("[data-edit]")){ ta.hidden = !ta.hidden; show(); if (!ta.hidden) ta.focus(); return; }
    if (b && b.matches("[data-clear]")){ ta.value = ""; ta.hidden = true; saveCfg(); show(); if (label === "Resume") rememberResumeFile(null); drop.querySelector("[data-choose]").focus(); return; }
    input.click();
  };
  input.onchange = () => input.files[0] && load(input.files[0]);
  drop.ondragover = e => { e.preventDefault(); drop.classList.add("over"); };
  drop.ondragleave = e => { if (!drop.contains(e.relatedTarget)) drop.classList.remove("over"); };
  drop.ondrop = e => { e.preventDefault(); drop.classList.remove("over"); const f = e.dataTransfer.files[0]; if (f) load(f); };
  ta.addEventListener("input", show);
  // Hand edits mean the file no longer matches what the interviewer reads.
  if (label === "Resume") ta.addEventListener("input", () => { if (RESUME_FILE) rememberResumeFile(null); });
  show();
}
/* Who runs round i. A panel gives each round type a fixed person; "Mixed" rotates names. */
function interviewerFor(i, key, panelId){
  const panel = PANELS.find(p => p.id === panelId);
  const iv = panel && panel.interviewers.find(x => x.id === (panel.round_owner || {})[key]);
  return iv ? {name: iv.name.split(" ")[0], full: iv.name, role: iv.role} : {name: CAT.NAMES[(i*5+3) % CAT.NAMES.length], full: null, role: null};
}
function persona(i){ return interviewerFor(i, selectedRounds()[i], $("#panel").value).name; }
function renderPanelPeople(){
  const panel = PANELS.find(p => p.id === $("#panel").value);
  $("#panelPeople").textContent = panel ? panel.interviewers.map(iv => `${iv.name} (${iv.role})`).join(" · ") : "Names rotate. Pick a panel to meet the same people every time.";
}
function tileMinutes(b){ return Number($("#roundLen").value) || Number(b.dataset.min); }

function renderAgenda(){
  if (!CAT) return;
  $$("#roundList .chip").forEach(b => b.querySelector(".min").textContent = `${tileMinutes(b)} min`);
  const on = pickedTiles();
  $("#roundCount").textContent = on.length ? `${on.length} selected` : "";
  if (!on.length){
    const why = MODE === "company" && !currentDossier() ? "Pick a dossier first." : "Pick at least one round.";
    $("#agenda").innerHTML = `<li><span class="empty">${why}</span></li>`;
    $("#duration").textContent = "—"; $("#startBtn").disabled = true; return;
  }
  $("#startBtn").disabled = starting;
  const total = on.reduce((a, b) => a + tileMinutes(b), 0);
  $("#duration").textContent = total >= 60 ? `${Math.floor(total/60)} h${total%60 ? " " + total%60 + " min" : ""}` : `${total} min`;
  $("#agenda").innerHTML = on.map((b,i) => `<li><span class="n">${i+1}</span><div><b>${esc(b.dataset.name)}</b><span>${tileMinutes(b)} min · with ${esc(persona(i))}</span></div></li>`).join("");
}

$("#startBtn").onclick = async () => {
  const rounds = selectedRounds(); if (!rounds.length || starting) return;
  saveCfg();
  const d = currentDossier(), on = pickedTiles();
  const setup = {
    rounds, company: d ? "auto" : $("#company").value, level: $("#level").value, temperament: $("#temper").value,
    lang: $("#lang").value, resume: $("#resume").value, jd: $("#jd").value.trim() || (d && d.jd) || "", notes: $("#extra").value,
    minutes: Number($("#roundLen").value) || null,
    panel: $("#panel").value,
    // Company mode: Rust copies the dossier into the loop. The rest is for this screen only.
    dossier_id: d ? d.id : "", dossier_rounds: d ? on.map(b => Number(b.dataset.di)) : [],
    dossier_company: d ? d.company : "", dossierRounds: on.map(b => ({name: b.dataset.name, minutes: Number(b.dataset.min)})),
  };
  const btn = $("#startBtn"), m = $("#startMsg");
  if (MODE === "company" && !d){ m.className = "form-msg err"; m.innerHTML = `Pick a dossier, or <button type="button" class="linkbtn" data-nav="dossiers">research a company</button> first.`; return; }
  if (!aiReady()){ m.className = "form-msg err"; m.innerHTML = `Connect an AI model first. <button type="button" class="linkbtn" data-nav="settings" data-sec="models">Open Settings</button>`; return; }
  if (!words(setup.resume)){ m.className = "form-msg err"; m.textContent = "Add your resume. Every interview starts with a resume check."; return; }
  btn.disabled = true; btn.setAttribute("aria-busy", "true"); btn.classList.add("is-busy");
  m.className = "form-msg"; m.textContent = "Checking your resume…";
  try{
    const check = await api.invoke("ats_check", resumeArgs(setup.jd));
    if (check.extractability === "none") throw "An ATS can’t read any text in your resume file, so neither can the panel. Open Resume check to see why.";
    RESUME_CHECK = check; atsShown = false;
    m.textContent = "Setting up your loop…";
    const id = await api.invoke("create_loop", {setup});
    loop = {id, setup, count: rounds.length};
    m.textContent = "";
    openReady(0);
  }catch(e){ m.className = "form-msg err"; m.textContent = errText(e); }
  finally{ btn.classList.remove("is-busy"); btn.removeAttribute("aria-busy"); renderAgenda(); }
};

/* ---------------- call room ---------------- */
function notesTemplate(k){
  if (k === "sysdesign") return "Requirements\n- Functional:\n- Non-functional:\n\nEstimates\n\nAPIs\n\nData model\n\nHigh-level design\n  client -> \n\nDeep dives\n\nTrade-offs and failure modes\n";
  if (k === "lld") return "Requirements\n\nEntities\n\nClasses and interfaces\n\nKey flows\n\nConcurrency and extensibility\n";
  if (k === "project") return "Architecture sketch\n  [service] -> [queue] -> [service]\n";
  return "";
}
/* "Indian fintech (Razorpay, …)" reads as "Indian fintech" in titles. */
const shortName = n => /^inferred from jd$/i.test(String(n || "").trim()) ? "General" : String(n || "").replace(/\s*\(.*\)\s*$/, "");
function companyLabel(setup){
  const dc = setup.dossier_company || (setup.dossier && setup.dossier.company);
  if (dc) return dc;
  return setup.company === "auto" ? "General" : shortName((CAT.COMPANIES[setup.company] || {}).name || setup.company); }
function temperLabel(v){ const o = [...$("#temper").options].find(o => o.value === v); return o ? o.text.split(" —")[0] : v; }

async function startRound(index){
  if (starting) return;
  starting = true;
  try{ await openRoom(index); }
  finally{ starting = false; }
}
async function openRoom(index){
  await refreshStatus();
  const k = loop.setup.rounds[index];
  const who = interviewerFor(index, loop.setup.rounds[index], loop.setup.panel);
  const name = who.name;
  const voiceOn = !!(SETTINGS && SETTINGS.voice.enabled && VS.up);
  const s = session = {roundId: null, index, key: k, name: loopRound(index).name, persona: name, minutes: loopRound(index).minutes, startedAt: Date.now(),
             shared: [], closed: false, busy: true, pendingEl: null, live: "", draftEl: null,
             voiceOn, muted: false, phase: "thinking", heldBase: null, fromHeld: false, transcribeT: null, abandoned: false};
  // The room fills in behind the loader, so the first question streams in while voice loads.
  openPrep(index, who);
  const comp = companyLabel(loop.setup);
  $("#roomIc").textContent = $("#ctxIc").textContent = comp[0];
  $("#roomTitle").textContent = `${comp} — ${s.name}`;
  $("#roomSub").textContent = `${CAT.LEVELS[loop.setup.level]} · ${s.minutes} minutes · round ${index+1} of ${loop.count} · with ${who.full ? `${who.full}, ${who.role}` : name}`;
  $("#ctxCompany").textContent = comp === "General" ? "General practice" : comp;
  $("#ctxRound").textContent = s.name;
  const codes = ["coding", "comprehension", "lld", "fundamentals"].includes(k);
  $("#ctxChips").innerHTML = [codes ? loop.setup.lang : null, temperLabel(loop.setup.temperament) + " interviewer", CAT.LEVELS[loop.setup.level]].filter(Boolean).map(x => `<span>${esc(x)}</span>`).join("");
  $("#tips").innerHTML = (TIPS[k] || []).map(t => `<li>${esc(t)}</li>`).join("");
  $("#transcript").innerHTML = ""; $("#answer").value = ""; hint(""); $("#turnInfo").textContent = "";
  s.lang = loop.setup.lang; s.buffers = {};
  ED.setLang(s.lang); ED.set(""); $("#edLang").value = s.lang;
  $("#notes").value = notesTemplate(k);
  if (BOARD) BOARD.reset();
  $("#wrapBanner").hidden = true;
  $("#sharedDot").hidden = true; $('.tab[data-tab="shared"]').hidden = true;
  applyVoiceUi();
  fillRoomMic();
  showProblem(null);
  renderShared(); selectTab(startTab(k));
  buildWaves();
  setPhase("thinking");
  s.pendingEl = addMsg("iv", "", true);
  let info;
  try{ info = await api.invoke("start_round", {loopId: loop.id, index}); }
  catch(e){
    if (session !== s || s.abandoned) return;
    session = null;
    prepFailed(`Couldn’t start the round. ${errText(e)}`, index);
    return;
  }
  if (s.abandoned || session !== s){
    // The user left while the round was starting: stop the interviewer we just created.
    deadRounds.add(info.round_id);
    api.invoke("end_round", {roundId: info.round_id, workspace: {code: "", notes: ""}}).catch(() => {});
    return;
  }
  s.roundId = info.round_id;
  s.minutes = info.plan.minutes;
  if (info.problem) showProblem(info.problem);
  // Rust decides whether voice really started. If it was wanted and didn't, say so before the call.
  const lostVoice = !!(SETTINGS && SETTINGS.voice.enabled) && info.voice === false;
  if (typeof info.voice === "boolean" && info.voice !== s.voiceOn){
    s.voiceOn = info.voice; applyVoiceUi();
    if (s.phase !== "thinking" || !s.busy) setPhase("idle");
  }
  if (lostVoice && !await prepTyped(s)) return;
  enterRoom(s);
}
function enterRoom(s){
  stopPrep();
  s.startedAt = Date.now();
  nav("room");
  startTimer();
  // The board may have loaded while the room was hidden; let it measure itself now.
  if (BOARD && !$('[data-pane="board"]').hidden) BOARD.refresh();
}

/* ---------------- preparing (between Get ready and the call) ---------------- */
/* Every line is a real check reported by Rust (`prep` events); the call opens when they're done. */
const PREP_STEPS = [
  ["claude", "AI model"], ["question", "First question"],
  ["voicestudio", "VoiceStudio"], ["voice", "Interviewer’s voice"],
  ["stt", "Live captions"], ["whisper", "Accurate transcript"],
  ["speaker", "Speaker"], ["mic", "Microphone"],
];
const PREP_WORD = {wait: "Waiting", ok: "Ready", warn: "Fallback", fail: "Failed", off: "Off"};
let prepT = null;
function openPrep(index, who){
  nav("prep");
  $("#prepIc").textContent = companyLabel(loop.setup)[0];
  $("#prepTitle").textContent = "Getting the room ready";
  $("#prepSub").textContent = `${loopRound(index).name} with ${who.full || who.name}. The call opens when everything below is ready.`;
  $("#prepList").innerHTML = PREP_STEPS.map(([id, name]) =>
    `<li class="wait" data-step="${id}"><i class="dot"></i><div><b>${esc(name)}</b><small></small></div><span>Waiting</span></li>`).join("");
  msg("#prepMsg", "");
  prepButtons(`<button class="btn ghost" data-prep="cancel">Cancel</button>`, {cancel: cancelPrep});
  clearInterval(prepT);
  prepT = setInterval(() => $$("#prepList li.run").forEach(li => li.querySelector("span").textContent = `${Math.round((Date.now() - li.dataset.since) / 1000)} s`), 1000);
}
function stopPrep(){ clearInterval(prepT); prepT = null; }
function prepButtons(html, actions){
  $("#prepBtns").innerHTML = html;
  $$("#prepBtns [data-prep]").forEach(b => b.onclick = actions[b.dataset.prep]);
}
api.listen("prep", ev => {
  if ($("#view-prep").hidden) return;
  const li = $(`#prepList li[data-step="${ev.step}"]`); if (!li) return;
  if (ev.state === "run" && !li.classList.contains("run")) li.dataset.since = Date.now();
  li.className = ev.state;
  li.querySelector("small").textContent = ev.detail || "";
  li.querySelector("span").textContent = ev.state === "run" ? `${Math.round((Date.now() - li.dataset.since) / 1000)} s` : PREP_WORD[ev.state] || "";
});
function cancelPrep(){
  const s = session;
  if (s){ s.abandoned = true; session = null; }
  api.invoke("cancel_start").catch(() => {});
  stopPrep();
  openReady(s ? s.index : readyIndex);
}
function prepFailed(message, index){
  stopPrep();
  $("#prepTitle").textContent = "The room isn’t ready";
  msg("#prepMsg", esc(message), "err");
  prepButtons(`<button class="btn primary" data-prep="retry">${icon("refresh")}Try again</button><button class="btn ghost" data-prep="back">Back</button>`, {
    retry: () => startRound(index),
    back: () => openReady(index),
  });
}
/* Claude is ready but voice isn't: the candidate picks typed or back. Resolves true to go on. */
function prepTyped(s){
  stopPrep();
  $("#prepTitle").textContent = "Voice isn’t working";
  msg("#prepMsg", "The interviewer is ready, but this round can only be typed. Fix the red line above, or carry on typing.", "err");
  return new Promise(done => prepButtons(
    `<button class="btn primary" data-prep="typed">Start typed</button><button class="btn ghost" data-prep="back">Back</button>`, {
    typed: () => done(true),
    back: () => { done(false); leaveRound(); openReady(s.index); },
  }));
}

function applyVoiceUi(){
  const s = session; if (!s) return;
  const mb = $("#muteBtn");
  mb.setAttribute("aria-pressed", String(s.muted));
  mb.querySelector("span").textContent = s.muted ? "Unmute" : "Mute";
  mb.setAttribute("aria-label", s.muted ? "Unmute" : "Mute");
  mb.querySelector("use").setAttribute("href", s.muted ? "#i-mic" : "#i-mic-off");
  mb.disabled = !s.voiceOn || s.closed;
  mb.title = !s.voiceOn ? "Voice is off for this round" : s.muted ? "Unmute" : "Mute";
  $("#roomMic").disabled = !s.voiceOn;
  $("#view-room").classList.toggle("typed", !s.voiceOn);
  $("#answer").placeholder = s.voiceOn ? "Type instead…" : "Type your answer…";
}

/* Stops the round. Resolves once Rust has saved it, so a report call sees the final transcript.
   Resolves false when Rust dropped the round because the candidate never answered. */
async function leaveRound(){
  const s = session; session = null;
  clearInterval(timerI);
  if (!s) return;
  clearTimeout(s.transcribeT);
  if (s.roundId === null){
    // Still on the loader: stop the start in Rust too.
    s.abandoned = true; stopPrep();
    api.invoke("cancel_start").catch(() => {});
    return;
  }
  deadRounds.add(s.roundId);
  return api.invoke("end_round", {roundId: s.roundId, workspace: currentWorkspace(s)}).catch(e => api.log("error", `end_round: ${errText(e)}`));
}

/* phases: thinking | speaking | cutting | listening | hearing | transcribing | idle | muted | closed
   cutting = the candidate tapped the mic over the interviewer; the mic opens next. */
function setPhase(p){
  if (!session) return;
  session.phase = p;
  if (p !== "transcribing") clearTimeout(session.transcribeT);
  const orb = $("#orb");
  const label = {
    thinking: `${session.persona} is thinking…`,
    speaking: `${session.persona} is speaking — tap to cut in`,
    cutting: "Go ahead…",
    listening: "Listening…",
    hearing: "Hearing you… tap when done",
    transcribing: "Got it. Writing it down…",
    idle: session.voiceOn ? "Mic off — tap to talk" : "Your turn — type below",
    muted: "Muted — tap to unmute",
    closed: "Round closed",
  }[p];
  orb.dataset.state = p === "hearing" || p === "cutting" ? "listening" : p === "transcribing" ? "thinking" : (p === "idle" && !session.voiceOn) || p === "closed" ? "off" : p;
  orb.setAttribute("aria-label", label);
  $("#orbLabel").textContent = label;
  $$(".wave").forEach(w => w.classList.toggle("live", p === "speaking" || p === "listening" || p === "hearing"));
  if (p !== "speaking" && p !== "listening" && p !== "hearing") setWave(0);
}
/* Where to go when nothing is happening: closed beats muted beats idle. */
function restPhase(){ return session.closed ? "closed" : session.muted ? "muted" : "idle"; }

function buildWaves(){
  const bars = n => Array.from({length:n}, () => "<i></i>").join("");
  $("#waveL").innerHTML = bars(7); $("#waveR").innerHTML = bars(7);
}
const SHAPE = [.35,.6,.9,1,.75,.5,.3];
const calm = matchMedia("(prefers-reduced-motion: reduce)");
function setWave(level){
  const v = Math.min(1, level * 9);
  const j = () => calm.matches ? 1 : 0.75 + Math.random()*0.5;
  $$("#waveL i").forEach((b, i) => b.style.height = `${6 + SHAPE[6-i] * v * 30 * j()}px`);
  $$("#waveR i").forEach((b, i) => b.style.height = `${6 + SHAPE[i] * v * 30 * j()}px`);
}

$("#orb").onclick = () => {
  const s = session;
  if (!s || !s.voiceOn || s.closed){ $("#answer").focus(); return; }
  if (s.phase === "listening" || s.phase === "hearing") api.invoke("voice_finish");
  else if (s.phase === "muted" || s.phase === "idle") setMuted(false);
  else if (s.phase === "speaking"){
    // Cut in: stop the interviewer now. The mic opens as soon as their turn is over.
    if (s.muted) setMuted(false);
    api.invoke("voice_interrupt").catch(() => {});
    setPhase("cutting");
  }
};
$("#muteBtn").onclick = () => session && session.voiceOn && !session.closed && setMuted(!session.muted);
function setMuted(m){
  const s = session;
  const wasMuted = s.muted;
  s.muted = m;
  applyVoiceUi();
  api.invoke("voice_mute", {muted: m}).catch(() => {});
  if (m){
    // Keep what we heard so far. Rust sends the authoritative text as a `held` event.
    const heard = s.draftEl ? s.draftEl.querySelector(".bubble").textContent.trim() : "";
    clearDraft();
    if (heard){ s.heldBase = $("#answer").value; toBox(heard); s.fromHeld = true; holdHint(); }
    if (["listening","hearing","transcribing","idle"].includes(s.phase)) setPhase("muted");
    announce("Muted.");
  } else {
    if (s.phase === "muted") setPhase("idle");
    if (wasMuted) announce("Unmuted.");
  }
}
function holdHint(){ hint("Muted. What you said is in the box. Edit it and press Enter to send, or unmute to keep talking."); }

/* Puts text into the answer box after anything already there, and focuses it. */
function toBox(text, msg){
  const a = $("#answer");
  const cur = a.value.trimEnd();
  a.value = cur ? cur + " " + text : text;
  a.focus({preventScroll:true});
  a.setSelectionRange(a.value.length, a.value.length);
  if (msg) hint(msg);
}
function hint(text){ const h = $("#composeHint"); h.textContent = text; h.hidden = !text; }

function startTimer(){
  clearInterval(timerI);
  const tick = () => {
    if (!session) return;
    const elapsed = Math.floor((Date.now() - session.startedAt) / 1000);
    const left = session.minutes * 60 - elapsed, over = left < 0, a = Math.abs(left);
    $("#timerText").textContent = `${String(Math.floor(a/60)).padStart(2,"0")}:${String(a%60).padStart(2,"0")}`;
    $("#timerLabel").textContent = over ? "over time" : "remaining";
    $("#timer").classList.toggle("over", over);
    renderPhases(elapsed / 60);
  };
  tick(); timerI = setInterval(tick, 1000);
}
function renderPhases(min){
  const plan = PLANS[session.key] || PLANS.hm;
  let acc = 0;
  $("#phases").innerHTML = plan.map(([name, frac]) => {
    const len = Math.max(1, Math.round(frac * session.minutes));
    const start = acc; acc += frac * session.minutes;
    const st = min >= acc ? "done" : min >= start ? "now" : "";
    return `<li class="${st}"${st === "now" ? ' aria-current="step"' : ""}><span class="pt">${st === "done" ? icon("check") : ""}</span><div><b>${esc(name)}</b><span>${len} min</span></div></li>`;
  }).join("");
}

/* events from the interviewer (Claude) */
api.listen("round", ev => {
  const s = session;
  if (!s || deadRounds.has(ev.round_id) || (s.roundId !== null && ev.round_id !== s.roundId)) return;
  switch (ev.kind){
    case "delta":
      s.live += ev.text;
      if (s.pendingEl) renderIvMsg(s.pendingEl, s.live);
      break;
    case "code":
      s.shared.push({lang: ev.lang || "code", code: ev.code});
      renderShared(); $("#sharedDot").hidden = false; $('.tab[data-tab="shared"]').hidden = false;
      if (s.key === "coding" || s.key === "comprehension") selectTab("shared");
      break;
    case "turn_done":
      if (s.pendingEl) renderIvMsg(s.pendingEl, ev.text);
      s.pendingEl = null; s.live = ""; s.busy = false;
      s.ttft = ev.ttft_ms;
      showTurnInfo();
      if (!s.voiceOn) announce(`${s.persona}: ${ev.text.replace(/\[\[END\]\]/g, "")}`);
      if (ev.ended){
        s.closed = true; $("#wrapBanner").hidden = false; applyVoiceUi();
        // Let the goodbye finish; the speaking-off event moves us to closed.
        if (s.phase !== "speaking") setPhase("closed");
      }
      else if (!s.voiceOn){ setPhase("idle"); $("#answer").focus({preventScroll:true}); }
      // Voice on: the reply may not have started playing yet. Stay on "thinking" until
      // the voice loop reports speaking-off (always sent at the end of a turn).
      break;
    case "error":
      failPending(ev.message);
      s.busy = false;
      if (s.phase === "thinking") setPhase(restPhase());
      break;
    case "exited":
      // Claude quit mid-turn. The next send restarts it with --resume.
      if (s.busy && !s.closed){ failPending("The interviewer dropped out."); s.busy = false; if (s.phase === "thinking") setPhase(restPhase()); }
      break;
  }
});

/* events from the voice loop (VoiceStudio + mic) */
api.listen("voice", ev => {
  const s = session;
  if (!s || !s.voiceOn) return;
  switch (ev.kind){
    case "speaking":
      if (ev.active){ if (s.phase !== "cutting") setPhase("speaking"); }
      else if (s.phase === "speaking" || s.phase === "cutting") setPhase(s.busy ? "thinking" : restPhase());
      else if (s.phase === "thinking" && !s.busy) setPhase(restPhase());
      break;
    case "listening":
      if (ev.active){ if (!s.muted && !s.closed) setPhase("listening"); }
      else if (s.phase === "hearing"){
        // Speech ended; the final transcript follows. If it never does (no words), fall back.
        setPhase("transcribing");
        s.transcribeT = setTimeout(() => { if (session === s && s.phase === "transcribing"){ salvageDraft(); setPhase(restPhase()); } }, 12000);
      }
      else if (s.phase === "listening") setPhase(restPhase());
      break;
    case "speech_start": if (s.phase === "listening") setPhase("hearing"); break;
    case "partial": if (!s.muted && ev.text) showDraft(ev.text); break;
    case "final":
      s.sttMs = ev.stt_ms;
      deliverVoice(ev.text || "");
      break;
    case "held": {
      const text = (ev.text || "").trim();
      clearDraft();
      if (text){
        const a = $("#answer");
        if (s.heldBase !== null){ a.value = s.heldBase; }
        s.heldBase = null;
        toBox(text); s.fromHeld = true; holdHint();
      }
      s.heldBase = null;
      if (s.phase === "transcribing" || s.phase === "hearing" || s.phase === "listening") setPhase(restPhase());
      break;
    }
    case "latency": s.voiceMs = ev.ms; showTurnInfo(); break;
    case "level": if (["speaking","listening","hearing"].includes(s.phase)) setWave(s.phase === "speaking" ? ev.out : ev.mic); break;
    case "error":
      toast(ev.message, "err");
      if (["listening","hearing","transcribing"].includes(s.phase)){ salvageDraft(); setPhase(restPhase()); }
      break;
  }
});

/* A voice answer came in. Never drop it: if it can't be sent now, it goes into the box. */
function deliverVoice(raw){
  const s = session;
  let text = raw.trim();
  if (s.phase === "transcribing") setPhase(restPhase());
  if (!text){ clearDraft(); return; }
  if (s.fromHeld && $("#answer").value.trim()){
    // They muted mid-answer, then unmuted and kept going: send both halves together.
    text = $("#answer").value.trim() + " " + text;
    $("#answer").value = ""; s.fromHeld = false; hint("");
  }
  if (s.busy || s.roundId === null){
    clearDraft();
    toBox(text, "We caught this while the interviewer was replying. Edit it if needed and press Enter to send.");
    return;
  }
  if (s.draftEl) s.draftEl.querySelector(".bubble").textContent = text;
  sendAnswer({text, fromVoice: true});
}
/* Moves an unfinished draft into the box instead of throwing it away. */
function salvageDraft(){
  const s = session; if (!s || !s.draftEl) return;
  const t = s.draftEl.querySelector(".bubble").textContent.trim();
  clearDraft();
  if (t) toBox(t, "The mic stopped before your answer was sent. It’s in the box. Edit it and press Enter.");
}

function showTurnInfo(){
  const s = session, bits = [];
  if (s.voiceMs != null) bits.push(`Voice reply in ${(s.voiceMs/1000).toFixed(1)} s after you stopped`);
  if (s.ttft != null) bits.push(`first word ${(s.ttft/1000).toFixed(1)} s`);
  $("#turnInfo").textContent = bits.join(" · ");
}

function showDraft(text){
  if (!session.draftEl) session.draftEl = addMsg("me", "", false, true);
  session.draftEl.querySelector(".bubble").textContent = text;
  scrollDown();
}
function clearDraft(){ if (session && session.draftEl){ session.draftEl.remove(); session.draftEl = null; } }

function failPending(message, extra = ""){
  const s = session;
  const el = s.pendingEl || addMsg("iv", "", true);
  // Backend messages that already say what to do ("Answer again to reconnect") get no second hint.
  const next = extra ? `<div class="btn-row">${extra}</div>`
    : /again/i.test(message) ? ""
    : `<br><span class="muted">${s.voiceOn ? "Say or type your answer again to retry." : "Send your answer again to retry."}</span>`;
  el.querySelector(".bubble").innerHTML = `<span class="err">${esc(message)}</span>${next}`;
  s.pendingEl = null; s.live = "";
}

/* Editor code, plus the notes (unless still the template) and a text reading of the drawing. */
function currentWorkspace(s = session){
  const notes = $("#notes").value.trim() === notesTemplate(s.key).trim() ? "" : $("#notes").value;
  const board = BOARD ? BOARD.text() : "";
  return {code: ED.get(), notes: [notes, board && `Diagram (read from the drawing):\n${board}`].filter(Boolean).join("\n\n")};
}

async function sendAnswer({text, force = false, fromVoice = false} = {}){
  const s = session;
  if (!s) return;
  if (s.roundId === null){ if (!fromVoice) toast("The interviewer is still joining. Your answer is kept in the box."); return; }
  if (s.busy){
    if (fromVoice){ clearDraft(); toBox(text, "We caught this while the interviewer was replying. Press Enter to send."); }
    else toast("The interviewer is still replying. Your answer stays in the box.");
    return;
  }
  const typed = text == null;
  if (typed) text = $("#answer").value;
  s.busy = true; s.voiceMs = null;
  try{
    const turn = await api.invoke("send_answer", {roundId: s.roundId, text, workspace: currentWorkspace(), force});
    if (session !== s) return;
    if (!turn){
      s.busy = false; clearDraft();
      if (force) toast("Nothing new in the workspace to share.");
      return;
    }
    if (typed){ $("#answer").value = ""; s.fromHeld = false; hint(""); }
    if (typed && s.voiceOn) api.invoke("voice_cancel_listen").catch(() => {});
    clearDraft();
    addMsg("me", turn.text || "(shared workspace)", false, false, turn.shared_workspace);
    s.pendingEl = addMsg("iv", "", true);
    setPhase("thinking");
  }catch(e){
    if (session !== s) return;
    s.busy = false;
    const m = errText(e), talking = /still talking/i.test(m);
    // The answer is never lost: typed text stays in the box; voice text is moved there.
    if (fromVoice){
      clearDraft();
      toBox(text, talking ? "The interviewer was still talking. Your words are in the box. Press Enter to send." : `Couldn’t send that. ${m} Your words are in the box. Press Enter to try again.`);
      if (s.phase !== "speaking") setPhase(restPhase());
    } else if (talking) toast("The interviewer is still talking. Your answer stays in the box. Send again when they finish.");
    else { toast(m, "err"); hint("Not sent. Your answer is still in the box. Press Enter to try again."); }
  }
}
$("#sendBtn").onclick = () => sendAnswer();
$("#sendWork").onclick = $("#sendWork2").onclick = $("#sendWork3").onclick = () => sendAnswer({force: true});
$("#answer").addEventListener("keydown", e => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing){ e.preventDefault(); sendAnswer(); }
});
// The answer box sizes itself to its text (CSS field-sizing), up to 160px.
$("#answer").addEventListener("input", () => {
  const s = session, a = $("#answer");
  if (!a.value.trim()){ s && (s.fromHeld = false); hint(""); }
  // Typing means "I'll write this one": close the mic so it doesn't send speech under the typed answer.
  if (s && s.voiceOn && s.phase === "listening" && a.value.trim()){
    api.invoke("voice_cancel_listen").catch(() => {});
  }
});

$("#endBtn").onclick = () => endRound();

/* ---------------- coding problem + runner ---------------- */
const STATUS_TEXT = {accepted: "Accepted", wrong_answer: "Wrong answer", runtime_error: "Runtime error", time_limit: "Time limit exceeded", compile_error: "Compile error"};
function showProblem(p){
  const on = !!p;
  if (session) session.problem = p;
  // Coding rounds: problem on top of the conversation, editor on the right, no rail.
  $("#convoProb").hidden = $("#probSplit").hidden = !on;
  $(".room-body").classList.toggle("coding", on);
  if (on) $("#convoProb").scrollTop = 0;
  $("#runBtns").hidden = !(on && p.runnable);
  $("#runOut").hidden = true; $("#runOut").innerHTML = "";
  $("#edHint").hidden = on;
  if (!on){ $("#probBody").innerHTML = ""; return; }
  const ex = p.examples.map((e, i) => `<h3>Example ${i+1}</h3><pre class="ex"><b>Input:</b> ${esc(e.input)}\n<b>Output:</b> ${esc(JSON.stringify(e.output))}${e.explanation ? `\n<b>Explanation:</b> ${esc(e.explanation)}` : ""}</pre>`).join("");
  $("#probBody").innerHTML = `
    <h2>${esc(p.title)} <span class="diff d-${esc(p.difficulty.toLowerCase())}">${esc(p.difficulty)}</span></h2>
    <div class="prob-text">${md(p.statement)}</div>
    ${ex}
    <h3>Constraints</h3><ul class="cons">${p.constraints.map(c => `<li><code>${esc(c)}</code></li>`).join("")}</ul>
    <p class="muted prob-foot">${p.hidden} hidden tests${p.runnable ? " plus one full-size test run on Submit" : ""}. ${p.topics.length ? "Topics: " + esc(p.topics.join(", ")) : ""}</p>
    <p class="form-msg err" id="probRunNote"></p>`;
  setRunnable(p.runnable);
  if (p.starter && !ED.get().trim()) ED.set(p.starter);
  selectTab("editor");
}
function setRunnable(on){
  $("#runBtns").hidden = !on;
  const note = $("#probRunNote"); if (!note) return;
  note.hidden = on;
  note.textContent = on ? "" : `Run isn’t available for ${session.lang} yet. Switch the editor to Java, Python, C++ or JavaScript to run tests.`;
}
/* Drag (or arrow keys on) the bar between problem and conversation. The share is remembered. */
const PROB_SHARE = "onsite.probShare";
function setProbShare(f){
  f = Math.min(.8, Math.max(.2, f));
  $("#convoProb").style.flexBasis = `${f * 100}%`;
  $("#transcript").style.flexBasis = `${(1 - f) * 100}%`;
  return f;
}
setProbShare(LS.get(PROB_SHARE, .5));
$("#probSplit").addEventListener("pointerdown", e => {
  const bar = e.currentTarget, top = $("#convoProb").getBoundingClientRect().top;
  const span = $("#convoProb").offsetHeight + $("#transcript").offsetHeight;
  bar.setPointerCapture(e.pointerId); bar.classList.add("dragging");
  const move = ev => setProbShare((ev.clientY - top) / span);
  bar.addEventListener("pointermove", move);
  bar.addEventListener("pointerup", ev => {
    bar.removeEventListener("pointermove", move); bar.classList.remove("dragging");
    LS.set(PROB_SHARE, setProbShare((ev.clientY - top) / span));
  }, {once: true});
});
$("#probSplit").addEventListener("keydown", e => {
  if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return;
  e.preventDefault();
  const cur = LS.get(PROB_SHARE, .5);
  LS.set(PROB_SHARE, setProbShare(cur + (e.key === "ArrowDown" ? .05 : -.05)));
});
async function runCode(submit){
  const s = session; if (!s || s.roundId === null || s.running) return;
  s.running = true;
  const btns = [$("#runBtn"), $("#submitBtn")]; btns.forEach(b => b.disabled = true);
  const out = $("#runOut"); out.hidden = false;
  out.innerHTML = `<div class="run-head"><span class="muted">${submit ? "Submitting… compiling and running every test." : "Running the examples…"}</span></div>`;
  try{
    const r = await api.invoke("run_code", {roundId: s.roundId, code: ED.get(), submit});
    if (session === s) out.innerHTML = runHtml(r, submit);
  }catch(e){
    if (session === s) out.innerHTML = `<div class="run-head bad">${esc(errText(e))}</div>`;
  }finally{
    s.running = false; btns.forEach(b => b.disabled = false);
  }
}
function runHtml(r, submit){
  const ok = r.status === "accepted";
  const head = `<div class="run-head ${ok ? "ok" : "bad"}"><b>${STATUS_TEXT[r.status] || r.status}</b>
    ${r.status === "compile_error" ? "" : `<span>${r.passed} / ${r.total} passed${ok ? ` · slowest ${Math.round(r.max_ms)} ms` : ""}</span>`}
    <span class="muted">${submit ? "Submit" : "Run"} · the interviewer sees this with your next message</span></div>`;
  if (r.status === "compile_error") return head + `<pre class="run-err">${esc(r.compile_error || "")}</pre>`;
  // Examples always; hidden and large tests only when they fail (as on LeetCode).
  const shown = r.cases.filter(c => !c.skipped && (c.kind === "example" || !c.passed));
  const label = c => c.kind === "example" ? "Example" : c.kind === "stress" ? "Large test" : "Hidden test";
  const firstBad = shown.findIndex(c => !c.passed);
  return head + shown.map((c, i) => `
    <details class="case ${c.passed ? "pass" : "fail"}" ${i === firstBad ? "open" : ""}>
      <summary><i class="dot"></i>${label(c)} ${c.kind === "example" ? r.cases.indexOf(c) + 1 : ""}${c.ms != null ? `<span class="muted">${c.ms < 1 ? "<1" : Math.round(c.ms)} ms</span>` : ""}</summary>
      <dl><dt>Input</dt><dd><code>${esc(c.input)}</code></dd>
      <dt>Expected</dt><dd><code>${esc(JSON.stringify(c.expected))}</code></dd>
      ${c.error ? `<dt>Error</dt><dd><pre class="run-err">${esc(c.error)}</pre></dd>` : `<dt>Output</dt><dd><code>${esc(JSON.stringify(c.got))}</code></dd>`}
      ${c.stdout ? `<dt>Stdout</dt><dd><pre>${esc(c.stdout)}</pre></dd>` : ""}</dl>
    </details>`).join("");
}
$("#runBtn").onclick = () => runCode(false);
$("#submitBtn").onclick = () => runCode(true);

/* Terminate: typed confirmation, then stop with no feedback and delete the round. */
const TERM_WORD = "terminate";
$("#termBtn").onclick = () => {
  if (!session) return;
  $("#termInput").value = ""; $("#termGo").disabled = true; $("#termDlg").returnValue = "";
  $("#termDlg").showModal(); $("#termInput").focus();
};
$("#termInput").oninput = e => $("#termGo").disabled = e.target.value.trim().toLowerCase() !== TERM_WORD;
$("#termDlg").addEventListener("close", async () => {
  if ($("#termDlg").returnValue !== "go" || $("#termInput").value.trim().toLowerCase() !== TERM_WORD) return;
  const s = session; if (!s) return;
  await leaveRound();
  if (s.roundId !== null){
    await api.invoke("delete_round", {roundId: s.roundId}).catch(e => toast(errText(e), "err"));
  }
  toast("Interview terminated.");
  nav("interviews");
});
$("#wrapReport").onclick = () => endRound();
async function endRound(){
  const s = session;
  if (!s) return;
  if (!s.closed && !await ask("End this round?", "You can get feedback afterwards.", "End round")) return;
  if (session !== s) return;
  const pending = leaveRound();
  if (s.roundId === null){ nav("interviews"); return; }
  nav("report");
  $("#repTitle").textContent = s.name || roundName(s.key); $("#repSub").textContent = `With ${s.persona}`;
  $("#repBody").innerHTML = `<div class="panel"><h2>Saving the round…</h2></div>`;
  if (await pending === false){
    toast("You didn’t answer anything, so the round wasn’t saved.");
    nav("interviews"); return;
  }
  openReport(s.roundId, true);
}

/* ---------------- rendering ---------------- */
function scrollDown(){ const t = $("#transcript"); if (t.scrollHeight - t.scrollTop - t.clientHeight < 200) t.scrollTop = t.scrollHeight; }
function addMsg(who, text, pending = false, draft = false, sharedWs = false){
  const el = document.createElement("div");
  const me = who === "me";
  el.className = "msg " + (me ? "me" : "iv") + (draft ? " draft" : "");
  const av = me ? "Y" : session.persona[0];
  const from = draft ? `You<span class="draft-tag">hearing…</span>` : me ? "You" : esc(session.persona);
  el.innerHTML = `<span class="av" aria-hidden="true">${esc(av)}</span><div class="body"><div class="from">${from}<time>${clock()}</time></div><div class="bubble">${pending ? '<span class="thinking" aria-label="Thinking"><i></i><i></i><i></i></span>' : md(text)}</div>${sharedWs ? `<span class="chip-note">${icon("code")}Shared workspace</span>` : ""}</div>`;
  $("#transcript").appendChild(el);
  $("#transcript").scrollTop = $("#transcript").scrollHeight;
  return el;
}
function renderIvMsg(el, text){
  el.querySelector(".bubble").innerHTML = md(text.replace(/\[\[END\]\]/g, ""));
  el.querySelectorAll("[data-open]").forEach(b => b.onclick = () => selectTab("shared"));
  scrollDown();
}
function inlineMd(s){ return esc(s).replace(/`([^`]+)`/g, "<code>$1</code>").replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>"); }
function md(text){
  const out = []; const re = new RegExp(F+"([\\w+#-]*)\\n?([\\s\\S]*?)(?:"+F+"|$)", "g");
  let last = 0, m;
  while ((m = re.exec(text))){
    if (m.index > last) out.push(mdBlock(text.slice(last, m.index)));
    out.push(`<div class="codeblock"><div class="cb-head"><span>${esc(m[1]||"code")}</span><button class="btn ghost sm" data-open>Open in workspace</button></div><pre><code>${esc(m[2])}</code></pre></div>`);
    last = re.lastIndex;
    if (m[0].length === 0) re.lastIndex++;
  }
  if (last < text.length) out.push(mdBlock(text.slice(last)));
  return out.join("");
}
function mdBlock(s){
  return s.trim().split(/\n{2,}/).map(par => {
    const lines = par.split("\n");
    if (lines.every(l => /^\s*[-*] /.test(l))) return "<ul>" + lines.map(l => `<li>${inlineMd(l.replace(/^\s*[-*] /,""))}</li>`).join("") + "</ul>";
    if (lines.every(l => /^\s*\d+[.)] /.test(l))) return "<ol>" + lines.map(l => `<li>${inlineMd(l.replace(/^\s*\d+[.)] /,""))}</li>`).join("") + "</ol>";
    return par.trim() ? `<p>${lines.map(inlineMd).join("<br>")}</p>` : "";
  }).join("");
}
function renderShared(){
  const pick = $("#sharedPick");
  if (!session || !session.shared.length){ pick.innerHTML = `<option>Nothing shared yet</option>`; pick.disabled = true; $("#sharedCode").innerHTML = `<div class="empty">Code the interviewer shares lands here, with line numbers you can refer to.</div>`; return; }
  pick.disabled = false;
  pick.innerHTML = session.shared.map((s,i) => `<option value="${i}">Snippet ${i+1} · ${esc(s.lang)} · ${s.code.split("\n").length} lines</option>`).join("");
  pick.value = String(session.shared.length - 1);
  showSnippet(session.shared.length - 1);
}
function showSnippet(i){
  const s = session && session.shared[i]; if (!s) return;
  $("#sharedCode").innerHTML = s.code.split("\n").map((l,n) => `<span class="ln">${n+1}</span>${esc(l)}`).join("\n");
}
$("#sharedPick").onchange = e => showSnippet(Number(e.target.value));
/* Rounds that are drawn (architecture, classes) open on the whiteboard; the catalog's "notes" tab
   means the text notes for talk-only rounds. Code shown by the interviewer opens the editor first. */
const BOARD_ROUNDS = new Set(["project", "lld", "sysdesign"]);
function startTab(k){
  const tab = CAT.ROUNDS[k].tab;
  return tab === "shared" ? "editor" : tab === "notes" && BOARD_ROUNDS.has(k) ? "board" : tab;
}
function selectTab(name){
  $$(".tab").forEach(t => { const on = t.dataset.tab === name; t.setAttribute("aria-selected", on); t.tabIndex = on ? 0 : -1; });
  $$(".pane").forEach(p => p.hidden = p.dataset.pane !== name);
  if (name === "shared") $("#sharedDot").hidden = true;
  if (name !== "board") setBoardMax(false);
  if (name === "board") loadBoard().then(b => b.refresh(), () => {});
}
/* Maximise: the whiteboard fills the window. The call keeps running; voice works as usual. */
function setBoardMax(on){
  const pane = $('[data-pane="board"]'), b = $("#boardMax");
  if (pane.classList.contains("maxed") === on) return;
  pane.classList.toggle("maxed", on);
  b.setAttribute("aria-pressed", String(on));
  b.title = on ? "Back to the call" : "Fill the window";
  b.querySelector("use").setAttribute("href", on ? "#i-shrink" : "#i-expand");
  b.querySelector("span").textContent = on ? "Restore" : "Maximise";
  if (BOARD) BOARD.refresh();
}
$("#boardMax").onclick = () => setBoardMax(!$('[data-pane="board"]').classList.contains("maxed"));
$$(".tab").forEach(t => t.onclick = () => selectTab(t.dataset.tab));
$(".tabs").addEventListener("keydown", e => {
  if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
  const tabs = $$(".tab").filter(t => !t.hidden), i = tabs.indexOf(document.activeElement); if (i < 0) return;
  const n = tabs[(i + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length];
  selectTab(n.dataset.tab); n.focus(); e.preventDefault();
});

/* Code editor (CodeMirror, ui/vendor/editor.js). Each language keeps its own code, like LeetCode. */
const ED = OnsiteEditor.create($("#code"), {lang: "Java"});
$("#edLang").innerHTML = $("#lang").innerHTML;
$("#edLang").onchange = async e => {
  const s = session, lang = e.target.value;
  if (!s || lang === s.lang) return;
  const prev = s.lang;
  s.buffers[prev] = ED.get();
  let info = {starter: "", runnable: false};
  try{ info = await api.invoke("set_round_lang", {roundId: s.roundId, lang}); }
  catch(err){ if (session === s) e.target.value = prev; toast(errText(err), "err"); return; }
  if (session !== s) return;
  s.lang = lang;
  ED.setLang(lang);
  ED.set(s.buffers[lang] ?? info.starter);
  const chip = $$("#ctxChips span").find(x => x.textContent === prev); if (chip) chip.textContent = lang;
  if (s.problem){ s.problem.runnable = info.runnable; setRunnable(info.runnable); $("#runOut").hidden = true; }
  ED.focus();
};

/* Whiteboard (Excalidraw, ui/vendor/whiteboard.js). About 2 MB, so it loads the first time it's opened. */
let BOARD = null, boardLoad = null;
function loadBoard(){
  return boardLoad ||= (async () => {
    window.EXCALIDRAW_ASSET_PATH = new URL("vendor/excalidraw/", location.href).href;
    const css = document.createElement("link");
    css.rel = "stylesheet"; css.href = "vendor/whiteboard/whiteboard.css";
    document.head.append(css);
    try{ await import("./vendor/whiteboard/whiteboard.js"); }
    catch(e){
      boardLoad = null;
      $("#board").innerHTML = `<p class="board-msg err">The whiteboard didn’t load. Use the Notes tab instead.</p>`;
      api.log("error", `whiteboard failed to load: ${errText(e)}`);
      throw e;
    }
    $("#board").innerHTML = "";
    return BOARD = OnsiteBoard.mount($("#board"));
  })();
}

/* The round context rail on the right: shown by default, hidden with the Context button. */
const RAIL = "onsite.rail";
function setRail(on){
  $(".room-body").classList.toggle("no-rail", !on);
  const b = $("#railBtn");
  b.setAttribute("aria-pressed", String(on));
  b.title = on ? "Hide round context" : "Show round context";
  if (BOARD && !$('[data-pane="board"]').hidden) BOARD.refresh();
}
setRail(LS.get(RAIL, true));
$("#railBtn").onclick = () => { const on = $(".room-body").classList.contains("no-rail"); setRail(on); LS.set(RAIL, on); };

/* ---------------- feedback + report ---------------- */
const decClass = d => /no hire|reject/i.test(d) ? "d-no" : /lean|borderline|more data/i.test(d) ? "d-lean" : "d-hire";
const scoreClass = n => n >= 70 ? "hi" : n >= 50 ? "mid" : "lo";
function rowHtml(x){
  return `<button class="row-item" data-round-id="${x.round_id}" data-loop-id="${x.loop_id}"><span class="row-ic">${esc((shortName(x.company) || x.name || "?")[0])}</span><span class="row-tx"><b>${esc(x.name)}</b><span>${esc(shortName(x.company))} · ${esc(new Date(x.started_at).toLocaleDateString())}${x.decision ? " · " + esc(x.decision) : ""}</span></span>${x.overall != null ? `<span class="score ${scoreClass(x.overall)}">${x.overall}</span>` : `<span class="pill">No feedback yet</span>`}</button>`;
}
function bindRows(sel){ $$(sel + " .row-item[data-round-id]").forEach(b => b.onclick = () => openInterview(Number(b.dataset.loopId), {focus: Number(b.dataset.roundId)})); }
/* One block per interview (loop), newest first, with its rounds in order. */
async function renderHistory(){
  let h = [];
  try{ h = await api.invoke("history"); }catch(e){ toast(errText(e), "err"); }
  const loops = [];
  h.forEach(x => { let g = loops.find(l => l.id === x.loop_id); if (!g) loops.push(g = {id: x.loop_id, rows: []}); g.rows.push(x); });
  $("#history").innerHTML = loops.length
    ? loops.map(g => {
        const rows = g.rows.slice().reverse();
        const sc = rows.filter(x => x.overall != null);
        const avg = sc.length ? Math.round(sc.reduce((a, x) => a + x.overall, 0) / sc.length) : null;
        return `<div class="loop-group">
          <button class="loop-head" data-loop="${g.id}"><span class="row-tx"><b>${esc(shortName(rows[0].company))} interview</b><span>${esc(new Date(rows[0].started_at).toLocaleDateString())} · ${rows.length} round${rows.length === 1 ? "" : "s"}</span></span>${avg != null ? `<span class="score ${scoreClass(avg)}" title="Average score">${avg}</span>` : ""}<span class="loop-open">Interview feedback${icon("arrow")}</span></button>
          ${rows.map(x => `<div class="row-wrap">${rowHtml(x)}<button class="btn ghost danger sm row-del" data-del="${x.round_id}" aria-label="Delete ${esc(x.name)}" title="Delete">${icon("trash")}</button></div>`).join("")}
        </div>`;
      }).join("")
    : `<div class="none">Your finished rounds and their feedback appear here.</div>`;
  bindRows("#history");
  $$("#history .loop-head").forEach(b => b.onclick = () => openInterview(Number(b.dataset.loop)));
  $$("#history .row-del").forEach(b => b.onclick = async () => { if (await deleteRound(Number(b.dataset.del))) renderHistory(); });
}
/* Deletes a round with its transcript and feedback. True when it was deleted. */
async function deleteRound(roundId){
  if (!await ask("Delete this round?", "Its transcript and feedback go too. This can’t be undone.", "Delete", true)) return false;
  try{ await api.invoke("delete_round", {roundId}); toast("Deleted."); return true; }
  catch(e){ toast(errText(e), "err"); return false; }
}

/* Same bar as Rust's generate_report: at least 2 real answers and 40 words. */
const MIN_ANSWERS = 2, MIN_WORDS = 40;
function answerStats(round){
  const mine = (round.transcript || []).filter(t => t.who === "me" && String(t.text || "").trim());
  return {answers: mine.length, words: mine.reduce((a, t) => a + words(t.text), 0)};
}
const NOT_ENOUGH = /^not enough to review/i;

let reportSeq = 0;
async function openReport(roundId, live){
  const seq = ++reportSeq;
  const stale = () => seq !== reportSeq || $("#view-report").hidden;
  nav("report");
  const del = $("#repDelete");
  del.hidden = !!live;
  del.onclick = async () => { if (await deleteRound(roundId)) nav("feedback"); };
  $("#repBody").innerHTML = `<div class="stack"><div class="skeleton" style="height:120px"></div></div>`;
  let round;
  try{ round = await api.invoke("get_round", {roundId}); }
  catch(e){ if (!stale()) renderReportError(errText(e), () => openReport(roundId, live)); return; }
  if (stale()) return;
  $("#repTitle").textContent = round.name;
  $("#repSub").textContent = `With ${round.persona} · ${new Date(round.started_at).toLocaleString()}`;
  let rep = await api.invoke("get_report", {roundId}).catch(() => null);
  if (stale()) return;
  if (!rep){
    const st = answerStats(round);
    if (st.answers < MIN_ANSWERS || st.words < MIN_WORDS){ renderNothing(round, live, st); return; }
    renderUnreviewed(round, live, st, seq);
    return;
  }
  renderReport(round, rep, live);
}
/* Feedback is written only when asked for: it costs a model call and the round may have been practice. */
function renderUnreviewed(round, live, st, seq){
  const {inLoop, hasNext} = loopButtons(round, live);
  $("#repBody").innerHTML = `
    <div class="empty-state panel">
      <span class="empty-ic">${icon("check")}</span>
      <h2>Round saved</h2>
      <p class="muted">You gave ${st.answers} answers, ${st.words} words in all. ${hasNext ? "Go on to the next round, or stop here and get feedback on every round so far in one go." : "Get feedback on the whole interview: every round plus the committee decision, on one page."}</p>
      <div class="btn-row">
        ${hasNext ? `<button class="btn primary" id="nextRound">Next: ${esc(loopRound(round.idx+1).name)}${icon("arrow")}</button>` : ""}
        <button class="btn ${hasNext ? "" : "primary"}" id="loopFb">${icon("sparkle")}${hasNext ? "Finish and get interview feedback" : "Get interview feedback"}</button>
        ${inLoop ? `<button class="btn ghost" id="retake">${icon("refresh")}Retake this round</button>` : ""}
        <button class="btn ghost" id="getFeedback">Feedback on this round only</button>
      </div>
    </div>`;
  bindStart("#retake", round.idx);
  bindStart("#nextRound", round.idx + 1);
  $("#loopFb").onclick = () => openInterview(round.loop_id, {generate: true});
  $("#getFeedback").onclick = () => writeFeedback(round, live, st, seq);
}
async function writeFeedback(round, live, st, seq){
  const stale = () => seq !== reportSeq || $("#view-report").hidden;
  $("#repBody").innerHTML = `<div class="stack"><div class="panel" role="status"><h2>Writing your feedback…</h2><p class="muted" style="margin-top:6px">The panel is reviewing the transcript. This takes 20–90 seconds.</p></div><div class="skeleton" style="height:120px"></div><div class="skeleton" style="height:220px"></div></div>`;
  let rep;
  try{ rep = await api.invoke("generate_report", {roundId: round.id}); }
  catch(e){
    if (stale()) return;
    if (NOT_ENOUGH.test(errText(e))) renderNothing(round, live, st, errText(e));
    else renderReportError(errText(e), () => writeFeedback(round, live, st, seq));
    return;
  }
  if (!stale()) renderReport(round, rep, live);
}
function renderReportError(message, retry){
  $("#repBody").innerHTML = `<div class="panel"><h2>Feedback didn’t come through</h2><p class="form-msg err" role="alert">${esc(message)}</p><div class="btn-row"><button class="btn primary" id="repRetry">${icon("refresh")}Try again</button><button class="btn ghost" data-nav="feedback">Back to feedback</button></div></div>`;
  $("#repRetry").onclick = retry;
}
function loopButtons(round, live){
  const inLoop = !!(live && loop && loop.id === round.loop_id);
  return {inLoop, hasNext: inLoop && round.idx < loop.count - 1, isLast: inLoop && round.idx === loop.count - 1 && loop.count > 1};
}
function bindStart(sel, idx){
  const b = $(sel); if (!b) return;
  b.onclick = () => {
    if (readyEvery()) return openReady(idx);
    b.disabled = true; b.classList.add("is-busy"); startRound(idx);
  };
}
function renderNothing(round, live, st, detail){
  const {inLoop, hasNext} = loopButtons(round, live);
  const short = st.answers < MIN_ANSWERS || st.words < MIN_WORDS;
  const said = st.answers === 0 ? "You didn’t answer any questions" : `You gave ${st.answers} answer${st.answers === 1 ? "" : "s"}, ${st.words} word${st.words === 1 ? "" : "s"} in all`;
  $("#repBody").innerHTML = `
    <div class="empty-state panel" role="status">
      <span class="empty-ic">${icon("chat")}</span>
      <h2>Nothing to review yet</h2>
      <p class="muted">${short
        ? `${esc(said)}. Feedback needs at least ${MIN_ANSWERS} real answers, about ${MIN_WORDS} words, so the panel has something to judge.`
        : esc(detail || "The panel needs more of your answers to judge.")} No feedback was generated.</p>
      <div class="btn-row">
        ${inLoop ? `<button class="btn primary" id="retake">${icon("refresh")}Retake this round</button>` : ""}
        ${hasNext ? `<button class="btn" id="nextRound">Skip to ${esc(loopRound(round.idx+1).name)}${icon("arrow")}</button>` : ""}
        ${inLoop && round.idx > 0 ? `<button class="btn" id="loopFb">${icon("sparkle")}Get interview feedback</button>` : ""}
        <button class="btn ${inLoop ? "ghost" : "primary"}" data-nav="${inLoop ? "interviews" : "feedback"}">${icon("back")}Back</button>
      </div>
    </div>`;
  bindStart("#retake", round.idx);
  bindStart("#nextRound", round.idx + 1);
  const lb = $("#loopFb"); if (lb) lb.onclick = () => openInterview(round.loop_id, {generate: true});
}

function dimHtml(d){
  const sc = d.score == null || d.score === "" || !Number.isFinite(Number(d.score)) ? null : Math.max(0, Math.min(4, Number(d.score)));
  const ev = String(d.evidence || "");
  if (sc === null){
    return `<div class="dim na"><div class="dim-top">${esc(d.name)}<span>Not assessed</span></div><div class="bar na" aria-hidden="true"></div><p>${esc(ev.replace(/^not assessed:\s*/i, ""))}</p></div>`;
  }
  return `<div class="dim"><div class="dim-top">${esc(d.name)}<span>${sc} / 4</span></div><div class="bar" role="img" aria-label="${sc} out of 4"><i style="width:${sc*25}%"></i></div><p>${esc(ev)}</p></div>`;
}

/* One round's feedback. `actions` goes under the headline. The transcript toggle uses data
   attributes, so several rounds can sit on one page. */
function reportHtml(round, r, actions = ""){
  const has = r.overall != null && Number.isFinite(Number(r.overall));
  const n = has ? Number(r.overall) : 0;
  const ringC = !has ? "var(--surface-3)" : n >= 70 ? "var(--ok)" : n >= 50 ? "var(--warn)" : "var(--bad)";
  const dims = (r.dimensions||[]).map(dimHtml).join("");
  const li = a => (a||[]).map(x => `<li>${esc(x)}</li>`).join("");
  const moments = (r.moments||[]).map(m => `<div class="moment"><div class="q">${esc(m.question)}</div><div class="said">You said: ${esc(m.said)}</div><div class="better"><b>Stronger answer:</b> ${esc(m.better)}</div></div>`).join("");
  const met = r.metrics || {};
  const section = (title, body) => body ? `<div class="panel"><h2>${title}</h2>${body}</div>` : "";
  return `
    <div class="stack">
      <div class="panel rep-head">
        <div class="ring" style="--v:${n}; --ring-c:${ringC}" role="img" aria-label="${has ? `Score ${n} of 100` : "No score"}"><div><div><b>${has ? n : "—"}</b><small>of 100</small></div></div></div>
        <div>
          <span class="decision ${decClass(r.decision||"")}">${esc(r.decision||"—")}</span><span class="muted">${esc(r.level_signal||"")}</span>
          <h2>${esc(r.headline||"")}</h2>
          <div class="btn-row">
            ${actions}
            <button class="btn ghost" data-show-tx="${round.id}" aria-expanded="false">Show transcript</button>
          </div>
        </div>
      </div>
      ${r.fix_first ? `<div class="fix"><span class="fix-ic">${icon("bulb")}</span><div><b>Fix this first</b>${esc(r.fix_first)}</div></div>` : ""}
      ${section("Rubric", dims && `<div class="dims">${dims}</div>`)}
      <div class="cols">
        <div class="panel"><h2>What went well</h2><ul class="list">${li(r.went_well) || "<li class='muted'>Nothing stood out yet.</li>"}</ul></div>
        <div class="panel"><h2>What held you back</h2><ul class="list">${li(r.held_back) || "<li class='muted'>Nothing major.</li>"}</ul></div>
      </div>
      ${section("Key moments", moments)}
      <div class="cols">
        <div class="panel"><h2>Practise next</h2><ul class="list">${li(r.practice_next)}</ul></div>
        <div class="panel"><h2>How you answered</h2><div class="metrics">
          <div class="metric"><b>${met.avg_words ?? "—"}</b><span>words per answer</span></div>
          <div class="metric"><b>${met.answers ?? "—"}</b><span>answers</span></div>
          <div class="metric"><b>${met.minutes ?? "—"}</b><span>minutes</span></div>
          <div class="metric"><b>${met.code_shares ?? 0}</b><span>workspace shares</span></div>
        </div></div>
      </div>
      <div class="panel" data-tx="${round.id}" hidden><h2>Transcript</h2>${transcriptHtml(round)}</div>
    </div>`;
}
function transcriptHtml(round){
  return `<div class="transcript" style="padding:0; overflow:visible">${(round.transcript||[]).map(m => `<div class="msg ${m.who==="me"?"me":"iv"}"><span class="av" aria-hidden="true">${m.who==="me" ? "Y" : esc(round.persona[0])}</span><div class="body"><div class="from">${m.who==="me" ? "You" : esc(round.persona)}</div><div class="bubble">${md(String(m.text||"(shared workspace)").replace(/\[\[END\]\]/g, ""))}</div></div></div>`).join("")}</div>`;
}
function bindTranscripts(root){
  root.querySelectorAll("[data-show-tx]").forEach(b => b.onclick = () => {
    const p = root.querySelector(`[data-tx="${b.dataset.showTx}"]`);
    p.hidden = !p.hidden; b.textContent = p.hidden ? "Show transcript" : "Hide transcript"; b.setAttribute("aria-expanded", String(!p.hidden));
  });
}
function renderReport(round, r, live){
  const {inLoop, hasNext} = loopButtons(round, live);
  const actions = `
    ${hasNext ? `<button class="btn primary" id="nextRound">Next round: ${esc(loopRound(round.idx+1).name)}${icon("arrow")}</button>` : ""}
    ${inLoop && loop.count > 1 ? `<button class="btn ${hasNext ? "" : "primary"}" id="loopFb">${icon("scale")}${hasNext ? "Finish and get interview feedback" : "Get interview feedback"}</button>` : ""}
    ${inLoop ? `<button class="btn" id="retake">${icon("refresh")}Retake this round</button>` : ""}`;
  $("#repBody").innerHTML = reportHtml(round, r, actions);
  bindTranscripts($("#repBody"));
  bindStart("#nextRound", round.idx + 1);
  bindStart("#retake", round.idx);
  const lb = $("#loopFb"); if (lb) lb.onclick = () => openInterview(round.loop_id, {generate: true});
}

/* ---------------- interview feedback (the whole loop on one page) ---------------- */
let interviewSeq = 0, loopFbUnlisten = null, progressLoop = null;
async function openInterview(loopId, {generate = false, focus = null} = {}){
  const seq = ++interviewSeq; reportSeq++;
  const stale = () => seq !== interviewSeq || $("#view-report").hidden;
  nav("report");
  $("#repDelete").hidden = true;
  $("#repTitle").textContent = "Interview feedback"; $("#repSub").textContent = "";
  $("#repBody").innerHTML = `<div class="stack"><div class="skeleton" style="height:120px"></div></div>`;
  let fb;
  try{ fb = await api.invoke("get_loop_feedback", {loopId}); }
  catch(e){ if (!stale()) renderReportError(errText(e), () => openInterview(loopId, {generate, focus})); return; }
  if (stale()) return;
  if (generate && fb.rounds.some(needsReview)){
    renderInterviewProgress(fb, {done: 0, total: fb.rounds.filter(needsReview).length, stage: "rounds"});
    progressLoop = loopId;
    if (!loopFbUnlisten) loopFbUnlisten = await api.listen("loop_feedback", ev => {
      if (ev.loop_id === progressLoop) renderInterviewProgress(null, ev);
    });
    try{ fb = await api.invoke("loop_feedback", {loopId}); }
    catch(e){ if (!stale()) renderReportError(errText(e), () => openInterview(loopId, {generate: true, focus})); return; }
    if (stale()) return;
  } else if (generate && !fb.verdict && fb.rounds.filter(r => r.report).length >= 2){
    renderInterviewProgress(fb, {done: 0, total: 0, stage: "verdict"});
    try{ fb = await api.invoke("loop_feedback", {loopId}); }catch(e){ toast(errText(e), "err"); }
    if (stale()) return;
  }
  renderInterview(fb, focus);
}
const needsReview = r => !r.report && !NOT_ENOUGH.test(r.missing || "");
function renderInterviewProgress(fb, ev){
  if (fb){
    $("#repBody").innerHTML = `<div class="stack"><div class="panel" role="status" id="ifProgress"><h2 id="ifStage"></h2><p class="muted" style="margin-top:6px">Rounds are reviewed in parallel. This takes one to three minutes. You can leave this page; feedback is saved as it’s written.</p><div class="meter ac-meter" aria-hidden="true"><i id="ifBar"></i></div></div><div class="skeleton" style="height:220px"></div></div>`;
  }
  const st = $("#ifStage"); if (!st) return;
  st.textContent = ev.stage === "verdict" ? "The hiring committee is deciding…" : `Reviewing your rounds… ${ev.done} of ${ev.total} done`;
  const pct = ev.stage === "verdict" ? 92 : ev.total ? Math.max(8, ev.done / ev.total * 85) : 8;
  $("#ifBar").style.width = `${pct}%`;
}
function renderInterview(fb, focus){
  const rounds = fb.rounds;
  if (!rounds.length){ nav("feedback"); return; }
  const first = rounds[0] && rounds[0].round;
  $("#repSub").textContent = [shortName(fb.company), fb.level, first ? new Date(first.started_at).toLocaleDateString() : "", `${rounds.length} round${rounds.length === 1 ? "" : "s"}`].filter(Boolean).join(" · ");
  const scored = rounds.filter(r => r.report && Number.isFinite(Number(r.report.overall)));
  const avg = scored.length ? Math.round(scored.reduce((a, r) => a + Number(r.report.overall), 0) / scored.length) : null;
  const pending = rounds.filter(needsReview);
  const v = fb.verdict;
  const verdict = v ? `<div class="panel"><div class="panel-head"><h2>Committee decision</h2><span class="decision ${decClass(v.decision||"")}">${esc(v.decision)}</span></div><p>${esc(v.rationale)}</p><div class="cols" style="margin-top:14px"><div><b>Strongest signal</b><p class="muted">${esc(v.strongest_signal)}</p></div><div><b>Biggest risk</b><p class="muted">${esc(v.biggest_risk)}</p></div></div><h3 style="margin:16px 0 8px">Two-week plan</h3><ul class="list">${(v.plan||[]).map(p => `<li>${esc(p)}</li>`).join("")}</ul></div>`
    : scored.length >= 2 ? "" : rounds.length > 1 ? `<div class="panel"><p class="muted">The committee decision needs feedback on at least two rounds.</p></div>` : "";
  const overview = `<div class="panel"><div class="panel-head"><h2>Rounds</h2>${avg != null ? `<span class="score ${scoreClass(avg)}" title="Average score">${avg}</span>` : ""}</div><div class="rows">${rounds.map((r, i) => `
      <a class="row-item" href="#rf-${r.round.id}"><span class="row-ic">${i + 1}</span><span class="row-tx"><b>${esc(r.round.name)}</b><span>With ${esc(r.round.persona)}${r.report && r.report.decision ? " · " + esc(r.report.decision) : ""}</span></span>${r.report && r.report.overall != null ? `<span class="score ${scoreClass(r.report.overall)}">${r.report.overall}</span>` : `<span class="pill">${NOT_ENOUGH.test(r.missing || "") ? "Too short to review" : "No feedback yet"}</span>`}</a>`).join("")}</div>
      ${pending.length ? `<div class="btn-row"><button class="btn primary" id="ifGenerate">${icon("sparkle")}Get feedback for ${pending.length === 1 ? "the missing round" : `all ${pending.length} rounds`}</button></div>` : ""}
    </div>`;
  const sections = rounds.map((r, i) => `
    <section class="rf" id="rf-${r.round.id}">
      <h2 class="rf-head"><span class="row-ic">${i + 1}</span>${esc(r.round.name)}<small class="muted">with ${esc(r.round.persona)}</small></h2>
      ${r.report ? reportHtml(r.round, r.report, `<button class="btn ghost" data-del-round="${r.round.id}">${icon("trash")}Delete round</button>`)
        : `<div class="panel"><p class="muted">${esc(r.missing || "No feedback yet.")}</p><div class="btn-row"><button class="btn ghost" data-show-tx="${r.round.id}" aria-expanded="false">Show transcript</button><button class="btn ghost" data-del-round="${r.round.id}">${icon("trash")}Delete round</button></div></div><div class="panel" data-tx="${r.round.id}" hidden><h2>Transcript</h2>${transcriptHtml(r.round)}</div>`}
    </section>`).join("");
  const live = loop && loop.id === fb.loop_id;
  // Retakes add rows with the same idx, so the next round follows the highest one played.
  const next = live ? Math.max(-1, ...rounds.map(r => r.round.idx)) + 1 : -1;
  const nextBtn = live && next < loop.count ? `<div class="btn-row"><button class="btn primary" id="ifNext">Next round: ${esc(loopRound(next).name)}${icon("arrow")}</button></div>` : "";
  $("#repBody").innerHTML = `<div class="stack">${verdict}${overview}${nextBtn}${sections}</div>`;
  bindTranscripts($("#repBody"));
  bindStart("#ifNext", next);
  const g = $("#ifGenerate"); if (g) g.onclick = () => openInterview(fb.loop_id, {generate: true});
  $$("#repBody [data-del-round]").forEach(b => b.onclick = async () => { if (await deleteRound(Number(b.dataset.delRound))) openInterview(fb.loop_id); });
  $$("#repBody a.row-item").forEach(a => a.onclick = e => { e.preventDefault(); const t = $(a.getAttribute("href")); if (t) t.scrollIntoView({behavior: calm.matches ? "auto" : "smooth"}); });
  if (focus){ const t = $(`#rf-${focus}`); if (t) t.scrollIntoView(); }
}

/* ---------------- resume check (ATS) ---------------- */
const MAX_FILE = 10 * 1024 * 1024;
let RESUME_FILE = LS.get("onsite.resumeFile", null);   // {name, data(base64), size} — the last resume file, for layout checks
let RESUME_CHECK = null;   // the check run when this loop started, shown on Get ready
/* The resume as both checks read it: the file when there is one (layout checks need it), else the text. */
function resumeArgs(jd){
  return RESUME_FILE ? {name: RESUME_FILE.name, data: RESUME_FILE.data, text: null, jd} : {name: null, data: null, text: $("#resume").value, jd};
}
function rememberResumeFile(file){
  RESUME_FILE = file;
  LS.set("onsite.resumeFile", file);
  if (!$("#view-resume").hidden) renderAtsInput();
}
function fileToB64(f){
  if (f.size > MAX_FILE) return Promise.reject("That file is over 10 MB. A resume should be far smaller.");
  return new Promise((ok, bad) => {
    const r = new FileReader();
    r.onload = () => ok({name: f.name, size: f.size, data: String(r.result).replace(/^data:[^,]*,/, "")});
    r.onerror = () => bad("Couldn’t read that file.");
    r.readAsDataURL(f);
  });
}
/* The parsed text shows inside the file box once that file has been checked; it replaces the old empty space. */
let ATS_LAST = null;
const SECTION_RE = /^(summary|profile|objective|experience|work experience|professional experience|employment|education|skills|technical skills|projects|certifications?|achievements|awards|publications|languages|interests|contact)\b/i;
function parsedHtml(text){
  return String(text || "").split("\n").map(l => {
    const t = l.trim();
    const head = t && t.length <= 40 && (SECTION_RE.test(t) || (t === t.toUpperCase() && /[A-Z]{3}/.test(t)));
    return head ? `<b class="pv-h">${esc(t)}</b>` : esc(l);
  }).join("\n");
}
function renderAtsInput(){
  const drop = $("#atsDrop"), msg = $("#atsMsg"), pv = $("#atsPreview");
  const pasted = words($("#resume").value);
  drop.classList.toggle("filled", !!RESUME_FILE || pasted > 0);
  msg.innerHTML = RESUME_FILE
    ? `${esc(RESUME_FILE.name)} · ${Math.max(1, Math.round(RESUME_FILE.size / 1024))} KB. <button type="button" class="linkbtn" data-choose>Choose another</button> · <button type="button" class="linkbtn" data-clear>Remove</button>`
    : pasted
      ? `Using the ${pasted}-word resume text from interview setup. Layout checks need the file: <button type="button" class="linkbtn" data-choose>choose the PDF or Word file</button>`
      : `Drop the exact PDF or Word file you send to employers, or <button type="button" class="linkbtn" data-choose>choose a file</button>`;
  const shown = ATS_LAST && ATS_LAST.preview && (!RESUME_FILE || ATS_LAST.file.name === RESUME_FILE.name);
  drop.classList.toggle("has-preview", !!shown);
  pv.hidden = !shown;
  if (shown){
    const cut = ATS_LAST.file.words > words(ATS_LAST.preview) + 5;
    pv.innerHTML = `<div class="pv-head"><b>What the ATS reads</b><span>in reading order${cut ? ", first 4,000 characters" : ""}</span></div><pre>${parsedHtml(ATS_LAST.preview)}</pre>`;
  }
}
function initAts(){
  const drop = $("#atsDrop"), input = $("#atsFile");
  const load = async f => {
    input.value = "";
    try{ rememberResumeFile(await fileToB64(f)); $("#atsStatus").textContent = ""; }
    catch(e){ toast(errText(e), "err"); }
  };
  drop.onclick = e => {
    if (e.target.closest("#atsPreview")) return;
    const b = e.target.closest("[data-clear]");
    if (b){ rememberResumeFile(null); return; }
    input.click();
  };
  input.onchange = () => input.files[0] && load(input.files[0]);
  drop.ondragover = e => { e.preventDefault(); drop.classList.add("over"); };
  drop.ondragleave = e => { if (!drop.contains(e.relatedTarget)) drop.classList.remove("over"); };
  drop.ondrop = e => { e.preventDefault(); drop.classList.remove("over"); const f = e.dataTransfer.files[0]; if (f) load(f); };
  $("#atsJd").addEventListener("input", () => LS.set("onsite.atsJd", $("#atsJd").value));
  $("#atsRun").onclick = runAts;
}
let atsShown = false;
async function openAts(){
  renderAtsInput();
  const jd = $("#atsJd");
  if (!jd.value.trim()) jd.value = LS.get("onsite.atsJd", "") || $("#jd").value;
  if (atsShown) return;
  try{
    const last = await api.invoke("ats_last");
    if (last && !atsShown){ atsShown = true; renderAts(last, true); }
  }catch(e){}
}
async function runAts(){
  const btn = $("#atsRun"), st = $("#atsStatus");
  const jd = $("#atsJd").value.trim();
  const text = $("#resume").value;
  if (!RESUME_FILE && !text.trim()){ st.className = "form-msg err"; st.textContent = "Add your resume file first."; return; }
  btn.disabled = true; btn.classList.add("is-busy"); btn.setAttribute("aria-busy", "true");
  st.className = "form-msg"; st.textContent = jd ? "Reading the file and the posting… (the posting takes Claude ~10 s the first time)" : "Reading the file…";
  try{
    const r = await api.invoke("ats_check", resumeArgs(jd));
    st.textContent = "";
    atsShown = true;
    renderAts(r, false);
  }catch(e){ st.className = "form-msg err"; st.textContent = errText(e); }
  finally{ btn.disabled = false; btn.classList.remove("is-busy"); btn.removeAttribute("aria-busy"); }
}
const ringColor = n => n >= 70 ? "var(--ok)" : n >= 50 ? "var(--warn)" : "var(--bad)";
function ring(n, label, has = true){
  return `<div class="ats-ring"><div class="ring" style="--v:${has ? n : 0}; --ring-c:${has ? ringColor(n) : "var(--surface-3)"}" role="img" aria-label="${esc(label)}: ${has ? `${n} of 100` : "not scored"}"><div><div><b>${has ? n : "—"}</b><small>of 100</small></div></div></div><span>${esc(label)}</span></div>`;
}
function renderAts(r, saved){
  ATS_LAST = r; renderAtsInput();
  const job = r.job, sub = r.matched.sub_scores;
  const passing = r.ats.filter(a => a.passes).length;
  const f = r.file;
  const facts = [f.kind_label, f.pages ? `${f.pages} page${f.pages > 1 ? "s" : ""}` : null, `${f.words} words`, r.experience_years != null ? `${r.experience_years} years of dated experience` : null].filter(Boolean).join(" · ");
  const headline = r.extractability === "none" ? "An ATS can't read any text in this file."
    : r.extractability === "partial" ? `Part of this file can't be read. Passes ${passing} of 6 simulated ATSs.`
    : `Passes ${passing} of 6 simulated ATSs${job ? ` for ${job.role || "this posting"}` : ""}.`;
  const when = saved ? ` · Checked ${new Date(r.checked_at).toLocaleString([], {dateStyle: "medium", timeStyle: "short"})}` : "";
  const [first, ...rest] = r.tips;
  const dot = s => s === "pass" ? "ok" : s === "warn" ? "warn" : "bad";
  const checks = r.checks.map(c => `<li class="${dot(c.status)}"><i class="dot"></i><div><b>${esc(c.title)}</b>${c.detail ? `<small>${esc(c.detail)}</small>` : ""}</div></li>`).join("");
  const ats = r.ats.map(a => `
    <div class="ats-card">
      <div class="ats-top"><div><b>${esc(a.name)}</b><small>${esc(a.vendor)}</small></div><span class="score ${scoreClass(a.score)}">${a.score}</span></div>
      <span class="pill ${a.passes ? "ok" : "bad"}">${a.passes ? "Likely through" : "Likely filtered"} · needs ${a.passing}</span>
      <p class="muted">${esc(a.note)}</p>
      ${a.issues.length ? `<ul class="list">${a.issues.map(i => `<li>${esc(i)}</li>`).join("")}</ul>` : ""}
    </div>`).join("");
  const chip = k => {
    const kind = k.exact ? "ok" : k.alias || k.stem ? "warn" : "bad";
    const how = k.exact ? "Found as written" : k.alias ? `Only as “${k.alias}”. Exact-match ATSs miss it.` : k.stem ? "Found in another word form" : "Not in your resume";
    return `<span class="kw ${kind}" title="${esc(how)}">${k.required ? "<i>must</i>" : ""}${esc(k.term)}</span>`;
  };
  const kws = r.keywords.length ? `
    <div class="panel"><h2>Keywords from the posting</h2>
      <div class="kws">${r.keywords.map(chip).join("")}</div>
      <p class="muted kw-key"><span class="kw ok">found</span><span class="kw warn">other spelling or form</span><span class="kw bad">missing</span> Hover a term for details.</p>
    </div>` : "";
  const dim = (name, v, note) => `<div class="dim${v == null ? " na" : ""}"><div class="dim-top">${esc(name)}<span>${v == null ? "not scored" : Math.round(v)}</span></div><div class="bar${v == null ? " na" : ""}"><i style="width:${v ?? 0}%"></i></div><p>${esc(note)}</p></div>`;
  const dims = [
    dim("Keyword match", sub.keyword_match, "Posting terms found word for word. 40% of the match score."),
    dim("Skills coverage", sub.skills_coverage, "Required and preferred skills found. 20%."),
    dim("Job title", sub.title_match, job && job.role ? `“${job.role}” under your name scores 100, in the summary 75. 15%.` : "Needs a posting with a title. 15%."),
    dim("Sections", sub.section_completeness, "Contact details, Summary, Experience, Education, Skills. 15%."),
    dim("Date format", sub.date_consistency, "Share of dates written the same way. 10%."),
  ].join("");
  $("#atsOut").innerHTML = `
    <div class="stack">
      <div class="panel ats-head">
        ${ring(r.parse_score, "Readable")}
        ${ring(r.matched.overall, job ? "Job match" : "Structure")}
        <div>
          <h2>${esc(headline)}</h2>
          <p class="muted">${esc(r.file.name)} · ${esc(facts)}${esc(when)}</p>
          <p class="muted small">Simulated from each product’s public behaviour. Real ATSs don’t publish their scoring, and many recruiters search rather than auto-reject. Scoring follows the open-source Resume-Matcher and ats-screener projects.</p>
        </div>
      </div>
      ${job && r.extractability !== "none" ? `<div id="atsAi"></div>` : ""}
      ${first ? `<div class="fix"><span class="fix-ic">${icon("bulb")}</span><div><b>Fix this first</b>${esc(first)}</div></div>` : ""}
      ${rest.length ? `<div class="panel"><h2>Then</h2><ul class="list">${rest.map(t => `<li>${esc(t)}</li>`).join("")}</ul></div>` : ""}
      <div class="ats-grid">${ats}</div>
      <div class="cols">
        <div class="panel"><h2>Can it be parsed?</h2><ul class="checks ats-checks">${checks}</ul></div>
        <div class="panel"><h2>${job ? "Match score" : "Structure score"}</h2><div class="dims">${dims}</div></div>
      </div>
      ${kws}
    </div>`;
  if (!job || r.extractability === "none") return;
  if (r.screen) renderAiScreen(r.screen);
  else if (!saved) runAiScreen();
  else renderAiScreen(null, "No AI screen for this check yet.");
}
let aiSeq = 0;
async function runAiScreen(){
  const seq = ++aiSeq;
  renderAiScreen(null);
  const jd = $("#atsJd").value.trim();
  const args = resumeArgs(jd);
  try{
    const s = await api.invoke("ats_screen", args);
    if (seq === aiSeq) renderAiScreen(s);
  }catch(e){ if (seq === aiSeq) renderAiScreen(null, errText(e)); }
}
function renderAiScreen(s, err){
  const el = $("#atsAi"); if (!el) return;
  if (!s){
    el.innerHTML = err
      ? `<div class="panel ai-card"><div class="panel-head"><h2>AI screen</h2></div><p class="form-msg err">${esc(err)}</p><div class="btn-row"><button class="btn" id="aiRetry">${icon("refresh")}Run AI screen</button></div></div>`
      : `<div class="panel ai-card"><div class="panel-head"><h2>AI screen</h2><span class="pill">Grading…</span></div><p class="muted">Claude is reading the extracted text the way HiredScore or Role Fit would. This takes 15–30 s.</p><div class="skeleton" style="height:64px; margin-top:12px"></div></div>`;
    const b = $("#aiRetry"); if (b) b.onclick = runAiScreen;
    return;
  }
  const gc = {A: "ok", B: "ok", C: "warn", D: "bad"}[s.grade] || "warn";
  const lvl = {below: "Below the level", at: "At the level", above: "Above the level"}[s.level] || "";
  const vc = {proven: "ok", claimed: "warn", missing: "bad"};
  const vt = {proven: "Shown in work", claimed: "Only listed", missing: "Missing"};
  const li = a => a.map(x => `<li>${esc(x)}</li>`).join("");
  el.innerHTML = `
    <div class="panel ai-card">
      <div class="ai-top">
        <div class="ai-grade ${gc}" role="img" aria-label="AI grade ${esc(s.grade)}">${esc(s.grade)}</div>
        <div>
          <div class="ai-meta"><b>AI screen</b><span class="pill ${s.advance ? "ok" : "bad"}">${s.advance ? "Advances" : "Screened out"}</span><span class="muted">Fit ${s.fit} of 100${lvl ? ` · ${lvl}` : ""}</span></div>
          <p class="ai-sum">${esc(s.summary)}</p>
          ${s.level_note ? `<p class="muted">${esc(s.level_note)}</p>` : ""}
        </div>
      </div>
      ${s.recruiter_take ? `<p class="ai-take"><b>A recruiter’s 7-second read:</b> ${esc(s.recruiter_take)}</p>` : ""}
      ${s.must_haves.length ? `<h3>Must-haves</h3><ul class="ai-must">${s.must_haves.map(m => `<li><span class="kw ${vc[m.verdict]}">${vt[m.verdict]}</span><div><b>${esc(m.skill)}</b><small>${esc(m.evidence)}</small></div></li>`).join("")}</ul>` : ""}
      <div class="cols">
        <div><h3>Strengths</h3><ul class="list">${li(s.strengths) || "<li class='muted'>None noted.</li>"}</ul></div>
        <div><h3>Red flags</h3><ul class="list">${li(s.red_flags) || "<li class='muted'>None noted.</li>"}</ul></div>
      </div>
      <p class="muted small">Graded by Claude${s.model ? ` (${esc(s.model)})` : ""} from the text as extracted. Real AI screeners are trained on hiring data and differ by company.</p>
    </div>`;
}

/* ---------------- dossiers and research ---------------- */
const RESEARCH_STEPS = ["Reading your request", "Finding the current process", "Collecting candidate reports",
  "Pulling out each round’s questions", "Values and interviewer style", "Building the dossier"];
let JOBS = [];           // research jobs, newest first (Rust JobView)
let dsOpen = null;       // dossier shown in detail
async function renderDossiersView(){
  await loadDossiers();
  try{ JOBS = await api.invoke("research_jobs"); }catch(e){ JOBS = []; }
  renderDossierList(); renderDossierDetail(); renderJobs();
}
function renderDossierList(){
  const el = $("#dossierList");
  if (!DOSSIERS.length){ el.innerHTML = `<p class="muted small">No dossiers yet. Research a company, or import a dossier file a friend shared.</p>`; return; }
  el.innerHTML = DOSSIERS.map(d => `<button class="row-item" data-ds="${esc(d.id)}" aria-pressed="${d.id === dsOpen}">
    <span class="row-ic">${esc((d.company || "?")[0])}</span>
    <span class="row-tx"><b>${esc(d.company)} — ${esc(d.role)}</b><span>${esc([CAT.LEVELS[d.level] || d.level, `${d.process.length} rounds`, `${(d.sources || []).length} sources`, String(d.preparedAt || "").slice(0, 10)].filter(Boolean).join(" · "))}</span></span>
    ${icon("arrow")}</button>`).join("");
}
$("#dossierList").onclick = e => {
  const b = e.target.closest("[data-ds]"); if (!b) return;
  dsOpen = dsOpen === b.dataset.ds ? null : b.dataset.ds;
  renderDossierList(); renderDossierDetail();
  if (dsOpen) $("#dossierDetail").scrollIntoView({behavior: "smooth", block: "nearest"});
};
function renderDossierDetail(){
  const d = DOSSIERS.find(x => x.id === dsOpen), el = $("#dossierDetail");
  el.hidden = !d; if (!d) return;
  const list = xs => `<ul>${xs.map(x => `<li>${esc(x)}</li>`).join("")}</ul>`;
  el.innerHTML = `<div class="panel-head"><h2>${esc(d.company)} — ${esc(d.role)}</h2><div class="ds-actions">
      <button class="btn primary sm" data-ds-act="use">${icon("play")}Use</button>
      <button class="btn ghost sm" data-ds-act="export">${icon("download")}Export</button>
      <button class="btn ghost danger sm" data-ds-act="delete" aria-label="Delete dossier">${icon("trash")}</button></div></div>
    <div class="ds-body">
      <p>${esc(d.summary)}</p>
      <h3>How they interview</h3><p>${esc(d.style)}</p>
      ${d.values ? `<h3>Values</h3><p>${esc(d.values)}</p>` : ""}
      <h3>Rounds</h3>
      ${d.process.map(r => `<div class="ds-round"><b>${esc(r.name)}</b> <span class="muted small">· ${r.minutes} min · ${esc(roundName(r.type))}</span>
        ${r.what ? `<p>${esc(r.what)}</p>` : ""}${(r.questions || []).length ? list(r.questions) : ""}</div>`).join("")}
      ${(d.pitfalls || []).length ? `<h3>Where people fail</h3>${list(d.pitfalls)}` : ""}
      ${(d.sources || []).length ? `<h3>Sources</h3><ul class="ds-sources">${d.sources.map(x => `<li>${esc(x.title ? `${x.title} — ${x.url}` : x.url)}</li>`).join("")}</ul>` : ""}
    </div>`;
}
function useDossier(id){
  setMode("company");
  $("#dossier").value = id;
  $("#dossier").dispatchEvent(new Event("change", {bubbles: true}));
  nav("interviews");
}
$("#dossierDetail").onclick = async e => {
  const b = e.target.closest("[data-ds-act]"), d = DOSSIERS.find(x => x.id === dsOpen);
  if (!b || !d) return;
  const act = b.dataset.dsAct;
  try{
    if (act === "use") useDossier(d.id);
    if (act === "export") toast(`Saved to ${await api.invoke("export_dossier", {id: d.id})}. Send it to a friend to import.`);
    if (act === "delete"){
      if (!await ask(`Delete the ${d.company} dossier?`, "Past interviews that used it keep their copy.", "Delete", true)) return;
      await api.invoke("delete_dossier", {id: d.id});
      dsOpen = null; renderDossiersView();
    }
  }catch(err){ toast(errText(err), "err"); }
};
$("#dsImport").onclick = () => $("#dsFile").click();
$("#dsFile").onchange = async () => {
  const f = $("#dsFile").files[0]; $("#dsFile").value = ""; if (!f) return;
  try{
    const d = await api.invoke("import_dossier", {text: await f.text()});
    toast(`Imported ${d.company} — ${d.role}.`);
    dsOpen = d.id; renderDossiersView();
  }catch(e){ toast(errText(e), "err"); }
};
$("#rqForm").onsubmit = async e => {
  e.preventDefault();
  const request = {company: $("#rqCompany").value.trim(), role: $("#rqRole").value.trim(), level: $("#rqLevel").value, jd: $("#rqJd").value.trim()};
  if (!request.company || !request.role){ msg("#rqMsg", "Name the company and the role.", "err"); (request.company ? $("#rqRole") : $("#rqCompany")).focus(); return; }
  if (!aiReady()){ msg("#rqMsg", `Connect an AI model first. <button type="button" class="linkbtn" data-nav="settings" data-sec="models">Open Settings</button>`, "err"); return; }
  const b = $("#rqStart"); b.disabled = true;
  try{
    await api.invoke("research_start", {request});
    msg("#rqMsg", "Started. Progress shows below.", "ok");
    ["#rqCompany", "#rqRole", "#rqJd"].forEach(sel => $(sel).value = "");
    JOBS = await api.invoke("research_jobs"); renderJobs();
  }catch(err){ msg("#rqMsg", esc(errText(err)), "err"); }
  finally{ b.disabled = false; }
};
function jobCard(j){
  const r = j.request || {}, live = j.live || {};
  const title = `${esc(r.company || "?")} — ${esc(r.role || "")}`;
  const log = j.log && j.log.trim() ? `<pre class="job-log">${esc(j.log.trim())}</pre>` : "";
  if (j.status === "running"){
    const stage = Math.max(1, Math.min(6, live.stage || 1));
    const pct = Math.max(3, Math.min(99, j.progress || Math.round((stage - 1) / 6 * 100)));
    const tally = live.searches || live.reads ? `${live.searches} searches · ${live.reads} pages read` : "";
    return `<div class="panel" data-job="${j.id}">
      <div class="job-head"><span class="spinner" aria-hidden="true"></span><b>Researching ${title}</b><span class="pct">${pct}%</span></div>
      <div class="pbar" role="progressbar" aria-label="Research progress" aria-valuenow="${pct}" aria-valuemin="0" aria-valuemax="100"><i style="width:${pct}%"></i></div>
      <ul class="job-steps">${RESEARCH_STEPS.map((x, i) => `<li class="${i + 1 < stage ? "done" : i + 1 === stage ? "now" : ""}"><i class="dot"></i>${esc(x)}</li>`).join("")}</ul>
      <p class="job-meta">${esc([live.note, tally, `started ${ago(j.created_at)}`].filter(Boolean).join(" · "))}</p>
      ${log}
      <div class="btn-row"><button class="btn ghost sm" data-job-act="cancel">Cancel</button></div></div>`;
  }
  const done = j.status === "done";
  const pill = done ? `<span class="pill ok">Ready</span>` : j.status === "cancelled" ? `<span class="pill">Cancelled</span>` : `<span class="pill bad">Stopped</span>`;
  const btns = done && j.dossier_id
    ? `<button class="btn primary sm" data-job-act="use">${icon("play")}Use this dossier</button>`
    : `<button class="btn sm" data-job-act="retry">${icon("refresh")}Try again</button>`;
  return `<div class="panel" data-job="${j.id}"><div class="job-head"><b>${title}</b>${pill}</div>${log}
    <div class="btn-row">${btns}<button class="btn ghost sm" data-job-act="dismiss">Dismiss</button></div></div>`;
}
function renderJobs(){
  $("#jobList").innerHTML = JOBS.map(jobCard).join("");
  $$("#jobList .job-log").forEach(p => p.scrollTop = p.scrollHeight);
}
$("#jobList").onclick = async e => {
  const b = e.target.closest("[data-job-act]"); if (!b) return;
  const id = Number(b.closest("[data-job]").dataset.job), j = JOBS.find(x => x.id === id);
  try{
    if (b.dataset.jobAct === "cancel") await api.invoke("research_cancel", {id});
    if (b.dataset.jobAct === "retry") await api.invoke("research_retry", {id});
    if (b.dataset.jobAct === "dismiss") await api.invoke("research_dismiss", {id});
    if (b.dataset.jobAct === "use" && j) return useDossier(j.dossier_id);
    JOBS = await api.invoke("research_jobs"); renderJobs();
  }catch(err){ toast(errText(err), "err"); }
};
api.listen("research", j => {
  const i = JOBS.findIndex(x => x.id === j.id);
  if (i < 0) JOBS.unshift(j); else JOBS[i] = j;
  if (!$("#view-dossiers").hidden) renderJobs();
  const who = (j.request || {}).company || "The";
  if (j.status === "done") toast(`${who} dossier is ready.`);
  if (j.status === "failed") toast(`Research on ${who} stopped. See Dossiers.`, "err");
});
api.listen("dossiers_changed", () => { if (!$("#view-dossiers").hidden) renderDossiersView(); else loadDossiers(); });

/* ---------------- settings ---------------- */
const describe = loc => !loc ? "Not found yet" : loc.kind === "windows" ? loc.exe : "WSL: claude";
async function renderSettings(){
  await refreshStatus();
  const s = SETTINGS;
  if (!s){ msg("#saveMsg", "Couldn’t load settings. Restart Onsite.", "err"); return; }
  $("#clPath").textContent = describe(s.claude);
  renderAi();
  $("#vEnabled").checked = s.voice.enabled;
  $("#vSteps").value = String(s.voice.steps); $("#vSpeed").value = String(s.voice.speed); $("#vPause").value = String(s.voice.pause_ms);
  const vp = $("#vsPill");
  vp.className = "pill " + (!VS.up ? "bad" : VS.loaded ? "ok" : "warn");
  vp.textContent = !VS.up ? "Not running" : VS.loaded ? "Ready" : "Loading voice model";
  $("#vsInfo").textContent = VS.up ? `VoiceStudio is running${VS.device ? " on " + VS.device : ""}. It speaks for the interviewer and transcribes your answers, locally.` : "VoiceStudio isn’t running. Interviews fall back to typing until it is.";
  $("#vsLaunchRow").hidden = VS.up || !VS.installed;
  $("#vsVer").textContent = VS.version ? `${VS.version}${VS.up ? "" : " (installed, not running)"}` : (VS.installed ? "Installed" : "Not installed");
  // Only real profiles: VoiceStudio's "default" is a random speaker on every call.
  const profiles = VS.profiles || [];
  let saved = s.voice.voice;
  // The saved voice was deleted in VoiceStudio: show and keep the one Rust falls back to.
  if (saved && VS.up && profiles.length && !profiles.some(p => p.id === saved)){
    const used = profiles.find(p => !p.name.startsWith("Onsite · ")) || profiles[0];
    msg("#vResult", `Your saved voice is no longer in VoiceStudio, so Onsite uses ${esc(used.name)}. Pick another below if you like.`);
    saved = s.voice.voice = used.id;
    $("#vVoice").innerHTML = `<option value="${esc(used.id)}"></option>`; $("#vVoice").value = used.id;
    saveSettings();
  }
  let opts = profiles.map(p => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join("");
  if (saved && !profiles.some(p => p.id === saved)) opts += `<option value="${esc(saved)}">${VS.up ? "Saved voice (missing in VoiceStudio)" : "Saved voice"}</option>`;
  if (!opts) opts = `<option value="" disabled>${VS.up ? "No voices in VoiceStudio — create one there" : "Start VoiceStudio to pick a voice"}</option>`;
  await loadDevices();
  fillDevice($("#vMic"), DEVICES.inputs, s.voice.input_device);
  fillDevice($("#vSpk"), DEVICES.outputs, s.voice.output_device);
  showLang("#vLang");
  const vv = $("#vVoice");
  vv.innerHTML = opts;
  vv.value = saved || (profiles[0] || {}).id || "";
  vv.disabled = !profiles.length;
  // Persist the voice actually in use, so it can't change between rounds.
  if (!saved && profiles.length) saveSettings();
  setTesting(false);
}
/* ---- sections ---- */
let SET_SEC = LS.get("onsite.setSec", "appearance");
function setSection(sec){
  if (!$(`.set-sec[data-sec="${sec}"]`)) sec = "appearance";
  SET_SEC = sec; LS.set("onsite.setSec", sec);
  $$(".set-sec").forEach(x => x.hidden = x.dataset.sec !== sec);
  $$("#setNav button").forEach(b => b.dataset.sec === sec ? b.setAttribute("aria-current", "page") : b.removeAttribute("aria-current"));
  $("#saveMsg").hidden = !["models", "voice"].includes(sec);
  if (sec === "appearance") renderAppearance();
  if (sec === "voice") renderPersonas();
  if (sec === "updates") renderUpdates();
  if (sec === "storage") renderStorage();
}
$("#setNav").onclick = e => { const b = e.target.closest("button[data-sec]"); if (b) setSection(b.dataset.sec); };

/* ---- appearance ---- */
/* Swatches are the themes' own palette colors (bg, panel, accent, text), as in themes.css. */
const THEMES = {
  onsite:     {name: "Onsite",     dark: ["#16122b", "#221d3d", "#8b5cf6", "#f3f0ff"], light: ["#f3f1fa", "#ffffff", "#6a3de8", "#1c1633"]},
  catppuccin: {name: "Catppuccin", dark: ["#1e1e2e", "#313244", "#cba6f7", "#cdd6f4"], light: ["#eff1f5", "#f9fafb", "#8839ef", "#4c4f69"]},
  rosepine:   {name: "Rosé Pine",  dark: ["#191724", "#26233a", "#c4a7e7", "#e0def4"], light: ["#faf4ed", "#fffaf3", "#907aa9", "#575279"]},
  gruvbox:    {name: "Gruvbox",    dark: ["#282828", "#3c3836", "#fe8019", "#ebdbb2"], light: ["#fbf1c7", "#f9f5d7", "#af3a03", "#3c3836"]},
};
function renderAppearance(){
  const t = OnsiteTheme.read();
  const sw = ([bg, panel, accent, ink]) => `<span style="background:${bg}"><i style="background:${accent}"></i><i style="background:${ink}; opacity:.55"></i><i style="background:${panel}"></i></span>`;
  $("#themeGrid").innerHTML = Object.entries(THEMES).map(([k, v]) =>
    `<button class="theme-card" role="radio" aria-checked="${t.family === k}" data-family="${k}"><span class="theme-sw">${sw(v.light)}${sw(v.dark)}</span><b>${esc(v.name)}</b></button>`).join("");
  $$("#modeSeg button").forEach(b => b.setAttribute("aria-checked", b.dataset.mode === t.mode));
  $("#bgClear").hidden = !t.image; $("#bgDimField").hidden = !t.image;
  $("#bgDim").value = t.dim; $("#bgDimVal").textContent = `${t.dim}%`;
}
function saveTheme(patch){ OnsiteTheme.save({...OnsiteTheme.read(), ...patch}); renderAppearance(); }
$("#themeGrid").onclick = e => { const b = e.target.closest("[data-family]"); if (b) saveTheme({family: b.dataset.family}); };
$("#modeSeg").onclick = e => { const b = e.target.closest("[data-mode]"); if (b) saveTheme({mode: b.dataset.mode}); };
$("#bgDim").addEventListener("input", () => {
  const dim = Number($("#bgDim").value);
  $("#bgDimVal").textContent = `${dim}%`;
  OnsiteTheme.save({...OnsiteTheme.read(), dim});
});
$("#bgPick").onclick = () => $("#bgFile").click();
/* Shrinks the picture to at most 2560×1600 JPEG before it goes to Rust, so it stays small. */
function shrinkImage(file){
  return new Promise((ok, fail) => {
    const url = URL.createObjectURL(file), img = new Image();
    img.onload = () => {
      const k = Math.min(1, 2560 / img.naturalWidth, 1600 / img.naturalHeight);
      const c = document.createElement("canvas");
      c.width = Math.round(img.naturalWidth * k); c.height = Math.round(img.naturalHeight * k);
      c.getContext("2d").drawImage(img, 0, 0, c.width, c.height);
      URL.revokeObjectURL(url);
      ok(c.toDataURL("image/jpeg", 0.86));
    };
    img.onerror = () => { URL.revokeObjectURL(url); fail("That file isn’t an image Onsite can read. Use a JPG, PNG or WebP."); };
    img.src = url;
  });
}
$("#bgFile").onchange = async () => {
  const f = $("#bgFile").files[0]; $("#bgFile").value = ""; if (!f) return;
  msg("#bgMsg", "Saving…");
  try{
    const data = await shrinkImage(f);
    await api.invoke("set_backdrop", {data});
    OnsiteTheme.setImage(data);
    saveTheme({image: true});
    msg("#bgMsg", "");
  }catch(e){ msg("#bgMsg", esc(errText(e)), "err"); }
};
$("#bgClear").onclick = async () => {
  try{ await api.invoke("clear_backdrop"); OnsiteTheme.setImage(null); saveTheme({image: false}); }
  catch(e){ msg("#bgMsg", esc(errText(e)), "err"); }
};

/* ---- interviewers ---- */
function renderPersonas(){
  const el = $("#personaGrid");
  if (!PANELS.length){ el.innerHTML = `<p class="muted small">No interviewer panels found.</p>`; return; }
  const dots = n => `<span class="strict" title="Strictness ${n} of 5">${[1,2,3,4,5].map(i => `<i class="${i <= n ? "on" : ""}"></i>`).join("")}</span>`;
  el.innerHTML = PANELS.map(p => `<div class="persona-panel"><h3>${esc(p.name)}</h3><p>${esc(p.description)}</p><div class="persona-grid">${p.interviewers.map(iv => `
    <article class="persona" data-person="${esc(iv.id)}">
      <div class="persona-top"><span class="persona-av">${esc(iv.name.split(" ").map(w => w[0]).join("").slice(0, 2))}</span>
        <div><b>${esc(iv.name)}</b><small>${esc(iv.role)}</small></div></div>
      <div class="traits">${iv.traits.map(t => `<span class="tag">${esc(t)}</span>`).join("")}</div>
      <p title="${esc(iv.backstory)}">${esc(iv.backstory || iv.style)}</p>
      <div class="meta">Strictness ${dots(iv.strictness)}<br>${iv.rounds.length ? `Runs: ${esc(iv.rounds.map(roundName).join(", "))}<br>` : ""}Voice: ${esc(iv.voice)}</div>
      <button class="btn sm" data-play="${esc(iv.id)}">${icon("speaker")}Play voice</button>
    </article>`).join("")}</div></div>`).join("");
}
$("#personaGrid").onclick = async e => {
  const b = e.target.closest("[data-play]"); if (!b || b.disabled) return;
  const label = b.innerHTML;
  b.disabled = true; b.classList.add("is-busy"); b.lastChild.textContent = "Loading voice…";
  try{ await api.invoke("voice_persona_preview", {id: b.dataset.play}); }
  catch(err){ toast(errText(err), "err"); }
  finally{ b.disabled = false; b.classList.remove("is-busy"); b.innerHTML = label; }
};

/* ---- updates ---- */
const fmtBytes = n => n >= 1e9 ? `${(n / 1e9).toFixed(1)} GB` : n >= 1e6 ? `${(n / 1e6).toFixed(1)} MB` : n >= 1e3 ? `${Math.round(n / 1e3)} KB` : `${n} B`;
let UPDATES = null;
async function renderUpdates(){
  try{ $("#upCurrent").textContent = `v${await api.invoke("app_version")}`; }catch(e){}
  if (UPDATES) drawReleases();
}
function drawReleases(){
  const u = UPDATES;
  $("#upLatest").textContent = u.latest || (u.releases.length ? "You have the newest" : "No releases yet");
  const pill = $("#upPill");
  pill.className = "pill " + (u.latest ? "warn" : "ok"); pill.textContent = u.latest ? "Update available" : "Up to date";
  const ib = $("#upInstall");
  ib.hidden = !u.latest; ib.dataset.tag = u.latest || ""; ib.innerHTML = `${icon("download")}Update to ${esc(u.latest || "")}`;
  $("#upList").innerHTML = u.releases.length ? u.releases.map(r => {
    const tagLine = [r.compared === 0 ? `<span class="pill ok">This version</span>` : "", r.prerelease ? `<span class="pill">Pre-release</span>` : ""].join(" ");
    const action = r.compared === 0 ? "" : !r.installer ? `<span class="muted small">No Windows installer</span>`
      : `<button class="btn ${r.compared > 0 ? "" : "ghost"} sm" data-install="${esc(r.tag)}">${r.compared > 0 ? "Install" : "Downgrade"}</button>`;
    return `<div class="ver-row"><div><b>${esc(r.name)}</b> ${tagLine}<br><small>${esc([r.tag !== r.name ? r.tag : "", r.published_at.slice(0, 10), r.size ? fmtBytes(r.size) : ""].filter(Boolean).join(" · "))}</small></div>${action}
      ${r.notes.trim() ? `<details><summary>Release notes</summary><pre>${esc(r.notes.trim())}</pre></details>` : ""}</div>`;
  }).join("") : `<p class="muted small">No versions have been published yet.</p>`;
}
$("#upCheck").onclick = async () => {
  const b = $("#upCheck"); b.disabled = true; b.classList.add("is-busy");
  msg("#upMsg", "Asking GitHub…");
  const [app, vs] = await Promise.allSettled([api.invoke("check_updates"), api.invoke("voicestudio_version")]);
  if (app.status === "fulfilled"){ UPDATES = app.value; drawReleases(); msg("#upMsg", ""); }
  else msg("#upMsg", esc(errText(app.reason)), "err");
  if (vs.status === "fulfilled"){
    const v = vs.value, p = $("#vsUpPill");
    $("#vsInstalled").textContent = v.installed ? `${v.installed}${v.running ? "" : " (not running)"}` : "Not found";
    $("#vsLatest").textContent = v.latest ? `${v.latest}${v.latest_date ? ` · ${v.latest_date.slice(0, 10)}` : ""}` : (v.note || "Unknown");
    p.className = "pill " + (v.update_available ? "warn" : v.installed ? "ok" : "bad");
    p.textContent = v.update_available ? "Update in VoiceStudio" : v.installed ? "Up to date" : "Not installed";
    $("#vsUpNote").textContent = v.update_available
      ? `VoiceStudio ${v.latest} is out. Update it from VoiceStudio itself; Onsite doesn’t install it.`
      : "Onsite doesn’t update VoiceStudio. When there’s a new version, update it from VoiceStudio itself.";
  }
  b.disabled = false; b.classList.remove("is-busy");
};
async function installVersion(tag){
  const r = UPDATES && UPDATES.releases.find(x => x.tag === tag);
  const older = r && r.compared < 0;
  if (!await ask(older ? `Go back to ${tag}?` : `Update to ${tag}?`,
    `Onsite downloads the installer from GitHub${r && r.size ? ` (${fmtBytes(r.size)})` : ""}, then closes so it can run. Your data stays.${older ? " An older version may not read data saved by this one." : ""}`,
    older ? "Downgrade" : "Update")) return;
  $("#upBar").hidden = false; $("#upBar i").style.width = "0%";
  msg("#upMsg", "Downloading…");
  $$("[data-install], #upInstall, #upCheck").forEach(x => x.disabled = true);
  try{ await api.invoke("install_update", {tag}); msg("#upMsg", "Starting the installer. Onsite will close.", "ok"); }
  catch(e){ msg("#upMsg", esc(errText(e)), "err"); $("#upBar").hidden = true; }
  finally{ $$("[data-install], #upInstall, #upCheck").forEach(x => x.disabled = false); }
}
$("#upInstall").onclick = () => installVersion($("#upInstall").dataset.tag);
$("#upList").onclick = e => { const b = e.target.closest("[data-install]"); if (b) installVersion(b.dataset.install); };
api.listen("update_progress", p => {
  const pct = p.total ? Math.round(p.received / p.total * 100) : 0;
  $("#upBar i").style.width = `${pct}%`; $("#upBar").setAttribute("aria-valuenow", pct);
  msg("#upMsg", `Downloading ${p.tag}… ${fmtBytes(p.received)}${p.total ? ` of ${fmtBytes(p.total)}` : ""}`);
});

/* ---- storage ---- */
let STORAGE = null;
async function renderStorage(){
  try{ STORAGE = await api.invoke("storage_info"); }
  catch(e){ $("#stItems").innerHTML = `<p class="form-msg err">${esc(errText(e))}</p>`; return; }
  const st = STORAGE, c = st.counts;
  $("#stTotal").textContent = `Onsite uses ${fmtBytes(st.total)}`;
  $("#stCounts").innerHTML = [[c.rounds, "rounds"], [c.reports, "with feedback"], [c.loops, "interviews"], [c.dossiers, "dossiers"], [c.research_jobs, "research jobs"]]
    .map(([n, l]) => `<span><b>${n}</b> ${l}</span>`).join("") + (c.oldest_round ? `<span>Oldest round <b>${esc(c.oldest_round.slice(0, 10))}</b></span>` : "");
  $("#stItems").innerHTML = st.items.map(i => `<div class="st-item${i.ours ? "" : " theirs"}"><b>${esc(i.label)}</b><span class="size">${fmtBytes(i.bytes)}</span><small>${esc(i.note)} · ${esc(i.path)}</small></div>`).join("");
  $("#stKeepRounds").value = String(st.limits.keep_rounds);
  $("#stKeepDays").value = String(st.limits.keep_days);
  $("#stLogMax").value = String(st.limits.log_max_mb);
  // A saved value that isn't in the list (set elsewhere) still shows.
  [["#stKeepRounds", st.limits.keep_rounds, "rounds"], ["#stKeepDays", st.limits.keep_days, "days"], ["#stLogMax", st.limits.log_max_mb, "MB"]].forEach(([sel, v, unit]) => {
    if ($(sel).value !== String(v)){ $(sel).insertAdjacentHTML("beforeend", `<option value="${v}">${v} ${unit}</option>`); $(sel).value = String(v); }
  });
}
$("#stSave").onclick = async () => {
  const limits = {keep_rounds: Number($("#stKeepRounds").value), keep_days: Number($("#stKeepDays").value), log_max_mb: Number($("#stLogMax").value)};
  if ((limits.keep_rounds || limits.keep_days) && !await ask("Apply these limits?", "Finished rounds past them are deleted now, with their transcripts and feedback. This can’t be undone.", "Apply", true)) return;
  try{
    const n = await api.invoke("set_storage_limits", {limits});
    msg("#stMsg", n ? `Saved. Deleted ${n} old round${n === 1 ? "" : "s"}.` : "Saved. Nothing needed deleting.", "ok");
    renderStorage();
  }catch(e){ msg("#stMsg", esc(errText(e)), "err"); }
};
async function cleanUp(cmd, done){
  try{ const r = await api.invoke(cmd); msg("#stCleanMsg", done(r), "ok"); renderStorage(); }
  catch(e){ msg("#stCleanMsg", esc(errText(e)), "err"); }
}
$("#stClearLog").onclick = () => cleanUp("clear_log", () => "Log cleared.");
$("#stClearTemp").onclick = () => cleanUp("clear_temp", n => `Freed ${fmtBytes(n)}.`);
$("#stVacuum").onclick = () => cleanUp("vacuum_db", () => "Database compacted.");
$("#view-settings").addEventListener("click", async e => {
  const b = e.target.closest("[data-open]"); if (!b) return;
  try{ await api.invoke("open_folder", {which: b.dataset.open}); }catch(err){ toast(errText(err), "err"); }
});

/* ---- AI model ---- */
const CLI_MODELS = {
  iv: [["sonnet", "Sonnet — fast replies"], ["opus", "Opus — sharper, slower"], ["haiku", "Haiku — fastest, lighter"]],
  rep: [["opus", "Opus — best judgment"], ["sonnet", "Sonnet — faster"]],
};
const MODELS = {};       // provider → model ids from its API
function renderAi(){
  const p = provider(), info = PROVIDERS[p], cli = p === "claude_code";
  $$("#aiPicks .ai-pick").forEach(b => b.setAttribute("aria-checked", b.dataset.provider === p));
  $("#aiCli").hidden = !cli; $("#aiKey").hidden = cli;
  if (!cli){
    $("#aiKeyLabel").textContent = info.key;
    $("#aiKeyHint").textContent = info.hint;
    $("#aiKeyInput").placeholder = AIKEYS[p] ? "Saved. Paste a new key to replace it." : "Paste your key";
    $("#aiKeyRemove").hidden = !AIKEYS[p];
  }
  const cp = $("#clPill"), ok = aiReady();
  cp.className = "pill " + (ok ? "ok" : "bad");
  cp.textContent = cli ? (ok ? "Found" : "Not found") : (ok ? "Key saved" : "No key");
  fillModels(false);
}
async function fillModels(fresh){
  const p = provider();
  let iv, rep;
  if (p === "claude_code"){ iv = CLI_MODELS.iv; rep = CLI_MODELS.rep; $("#aiModelHint").textContent = ""; }
  else {
    if (AIKEYS[p] && (fresh || !MODELS[p])){
      try{ MODELS[p] = await api.invoke("list_models"); $("#aiModelHint").textContent = ""; }
      catch(e){ $("#aiModelHint").textContent = `Couldn’t list models: ${errText(e)}`; }
    } else if (!AIKEYS[p]) $("#aiModelHint").textContent = "Save a key to see the models it can use.";
    if (provider() !== p) return;
    const label = id => (PROVIDERS[p].models || {})[id] || id;
    iv = rep = (MODELS[p] || []).map(id => [id, label(id)]);
  }
  const put = (sel, opts, cur) => {
    if (cur && !opts.some(o => o[0] === cur)) opts = [[cur, (PROVIDERS[p].models || {})[cur] || cur], ...opts];
    sel.innerHTML = opts.map(([v, l]) => `<option value="${esc(v)}">${esc(l)}</option>`).join("");
    sel.value = cur;
  };
  put($("#ivModel"), iv, SETTINGS.interviewer_model);
  put($("#repModel"), rep, SETTINGS.report_model);
}
$("#aiPicks").onclick = async e => {
  const b = e.target.closest(".ai-pick"); if (!b || b.dataset.provider === provider()) return;
  try{
    SETTINGS = await api.invoke("set_provider", {provider: b.dataset.provider});
    msg("#clResult", "");
    renderAi(); refreshStatus();
    if (b.dataset.provider === "claude_code" && !SETTINGS.claude) $("#clDetect").click();
    else if (b.dataset.provider !== "claude_code" && !AIKEYS[b.dataset.provider]) $("#aiKeyInput").focus();
  }catch(e){ toast(errText(e), "err"); }
};
async function saveKey(key){
  const p = provider();
  try{
    await api.invoke("set_api_key", {provider: p, key});
    $("#aiKeyInput").value = "";
    AIKEYS[p] = !!key; delete MODELS[p];
    renderAi(); refreshStatus();
    if (key){ msg("#clResult", "Key saved. Run the health check to test it.", "ok"); fillModels(true); }
    else msg("#clResult", "Key removed.");
  }catch(e){ msg("#clResult", esc(errText(e)), "err"); }
}
$("#aiKeySave").onclick = () => {
  const key = $("#aiKeyInput").value.trim();
  if (!key){ msg("#clResult", "Paste the key first.", "err"); $("#aiKeyInput").focus(); return; }
  saveKey(key);
};
$("#aiKeyInput").addEventListener("keydown", e => { if (e.key === "Enter") $("#aiKeySave").click(); });
$("#aiKeyRemove").onclick = () => saveKey("");

function formVoice(){
  return {...SETTINGS.voice, enabled: $("#vEnabled").checked, voice: $("#vVoice").value, steps: Number($("#vSteps").value), speed: Number($("#vSpeed").value), pause_ms: Number($("#vPause").value)};
}
function msg(sel, text, kind = ""){ const el = $(sel); el.className = "form-msg " + kind; el.innerHTML = text; }

/* Settings save as you change them. Saves run one at a time, in order. */
let saveChain = Promise.resolve(), savedT = null;
function saveSettings(){
  if (!SETTINGS) return;
  const s = {...SETTINGS, interviewer_model: $("#ivModel").value, report_model: $("#repModel").value, voice: formVoice()};
  msg("#saveMsg", "Saving…");
  saveChain = saveChain.then(async () => {
    try{
      await api.invoke("save_settings", {settings: s}); SETTINGS = s;
      msg("#saveMsg", `${icon("check")}Saved. Applies from the next round.`, "ok");
      clearTimeout(savedT); savedT = setTimeout(() => msg("#saveMsg", "Changes save automatically."), 4000);
    }catch(e){ msg("#saveMsg", `Couldn’t save: ${esc(errText(e))}`, "err"); }
  });
}
["#ivModel","#repModel","#vEnabled","#vVoice","#vSteps","#vSpeed","#vPause"].forEach(sel => $(sel).addEventListener("change", saveSettings));
$("#vMic").addEventListener("change", () => setDevices($("#vMic").value, null));
$("#vSpk").addEventListener("change", () => setDevices(null, $("#vSpk").value));

$("#clDetect").onclick = async () => {
  const b = $("#clDetect"); b.disabled = true;
  msg("#clResult", "Looking for Claude Code…");
  try{ const loc = await api.invoke("detect_claude"); msg("#clResult", "Found it.", "ok"); await renderSettings(); $("#clPath").textContent = describe(loc); }
  catch(e){ msg("#clResult", esc(errText(e)), "err"); }
  finally{ b.disabled = false; }
};
$("#clSavePath").onclick = async () => {
  const exe = $("#clExe").value.trim();
  if (!exe){ msg("#clResult", "Type the full path to claude.exe first.", "err"); $("#clExe").focus(); return; }
  const s = {...SETTINGS, claude: {kind: "windows", exe}};
  try{
    await api.invoke("save_settings", {settings: s}); SETTINGS = s;
    $("#clPath").textContent = exe; msg("#clResult", "Saved. Run the health check to test it.");
    refreshStatus();
  }catch(e){ msg("#clResult", esc(errText(e)), "err"); }
};
$("#clHealth").onclick = async () => {
  const b = $("#clHealth"); b.disabled = true; b.classList.add("is-busy");
  msg("#clResult", `Asking ${esc(PROVIDERS[provider()].name)} to reply with OK…`);
  try{
    const h = await api.invoke("health_check");
    msg("#clResult", `${h.ok ? "Working" : "Unexpected reply"} · “${esc(h.reply)}” · ${(h.latency_ms/1000).toFixed(1)} s · ${esc(h.model)} · ${esc(h.location)}`, h.ok ? "ok" : "err");
  }catch(e){ msg("#clResult", esc(errText(e)), "err"); }
  finally{ b.disabled = false; b.classList.remove("is-busy"); }
};
let waitingVoice = false;
$("#vsLaunch").onclick = async () => {
  if (waitingVoice) return;
  const b = $("#vsLaunch"); b.disabled = true;
  try{ await api.invoke("voice_launch"); msg("#vResult", "Starting VoiceStudio. This page updates when it’s ready."); await waitForVoice(); }
  catch(e){ msg("#vResult", esc(errText(e)), "err"); }
  finally{ b.disabled = false; }
};
async function waitForVoice(){
  waitingVoice = true;
  try{
    for (let i = 0; i < 60; i++){
      await new Promise(r => setTimeout(r, 3000));
      await refreshStatus();
      if (VS.up && !VS.loaded) msg("#vResult", "VoiceStudio is up. Loading its voice model…");
      if (VS.up && VS.loaded){ if (!$("#view-settings").hidden) await renderSettings(); msg("#vResult", "VoiceStudio is ready.", "ok"); return; }
    }
    msg("#vResult", "VoiceStudio didn’t come up after 3 minutes. Open it yourself and check it for errors.", "err");
  } finally { waitingVoice = false; }
}
/* Voice tests run from Settings and from the setup screen's audio check. One at a time. */
const TESTERS = {
  settings: {speak: "#vTestSpeak", mic: "#vTestMic", meter: "#vMeter", out: "#vResult", voice: () => formVoice()},
  setup:    {speak: "#acSpeak",    mic: "#acListen", meter: "#acMeter", out: "#acResult", voice: () => ({...SETTINGS.voice})},
};
let testing = false, testTarget = TESTERS.settings;
function setTesting(on){
  testing = on;
  Object.values(TESTERS).forEach(t => [t.speak, t.mic].forEach(sel => {
    $(sel).disabled = on || !VS.up;
    $(sel).title = !VS.up && !on ? "Start VoiceStudio first" : "";
  }));
}
async function testSpeak(t){
  if (testing || !SETTINGS) return;
  testTarget = t;
  const b = $(t.speak); setTesting(true); b.classList.add("is-busy"); msg(t.out, "Generating…");
  try{
    const ms = await api.invoke("voice_test_speak", {settings: t.voice(), text: "Hi, I'm your interviewer today. Let's start with a quick intro."});
    msg(t.out, `First audio in ${(ms/1000).toFixed(1)} s. Didn’t hear it? Pick another speaker.`, "ok");
  }catch(e){ msg(t.out, esc(errText(e)), "err"); }
  finally{ b.classList.remove("is-busy"); setTesting(false); }
}
let testUnlisten = null;
async function testMic(t){
  if (testing || !SETTINGS) return;
  testTarget = t;
  const b = $(t.mic); setTesting(true); b.classList.add("is-busy"); msg(t.out, "Say a sentence. I’ll stop when you pause.");
  if (!testUnlisten) testUnlisten = await api.listen("voice_test", ev => {
    if (ev.kind === "partial") msg(testTarget.out, `Hearing: “${esc(ev.text)}”`);
    if (ev.kind === "level") $(testTarget.meter).style.width = `${Math.min(100, ev.mic * 900)}%`;
  });
  try{
    const text = await api.invoke("voice_test_listen", {settings: t.voice()});
    msg(t.out, `Heard: “${esc(text)}”`, "ok");
  }catch(e){ msg(t.out, esc(errText(e)), "err"); }
  finally{ $(t.meter).style.width = "0"; b.classList.remove("is-busy"); setTesting(false); }
}
$("#vTestSpeak").onclick = () => testSpeak(TESTERS.settings);
$("#vTestMic").onclick = () => testMic(TESTERS.settings);
$("#acSpeak").onclick = () => testSpeak(TESTERS.setup);
$("#acListen").onclick = () => testMic(TESTERS.setup);

/* ---------------- audio devices ---------------- */
let DEVICES = {inputs: [], outputs: []};
async function loadDevices(){
  try{ DEVICES = await api.invoke("audio_devices"); }catch(e){ api.log("error", `audio_devices: ${errText(e)}`); }
}
/* "" = follow the Windows default. A saved device that is unplugged stays listed so the pick isn't lost. */
function fillDevice(sel, list, saved){
  const def = list.find(d => d.default);
  let opts = `<option value="">Windows default${def ? ` (${esc(def.name)})` : ""}</option>`;
  opts += list.map(d => `<option value="${esc(d.id)}">${esc(d.name)}</option>`).join("");
  if (saved && !list.some(d => d.id === saved)) opts += `<option value="${esc(saved)}">Saved device (not connected — using default)</option>`;
  sel.innerHTML = opts;
  sel.value = saved || "";
}
/* Saves only the device fields. `null` keeps the current pick. */
async function setDevices(input, output, relisten = false){
  if (!SETTINGS) return;
  const v = SETTINGS.voice;
  if (input !== null) v.input_device = input;
  if (output !== null) v.output_device = output;
  try{ await api.invoke("voice_set_devices", {input: v.input_device, output: v.output_device, relisten}); }
  catch(e){ toast(`Couldn’t switch device: ${errText(e)}`, "err"); }
}
/* Where English-only speech recognition stands. */
function showLang(sel){
  const e = VS.english, w = VS.whisper || {}, el = $(sel);
  if (!VS.up || !e){ el.textContent = ""; return; }
  const engine = w.state === "ready" ? "Speech recognition: Whisper large-v3 on your GPU, English only."
    : w.state === "loading" ? "Loading Whisper onto your GPU… until then, answers use the faster but less accurate Parakeet."
    : `Whisper is off (${w.reason || "not started"}), so answers use Parakeet, which mishears some tech terms.`;
  el.textContent = e.state === "installing" ? `${engine} Downloading the English caption model… ${Math.round(e.pct)}%.`
    : e.state === "missing" ? `${engine} English caption model missing (${e.reason}).`
    : engine;
}
/* ---------------- get ready (before the call) ---------------- */
/* Shown before round 1 of every loop, and before later rounds when "Check before every round" is on. */
let readyIndex = 0;
const readyEvery = () => !!LS.get("onsite.readyEvery", false);
async function openReady(index){
  readyIndex = index;
  nav("ready");
  const k = loop.setup.rounds[index];
  const who = interviewerFor(index, k, loop.setup.panel);
  const comp = companyLabel(loop.setup);
  $("#readyIc").textContent = comp[0];
  $("#readyRound").textContent = `${comp} — ${loopRound(index).name}`;
  $("#readyWho").textContent = `Round ${index+1} of ${loop.count} · ${loopRound(index).minutes} minutes · with ${who.full ? `${who.full}, ${who.role}` : who.name}`;
  $("#readyEvery").checked = readyEvery();
  renderReadyResume();
  $("#readyJoin").disabled = false; $("#readyJoin").classList.remove("is-busy");
  msg("#acResult", "");
  await renderAudioCheck();
}
/* The resume check from Start, in short. The full report is on the Resume check page. */
function renderReadyResume(){
  const el = $("#readyResume"), r = RESUME_CHECK;
  el.hidden = !r; if (!r) return;
  const passing = r.ats.filter(a => a.passes).length;
  const [tip] = r.tips;
  const head = r.extractability === "partial" ? "Part of your resume file can’t be read" : `Passes ${passing} of 6 simulated ATSs`;
  el.innerHTML = `
    <div class="panel-head"><h2>Resume check</h2><span class="pill ${scoreClass(r.matched.overall) === "hi" ? "ok" : scoreClass(r.matched.overall) === "mid" ? "warn" : "bad"}">Checked</span></div>
    <div class="ready-resume">
      <div><span class="score ${scoreClass(r.matched.overall)}">${r.matched.overall}</span><small class="muted"> / 100 ${r.job ? "job match" : "structure"}</small></div>
      <div><b>${esc(head)}.</b>${tip ? `<p class="muted small">${esc(tip)}</p>` : ""}</div>
    </div>
    <div class="btn-row"><button class="btn sm ghost" id="readyAts">See the full check</button></div>`;
  $("#readyAts").onclick = () => { atsShown = true; nav("resume"); renderAts(r, true); };
}
async function renderAudioCheck(){
  await Promise.all([refreshStatus(), loadDevices()]);
  if (!SETTINGS || $("#view-ready").hidden) return;
  fillDevice($("#acMic"), DEVICES.inputs, SETTINGS.voice.input_device);
  fillDevice($("#acSpk"), DEVICES.outputs, SETTINGS.voice.output_device);
  const on = SETTINGS.voice.enabled && VS.up;
  $("#acBody").hidden = $("#acLang").hidden = !on;
  $("#acTitle").textContent = on ? "Check your mic and speaker" : "Typed round";
  $("#readySub").textContent = on ? "Pick your mic and speaker, then check they work. Takes ten seconds." : "You type your answers. Join when you’re ready.";
  $("#acVoiceRow").hidden = on || !VS.up;
  const p = $("#acPill");
  p.className = "pill " + (!SETTINGS.voice.enabled ? "" : !on ? "bad" : VS.loaded ? "ok" : "warn");
  p.textContent = !SETTINGS.voice.enabled ? "Voice off" : !VS.up ? "VoiceStudio off" : VS.loaded ? "Ready" : "Loading";
  const off = !SETTINGS.voice.enabled ? "Spoken interviews are off, so this round will be typed."
    : !VS.up ? "VoiceStudio isn’t running, so this round will be typed. Start it in Settings to talk." : "";
  if (!testing && (off || / will be typed/.test($("#acResult").textContent))) msg("#acResult", off);
  $("#readyJoin").firstChild.textContent = on ? "Join interview" : "Join and type";
  showLang("#acLang");
  setTesting(testing);
  if (on) startMonitor(); else stopMonitor();
}
$("#acVoiceOn").onclick = async () => {
  const b = $("#acVoiceOn"); b.disabled = true;
  try{ SETTINGS.voice.enabled = true; await api.invoke("save_settings", {settings: SETTINGS}); }
  catch(e){ SETTINGS.voice.enabled = false; toast(errText(e), "err"); }
  b.disabled = false;
  renderAudioCheck();
};
/* Live mic meter: moves as soon as the screen opens, no speech recognition involved. */
let monitorUnlisten = null, monitorOn = false;
async function startMonitor(){
  if (!SETTINGS) return;
  if (!monitorUnlisten) monitorUnlisten = await api.listen("mic_level", ev => {
    if (!monitorOn || testing) return;
    if (ev.mic < 0){ $("#acMeterHint").textContent = "The mic was disconnected. Pick another one."; $("#acMeter").style.width = "0"; return; }
    $("#acMeter").style.width = `${Math.min(100, ev.mic * 900)}%`;
    if (ev.mic * 900 > 8) $("#acMeterHint").textContent = "We can hear you.";
  });
  monitorOn = true;
  $("#acMeterHint").textContent = "Say something. The bar should move.";
  try{ await api.invoke("mic_monitor_start", {input: SETTINGS.voice.input_device}); }
  catch(e){ monitorOn = false; $("#acMeterHint").textContent = `Can’t open the mic: ${errText(e)}`; }
}
function stopMonitor(){
  if (!monitorOn) return;
  monitorOn = false;
  $("#acMeter").style.width = "0";
  api.invoke("mic_monitor_stop").catch(() => {});
}
$("#acMic").addEventListener("change", async () => { await setDevices($("#acMic").value, null); startMonitor(); });
$("#acSpk").addEventListener("change", () => setDevices(null, $("#acSpk").value));
$("#readyEvery").addEventListener("change", e => LS.set("onsite.readyEvery", e.target.checked));
$("#readyBack").onclick = () => nav("interviews");
$("#readyJoin").onclick = () => {
  if (starting) return;
  const b = $("#readyJoin"); b.disabled = true; b.classList.add("is-busy");
  stopMonitor();
  startRound(readyIndex);
};
function fillRoomMic(){
  fillDevice($("#roomMic"), DEVICES.inputs, SETTINGS ? SETTINGS.voice.input_device : "");
  loadDevices().then(() => { if (session) fillDevice($("#roomMic"), DEVICES.inputs, SETTINGS ? SETTINGS.voice.input_device : ""); });
}
$("#roomMic").addEventListener("change", () => {
  const s = session;
  // Reopen the mic on the new device only if it's waiting and hasn't heard words yet.
  setDevices($("#roomMic").value, null, !!s && s.phase === "listening");
  toast(s && (s.phase === "hearing" || s.phase === "transcribing") ? "The new mic is used from your next answer." : "Microphone switched.");
});

/* ---------------- init ---------------- */
(async function init(){
  try{ PANELS = await api.invoke("get_panels"); }catch(e){ PANELS = []; }
  try{ CAT = await api.invoke("get_catalog"); }
  catch(e){ document.body.innerHTML = `<div class="view"><div class="panel"><h2>Onsite couldn’t start</h2><p class="form-msg err">${esc(errText(e))}</p></div></div>`; return; }
  if (OnsiteTheme.read().image) api.invoke("get_backdrop").then(d => { if (d) OnsiteTheme.setImage(d); else OnsiteTheme.save({...OnsiteTheme.read(), image: false}); }).catch(() => {});
  initSetup();
  initAts();
  await refreshStatus();
  if (SETTINGS && provider() === "claude_code" && !SETTINGS.claude){
    try{ await api.invoke("detect_claude"); }catch(e){}
    await refreshStatus();
  }
  // First run with nothing to talk to: start on the AI picker.
  if (!aiReady()){
    SET_SEC = "models";
    nav("settings");
    msg("#clResult", "Connect an AI model to start. Claude Code needs Claude Pro or Max; otherwise paste a Claude, OpenAI or Gemini key.");
    return;
  }
  nav("dashboard");
})();
