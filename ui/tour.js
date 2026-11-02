/* First-run help: the "Getting started" checklist on the Dashboard and a short guided tour.
   Loaded after app.js and uses its globals ($, LS, nav, aiReady, wantsVoice, VS, SETTINGS, words, esc).
   Both are per-viewer conveniences kept in localStorage, so nothing here needs the backend. */

/* ---------------- getting-started checklist ---------------- */
const START_KEY = "onsite.startHidden";

function startItems(h){
  const aiOk = aiReady();
  const voiceOk = !wantsVoice() || !!(VS && VS.up);
  const resumeOk = words($("#resume").value) > 0 || (typeof RESUME_FILE !== "undefined" && !!RESUME_FILE) || h.length > 0;
  return [
    {done: aiOk, title: "Connect an AI model", text: aiOk ? "Connected." : "The interviewer needs one. Claude Code, or a free Gemini or Groq key, all work.",
      btn: "Connect", go: ["settings", "models"]},
    {done: voiceOk, title: "Get a voice, or switch to typing", text: voiceOk ? (wantsVoice() ? "Voice is ready." : "Typed interviews. You can turn voice on later.") : "Spoken rounds need a voice. Download the built-in one, or switch to typed rounds.",
      btn: "Set up voice", go: ["settings", "voice"]},
    {done: resumeOk, title: "Add and check your resume", text: resumeOk ? "Resume added." : "Every interview starts from your resume. PDF or Word works best.",
      btn: "Add resume", go: ["resume"]},
    {done: h.length > 0, title: "Do your first interview", text: h.length ? "Done. Your results are on this page." : "A 10-minute round is enough to start. You get written feedback after.",
      btn: "Start", go: ["interviews"]},
  ];
}

function renderStart(h){
  const box = $("#dashStart");
  if (!box) return;
  const items = startItems(h || []);
  const left = items.filter(i => !i.done).length;
  if (!left || LS.get(START_KEY, false)){ box.hidden = true; return; }
  box.hidden = false;
  box.innerHTML = `
    <div class="panel-head"><h2>Getting started <small class="muted">${items.length - left} of ${items.length} done</small></h2>
      <span><button type="button" class="btn ghost sm" id="tourOpen">Take the tour</button> <button type="button" class="linkbtn" id="startHide">Hide</button></span></div>
    <ol class="start-list">${items.map((i, n) => `
      <li class="${i.done ? "done" : ""}">
        <span class="start-dot">${i.done ? icon("check") : n + 1}</span>
        <span class="start-tx"><b>${esc(i.title)}</b><small>${esc(i.text)}</small></span>
        ${i.done ? "" : `<button type="button" class="btn sm" data-startgo="${n}">${esc(i.btn)}</button>`}
      </li>`).join("")}</ol>`;
  box.querySelectorAll("[data-startgo]").forEach(b => b.onclick = () => {
    const [view, sec] = items[Number(b.dataset.startgo)].go;
    nav(view);
    if (sec) openSettingsSection(sec);
  });
  $("#tourOpen").onclick = () => startTour();
  $("#startHide").onclick = () => { LS.set(START_KEY, true); box.hidden = true; toast("Hidden. Find it under “Take the tour” in the sidebar.", "ok"); };
}

/* Settings opens a section when a nav link carries data-sec; do the same from code. */
function openSettingsSection(sec){
  const a = document.querySelector(`[data-nav="settings"][data-sec="${sec}"]`);
  if (a) a.click();
}

/* ---------------- guided tours ---------------- */
/* One tour per screen. A step with no `sel` (or whose target is hidden) shows as a centred card.
   `pre` runs before the step is shown (e.g. opens a Settings section). */
