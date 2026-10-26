"use strict";
/* Onsite UI. Model calls, storage and audio all live in Rust (window.ipc). */
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
  if (view === "interviews") renderAgenda();
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
  $("#toasts").appendChild(el);
  setTimeout(() => el.remove(), 6000);
}

async function refreshStatus(){
  try{ SETTINGS = await ipc.invoke("get_settings"); }catch(e){}
  try{ VS = await ipc.invoke("voice_status"); }catch(e){ VS = {up:false, loaded:false, profiles:[]}; }
  const cl = $("#sysClaude"); cl.className = "sys " + (SETTINGS && SETTINGS.claude ? "ok" : "bad");
  const vs = $("#sysVoice"); vs.className = "sys " + (!VS.up ? "bad" : VS.loaded ? "ok" : "warn");
  vs.title = !VS.up ? "VoiceStudio is not running" : VS.loaded ? "VoiceStudio ready" : "VoiceStudio is loading its voice model";
}
setInterval(() => { if (!session) refreshStatus(); }, 15000);

/* ---------------- dashboard ---------------- */
async function renderDashboard(){
  let h = [];
  try{ h = await ipc.invoke("history"); }catch(e){}
  $("#dashRecent").innerHTML = h.length ? h.slice(0, 5).map(rowHtml).join("") : `<div class="none">No rounds yet. Your first one takes about a minute to set up.</div>`;
  bindRows("#dashRecent");
  const latest = h.find(x => x.overall != null);
  if (latest){
    const rep = await ipc.invoke("get_report", {roundId: latest.round_id}).catch(() => null);
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
const cfgFields = ["company","level","lang","temper","resume","jd","extra","roundLen"];
function fillSelect(sel, obj, label){ sel.innerHTML = Object.entries(obj).map(([k,v]) => `<option value="${esc(k)}">${esc(label(v))}</option>`).join(""); }

function initSetup(){
  fillSelect($("#company"), CAT.COMPANIES, v => v.name);
  fillSelect($("#level"), CAT.LEVELS, v => v);
  $("#roundList").innerHTML = Object.entries(CAT.ROUNDS).map(([k,r]) =>
    `<button type="button" class="chip" data-round="${esc(k)}" aria-pressed="false" title="${esc(r.blurb)}">${esc(r.name)} <span class="min">${r.min}m</span></button>`).join("");
  const c = LS.get("onsite.cfg", {company:"auto", level:"sde2", lang:"Java", temper:"realistic", roundLen:"", rounds:["resume","coding","sysdesign","behavioral"]});
  cfgFields.forEach(f => { if (c[f] != null) $("#"+f).value = c[f]; });
  selectRounds(c.rounds || []);
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
function hookDrop(dropSel, inputSel, textSel, msgSel, label){
  const drop = $(dropSel), input = $(inputSel), ta = $(textSel);
  const show = () => {
    const n = words(ta.value);
    drop.classList.toggle("filled", n > 0);
    if (n) $(msgSel).innerHTML = `${n} words loaded. <u data-edit>Edit</u> · <u data-clear>Remove</u>`;
  };
  const load = async f => {
    try{ ta.value = (await f.text()).trim(); saveCfg(); show(); }
    catch(e){ toast(`Couldn’t read that file. Paste the ${label.toLowerCase()} instead.`, "err"); }
  };
  drop.onclick = e => {
    if (e.target.matches("[data-paste],[data-edit]")){ ta.hidden = false; ta.focus(); return; }
    if (e.target.matches("[data-clear]")){ ta.value = ""; ta.hidden = true; drop.classList.remove("filled"); saveCfg(); $(msgSel).innerHTML = `Removed. <u>Choose a file</u> or <u data-paste>paste text</u>`; return; }
    input.click();
  };
  drop.onkeydown = e => { if (e.key === "Enter" || e.key === " "){ e.preventDefault(); input.click(); } };
  input.onchange = () => input.files[0] && load(input.files[0]);
  drop.ondragover = e => { e.preventDefault(); drop.classList.add("over"); };
  drop.ondragleave = () => drop.classList.remove("over");
  drop.ondrop = e => { e.preventDefault(); drop.classList.remove("over"); const f = e.dataTransfer.files[0]; if (f) load(f); };
  ta.addEventListener("input", show);
  show();
}
function persona(i){ return CAT.NAMES[(i*5+3) % CAT.NAMES.length]; }
function roundMinutes(k){ return Number($("#roundLen").value) || CAT.ROUNDS[k].min; }

function renderAgenda(){
  if (!CAT) return;
  const rs = selectedRounds();
  $("#roundCount").textContent = rs.length ? `· ${rs.length} selected` : "";
  if (!rs.length){
    $("#agenda").innerHTML = `<li><span class="empty">Pick at least one round.</span></li>`;
    $("#duration").textContent = "—"; $("#startBtn").disabled = true; return;
  }
  $("#startBtn").disabled = false;
  const total = rs.reduce((a, k) => a + roundMinutes(k), 0);
  $("#duration").textContent = total >= 60 ? `${Math.floor(total/60)} h ${total%60 ? total%60 + " min" : ""}` : `${total} min`;
  $("#agenda").innerHTML = rs.map((k,i) => `<li><span class="n">${i+1}</span><div><b>${esc(CAT.ROUNDS[k].name)}</b><span>${roundMinutes(k)} min · with ${esc(persona(i))}</span></div></li>`).join("");
}

$("#startBtn").onclick = async () => {
  const rounds = selectedRounds(); if (!rounds.length) return;
  saveCfg();
  const setup = {
    rounds, company: $("#company").value, level: $("#level").value, temperament: $("#temper").value,
    lang: $("#lang").value, resume: $("#resume").value, jd: $("#jd").value, notes: $("#extra").value,
    minutes: Number($("#roundLen").value) || null,
  };
  const btn = $("#startBtn"); btn.classList.add("is-busy"); $("#startMsg").className = "form-msg"; $("#startMsg").textContent = "";
  try{
    const id = await ipc.invoke("create_loop", {setup});
    loop = {id, setup, count: rounds.length};
    await startRound(0);
  }catch(e){ $("#startMsg").className = "form-msg err"; $("#startMsg").textContent = errText(e); }
  finally{ btn.classList.remove("is-busy"); }
};

/* ---------------- call room ---------------- */
function notesTemplate(k){
  if (k === "sysdesign") return "Requirements\n- Functional:\n- Non-functional:\n\nEstimates\n\nAPIs\n\nData model\n\nHigh-level design\n  client -> \n\nDeep dives\n\nTrade-offs and failure modes\n";
  if (k === "lld") return "Requirements\n\nEntities\n\nClasses and interfaces\n\nKey flows\n\nConcurrency and extensibility\n";
  if (k === "project") return "Architecture sketch\n  [service] -> [queue] -> [service]\n";
  return "";
}
/* "Indian fintech (Razorpay, …)" reads as "Indian fintech" in titles. */
const shortName = n => n.replace(/\s*\(.*\)\s*$/, "");
function companyLabel(setup){ return setup.company === "auto" ? "General" : shortName(CAT.COMPANIES[setup.company].name); }

async function startRound(index){
  await refreshStatus();
  const k = loop.setup.rounds[index];
  const name = persona(index);
  const voiceOn = !!(SETTINGS && SETTINGS.voice.enabled && VS.up);
  session = {roundId: null, index, key: k, persona: name, minutes: roundMinutes(k), startedAt: Date.now(),
             shared: [], closed: false, busy: true, pendingEl: null, live: "", draftEl: null,
             voiceOn, muted: false, phase: "thinking"};
  nav("room");
  const comp = companyLabel(loop.setup);
  $("#roomIc").textContent = $("#ctxIc").textContent = comp[0];
  $("#muteBtn").title = "Mute"; $("#endBtn").title = "End interview";
  $("#roomTitle").textContent = `${comp} — ${CAT.ROUNDS[k].name}`;
  $("#roomSub").textContent = `${CAT.LEVELS[loop.setup.level]} · ${session.minutes} minutes · round ${index+1} of ${loop.count} · with ${name}`;
  $("#ctxCompany").textContent = comp === "General" ? "General practice" : comp;
  $("#ctxRound").textContent = CAT.ROUNDS[k].name;
  $("#ctxChips").innerHTML = [loop.setup.lang, $("#temper").selectedOptions[0].text.split(" —")[0] + " interviewer", CAT.LEVELS[loop.setup.level]].map(x => `<span>${esc(x)}</span>`).join("");
  $("#tips").innerHTML = (TIPS[k] || []).map(t => `<li>${esc(t)}</li>`).join("");
  $("#transcript").innerHTML = ""; $("#answer").value = ""; $("#turnInfo").textContent = "";
  $("#code").value = ""; $("#notes").value = notesTemplate(k); updateGutter();
  $("#edLang").textContent = loop.setup.lang;
  $("#wrapBanner").hidden = true;
  $("#muteBtn").setAttribute("aria-pressed", "false"); $("#muteBtn").querySelector("span").textContent = "Mute";
  $("#muteBtn").disabled = !voiceOn;
  if (!voiceOn && SETTINGS && SETTINGS.voice.enabled) toast("VoiceStudio isn’t running, so this round is typed. Start it in Settings.", "err");
  renderShared(); selectTab(CAT.ROUNDS[k].tab === "shared" ? "editor" : CAT.ROUNDS[k].tab);
  buildWaves();
  setPhase("thinking");
  session.pendingEl = addMsg("iv", "", true);
  startTimer();
  try{
    const info = await ipc.invoke("start_round", {loopId: loop.id, index});
    session.roundId = info.round_id;
    session.minutes = info.plan.minutes;
  }catch(e){
    failPending(errText(e));
    session.busy = false; setPhase("idle");
  }
}

function leaveRound(){
  const s = session; session = null;
  clearInterval(timerI);
  if (s && s.roundId !== null) ipc.invoke("end_round", {roundId: s.roundId, workspace: {code: $("#code").value, notes: $("#notes").value}}).catch(() => {});
}

/* phases: thinking | speaking | listening | hearing | idle | muted | closed */
function setPhase(p){
  if (!session) return;
  session.phase = p;
  const orb = $("#orb");
  const label = {
    thinking: "Thinking…",
    speaking: `${session.persona} is speaking`,
    listening: "Listening…",
    hearing: "Hearing you… tap when done",
    idle: session.voiceOn ? "Tap to talk" : "Your turn — type below",
    muted: "Muted — tap to unmute",
    closed: "Round closed",
  }[p];
  orb.dataset.state = p === "hearing" ? "listening" : (p === "idle" && !session.voiceOn ? "off" : p === "closed" ? "idle" : p);
  orb.setAttribute("aria-label", label);
  $("#orbLabel").textContent = label;
  $$(".wave").forEach(w => w.classList.toggle("live", p === "speaking" || p === "listening" || p === "hearing"));
  if (p !== "speaking" && p !== "listening" && p !== "hearing") setWave(0);
}

function buildWaves(){
  const bars = n => Array.from({length:n}, () => "<i></i>").join("");
  $("#waveL").innerHTML = bars(7); $("#waveR").innerHTML = bars(7);
}
const SHAPE = [.35,.6,.9,1,.75,.5,.3];
function setWave(level){
  const v = Math.min(1, level * 9);
  $$("#waveL i").forEach((b, i) => b.style.height = `${6 + SHAPE[6-i] * v * 30 * (0.75 + Math.random()*0.5)}px`);
  $$("#waveR i").forEach((b, i) => b.style.height = `${6 + SHAPE[i] * v * 30 * (0.75 + Math.random()*0.5)}px`);
}

$("#orb").onclick = () => {
  if (!session || !session.voiceOn || session.closed) { $("#answer").focus(); return; }
  if (session.phase === "listening" || session.phase === "hearing") ipc.invoke("voice_finish");
  else if (session.phase === "muted" || session.phase === "idle") setMuted(false);
};
$("#muteBtn").onclick = () => session && session.voiceOn && setMuted(!session.muted);
function setMuted(m){
  session.muted = m;
  $("#muteBtn").setAttribute("aria-pressed", String(m));
  $("#muteBtn").querySelector("span").textContent = m ? "Unmute" : "Mute";
  $("#muteBtn").querySelector("use").setAttribute("href", m ? "#i-mic" : "#i-mic-off");
  ipc.invoke("voice_mute", {muted: m});
  if (m){ clearDraft(); if (session.phase === "listening" || session.phase === "hearing" || session.phase === "idle") setPhase("muted"); }
  else if (session.phase === "muted") setPhase("idle");
}

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
    return `<li class="${st}"><span class="pt">${st === "done" ? icon("check") : ""}</span><div><b>${esc(name)}</b><span>${len} min</span></div></li>`;
  }).join("");
}

