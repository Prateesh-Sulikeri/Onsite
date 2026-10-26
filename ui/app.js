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

/* ---------------- shell ---------------- */
function nav(view){
  $$(".view").forEach(v => v.hidden = v.id !== "view-" + view);
  const section = view === "report" ? "feedback" : view === "room" ? "interviews" : view;
  $$(".nav-item").forEach(b => b.dataset.nav === section ? b.setAttribute("aria-current", "page") : b.removeAttribute("aria-current"));
  $("#app").classList.toggle("in-call", view === "room");
  $(".main").scrollTop = 0;
  if (view === "dashboard") renderDashboard();
  if (view === "interviews"){ renderAgenda(); renderAudioCheck(); }
  if (view === "feedback") renderHistory();
  if (view === "settings") renderSettings();
}
document.addEventListener("click", e => {
  const t = e.target.closest("[data-nav]");
  if (!t) return;
  if (session && !confirm("Leave the call? The round keeps its transcript, but the interviewer stops.")) return;
  if (session) leaveRound();
  nav(t.dataset.nav);
});

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

async function refreshStatus(){
  try{ SETTINGS = await api.invoke("get_settings"); }catch(e){}
  try{ VS = await api.invoke("voice_status"); }catch(e){ VS = {up:false, loaded:false, profiles:[]}; }
  const cl = $("#sysClaude"); cl.className = "sys " + (SETTINGS && SETTINGS.claude ? "ok" : "bad");
  cl.title = SETTINGS && SETTINGS.claude ? "Claude Code found" : "Claude Code not found";
  const vs = $("#sysVoice"); vs.className = "sys " + (!VS.up ? "bad" : VS.loaded ? "ok" : "warn");
  vs.title = !VS.up ? "VoiceStudio is not running" : VS.loaded ? "VoiceStudio ready" : "VoiceStudio is loading its voice model";
}
setInterval(() => { if (!session) refreshStatus(); }, 15000);

/* ---------------- dashboard ---------------- */
async function renderDashboard(){
  let h = [];
  try{ h = await api.invoke("history"); }catch(e){}
  $("#dashRecent").innerHTML = h.length ? h.slice(0, 5).map(rowHtml).join("") : `<div class="none">No rounds yet. Your first one takes about a minute to set up.</div>`;
  bindRows("#dashRecent");
  const latest = h.find(x => x.overall != null);
  if (latest){
    const rep = await api.invoke("get_report", {roundId: latest.round_id}).catch(() => null);
    if (rep && rep.fix_first) $("#dashFix").innerHTML = `<p>${esc(rep.fix_first)}</p><p class="muted" style="margin-top:8px">From ${esc(latest.name)}, ${esc(new Date(latest.started_at).toLocaleDateString())}</p>`;
  }
  await refreshStatus();
  const checks = [
    [SETTINGS && SETTINGS.claude ? "ok" : "bad", "Claude Code", SETTINGS && SETTINGS.claude ? "Found" : "Not found"],
    [!VS.up ? "bad" : VS.loaded ? "ok" : "warn", "VoiceStudio", !VS.up ? "Not running" : VS.loaded ? (VS.device ? VS.device.replace(/^\w+ \((.*)\)$/, "$1") : "Ready") : "Loading voice"],
    [SETTINGS && SETTINGS.voice.enabled ? "ok" : "warn", "Spoken interviews", SETTINGS && SETTINGS.voice.enabled ? "On" : "Off"],
  ];
  $("#dashChecks").innerHTML = checks.map(([k, n, s]) => `<li class="${k}"><i class="dot"></i>${esc(n)}<span>${esc(s)}</span></li>`).join("");
}

/* ---------------- setup ---------------- */
const cfgFields = ["company","panel","level","lang","temper","resume","jd","extra","roundLen"];
let PANELS = [];
function fillSelect(sel, obj, label){ sel.innerHTML = Object.entries(obj).map(([k,v]) => `<option value="${esc(k)}">${esc(label(v))}</option>`).join(""); }