const TOURS = {
  app: [
    {sel: '.nav-item[data-nav="dashboard"]', title: "Dashboard", text: "Your progress at a glance: scores per skill, your weekly goal and a practice calendar. It fills in as you do rounds."},
    {sel: '.nav-item[data-nav="interviews"]', title: "Interviews: start here", text: "Pick your rounds, add your resume and begin. Answer out loud or type. Every screen on the way has its own “Show me around” button."},
    {sel: '.nav-item[data-nav="resume"]', title: "Resume check", text: "Shows what an applicant tracking system (ATS) really reads from your file. If your resume won’t parse, you see why and how to fix it."},
    {sel: '.nav-item[data-nav="dossiers"]', title: "Company research", text: "Practise for one company. Onsite researches how they interview and builds rounds to match. It takes a few minutes."},
    {sel: '.nav-item[data-nav="feedback"]', title: "Past rounds", text: "Every finished round is saved with a written report: what went well, what to fix, and a score per skill."},
    {sel: "#sysClaude", title: "Status lights", text: "Green means ready. Red means something needs fixing. Click a light and it opens the right setting."},
    {sel: '.nav-item[data-nav="settings"]', title: "Settings", text: "Choose the AI model, the voice and the look. Your keys and data stay on this computer."},
  ],
  setup: [
    {sel: ".stepper", pre: () => showStage(1), title: "Five short steps", text: "Experience, rounds, resume, time, then a mic check. Next and Back move between them, and you can click a finished step to change it. Nothing starts until the last step."},
    {sel: "#quickStart", pre: () => showStage(1), title: "In a hurry?", text: "Quick start sets up one 10-minute coding round. You only need to add your resume."},
    {sel: ".modes", pre: () => showStage(1), title: "Two ways to practise", text: "General practice: pick any rounds and start right away. Company-specific: runs a real company’s loop from a dossier. For that, make a dossier first under Company research."},
    {sel: '[data-stage="1"] .step', pre: () => showStage(1), title: "Your experience", text: "Level sets how hard the questions are and what a good answer looks like. Language is for coding rounds. The defaults are fine. Open “Interviewer” to make the interviewer easier or harder."},
    {sel: "#roundList", pre: () => showStage(2), title: "Choose your rounds", text: "Pick one or more: coding, system design, behavioural and more. They run in the order shown on the right. Two or three rounds give sharper feedback than a whole loop."},
    {sel: "#dropResume", pre: () => showStage(3), title: "Add your resume", text: "Every interview starts from your resume. Use a PDF, Word or text file, or paste the text. If a file can’t be read, this box tells you why and what to do."},
    {sel: "#dropJd", pre: () => showStage(3), title: "Job description", text: "Optional. Add the posting and the questions become more specific to that job."},
    {sel: "#roundLen", pre: () => showStage(4), title: "How long?", text: "“Real length” runs each round as long as a real one. Pick 10 minutes for a quick first try."},
    {sel: "#startBtn", pre: () => showStage(4), title: "On to the mic check", text: "Onsite checks your resume, then opens the mic and speaker check. After that you join, and the room opens."},
    {sel: ".loop-card", pre: () => showStage(4), title: "Your loop", text: "The rounds you picked, in order, with the total time."},
  ],
  ready: [
    {sel: "#audioCheck", title: "Check your mic and speaker", text: "Press Test speaker, then Test mic and say something. The bar should move. If not, pick another device here."},
    {sel: "#readyTalk", title: "How you answer", text: "Spoken: you both talk. Listen & type: the interviewer talks and you type. Typed: chat only, no sound at all."},
    {sel: "#readyJoin", title: "Join the interview", text: "Onsite loads the interviewer and the voice first. That takes a few seconds, then the room opens."},
    {title: "In the room: talking", text: "Just answer out loud. When you stop for a moment, your answer is sent. How long that pause is can be set in Settings > Voice. Tap the round mic button to send at once, or to cut in while the interviewer is talking."},
    {title: "In the room: your workspace", text: "On the right are an Editor for code (Run to test it, Submit when done), a Whiteboard for diagrams and Notes. The interviewer sees your work when you press Share now or send an answer."},
    {title: "In the room: ending", text: "End interview finishes the round and gets your feedback. Terminate stops right away and saves nothing. Mute keeps what you said as text, so you can edit it before sending."},
  ],
  room: [
    {sel: "#timer", title: "Time left", text: "The time left in this round. When it runs out the interviewer wraps up."},
    {sel: "#orb", title: "The mic button", text: "Shows who has the floor. Tap it to send your answer now, or to cut in while the interviewer is talking."},
    {sel: "#muteBtn", title: "Mute", text: "Stops the mic. Words already heard go into the typing box so you can edit them."},
    {sel: ".composer", title: "Type instead", text: "Type an answer and press Enter. This works in every mode."},
    {sel: ".work .tabs", title: "Your workspace", text: "Editor for code, Whiteboard for diagrams, Notes for scratch work. “From interviewer” appears when they share code with you."},
    {sel: "#sendWork", title: "Share your work", text: "The interviewer reads your code, notes and drawing when you send an answer. Press Share now to show them without answering."},
    {sel: "#endBtn", title: "End interview", text: "Finishes the round and builds your written feedback."},
    {sel: "#termBtn", title: "Terminate", text: "Stops at once. No feedback and nothing saved. Use End interview instead if you want a report."},
    {sel: "#rail", title: "Round context", text: "The plan for this round and a few tips. Hide it with the Context button when you need room."},
  ],
  settings: [
    {sel: "#setNav", title: "Settings sections", text: "Most people only need AI model and Voice. The rest is look, updates, privacy and storage."},
    {sel: '.set-sec[data-sec="models"]', pre: () => setSection("models"), title: "AI model", text: "Choose what runs the interviewer: Claude Code (your Claude login), or your own key for Claude API, OpenAI, Gemini or Groq. Gemini and Groq have free tiers. Keys stay on this computer."},
    {sel: '.set-sec[data-sec="voice"]', pre: () => setSection("voice"), title: "Voice", text: "The built-in voice works offline after a one-time download. VoiceStudio is the other engine. You also set the pause that ends your answer, and your mic and speaker."},
    {sel: '.set-sec[data-sec="appearance"]', pre: () => setSection("appearance"), title: "Appearance", text: "Themes, light or dark, and a background image."},
  ],
};
let tour = null;