/* events from the interviewer (Claude) */
ipc.listen("round", ev => {
  if (!session || (session.roundId !== null && ev.round_id !== session.roundId)) return;
  switch (ev.kind){
    case "delta":
      session.live += ev.text;
      if (session.pendingEl) renderIvMsg(session.pendingEl, session.live);
      break;
    case "code":
      session.shared.push({lang: ev.lang || "code", code: ev.code});
      renderShared(); $("#sharedDot").hidden = false;
      if (session.key === "coding" || session.key === "comprehension") selectTab("shared");
      break;
    case "turn_done":
      if (session.pendingEl) renderIvMsg(session.pendingEl, ev.text);
      session.pendingEl = null; session.live = ""; session.busy = false;
      session.ttft = ev.ttft_ms;
      showTurnInfo();
      if (ev.ended){ session.closed = true; $("#wrapBanner").hidden = false; setPhase("closed"); }
      else if (!session.voiceOn){ setPhase("idle"); $("#answer").focus({preventScroll:true}); }
      else if (session.phase === "thinking") setPhase(session.muted ? "muted" : "idle");
      break;
    case "error":
      failPending(ev.message);
      session.busy = false; if (!session.voiceOn) setPhase("idle");
      break;
  }
});

/* events from the voice loop (VoiceStudio + mic) */
ipc.listen("voice", ev => {
  if (!session) return;
  switch (ev.kind){
    case "speaking": if (ev.active) setPhase("speaking"); else if (!session.closed) setPhase(session.muted ? "muted" : "idle"); break;
    case "listening": if (ev.active) setPhase("listening"); else if (session.phase === "listening" || session.phase === "hearing") setPhase(session.muted ? "muted" : "idle"); break;
    case "speech_start": setPhase("hearing"); break;
    case "partial": showDraft(ev.text); break;
    case "final":
      clearDraft();
      session.sttMs = ev.stt_ms;
      sendAnswer({text: ev.text, fromVoice: true});
      break;
    case "latency": session.voiceMs = ev.ms; showTurnInfo(); break;
    case "level": if (["speaking","listening","hearing"].includes(session.phase)) setWave(session.phase === "speaking" ? ev.out : ev.mic); break;
    case "error": toast(ev.message, "err"); break;
  }
});