function initSetup(){
  fillSelect($("#company"), CAT.COMPANIES, v => v.name);
  fillSelect($("#level"), CAT.LEVELS, v => v);
  $("#panel").innerHTML += PANELS.map(p => `<option value="${esc(p.id)}">${esc(p.name)} — the same five people every time</option>`).join("");
  $("#roundList").innerHTML = Object.entries(CAT.ROUNDS).map(([k,r]) =>
    `<button type="button" class="chip" data-round="${esc(k)}" aria-pressed="false" title="${esc(r.blurb)}">${esc(r.name)} <span class="min">${r.min}m</span></button>`).join("");
  const c = LS.get("onsite.cfg", {company:"auto", level:"sde2", lang:"Java", temper:"realistic", roundLen:"", rounds:["resume","coding","sysdesign","behavioral"]});
  cfgFields.forEach(f => {
    const el = $("#"+f); if (c[f] == null) return;
    // A saved value the catalog no longer has would blank the select; keep the default instead.
    if (el.tagName === "SELECT" && ![...el.options].some(o => o.value === String(c[f]))) return;
    el.value = c[f];
  });
  selectRounds(c.rounds || []);
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
function selectedRounds(){ return $$('#roundList .chip[aria-pressed="true"]').map(b => b.dataset.round); }
function saveCfg(){
  const c = {}; cfgFields.forEach(f => c[f] = $("#"+f).value);
  c.rounds = selectedRounds();
  LS.set("onsite.cfg", c);
}
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
    if (!TEXT_FILE.test(f.name) && f.type && !f.type.startsWith("text/")){
      toast(`That file isn’t plain text. Use a .txt or .md file, or paste the ${lower}.`, "err"); return;
    }
    try{ ta.value = (await f.text()).trim(); ta.hidden = true; saveCfg(); show(); }
    catch(e){ toast(`Couldn’t read that file. Paste the ${lower} instead.`, "err"); }
    input.value = "";
  };
  drop.onclick = e => {
    const b = e.target.closest("[data-paste],[data-edit],[data-clear],[data-choose]");
    if (b && b.matches("[data-paste]")){ ta.hidden = false; show(); ta.focus(); return; }
    if (b && b.matches("[data-edit]")){ ta.hidden = !ta.hidden; show(); if (!ta.hidden) ta.focus(); return; }
    if (b && b.matches("[data-clear]")){ ta.value = ""; ta.hidden = true; saveCfg(); show(); drop.querySelector("[data-choose]").focus(); return; }
    input.click();
  };
  input.onchange = () => input.files[0] && load(input.files[0]);
  drop.ondragover = e => { e.preventDefault(); drop.classList.add("over"); };
  drop.ondragleave = e => { if (!drop.contains(e.relatedTarget)) drop.classList.remove("over"); };
  drop.ondrop = e => { e.preventDefault(); drop.classList.remove("over"); const f = e.dataTransfer.files[0]; if (f) load(f); };
  ta.addEventListener("input", show);
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
function roundMinutes(k){ return Number($("#roundLen").value) || CAT.ROUNDS[k].min; }

function renderAgenda(){
  if (!CAT) return;
  const rs = selectedRounds();
  $("#roundCount").textContent = rs.length ? `· ${rs.length} selected` : "";
  if (!rs.length){
    $("#agenda").innerHTML = `<li><span class="empty">Pick at least one round.</span></li>`;
    $("#duration").textContent = "—"; $("#startBtn").disabled = true; return;
  }
  $("#startBtn").disabled = starting;
  const total = rs.reduce((a, k) => a + roundMinutes(k), 0);
  $("#duration").textContent = total >= 60 ? `${Math.floor(total/60)} h${total%60 ? " " + total%60 + " min" : ""}` : `${total} min`;
  $("#agenda").innerHTML = rs.map((k,i) => `<li><span class="n">${i+1}</span><div><b>${esc(CAT.ROUNDS[k].name)}</b><span>${roundMinutes(k)} min · with ${esc(persona(i))}</span></div></li>`).join("");
}

