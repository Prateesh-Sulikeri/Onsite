"use strict";
/* Onsite UI. All model calls and storage go through Rust (window.ipc). */
const F = "```";
const $ = s => document.querySelector(s);
const $$ = s => [...document.querySelectorAll(s)];
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
const words = s => (String(s||"").trim().match(/\S+/g)||[]).length;
const LS = {
  get(k, d){ try{ const v = localStorage.getItem(k); return v ? JSON.parse(v) : d; }catch(e){ return d; } },
  set(k, v){ try{ localStorage.setItem(k, JSON.stringify(v)); }catch(e){} },
};
const errText = e => typeof e === "string" ? e : (e && e.message) || "Something went wrong.";

let CAT = null;          // catalog from prompts/catalog.json
let loop = null;         // {id, setup, count}
let session = null;      // current round in the call room
let timerI = null;

/* ---------------- theme ---------------- */
(function(){
  const t = LS.get("onsite.theme", null);
  if (t) document.documentElement.dataset.theme = t;
  $("#themeBtn").onclick = () => {
    const cur = document.documentElement.dataset.theme || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
    const nx = cur === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = nx; LS.set("onsite.theme", nx);
  };
})();

/* ---------------- views ---------------- */
function showView(v){
  for (const id of ["setup","settings","room","report"]) $("#"+id+"View").classList.toggle("hidden", v !== id);
  if (v === "setup"){ renderHistory(); renderAgenda(); checkClaude(); }
  if (v === "settings") renderSettings();
  window.scrollTo(0,0);
}

/* ---------------- setup form ---------------- */
const cfgFields = ["company","level","lang","temper","resume","jd","extra","roundLen"];
function fillSelect(sel, obj, getLabel){ sel.innerHTML = Object.entries(obj).map(([k,v]) => `<option value="${esc(k)}">${esc(getLabel(v))}</option>`).join(""); }

function initSetup(){
  fillSelect($("#company"), CAT.COMPANIES, v => v.name);
  fillSelect($("#level"), CAT.LEVELS, v => v);
  $("#roundList").innerHTML = Object.entries(CAT.ROUNDS).map(([k,r]) =>
    `<label class="round-opt"><input type="checkbox" value="${esc(k)}"><span><b>${esc(r.name)}</b> <span class="kbd-hint">${r.min} min</span><small>${esc(r.blurb)}</small></span></label>`).join("");
  const c = LS.get("onsite.cfg", {company:"auto", level:"sde2", lang:"Java", temper:"realistic", roundLen:"", rounds:["resume","coding","sysdesign","behavioral"]});
  cfgFields.forEach(f => { if (c[f] != null) $("#"+f).value = c[f]; });
  $$("#roundList input").forEach(i => i.checked = (c.rounds||[]).includes(i.value));
  document.addEventListener("change", e => { if (e.target.closest("#setupView")){ saveCfg(); renderAgenda(); } });
  let t; document.addEventListener("input", e => { if (e.target.closest("#setupView")){ clearTimeout(t); t = setTimeout(saveCfg, 400); } });
  hookFile("#resumeFile", "#resume", "#resumeMsg");
  hookFile("#jdFile", "#jd", "#jdMsg");
}
function saveCfg(){
  const c = {}; cfgFields.forEach(f => c[f] = $("#"+f).value);
  c.rounds = $$("#roundList input").filter(i => i.checked).map(i => i.value);
  LS.set("onsite.cfg", c);
}
function hookFile(inputSel, targetSel, msgSel){
  $(inputSel).onchange = async e => {
    const f = e.target.files[0]; if (!f) return;
    try{ const t = (await f.text()).trim(); $(targetSel).value = t; $(msgSel).textContent = `Read ${words(t)} words. Check it below.`; saveCfg(); }
    catch(err){ $(msgSel).textContent = "Couldn’t read that file. Paste the text instead."; }
  };
}
function persona(i){ return CAT.NAMES[(i*5+3) % CAT.NAMES.length]; }
function selectedRounds(){ return $$("#roundList input").filter(i => i.checked).map(i => i.value); }
function roundMinutes(k){ return Number($("#roundLen").value) || CAT.ROUNDS[k].min; }