function showTurnInfo(){
  const bits = [];
  if (session.voiceMs != null) bits.push(`Voice reply in ${(session.voiceMs/1000).toFixed(1)} s after you stopped`);
  if (session.ttft != null) bits.push(`first word ${(session.ttft/1000).toFixed(1)} s`);
  $("#turnInfo").textContent = bits.join(" · ");
}

function showDraft(text){
  if (!text) return;
  if (!session.draftEl) session.draftEl = addMsg("me", "", false, true);
  session.draftEl.querySelector(".bubble").textContent = text;
  scrollDown();
}
function clearDraft(){ if (session && session.draftEl){ session.draftEl.remove(); session.draftEl = null; } }

function failPending(message){
  const el = session.pendingEl || addMsg("iv", "", true);
  el.querySelector(".bubble").innerHTML = `<span class="err">${esc(message)}</span><br><span class="muted">Send your answer again to retry.</span>`;
  session.pendingEl = null; session.live = "";
}

function currentWorkspace(){
  const notes = $("#notes").value;
  return {code: $("#code").value, notes: notes.trim() === notesTemplate(session.key).trim() ? "" : notes};
}

async function sendAnswer({text, force = false, fromVoice = false} = {}){
  if (!session || session.roundId === null) return;
  if (session.busy){ toast("The interviewer is still replying."); return; }
  const typed = text == null;
  if (typed) text = $("#answer").value;
  if (typed && session.voiceOn) ipc.invoke("voice_cancel_listen");
  session.busy = true; session.voiceMs = null;
  try{
    const turn = await ipc.invoke("send_answer", {roundId: session.roundId, text, workspace: currentWorkspace(), force});
    if (!turn){ session.busy = false; return; }
    if (typed){ $("#answer").value = ""; autosize(); }
    addMsg("me", turn.text || "(shared workspace)", false, false, turn.shared_workspace);
    session.pendingEl = addMsg("iv", "", true);
    setPhase("thinking");
  }catch(e){
    session.busy = false;
    toast(errText(e), "err");
    if (fromVoice) setPhase("idle");
  }
}
$("#sendBtn").onclick = () => sendAnswer();
$("#sendWork").onclick = $("#sendWork2").onclick = () => sendAnswer({force: true});
$("#answer").addEventListener("keydown", e => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing){ e.preventDefault(); sendAnswer(); }
});
function autosize(){ const a = $("#answer"); a.style.height = "auto"; a.style.height = Math.min(160, a.scrollHeight) + "px"; }
$("#answer").addEventListener("input", autosize);