$("#startBtn").onclick = async () => {
  const rounds = selectedRounds(); if (!rounds.length || starting) return;
  saveCfg();
  const setup = {
    rounds, company: $("#company").value, level: $("#level").value, temperament: $("#temper").value,
    lang: $("#lang").value, resume: $("#resume").value, jd: $("#jd").value, notes: $("#extra").value,
    minutes: Number($("#roundLen").value) || null,
    panel: $("#panel").value,
  };
  const btn = $("#startBtn"), m = $("#startMsg");
  btn.disabled = true; btn.setAttribute("aria-busy", "true"); btn.classList.add("is-busy");
  m.className = "form-msg"; m.textContent = "Setting up your loop…";
  try{
    const id = await api.invoke("create_loop", {setup});
    loop = {id, setup, count: rounds.length};
    m.textContent = "";
    await startRound(0);
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
const shortName = n => String(n || "").replace(/\s*\(.*\)\s*$/, "");
function companyLabel(setup){ return setup.company === "auto" ? "General" : shortName((CAT.COMPANIES[setup.company] || {}).name || setup.company); }
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
  const s = session = {roundId: null, index, key: k, persona: name, minutes: roundMinutes(k), startedAt: Date.now(),
             shared: [], closed: false, busy: true, pendingEl: null, live: "", draftEl: null,
             voiceOn, muted: false, phase: "thinking", heldBase: null, fromHeld: false, transcribeT: null, abandoned: false};
  nav("room");
  const comp = companyLabel(loop.setup);
  $("#roomIc").textContent = $("#ctxIc").textContent = comp[0];
  $("#roomTitle").textContent = `${comp} — ${CAT.ROUNDS[k].name}`;
  $("#roomSub").textContent = `${CAT.LEVELS[loop.setup.level]} · ${s.minutes} minutes · round ${index+1} of ${loop.count} · with ${who.full ? `${who.full}, ${who.role}` : name}`;
  $("#ctxCompany").textContent = comp === "General" ? "General practice" : comp;
  $("#ctxRound").textContent = CAT.ROUNDS[k].name;
  $("#ctxChips").innerHTML = [loop.setup.lang, temperLabel(loop.setup.temperament) + " interviewer", CAT.LEVELS[loop.setup.level]].map(x => `<span>${esc(x)}</span>`).join("");
  $("#tips").innerHTML = (TIPS[k] || []).map(t => `<li>${esc(t)}</li>`).join("");
  $("#transcript").innerHTML = ""; $("#answer").value = ""; autosize(); hint(""); $("#turnInfo").textContent = "";
  $("#code").value = ""; $("#notes").value = notesTemplate(k); updateGutter();
  $("#edLang").textContent = loop.setup.lang;
  $("#wrapBanner").hidden = true;
  $("#sharedDot").hidden = true;
  applyVoiceUi();
  fillRoomMic();
  if (!voiceOn && SETTINGS && SETTINGS.voice.enabled) toast("VoiceStudio isn’t running, so this round is typed. Start it in Settings.", "err");
  showProblem(null);
  renderShared(); selectTab(CAT.ROUNDS[k].tab === "shared" ? "editor" : CAT.ROUNDS[k].tab);
  buildWaves();
  setPhase("thinking");
  s.pendingEl = addMsg("iv", "", true);
  startTimer();
  try{
    const info = await api.invoke("start_round", {loopId: loop.id, index});
    if (s.abandoned || session !== s){
      // The user left while the round was starting: stop the interviewer we just created.
      deadRounds.add(info.round_id);
      api.invoke("end_round", {roundId: info.round_id, workspace: {code: "", notes: ""}}).catch(() => {});
      return;
    }
    s.roundId = info.round_id;
    s.minutes = info.plan.minutes;
    if (info.problem) showProblem(info.problem);
    // Rust decides whether voice really started (VoiceStudio health, audio device). Trust it when it says.
    if (typeof info.voice === "boolean" && info.voice !== s.voiceOn){
      s.voiceOn = info.voice; applyVoiceUi();
      if (!info.voice) toast("Voice couldn’t start, so this round is typed.", "err");
      if (s.phase !== "thinking" || !s.busy) setPhase("idle");
    }
  }catch(e){
    if (session !== s) return;
    failPending(`Couldn’t start the round. ${errText(e)}`, `<button class="btn sm" data-retry-start>${icon("refresh")}Try again</button>`);
    const b = $("[data-retry-start]");
    if (b) b.onclick = () => { session = null; clearInterval(timerI); startRound(index); };
    s.busy = false; setPhase("idle");
  }
}

function applyVoiceUi(){
  const s = session; if (!s) return;
  const mb = $("#muteBtn");
  mb.setAttribute("aria-pressed", String(s.muted));
  mb.querySelector("span").textContent = s.muted ? "Unmute" : "Mute";
  mb.querySelector("use").setAttribute("href", s.muted ? "#i-mic" : "#i-mic-off");
  mb.disabled = !s.voiceOn || s.closed;
  mb.title = !s.voiceOn ? "Voice is off for this round" : s.muted ? "Unmute" : "Mute";
  $("#roomMic").disabled = !s.voiceOn;
  $("#answer").placeholder = s.voiceOn ? "Type instead…" : "Type your answer…";
}

/* Stops the round. Resolves once Rust has saved it, so a report call sees the final transcript. */
async function leaveRound(){
  const s = session; session = null;
  clearInterval(timerI);
  if (!s) return;
  clearTimeout(s.transcribeT);
  if (s.roundId === null){ s.abandoned = true; return; }
  deadRounds.add(s.roundId);
  await api.invoke("end_round", {roundId: s.roundId, workspace: {code: $("#code").value, notes: $("#notes").value}}).catch(e => api.log("error", `end_round: ${errText(e)}`));
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
  autosize();
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
      renderShared(); $("#sharedDot").hidden = false;
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
    $("#answer").value = ""; autosize(); s.fromHeld = false; hint("");
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

function currentWorkspace(){
  const notes = $("#notes").value;
  return {code: $("#code").value, notes: notes.trim() === notesTemplate(session.key).trim() ? "" : notes};
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
    if (typed){ $("#answer").value = ""; autosize(); s.fromHeld = false; hint(""); }
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
$("#sendWork").onclick = $("#sendWork2").onclick = () => sendAnswer({force: true});
$("#answer").addEventListener("keydown", e => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing){ e.preventDefault(); sendAnswer(); }
});
function autosize(){ const a = $("#answer"); a.style.height = "auto"; a.style.height = Math.min(160, a.scrollHeight) + "px"; }
$("#answer").addEventListener("input", () => {
  autosize();
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
  $("#problemTab").hidden = !on;
  $(".room-body").classList.toggle("coding", on);
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
    ${p.runnable ? "" : `<p class="form-msg err">Run isn’t available for ${esc(loop.setup.lang)} yet. Use Java, Python, C++ or JavaScript to run tests.</p>`}`;
  if (p.starter && !$("#code").value.trim()){ $("#code").value = p.starter; updateGutter(); }
  selectTab("problem");
}
async function runCode(submit){
  const s = session; if (!s || s.roundId === null || s.running) return;
  s.running = true;
  const btns = [$("#runBtn"), $("#submitBtn")]; btns.forEach(b => b.disabled = true);
  const out = $("#runOut"); out.hidden = false;
  out.innerHTML = `<div class="run-head"><span class="muted">${submit ? "Submitting… compiling and running every test." : "Running the examples…"}</span></div>`;
  try{
    const r = await api.invoke("run_code", {roundId: s.roundId, code: $("#code").value, submit});
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
  if (!s.closed && !confirm("End this round? You can get feedback afterwards.")) return;
  const pending = leaveRound();
  if (s.roundId === null){ nav("interviews"); return; }
  nav("report");
  $("#repTitle").textContent = CAT.ROUNDS[s.key].name; $("#repSub").textContent = `With ${s.persona}`;
  $("#repBody").innerHTML = `<div class="panel"><h2>Saving the round…</h2></div>`;
  await pending;
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
function selectTab(name){
  $$(".tab").forEach(t => { const on = t.dataset.tab === name; t.setAttribute("aria-selected", on); t.tabIndex = on ? 0 : -1; });
  $$(".pane").forEach(p => p.hidden = p.dataset.pane !== name);
  if (name === "shared") $("#sharedDot").hidden = true;
}
$$(".tab").forEach(t => t.onclick = () => selectTab(t.dataset.tab));
$(".tabs").addEventListener("keydown", e => {
  if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
  const tabs = $$(".tab"), i = tabs.indexOf(document.activeElement); if (i < 0) return;
  const n = tabs[(i + (e.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length];
  selectTab(n.dataset.tab); n.focus(); e.preventDefault();
});

function updateGutter(){
  const n = $("#code").value.split("\n").length;
  $("#gutter").textContent = Array.from({length:n}, (_,i) => i+1).join("\n");
  $("#gutter").scrollTop = $("#code").scrollTop;
}
$("#code").addEventListener("input", updateGutter);
$("#code").addEventListener("scroll", () => $("#gutter").scrollTop = $("#code").scrollTop);
$("#code").addEventListener("keydown", e => {
  const ta = e.target;
  if (e.key === "Tab" && !e.shiftKey){
    // Esc first, then Tab, leaves the editor (keyboard users aren't trapped).
    if (ta.dataset.escaped){ delete ta.dataset.escaped; return; }
    e.preventDefault(); ta.setRangeText("    ", ta.selectionStart, ta.selectionEnd, "end"); updateGutter();
  } else if (e.key === "Escape"){
    ta.dataset.escaped = "1";
  } else if (e.key === "Enter"){
    const s = ta.selectionStart; const lineStart = ta.value.lastIndexOf("\n", s-1) + 1;
    const indent = (ta.value.slice(lineStart, s).match(/^\s*/)||[""])[0];
    const extra = /[{(\[:]\s*$/.test(ta.value.slice(lineStart, s)) ? "    " : "";
    e.preventDefault(); ta.setRangeText("\n" + indent + extra, s, ta.selectionEnd, "end"); updateGutter();
  } else delete ta.dataset.escaped;
});

/* ---------------- feedback + report ---------------- */
const decClass = d => /no hire/i.test(d) ? "d-no" : /lean/i.test(d) ? "d-lean" : "d-hire";
const scoreClass = n => n >= 70 ? "hi" : n >= 50 ? "mid" : "lo";
function rowHtml(x){
  return `<button class="row-item" data-round-id="${x.round_id}"><span class="row-ic">${esc((x.company || x.name || "?")[0])}</span><span class="row-tx"><b>${esc(x.name)}</b><span>${esc(shortName(x.company))} · ${esc(new Date(x.started_at).toLocaleDateString())}${x.decision ? " · " + esc(x.decision) : ""}</span></span>${x.overall != null ? `<span class="score ${scoreClass(x.overall)}">${x.overall}</span>` : `<span class="pill">No feedback yet</span>`}</button>`;
}
function bindRows(sel){ $$(sel + " .row-item").forEach(b => b.onclick = () => openReport(Number(b.dataset.roundId), false)); }
async function renderHistory(){
  let h = [];
  try{ h = await api.invoke("history"); }catch(e){ toast(errText(e), "err"); }
  $("#history").innerHTML = h.length
    ? h.map(x => `<div class="row-wrap">${rowHtml(x)}<button class="btn ghost danger sm row-del" data-del="${x.round_id}" aria-label="Delete ${esc(x.name)}" title="Delete">${icon("trash")}</button></div>`).join("")
    : `<div class="none">Your finished rounds and their feedback appear here.</div>`;
  bindRows("#history");
  $$("#history .row-del").forEach(b => b.onclick = async () => { if (await deleteRound(Number(b.dataset.del))) renderHistory(); });
}
/* Deletes a round with its transcript and feedback. True when it was deleted. */
async function deleteRound(roundId){
  if (!confirm("Delete this round and its feedback? This can’t be undone.")) return false;
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
      <p class="muted">You gave ${st.answers} answers, ${st.words} words in all. Get written feedback now or come back to it later from Feedback.</p>
      <div class="btn-row">
        <button class="btn primary" id="getFeedback">${icon("sparkle")}Get feedback</button>
        ${hasNext ? `<button class="btn" id="nextRound">Next: ${esc(CAT.ROUNDS[loop.setup.rounds[round.idx+1]].name)}${icon("arrow")}</button>` : ""}
        ${inLoop ? `<button class="btn ghost" id="retake">${icon("refresh")}Retake this round</button>` : ""}
        <button class="btn ghost" data-nav="${inLoop ? "interviews" : "feedback"}">${icon("back")}Back</button>
      </div>
    </div>`;
  bindStart("#retake", round.idx);
  bindStart("#nextRound", round.idx + 1);
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
  b.onclick = () => { b.disabled = true; b.classList.add("is-busy"); startRound(idx); };
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
        ${hasNext ? `<button class="btn" id="nextRound">Skip to ${esc(CAT.ROUNDS[loop.setup.rounds[round.idx+1]].name)}${icon("arrow")}</button>` : ""}
        <button class="btn ${inLoop ? "ghost" : "primary"}" data-nav="${inLoop ? "interviews" : "feedback"}">${icon("back")}Back</button>
      </div>
    </div>`;
  bindStart("#retake", round.idx);
  bindStart("#nextRound", round.idx + 1);
}

function dimHtml(d){
  const sc = d.score == null || d.score === "" || !Number.isFinite(Number(d.score)) ? null : Math.max(0, Math.min(4, Number(d.score)));
  const ev = String(d.evidence || "");
  if (sc === null){
    return `<div class="dim na"><div class="dim-top">${esc(d.name)}<span>Not assessed</span></div><div class="bar na" aria-hidden="true"></div><p>${esc(ev.replace(/^not assessed:\s*/i, ""))}</p></div>`;
  }
  return `<div class="dim"><div class="dim-top">${esc(d.name)}<span>${sc} / 4</span></div><div class="bar" role="img" aria-label="${sc} out of 4"><i style="width:${sc*25}%"></i></div><p>${esc(ev)}</p></div>`;
}

function renderReport(round, r, live){
  const has = r.overall != null && Number.isFinite(Number(r.overall));
  const n = has ? Number(r.overall) : 0;
  const ringC = !has ? "var(--surface-3)" : n >= 70 ? "var(--ok)" : n >= 50 ? "var(--warn)" : "var(--bad)";
  const dims = (r.dimensions||[]).map(dimHtml).join("");
  const li = a => (a||[]).map(x => `<li>${esc(x)}</li>`).join("");
  const moments = (r.moments||[]).map(m => `<div class="moment"><div class="q">${esc(m.question)}</div><div class="said">You said: ${esc(m.said)}</div><div class="better"><b>Stronger answer:</b> ${esc(m.better)}</div></div>`).join("");
  const met = r.metrics || {};
  const {inLoop, hasNext, isLast} = loopButtons(round, live);
  const section = (title, body) => body ? `<div class="panel"><h2>${title}</h2>${body}</div>` : "";
  $("#repBody").innerHTML = `
    <div class="stack">
      <div class="panel rep-head">
        <div class="ring" style="--v:${n}; --ring-c:${ringC}" role="img" aria-label="${has ? `Score ${n} of 100` : "No score"}"><div><div><b>${has ? n : "—"}</b><small>of 100</small></div></div></div>
        <div>
          <span class="decision ${decClass(r.decision||"")}">${esc(r.decision||"—")}</span><span class="muted">${esc(r.level_signal||"")}</span>
          <h2>${esc(r.headline||"")}</h2>
          <div class="btn-row">
            ${hasNext ? `<button class="btn primary" id="nextRound">Next round: ${esc(CAT.ROUNDS[loop.setup.rounds[round.idx+1]].name)}${icon("arrow")}</button>` : ""}
            ${isLast ? `<button class="btn primary" id="verdictBtn">${icon("scale")}Get the hiring committee verdict</button>` : ""}
            ${inLoop ? `<button class="btn" id="retake">${icon("refresh")}Retake this round</button>` : ""}
            <button class="btn ghost" id="showTx" aria-expanded="false" aria-controls="txPanel">Show transcript</button>
          </div>
        </div>
      </div>
      ${r.fix_first ? `<div class="fix"><span class="fix-ic">${icon("bulb")}</span><div><b>Fix this first</b>${esc(r.fix_first)}</div></div>` : ""}
      <div id="verdict"></div>
      ${section("Rubric", dims && `<div class="dims">${dims}</div>`)}
      <div class="cols">
        <div class="panel"><h2>What went well</h2><ul class="list">${li(r.went_well) || "<li class='muted'>Nothing stood out yet.</li>"}</ul></div>
        <div class="panel"><h2>What held you back</h2><ul class="list">${li(r.held_back) || "<li class='muted'>Nothing major.</li>"}</ul></div>
      </div>
      ${section("Key moments", moments)}
      <div class="cols">
        <div class="panel"><h2>Practise next</h2><ul class="list">${li(r.practice_next)}</ul></div>
        <div class="panel"><h2>How you spoke</h2><div class="metrics">
          <div class="metric"><b>${met.avg_words ?? "—"}</b><span>words per answer</span></div>
          <div class="metric"><b>${met.answers ?? "—"}</b><span>answers</span></div>
          <div class="metric"><b>${met.minutes ?? "—"}</b><span>minutes</span></div>
          <div class="metric"><b>${met.code_shares ?? 0}</b><span>workspace shares</span></div>
        </div></div>
      </div>
      <div class="panel" id="txPanel" hidden><h2>Transcript</h2><div class="transcript" style="padding:0; overflow:visible">${(round.transcript||[]).map(m => `<div class="msg ${m.who==="me"?"me":"iv"}"><span class="av" aria-hidden="true">${m.who==="me" ? "Y" : esc(round.persona[0])}</span><div class="body"><div class="from">${m.who==="me" ? "You" : esc(round.persona)}</div><div class="bubble">${md(String(m.text||"(shared workspace)").replace(/\[\[END\]\]/g, ""))}</div></div></div>`).join("")}</div></div>
    </div>`;
  bindStart("#nextRound", round.idx + 1);
  bindStart("#retake", round.idx);
  const vb = $("#verdictBtn"); if (vb) vb.onclick = () => loopVerdict(round.loop_id, vb);
  $("#showTx").onclick = () => { const p = $("#txPanel"), b = $("#showTx"); p.hidden = !p.hidden; b.textContent = p.hidden ? "Show transcript" : "Hide transcript"; b.setAttribute("aria-expanded", String(!p.hidden)); };
}

async function loopVerdict(loopId, btn){
  const el = $("#verdict");
  btn.disabled = true; btn.classList.add("is-busy");
  el.innerHTML = `<div class="panel" role="status"><h2>The hiring committee is deliberating…</h2><div class="skeleton" style="height:80px; margin-top:12px"></div></div>`;
  try{
    const v = await api.invoke("loop_verdict", {loopId});
    el.innerHTML = `<div class="panel"><div class="panel-head"><h2>Committee decision</h2><span class="decision ${decClass(v.decision||"")}">${esc(v.decision)}</span></div><p>${esc(v.rationale)}</p><div class="cols" style="margin-top:14px"><div><b>Strongest signal</b><p class="muted">${esc(v.strongest_signal)}</p></div><div><b>Biggest risk</b><p class="muted">${esc(v.biggest_risk)}</p></div></div><h3 style="margin:16px 0 8px">Two-week plan</h3><ul class="list">${(v.plan||[]).map(p => `<li>${esc(p)}</li>`).join("")}</ul></div>`;
    btn.hidden = true;
  }catch(e){
    el.innerHTML = `<div class="panel"><p class="form-msg err" role="alert">${esc(errText(e))}</p></div>`;
    btn.disabled = false;
  }
  finally{ btn.classList.remove("is-busy"); }
}

/* ---------------- settings ---------------- */
const describe = loc => !loc ? "Not found yet" : loc.kind === "windows" ? loc.exe : "WSL: claude";
async function renderSettings(){
  await refreshStatus();
  const s = SETTINGS;
  if (!s){ msg("#saveMsg", "Couldn’t load settings. Restart Onsite.", "err"); return; }
  $("#clPath").textContent = describe(s.claude);
  const cp = $("#clPill"); cp.className = "pill " + (s.claude ? "ok" : "bad"); cp.textContent = s.claude ? "Found" : "Not found";
  $("#ivModel").value = s.interviewer_model; $("#repModel").value = s.report_model;
  $("#vEnabled").checked = s.voice.enabled;
  $("#vSteps").value = String(s.voice.steps); $("#vSpeed").value = String(s.voice.speed); $("#vPause").value = String(s.voice.pause_ms);
  const vp = $("#vsPill");
  vp.className = "pill " + (!VS.up ? "bad" : VS.loaded ? "ok" : "warn");
  vp.textContent = !VS.up ? "Not running" : VS.loaded ? "Ready" : "Loading voice model";
  $("#vsInfo").textContent = VS.up ? `VoiceStudio is running${VS.device ? " on " + VS.device : ""}. It speaks for the interviewer and transcribes your answers, locally.` : "VoiceStudio isn’t running. Interviews fall back to typing until it is.";
  $("#vsLaunchRow").hidden = VS.up || !VS.installed;
  // Only real profiles: VoiceStudio's "default" is a random speaker on every call.
  const profiles = VS.profiles || [];
  const saved = s.voice.voice;
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
  msg("#clResult", "Asking Claude to reply with OK…");
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
  const e = VS.english, el = $(sel);
  if (!VS.up || !e){ el.textContent = ""; return; }
  el.textContent = e.state === "ready" ? "Speech recognition: English only."
    : e.state === "installing" ? `Downloading the English speech model… ${Math.round(e.pct)}%. Until it’s done, a strong accent may come out as another language.`
    : `English speech model missing (${e.reason}). Install Parakeet TDT v2 in VoiceStudio’s dictation settings.`;
}
async function renderAudioCheck(){
  await Promise.all([refreshStatus(), loadDevices()]);
  if (!SETTINGS) return;
  fillDevice($("#acMic"), DEVICES.inputs, SETTINGS.voice.input_device);
  fillDevice($("#acSpk"), DEVICES.outputs, SETTINGS.voice.output_device);
  const p = $("#acPill");
  p.className = "pill " + (!VS.up ? "bad" : VS.loaded ? "ok" : "warn");
  p.textContent = !VS.up ? "VoiceStudio off" : VS.loaded ? "Ready" : "Loading";
  const off = !SETTINGS.voice.enabled ? "Spoken interviews are off in Settings, so rounds will be typed."
    : !VS.up ? "VoiceStudio isn’t running, so rounds will be typed. Start it in Settings." : "";
  if (!testing && (off || / so rounds will be typed/.test($("#acResult").textContent))) msg("#acResult", off);
  showLang("#acLang");
  setTesting(testing);
}
$("#acMic").addEventListener("change", () => setDevices($("#acMic").value, null));
$("#acSpk").addEventListener("change", () => setDevices(null, $("#acSpk").value));
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
  initSetup();
  await refreshStatus();
  if (SETTINGS && !SETTINGS.claude) api.invoke("detect_claude").then(refreshStatus).catch(() => {});
  nav("dashboard");
})();