function renderAgenda(){
  const rs = selectedRounds();
  const el = $("#agenda");
  if (!rs.length){ el.innerHTML = `<div class="agenda-empty">Tick at least one round.</div>`; $("#startBtn").disabled = true; return; }
  $("#startBtn").disabled = false;
  let t = 10*60;
  el.innerHTML = rs.map((k,i) => {
    const min = roundMinutes(k);
    const hh = String(Math.floor(t/60)).padStart(2,"0"), mm = String(t%60).padStart(2,"0");
    t += min + 15;
    return `<div class="slot"><div class="time">${hh}:${mm} · ${min} min</div><div class="name">${esc(CAT.ROUNDS[k].name)}</div><div class="who">with ${esc(persona(i))}</div></div>`;
  }).join("");
}

async function checkClaude(){
  const s = await ipc.invoke("get_settings");
  const b = $("#aiBanner");
  if (s.claude){ b.classList.add("hidden"); return; }
  try{ await ipc.invoke("detect_claude"); b.classList.add("hidden"); }
  catch(e){ b.innerHTML = `${esc(errText(e))} <button class="btn small" id="goSettings">Open settings</button>`; b.classList.remove("hidden"); $("#goSettings").onclick = () => showView("settings"); }
}

$("#startBtn").onclick = async () => {
  const rounds = selectedRounds(); if (!rounds.length) return;
  saveCfg();
  const setup = {
    rounds, company: $("#company").value, level: $("#level").value, temperament: $("#temper").value,
    lang: $("#lang").value, resume: $("#resume").value, jd: $("#jd").value, notes: $("#extra").value,
    minutes: Number($("#roundLen").value) || null,
  };
  $("#startBtn").disabled = true; $("#startMsg").textContent = "Starting…";
  try{
    const id = await ipc.invoke("create_loop", {setup});
    loop = {id, setup, count: rounds.length};
    $("#startMsg").textContent = "";
    await startRound(0);
  }catch(e){ $("#startMsg").textContent = errText(e); showView("setup"); }
  finally{ $("#startBtn").disabled = false; }
};

/* ---------------- call room ---------------- */
function notesTemplate(k){
  if (k === "sysdesign") return "Requirements\n- Functional:\n- Non-functional:\n\nEstimates\n\nAPIs\n\nData model\n\nHigh-level design\n  client -> \n\nDeep dives\n\nTrade-offs and failure modes\n";
  if (k === "lld") return "Requirements\n\nEntities\n\nClasses and interfaces\n\nKey flows\n\nConcurrency and extensibility\n";
  if (k === "project") return "Architecture sketch\n  [service] -> [queue] -> [service]\n";
  return "";
}

async function startRound(index){
  const k = loop.setup.rounds[index];
  const name = persona(index);
  session = {roundId: null, index, key: k, persona: name, minutes: roundMinutes(k), startedAt: Date.now(),
             shared: [], closed: false, busy: true, pendingEl: null, live: ""};
  showView("room");
  $("#roomTitle").textContent = CAT.ROUNDS[k].name;
  $("#roomSub").textContent = `${CAT.COMPANIES[loop.setup.company].name} · ${CAT.LEVELS[loop.setup.level]} · round ${index+1} of ${loop.count}`;
  $("#ivName").textContent = `${name} · interviewer`;
  $("#avatar").textContent = name[0];
  $("#transcript").innerHTML = ""; $("#answer").value = ""; $("#caption").textContent = ""; $("#turnInfo").textContent = "";
  $("#code").value = ""; $("#notes").value = notesTemplate(k); updateGutter();
  $("#edLang").textContent = loop.setup.lang;
  $("#wrapBanner").classList.add("hidden");
  renderShared(); selectTab(CAT.ROUNDS[k].tab === "shared" ? "editor" : CAT.ROUNDS[k].tab);
  setStage("thinking");
  session.pendingEl = addMsg("iv", "", null, true);
  startTimer();
  try{
    const info = await ipc.invoke("start_round", {loopId: loop.id, index});
    session.roundId = info.round_id;
    session.minutes = info.plan.minutes;
  }catch(e){
    failPending(errText(e));
    session.busy = false;
  }
}