$("#endBtn").onclick = () => endRound();
$("#wrapReport").onclick = () => endRound();
async function endRound(){
  if (!session) return;
  if (!session.closed && !confirm("End this round and get your feedback?")) return;
  const s = session;
  leaveRound();
  if (s.roundId === null){ nav("interviews"); return; }
  openReport(s.roundId, true);
}

/* ---------------- rendering ---------------- */
function scrollDown(){ const t = $("#transcript"); if (t.scrollHeight - t.scrollTop - t.clientHeight < 200) t.scrollTop = t.scrollHeight; }
function addMsg(who, text, pending = false, draft = false, sharedWs = false){
  const el = document.createElement("div");
  const me = who === "me";
  el.className = "msg " + (me ? "me" : "iv") + (draft ? " draft" : "");
  const av = me ? "You"[0] : session.persona[0];
  el.innerHTML = `<span class="av">${esc(av)}</span><div class="body"><div class="from">${me ? "You" : esc(session.persona)}<time>${clock()}</time></div><div class="bubble">${pending ? '<span class="thinking"><i></i><i></i><i></i></span>' : md(text)}</div>${sharedWs ? `<span class="chip-note">${icon("code")}Shared workspace</span>` : ""}</div>`;
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
  if (!session || !session.shared.length){ pick.innerHTML = `<option>Nothing shared yet</option>`; $("#sharedCode").innerHTML = `<div class="empty">Code the interviewer shares lands here, with line numbers you can refer to.</div>`; return; }
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
  $$(".tab").forEach(t => t.setAttribute("aria-selected", t.dataset.tab === name));
  $$(".pane").forEach(p => p.hidden = p.dataset.pane !== name);
  if (name === "shared") $("#sharedDot").hidden = true;
}
$$(".tab").forEach(t => t.onclick = () => selectTab(t.dataset.tab));

