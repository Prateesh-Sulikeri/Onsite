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
  const section = view === "report" ? "feedback" : ["room", "ready", "prep"].includes(view) ? "interviews" : view;
  $$(".nav-item").forEach(b => b.dataset.nav === section ? b.setAttribute("aria-current", "page") : b.removeAttribute("aria-current"));
  $("#app").classList.toggle("in-call", view === "room");
  $(".main").scrollTop = 0;
  if (view === "dashboard") renderDashboard();
  if (view === "interviews") renderAgenda();
  if (view !== "ready") stopMonitor();
  if (view === "feedback") renderHistory();
  if (view === "settings") renderSettings();
  if (view === "resume") openAts();
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
function roundMinutes(k){ return Number($("#roundLen").value) || CAT.ROUNDS[k].min; }

function renderAgenda(){
  if (!CAT) return;
  const rs = selectedRounds();
  $$("#roundList .chip").forEach(b => b.querySelector(".min").textContent = `${roundMinutes(b.dataset.round)}m`);
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
  // The room fills in behind the loader, so the first question streams in while voice loads.
  openPrep(index, who);
  const comp = companyLabel(loop.setup);
  $("#roomIc").textContent = $("#ctxIc").textContent = comp[0];
  $("#roomTitle").textContent = `${comp} — ${CAT.ROUNDS[k].name}`;
  $("#roomSub").textContent = `${CAT.LEVELS[loop.setup.level]} · ${s.minutes} minutes · round ${index+1} of ${loop.count} · with ${who.full ? `${who.full}, ${who.role}` : name}`;
  $("#ctxCompany").textContent = comp === "General" ? "General practice" : comp;
  $("#ctxRound").textContent = CAT.ROUNDS[k].name;
  const codes = ["coding", "comprehension", "lld", "fundamentals"].includes(k);
  $("#ctxChips").innerHTML = [codes ? loop.setup.lang : null, temperLabel(loop.setup.temperament) + " interviewer", CAT.LEVELS[loop.setup.level]].filter(Boolean).map(x => `<span>${esc(x)}</span>`).join("");
  $("#tips").innerHTML = (TIPS[k] || []).map(t => `<li>${esc(t)}</li>`).join("");
  $("#transcript").innerHTML = ""; $("#answer").value = ""; autosize(); hint(""); $("#turnInfo").textContent = "";
  $("#code").value = ""; $("#notes").value = notesTemplate(k); updateGutter();
  $("#edLang").textContent = loop.setup.lang;
  $("#wrapBanner").hidden = true;
  $("#sharedDot").hidden = true;
  applyVoiceUi();
  fillRoomMic();
  showProblem(null);
  renderShared(); selectTab(CAT.ROUNDS[k].tab === "shared" ? "editor" : CAT.ROUNDS[k].tab);
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
}

/* ---------------- preparing (between Get ready and the call) ---------------- */
/* Every line is a real check reported by Rust (`prep` events); the call opens when they're done. */
const PREP_STEPS = [
  ["claude", "Claude Code"], ["question", "First question"],
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
  $("#prepSub").textContent = `${CAT.ROUNDS[loop.setup.rounds[index]].name} with ${who.full || who.name}. The call opens when everything below is ready.`;
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
  return api.invoke("end_round", {roundId: s.roundId, workspace: {code: $("#code").value, notes: $("#notes").value}}).catch(e => api.log("error", `end_round: ${errText(e)}`));
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
    ${p.runnable ? "" : `<p class="form-msg err">Run isn’t available for ${esc(loop.setup.lang)} yet. Use Java, Python, C++ or JavaScript to run tests.</p>`}`;
  if (p.starter && !$("#code").value.trim()){ $("#code").value = p.starter; updateGutter(); }
  selectTab("editor");
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
  if (!s.closed && !await ask("End this round?", "You can get feedback afterwards.", "End round")) return;
  if (session !== s) return;
  const pending = leaveRound();
  if (s.roundId === null){ nav("interviews"); return; }
  nav("report");
  $("#repTitle").textContent = CAT.ROUNDS[s.key].name; $("#repSub").textContent = `With ${s.persona}`;
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
        ${hasNext ? `<button class="btn primary" id="nextRound">Next: ${esc(CAT.ROUNDS[loop.setup.rounds[round.idx+1]].name)}${icon("arrow")}</button>` : ""}
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
        ${hasNext ? `<button class="btn" id="nextRound">Skip to ${esc(CAT.ROUNDS[loop.setup.rounds[round.idx+1]].name)}${icon("arrow")}</button>` : ""}
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
    ${hasNext ? `<button class="btn primary" id="nextRound">Next round: ${esc(CAT.ROUNDS[loop.setup.rounds[round.idx+1]].name)}${icon("arrow")}</button>` : ""}
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
  const nextBtn = live && next < loop.count ? `<div class="btn-row"><button class="btn primary" id="ifNext">Next round: ${esc(CAT.ROUNDS[loop.setup.rounds[next]].name)}${icon("arrow")}</button></div>` : "";
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
function renderAtsInput(){
  const drop = $("#atsDrop"), msg = $("#atsMsg");
  const pasted = words($("#resume").value);
  drop.classList.toggle("filled", !!RESUME_FILE || pasted > 0);
  msg.innerHTML = RESUME_FILE
    ? `${esc(RESUME_FILE.name)} · ${Math.max(1, Math.round(RESUME_FILE.size / 1024))} KB. <button type="button" class="linkbtn" data-choose>Choose another</button> · <button type="button" class="linkbtn" data-clear>Remove</button>`
    : pasted
      ? `Using the ${pasted}-word resume text from interview setup. Layout checks need the file: <button type="button" class="linkbtn" data-choose>choose the PDF or Word file</button>`
      : `Drop the exact PDF or Word file you send to employers, or <button type="button" class="linkbtn" data-choose>choose a file</button>`;
}
function initAts(){
  const drop = $("#atsDrop"), input = $("#atsFile");
  const load = async f => {
    input.value = "";
    try{ rememberResumeFile(await fileToB64(f)); $("#atsStatus").textContent = ""; }
    catch(e){ toast(errText(e), "err"); }
  };
  drop.onclick = e => {
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
      <details class="panel ats-raw"><summary><b>What the ATS sees</b> <span class="muted">— the text exactly as extracted, in reading order</span></summary><pre>${esc(r.preview)}</pre></details>
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
  $("#readyRound").textContent = `${comp} — ${CAT.ROUNDS[k].name}`;
  $("#readyWho").textContent = `Round ${index+1} of ${loop.count} · ${roundMinutes(k)} minutes · with ${who.full ? `${who.full}, ${who.role}` : who.name}`;
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
  initSetup();
  initAts();
  await refreshStatus();
  if (SETTINGS && !SETTINGS.claude) api.invoke("detect_claude").then(refreshStatus).catch(() => {});
  nav("dashboard");
})();