function setStage(s){
  $("#avatar").classList.toggle("speaking", s === "thinking");
  $("#callState").textContent = s === "thinking" ? "Interviewer is talking…" : s === "closed" ? "Round closed" : "Your turn";
}

function startTimer(){
  clearInterval(timerI);
  const tick = () => {
    if (!session) return;
    const sec = Math.floor((Date.now() - session.startedAt) / 1000);
    $("#timer").textContent = `${String(Math.floor(sec/60)).padStart(2,"0")}:${String(sec%60).padStart(2,"0")}`;
    $("#timer").classList.toggle("over", sec > session.minutes * 60);
  };
  tick(); timerI = setInterval(tick, 1000);
}

ipc.listen("round", ev => {
  if (!session || (session.roundId !== null && ev.round_id !== session.roundId)) return;
  switch (ev.kind){
    case "delta":
      session.live += ev.text;
      if (session.pendingEl) renderIvMsg(session.pendingEl, session.live);
      break;
    case "sentence":
      $("#caption").textContent = ev.text;
      break;
    case "code":
      session.shared.push({lang: ev.lang || "code", code: ev.code});
      renderShared(); $("#sharedDot").classList.remove("hidden");
      if (session.key === "coding" || session.key === "comprehension") selectTab("shared");
      break;
    case "turn_done":
      if (session.pendingEl) renderIvMsg(session.pendingEl, ev.text);
      session.pendingEl = null; session.live = ""; session.busy = false;
      $("#turnInfo").textContent = `First word in ${(ev.ttft_ms/1000).toFixed(1)} s · full reply ${(ev.total_ms/1000).toFixed(1)} s`;
      if (ev.ended){ session.closed = true; $("#wrapBanner").classList.remove("hidden"); setStage("closed"); }
      else { setStage("idle"); $("#answer").focus({preventScroll:true}); }
      break;
    case "error":
      failPending(ev.message);
      session.busy = false; setStage("idle");
      break;
  }
});

function failPending(message){
  const el = session.pendingEl || addMsg("iv", "", null, true);
  el.querySelector(".bubble").innerHTML = `<span class="thinking">${esc(message)}</span><br><span class="kbd-hint">Send your answer again to retry.</span>`;
  session.pendingEl = null; session.live = "";
}

function currentWorkspace(){
  const notes = $("#notes").value;
  return {code: $("#code").value, notes: notes.trim() === notesTemplate(session.key).trim() ? "" : notes};
}

async function sendAnswer(force){
  if (!session || session.busy || session.roundId === null) return;
  const text = $("#answer").value;
  session.busy = true;
  try{
    const turn = await ipc.invoke("send_answer", {roundId: session.roundId, text, workspace: currentWorkspace(), force: !!force});
    if (!turn){ session.busy = false; return; }
    $("#answer").value = "";
    addMsg("me", turn.text || "(shared workspace)", turn.shared_workspace ? "Shared workspace" : null);
    session.pendingEl = addMsg("iv", "", null, true);
    setStage("thinking");
  }catch(e){
    session.busy = false;
    addMsg("iv", "", null, true).querySelector(".bubble").innerHTML = `<span class="thinking">${esc(errText(e))}</span>`;
  }
}
$("#sendBtn").onclick = () => sendAnswer(false);
$("#sendWork").onclick = $("#sendWork2").onclick = () => sendAnswer(true);
$("#answer").addEventListener("keydown", e => {
  if (e.key === "Enter" && !e.shiftKey && !e.isComposing){ e.preventDefault(); sendAnswer(false); }
});