function updateGutter(){
  const n = $("#code").value.split("\n").length;
  $("#gutter").textContent = Array.from({length:n}, (_,i) => i+1).join("\n");
  $("#gutter").scrollTop = $("#code").scrollTop;
}
$("#code").addEventListener("input", updateGutter);
$("#code").addEventListener("scroll", () => $("#gutter").scrollTop = $("#code").scrollTop);
$("#code").addEventListener("keydown", e => {
  const ta = e.target;
  if (e.key === "Tab"){
    e.preventDefault(); ta.setRangeText("    ", ta.selectionStart, ta.selectionEnd, "end"); updateGutter();
  } else if (e.key === "Enter"){
    const s = ta.selectionStart; const lineStart = ta.value.lastIndexOf("\n", s-1) + 1;
    const indent = (ta.value.slice(lineStart, s).match(/^\s*/)||[""])[0];
    const extra = /[{(\[:]\s*$/.test(ta.value.slice(lineStart, s)) ? "    " : "";
    e.preventDefault(); ta.setRangeText("\n" + indent + extra, s, ta.selectionEnd, "end"); updateGutter();
  }
});

/* ---------------- feedback + report ---------------- */
const decClass = d => /no hire/i.test(d) ? "d-no" : /lean/i.test(d) ? "d-lean" : "d-hire";
const scoreClass = n => n >= 70 ? "hi" : n >= 50 ? "mid" : "lo";
function rowHtml(x){
  return `<button class="row-item" data-round-id="${x.round_id}"><span class="row-ic">${esc((x.company || x.name)[0])}</span><span class="row-tx"><b>${esc(x.name)}</b><span>${esc(shortName(x.company))} · ${esc(new Date(x.started_at).toLocaleDateString())}${x.decision ? " · " + esc(x.decision) : ""}</span></span>${x.overall != null ? `<span class="score ${scoreClass(x.overall)}">${x.overall}</span>` : `<span class="pill warn">No feedback yet</span>`}</button>`;
}
function bindRows(sel){ $$(sel + " .row-item").forEach(b => b.onclick = () => openReport(Number(b.dataset.roundId), false)); }
async function renderHistory(){
  let h = [];
  try{ h = await ipc.invoke("history"); }catch(e){ toast(errText(e), "err"); }
  $("#history").innerHTML = h.length ? h.map(rowHtml).join("") : `<div class="none">Your finished rounds and their feedback appear here.</div>`;
  bindRows("#history");
}

async function openReport(roundId, live){
  nav("report");
  const round = await ipc.invoke("get_round", {roundId});
  $("#repTitle").textContent = round.name;
  $("#repSub").textContent = `With ${round.persona} · ${new Date(round.started_at).toLocaleString()}`;
  let rep = await ipc.invoke("get_report", {roundId});
  if (!rep){
    $("#repBody").innerHTML = `<div class="stack"><div class="panel"><h2>Writing your feedback…</h2><p class="muted" style="margin-top:6px">The panel is reviewing the transcript. This takes 20–90 seconds.</p></div><div class="skeleton" style="height:120px"></div><div class="skeleton" style="height:220px"></div></div>`;
    try{ rep = await ipc.invoke("generate_report", {roundId}); }
    catch(e){
      $("#repBody").innerHTML = `<div class="panel"><h2>Feedback didn’t come through</h2><p class="form-msg err">${esc(errText(e))}</p><div class="btn-row"><button class="btn primary" id="repRetry">${icon("refresh")}Try again</button></div></div>`;
      $("#repRetry").onclick = () => openReport(roundId, live);
      return;
    }
  }
  renderReport(round, rep, live);
}

function renderReport(round, r, live){
  const n = Number(r.overall) || 0;
  const ringC = n >= 70 ? "var(--ok)" : n >= 50 ? "var(--warn)" : "var(--bad)";
  const dims = (r.dimensions||[]).map(d => `<div class="dim"><div class="dim-top">${esc(d.name)}<span>${Number(d.score)||0} / 4</span></div><div class="bar"><i style="width:${(Number(d.score)||0)*25}%"></i></div><p>${esc(d.evidence)}</p></div>`).join("");
  const li = a => (a||[]).map(x => `<li>${esc(x)}</li>`).join("");
  const moments = (r.moments||[]).map(m => `<div class="moment"><div class="q">${esc(m.question)}</div><div class="said">You said: ${esc(m.said)}</div><div class="better"><b>Stronger answer:</b> ${esc(m.better)}</div></div>`).join("");
  const met = r.metrics || {};
  const inLoop = live && loop && loop.id === round.loop_id;
  const hasNext = inLoop && round.idx < loop.count - 1;
  const isLast = inLoop && round.idx === loop.count - 1 && loop.count > 1;
  $("#repBody").innerHTML = `
    <div class="stack">
      <div class="panel rep-head">
        <div class="ring" style="--v:${n}; --ring-c:${ringC}"><div><div><b>${n}</b><small>of 100</small></div></div></div>
        <div>
          <span class="decision ${decClass(r.decision||"")}">${esc(r.decision||"—")}</span><span class="muted">${esc(r.level_signal||"")}</span>
          <h2>${esc(r.headline||"")}</h2>
          <div class="btn-row">
            ${hasNext ? `<button class="btn primary" id="nextRound">Next round: ${esc(CAT.ROUNDS[loop.setup.rounds[round.idx+1]].name)}${icon("arrow")}</button>` : ""}
            ${isLast ? `<button class="btn primary" id="verdictBtn">${icon("scale")}Get the hiring committee verdict</button>` : ""}
            ${inLoop ? `<button class="btn" id="retake">${icon("refresh")}Retake this round</button>` : ""}
            <button class="btn ghost" id="showTx">Show transcript</button>
          </div>
        </div>
      </div>
      <div class="fix"><span class="fix-ic">${icon("bulb")}</span><div><b>Fix this first</b>${esc(r.fix_first||"")}</div></div>
      <div id="verdict"></div>
      <div class="panel"><h2>Rubric</h2><div class="dims">${dims}</div></div>
      <div class="cols">
        <div class="panel"><h2>What went well</h2><ul class="list">${li(r.went_well)}</ul></div>
        <div class="panel"><h2>What held you back</h2><ul class="list">${li(r.held_back)}</ul></div>
      </div>
      <div class="panel"><h2>Key moments</h2>${moments}</div>
      <div class="cols">
        <div class="panel"><h2>Practise next</h2><ul class="list">${li(r.practice_next)}</ul></div>
        <div class="panel"><h2>How you spoke</h2><div class="metrics">
          <div class="metric"><b>${met.avg_words ?? "—"}</b><span>words per answer</span></div>
          <div class="metric"><b>${met.answers ?? "—"}</b><span>answers</span></div>
          <div class="metric"><b>${met.minutes ?? "—"}</b><span>minutes</span></div>
          <div class="metric"><b>${met.code_shares ?? 0}</b><span>workspace shares</span></div>
        </div></div>
      </div>
      <div class="panel" id="txPanel" hidden><h2>Transcript</h2><div class="transcript" style="padding:0; overflow:visible">${(round.transcript||[]).map(m => `<div class="msg ${m.who==="me"?"me":"iv"}"><span class="av">${m.who==="me" ? "Y" : esc(round.persona[0])}</span><div class="body"><div class="from">${m.who==="me" ? "You" : esc(round.persona)}</div><div class="bubble">${md(m.text||"(shared workspace)")}</div></div></div>`).join("")}</div></div>
    </div>`;
  const nb = $("#nextRound"); if (nb) nb.onclick = () => startRound(round.idx + 1);
  const vb = $("#verdictBtn"); if (vb) vb.onclick = () => loopVerdict(round.loop_id);
  const rt = $("#retake"); if (rt) rt.onclick = () => startRound(round.idx);
  $("#showTx").onclick = () => { const p = $("#txPanel"); p.hidden = !p.hidden; $("#showTx").textContent = p.hidden ? "Show transcript" : "Hide transcript"; };
}

async function loopVerdict(loopId){
  const el = $("#verdict");
  el.innerHTML = `<div class="panel"><h2>The hiring committee is deliberating…</h2><div class="skeleton" style="height:80px; margin-top:12px"></div></div>`;
  try{
    const v = await ipc.invoke("loop_verdict", {loopId});
    el.innerHTML = `<div class="panel"><div class="panel-head"><h2>Committee decision</h2><span class="decision ${decClass(v.decision||"")}">${esc(v.decision)}</span></div><p>${esc(v.rationale)}</p><div class="cols" style="margin-top:14px"><div><b>Strongest signal</b><p class="muted">${esc(v.strongest_signal)}</p></div><div><b>Biggest risk</b><p class="muted">${esc(v.biggest_risk)}</p></div></div><h3 style="margin:16px 0 8px">Two-week plan</h3><ul class="list">${(v.plan||[]).map(p => `<li>${esc(p)}</li>`).join("")}</ul></div>`;
  }catch(e){ el.innerHTML = `<div class="panel"><p class="form-msg err">${esc(errText(e))}</p></div>`; }
}

/* ---------------- settings ---------------- */
const describe = loc => !loc ? "Not found yet" : loc.kind === "windows" ? loc.exe : "WSL: claude";
async function renderSettings(){
  await refreshStatus();
  const s = SETTINGS;
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
  $("#vVoice").innerHTML = `<option value="">VoiceStudio default</option>` + (VS.profiles||[]).map(p => `<option value="${esc(p.id)}">${esc(p.name)}</option>`).join("");
  $("#vVoice").value = s.voice.voice || ((VS.profiles||[])[0] || {}).id || "";
}
function formVoice(){
  return {...SETTINGS.voice, enabled: $("#vEnabled").checked, voice: $("#vVoice").value, steps: Number($("#vSteps").value), speed: Number($("#vSpeed").value), pause_ms: Number($("#vPause").value)};
}
function msg(sel, text, kind = ""){ const el = $(sel); el.className = "form-msg " + kind; el.innerHTML = text; }

$("#saveSettings").onclick = async () => {
  const s = {...SETTINGS, interviewer_model: $("#ivModel").value, report_model: $("#repModel").value, voice: formVoice()};
  try{ await ipc.invoke("save_settings", {settings: s}); SETTINGS = s; msg("#saveMsg", "Saved.", "ok"); }
  catch(e){ msg("#saveMsg", esc(errText(e)), "err"); }
};
$("#clDetect").onclick = async () => {
  msg("#clResult", "Looking for Claude Code…");
  try{ const loc = await ipc.invoke("detect_claude"); $("#clPath").textContent = describe(loc); msg("#clResult", "Found it.", "ok"); refreshStatus(); }
  catch(e){ msg("#clResult", esc(errText(e)), "err"); }
};
$("#clSavePath").onclick = async () => {
  const exe = $("#clExe").value.trim(); if (!exe) return;
  const s = {...SETTINGS, claude: {kind: "windows", exe}};
  await ipc.invoke("save_settings", {settings: s}); SETTINGS = s;
  $("#clPath").textContent = exe; msg("#clResult", "Saved. Run the health check to test it.");
};
$("#clHealth").onclick = async () => {
  const b = $("#clHealth"); b.classList.add("is-busy");
  msg("#clResult", "Asking Claude to reply with OK…");
  try{
    const h = await ipc.invoke("health_check");
    msg("#clResult", `${h.ok ? "Working" : "Unexpected reply"} · “${esc(h.reply)}” · ${(h.latency_ms/1000).toFixed(1)} s · ${esc(h.model)} · ${esc(h.location)}`, h.ok ? "ok" : "err");
  }catch(e){ msg("#clResult", esc(errText(e)), "err"); }
  finally{ b.classList.remove("is-busy"); }
};
$("#vsLaunch").onclick = async () => {
  try{ await ipc.invoke("voice_launch"); msg("#vResult", "Starting VoiceStudio. This page updates when it’s ready."); waitForVoice(); }
  catch(e){ msg("#vResult", esc(errText(e)), "err"); }
};
async function waitForVoice(){
  for (let i = 0; i < 60; i++){
    await new Promise(r => setTimeout(r, 3000));
    await refreshStatus();
    if (VS.up && VS.loaded){ if (!$("#view-settings").hidden) renderSettings(); msg("#vResult", "VoiceStudio is ready.", "ok"); return; }
  }
}
$("#vTestSpeak").onclick = async () => {
  const b = $("#vTestSpeak"); b.classList.add("is-busy"); msg("#vResult", "Generating…");
  try{
    const ms = await ipc.invoke("voice_test_speak", {settings: formVoice(), text: "Hi, I'm your interviewer today. Let's start with a quick intro."});
    msg("#vResult", `First audio in ${(ms/1000).toFixed(1)} s.`, "ok");
  }catch(e){ msg("#vResult", esc(errText(e)), "err"); }
  finally{ b.classList.remove("is-busy"); }
};
let testUnlisten = null;
$("#vTestMic").onclick = async () => {
  const b = $("#vTestMic"); b.classList.add("is-busy"); msg("#vResult", "Say a sentence. I’ll stop when you pause.");
  if (!testUnlisten) testUnlisten = await ipc.listen("voice_test", ev => {
    if (ev.kind === "partial") msg("#vResult", `Hearing: “${esc(ev.text)}”`);
    if (ev.kind === "level") $("#vMeter").style.width = `${Math.min(100, ev.mic * 900)}%`;
  });
  try{
    const text = await ipc.invoke("voice_test_listen", {settings: formVoice()});
    msg("#vResult", `Heard: “${esc(text)}”`, "ok");
  }catch(e){ msg("#vResult", esc(errText(e)), "err"); }
  finally{ $("#vMeter").style.width = "0"; b.classList.remove("is-busy"); }
};

/* ---------------- init ---------------- */
(async function init(){
  try{ CAT = await ipc.invoke("get_catalog"); }
  catch(e){ document.body.innerHTML = `<div class="view"><div class="panel"><h2>Onsite couldn’t start</h2><p class="form-msg err">${esc(errText(e))}</p></div></div>`; return; }
  initSetup();
  await refreshStatus();
  if (SETTINGS && !SETTINGS.claude) ipc.invoke("detect_claude").then(refreshStatus).catch(() => {});
  nav("dashboard");
})();