const visible = sel => { const el = sel && document.querySelector(sel); if (!el) return null; const r = el.getBoundingClientRect(); return r.width > 0 && r.height > 0 ? el : null; };

function startTour(key = "app", i = 0){
  endTour(false);
  const steps = TOURS[key];
  if (!steps) return;
  LS.set("onsite.tourSeen." + key, true);
  tour = {key, steps, i, el: document.createElement("div")};
  tour.el.className = "tour";
  tour.el.innerHTML = `<div class="tour-hole"></div>
    <div class="tour-card" role="dialog" aria-modal="true" aria-labelledby="tourTitle">
      <small class="tour-step"></small><h3 id="tourTitle"></h3><p></p>
      <div class="tour-actions"><button type="button" class="linkbtn" data-skip>Skip tour</button><span><button type="button" class="btn ghost sm" data-back>Back</button> <button type="button" class="btn primary sm" data-next>Next</button></span></div>
    </div>`;
  document.body.appendChild(tour.el);
  tour.el.querySelector("[data-skip]").onclick = () => endTour();
  tour.el.querySelector("[data-back]").onclick = () => tourGo(tour.i - 1);
  tour.el.querySelector("[data-next]").onclick = () => tour.i >= tour.steps.length - 1 ? endTour() : tourGo(tour.i + 1);
  document.addEventListener("keydown", tourKey, true);
  window.addEventListener("resize", tourPlace);
  tourGo(i);
}

function tourKey(e){
  if (!tour) return;
  if (e.key === "Escape"){ e.preventDefault(); e.stopPropagation(); endTour(); }
  else if (e.key === "ArrowRight"){ e.preventDefault(); tour.el.querySelector("[data-next]").click(); }
  else if (e.key === "ArrowLeft"){ e.preventDefault(); tour.el.querySelector("[data-back]").click(); }
}

function tourGo(i){
  if (!tour) return;
  tour.i = Math.max(0, Math.min(tour.steps.length - 1, i));
  const s = tour.steps[tour.i], c = tour.el.querySelector(".tour-card");
  try{ if (s.pre) s.pre(); }catch(e){}
  c.querySelector(".tour-step").textContent = `${tour.i + 1} of ${tour.steps.length}`;
  c.querySelector("h3").textContent = s.title;
  c.querySelector("p").textContent = s.text;
  c.querySelector("[data-back]").hidden = tour.i === 0;
  c.querySelector("[data-next]").textContent = tour.i === tour.steps.length - 1 ? "Done" : "Next";
  const t = visible(s.sel);
  if (t) t.scrollIntoView({block: "nearest"});
  // Wait a frame so a section opened by `pre` has its size.
  requestAnimationFrame(() => { tourPlace(); c.querySelector("[data-next]").focus({preventScroll: true}); });
}