$("#endBtn").onclick = () => endRound();
$("#wrapReport").onclick = () => endRound();

async function endRound(){
  if (!session) return;
  if (!session.closed && !confirm("End this round and get feedback?")) return;
  clearInterval(timerI);
  const s = session; session = null;
  if (s.roundId !== null){
    try{ await ipc.invoke("end_round", {roundId: s.roundId, workspace: {code: $("#code").value, notes: $("#notes").value}}); }
    catch(e){ console.error(e); }
  }
  if (s.roundId === null){ showView("setup"); return; }
  await openReport(s.roundId, true);
}

/* ---------------- rendering ---------------- */
function addMsg(who, text, chip, pending){
  const el = document.createElement("div");
  el.className = "msg " + (who === "me" ? "me-msg" : "iv-msg");
  const from = who === "me" ? "You" : (session ? session.persona : "Interviewer");
  el.innerHTML = `<div class="from">${esc(from)}</div><div class="bubble">${pending ? '<span class="thinking">Thinking…</span>' : md(text)}</div>${chip ? `<span class="chip">${esc(chip)}</span>` : ""}`;
  $("#transcript").appendChild(el);
  $("#transcript").scrollTop = $("#transcript").scrollHeight;
  return el;
}
function renderIvMsg(el, text){
  el.querySelector(".bubble").innerHTML = md(text.replace(/\[\[END\]\]/g, ""));
  el.querySelectorAll("[data-open]").forEach(b => b.onclick = () => selectTab("shared"));
  const t = $("#transcript"); if (t.scrollHeight - t.scrollTop - t.clientHeight < 160) t.scrollTop = t.scrollHeight;
}
function inlineMd(s){
  return esc(s).replace(/`([^`]+)`/g, "<code>$1</code>").replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>");
}
function md(text){
  const out = []; const re = new RegExp(F+"([\\w+#-]*)\\n?([\\s\\S]*?)(?:"+F+"|$)", "g");
  let last = 0, m;
  while ((m = re.exec(text))){
    if (m.index > last) out.push(mdBlock(text.slice(last, m.index)));
    out.push(`<div class="codeblock"><div class="cb-head"><span>${esc(m[1]||"code")}</span><button class="btn small" data-open>Open in workspace</button></div><pre><code>${esc(m[2])}</code></pre></div>`);
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
  if (!session || !session.shared.length){ pick.innerHTML = `<option>Nothing shared yet</option>`; $("#sharedCode").innerHTML = `<div class="empty">Code the interviewer shares appears here.</div>`; return; }
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
  $$(".pane").forEach(p => p.classList.toggle("hidden", p.dataset.pane !== name));
  if (name === "shared") $("#sharedDot").classList.add("hidden");
}
$$(".tab").forEach(t => t.onclick = () => selectTab(t.dataset.tab));
$$(".mobile-switch [data-view]").forEach(b => b.onclick = () => $("#roomMain").dataset.view = b.dataset.view);

/* editor */
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
    e.preventDefault();
    ta.setRangeText("    ", ta.selectionStart, ta.selectionEnd, "end"); updateGutter();
  } else if (e.key === "Enter"){
    const s = ta.selectionStart; const lineStart = ta.value.lastIndexOf("\n", s-1) + 1;
    const indent = (ta.value.slice(lineStart, s).match(/^\s*/)||[""])[0];
    const extra = /[{(\[:]\s*$/.test(ta.value.slice(lineStart, s)) ? "    " : "";
    e.preventDefault(); ta.setRangeText("\n" + indent + extra, s, ta.selectionEnd, "end"); updateGutter();
  }
});

/* ---------------- report ---------------- */
function decClass(d){ return /no hire/i.test(d) ? "d-no" : /lean/i.test(d) ? "d-lean" : "d-hire"; }

async function openReport(roundId, live){
  showView("report");
  const round = await ipc.invoke("get_round", {roundId});
  $("#repCrumb").textContent = round.name;
  let rep = await ipc.invoke("get_report", {roundId});
  if (!rep){
    $("#repBody").innerHTML = `<div class="panel"><h2>Writing your feedback…</h2><p class="hint">The panel is reviewing the transcript. This takes 20–90 seconds.</p></div>`;
    try{ rep = await ipc.invoke("generate_report", {roundId}); }
    catch(e){
      $("#repBody").innerHTML = `<div class="banner err">${esc(errText(e))}</div><button class="btn primary" id="repRetry">Try again</button>`;
      $("#repRetry").onclick = () => openReport(roundId, live);
      return;
    }
  }
  renderReport(round, rep, live);
}

function renderReport(round, r, live){
  const dims = (r.dimensions||[]).map(d => `<div class="dim"><div class="row" style="justify-content:space-between"><b>${esc(d.name)}</b><span>${Number(d.score)||0} / 4</span></div><div class="bar"><i style="width:${(Number(d.score)||0)*25}%"></i></div><div class="ev">${esc(d.evidence)}</div></div>`).join("");
  const li = a => (a||[]).map(x => `<li>${esc(x)}</li>`).join("");
  const moments = (r.moments||[]).map(m => `<div class="moment"><div class="q">${esc(m.question)}</div><div class="said">You said: ${esc(m.said)}</div><div class="better"><b>Stronger answer:</b> ${esc(m.better)}</div></div>`).join("");
  const met = r.metrics || {};
  const inLoop = live && loop && loop.id === round.loop_id;
  const hasNext = inLoop && round.idx < loop.count - 1;
  const isLast = inLoop && round.idx === loop.count - 1 && loop.count > 1;
  $("#repBody").innerHTML = `
    <div class="report-head">
      <div class="big-score">${Number(r.overall)||0}<small>/100</small></div>
      <div>
        <span class="decision ${decClass(r.decision||"")}">${esc(r.decision||"—")}</span> <span class="hint">${esc(r.level_signal||"")}</span>
        <h2 style="font-size:24px;margin-top:6px">${esc(r.headline||"")}</h2>
        <div class="hint" style="margin-top:6px">${esc(round.name)} · with ${esc(round.persona)} · ${esc(new Date(round.started_at).toLocaleString())}</div>
      </div>
    </div>
    <div class="stack">
      <div class="fix"><b>Fix this first.</b> ${esc(r.fix_first||"")}</div>
      <div class="row">
        ${hasNext ? `<button class="btn primary" id="nextRound">Next round: ${esc(CAT.ROUNDS[loop.setup.rounds[round.idx+1]].name)}</button>` : ""}
        ${isLast ? `<button class="btn primary" id="verdictBtn">Get the hiring committee verdict</button>` : ""}
        ${inLoop ? `<button class="btn" id="retake">Retake this round</button>` : ""}
        <button class="btn" id="showTx">Show transcript</button>
      </div>
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
      <div class="panel hidden" id="txPanel"><h2>Transcript</h2><div class="transcript" style="max-height:none;padding:0">${(round.transcript||[]).map(m => `<div class="msg ${m.who==="me"?"me-msg":""}"><div class="from">${m.who==="me"?"You":esc(round.persona)}</div><div class="bubble">${md(m.text||"(shared workspace)")}</div></div>`).join("")}</div></div>
      <div id="verdict"></div>
    </div>`;
  const nb = $("#nextRound"); if (nb) nb.onclick = () => startRound(round.idx + 1);
  const vb = $("#verdictBtn"); if (vb) vb.onclick = () => loopVerdict(round.loop_id);
  const rt = $("#retake"); if (rt) rt.onclick = () => startRound(round.idx);
  $("#showTx").onclick = () => $("#txPanel").classList.toggle("hidden");
}

async function loopVerdict(loopId){
  const el = $("#verdict");
  el.innerHTML = `<div class="panel"><h2>Hiring committee is deliberating…</h2></div>`;
  try{
    const v = await ipc.invoke("loop_verdict", {loopId});
    el.innerHTML = `<div class="panel"><h2>Committee decision: ${esc(v.decision)}</h2><p>${esc(v.rationale)}</p><p><b>Strongest signal:</b> ${esc(v.strongest_signal)}</p><p><b>Biggest risk:</b> ${esc(v.biggest_risk)}</p><h3 style="font-size:17px;margin:10px 0 6px">Two-week plan</h3><ul class="list">${(v.plan||[]).map(p => `<li>${esc(p)}</li>`).join("")}</ul></div>`;
  }catch(e){ el.innerHTML = `<div class="banner err">${esc(errText(e))}</div>`; }
}
$("#repBack").onclick = () => showView("setup");

/* ---------------- history ---------------- */
async function renderHistory(){
  let h = [];
  try{ h = await ipc.invoke("history"); }catch(e){ console.error(e); }
  $("#history").innerHTML = h.length ? h.map(x => `<button data-id="${x.round_id}"><span><b>${esc(x.name)}</b><br><small>${esc(x.company)} · ${esc(new Date(x.started_at).toLocaleDateString())}${x.decision ? " · " + esc(x.decision) : ""}</small></span>${x.overall != null ? `<span class="score">${x.overall}</span>` : `<span class="pending">No feedback yet</span>`}</button>`).join("") : `<div class="hint">Your finished rounds and feedback appear here.</div>`;
  $$("#history button").forEach(b => b.onclick = () => openReport(Number(b.dataset.id), false));
}

/* ---------------- settings ---------------- */
function describe(loc){ return !loc ? "Not found yet" : loc.kind === "windows" ? loc.exe : "WSL: claude"; }
async function renderSettings(){
  const s = await ipc.invoke("get_settings");
  $("#clPath").textContent = describe(s.claude);
  $("#ivModel").value = s.interviewer_model; $("#repModel").value = s.report_model;
  $("#clResult").textContent = "";
}
$("#settingsBtn").onclick = () => showView("settings");
$("#setBack").onclick = () => showView("setup");
$("#clDetect").onclick = async () => {
  $("#clResult").textContent = "Looking…";
  try{ const loc = await ipc.invoke("detect_claude"); $("#clPath").textContent = describe(loc); $("#clResult").textContent = "Found."; }
  catch(e){ $("#clResult").textContent = errText(e); }
};
$("#clSavePath").onclick = async () => {
  const exe = $("#clExe").value.trim(); if (!exe) return;
  const s = await ipc.invoke("get_settings");
  s.claude = {kind: "windows", exe};
  await ipc.invoke("save_settings", {settings: s});
  $("#clPath").textContent = exe; $("#clResult").textContent = "Saved. Run the health check to test it.";
};
$("#clHealth").onclick = async () => {
  $("#clResult").innerHTML = "Asking Claude to reply with OK…";
  try{
    const h = await ipc.invoke("health_check");
    $("#clResult").innerHTML = `<span class="${h.ok ? "ok" : "bad"}">${h.ok ? "Working" : "Unexpected reply"}</span> · reply “${esc(h.reply)}” · ${(h.latency_ms/1000).toFixed(1)} s · ${esc(h.model)} · ${esc(h.location)}`;
  }catch(e){ $("#clResult").innerHTML = `<span class="bad">Failed</span> · ${esc(errText(e))}`; }
};
$("#saveModels").onclick = async () => {
  const s = await ipc.invoke("get_settings");
  s.interviewer_model = $("#ivModel").value; s.report_model = $("#repModel").value;
  await ipc.invoke("save_settings", {settings: s});
  $("#modelsMsg").textContent = "Saved.";
};

/* ---------------- init ---------------- */
(async function init(){
  try{ CAT = await ipc.invoke("get_catalog"); }
  catch(e){ document.body.innerHTML = `<div class="wrap"><div class="banner err">Could not load the catalog: ${esc(errText(e))}</div></div>`; return; }
  initSetup();
  showView("setup");
})();