/* Cuts the spotlight around the target and puts the card beside it (below or above on a narrow window). */
function tourPlace(){
  if (!tour) return;
  const s = tour.steps[tour.i], t = visible(s.sel);
  const hole = tour.el.querySelector(".tour-hole"), card = tour.el.querySelector(".tour-card");
  if (!t){
    // No target: dim the screen and centre the card.
    Object.assign(hole.style, {left: "50%", top: "50%", width: "0px", height: "0px"});
    card.style.left = "50%"; card.style.top = "50%"; card.style.transform = "translate(-50%,-50%)";
    return;
  }
  const r = t.getBoundingClientRect(), pad = 6;
  Object.assign(hole.style, {left: r.left - pad + "px", top: r.top - pad + "px", width: r.width + pad * 2 + "px", height: r.height + pad * 2 + "px"});
  card.style.transform = "";
  const cw = card.offsetWidth, ch = card.offsetHeight, vw = innerWidth, vh = innerHeight;
  let x = r.right + 18, y = r.top + r.height / 2 - ch / 2;
  if (x + cw > vw - 12){
    x = Math.max(12, Math.min(vw - cw - 12, r.left));
    y = r.bottom + 16 + ch > vh - 12 ? Math.max(12, r.top - ch - 16) : r.bottom + 16;
  }
  y = Math.max(12, Math.min(vh - ch - 12, y));
  card.style.left = x + "px"; card.style.top = y + "px";
}

function endTour(){
  if (!tour) return;
  tour.el.remove(); tour = null;
  document.removeEventListener("keydown", tourKey, true);
  window.removeEventListener("resize", tourPlace);
}

/* ---------------- entry points ---------------- */
/* Which tour belongs to which screen, and where its button and first-visit offer go. */
const TOUR_SCREENS = {
  interviews: {key: "setup", head: "#view-interviews .page-head", offer: "Setting up an interview has a few steps. Want a quick look at each?"},
  ready: {key: "ready", head: "#view-ready .page-head", offer: "First time? A short tour covers the mic check and how the interview room works."},
  settings: {key: "settings", head: "#view-settings .page-head", offer: "Most people only need AI model and Voice. Want a quick look?"},
  room: {key: "room", head: "#view-room .room-meta", offer: "New to the room? A short tour shows the controls. The timer keeps running."},
};

for (const [view, cfg] of Object.entries(TOUR_SCREENS)){
  const head = document.querySelector(cfg.head);
  if (!head) continue;
  const b = document.createElement("button");
  b.type = "button"; b.className = "btn tour-help"; b.title = "A short tour of this screen";
  b.innerHTML = icon("bulb") + `<span>${view === "room" ? "Tour" : "New here? Show me around"}</span>`;
  b.onclick = () => startTour(cfg.key);
  if (view === "room"){ head.insertBefore(b, head.firstChild); continue; }
  // Page headers: the title on the left, every button grouped on the right.
  let box = head.querySelector(":scope > .head-actions");
  if (!box){
    box = document.createElement("div"); box.className = "head-actions";
    [...head.children].filter(c => c.tagName === "BUTTON").forEach(c => box.appendChild(c));
    head.appendChild(box);
  }
  box.appendChild(b);
}

/* A small strip, once per screen, the first time it opens. Never blocks the screen. */
function offerTour(view){
  const cfg = TOUR_SCREENS[view];
  if (!cfg || tour || LS.get("onsite.tourSeen." + cfg.key, false) || LS.get("onsite.tourOff", false)) return;
  const host = document.getElementById("view-" + view);
  if (!host || host.hidden || host.querySelector(".tour-offer")) return;
  const el = document.createElement("div");
  el.className = "tour-offer"; el.setAttribute("role", "region"); el.setAttribute("aria-label", "Tour");
  el.innerHTML = `<span class="tour-offer-ic">${icon("bulb")}</span><span class="tour-offer-tx">${esc(cfg.offer)}</span>
    <span class="tour-offer-btns"><button type="button" class="btn primary sm" data-yes>Show me</button> <button type="button" class="linkbtn" data-no>Not now</button> <button type="button" class="linkbtn" data-off>Don’t offer tours</button></span>`;
  const after = host.querySelector(".page-head, .room-head");
  after ? after.after(el) : host.prepend(el);
  const close = () => { LS.set("onsite.tourSeen." + cfg.key, true); el.remove(); };
  el.querySelector("[data-yes]").onclick = () => { el.remove(); startTour(cfg.key); };
  el.querySelector("[data-no]").onclick = close;
  el.querySelector("[data-off]").onclick = () => { LS.set("onsite.tourOff", true); el.remove(); };
}

/* Offer after the screen is drawn: wrap nav() from app.js. */
{
  const base = nav;
  nav = function(view){ base.apply(this, arguments); if (TOUR_SCREENS[view]) setTimeout(() => offerTour(view), 400); };
}

document.getElementById("helpTour")?.addEventListener("click", () => startTour("app"));
