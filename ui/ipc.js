"use strict";
/* Thin wrapper over the Tauri globals (withGlobalTauri: true). Failures are logged to the app log.
   Named `api`, not `ipc`: Tauri owns a read-only window.ipc on Windows. */
(function(){
  const T = window.__TAURI__;
  const log = (level, message) => { try{ T.core.invoke("ui_log", {level, message: String(message).slice(0, 4000)}); }catch(e){} };

  /* Slow-action pill: a call that runs past ~0.7 s shows "Still working..." in the corner.
     Calls that already show their own progress (rounds, research, ATS, reports) or that poll often stay silent. */
  const SILENT = new Set(["ui_log", "voice_status", "start_round", "send_answer", "end_round", "run_code", "project_diff",
    "generate_report", "research_start", "research_jobs", "research_cancel", "research_retry", "research_dismiss",
    "ats_check", "ats_screen", "ats_last", "voice_model_download", "voice_builtin_preview", "voice_finish",
    "voice_interrupt", "voice_mute", "voice_cancel_listen", "voice_launch", "voice_models_reload", "cancel_start"]);
  let active = 0, timer = null;
  const pill = () => document.getElementById("busy");
  const show = () => { timer = null; const p = pill(); if (p && active > 0) p.hidden = false; };
  const busyStart = () => { active++; if (!timer) timer = setTimeout(show, 700); };
  const busyEnd = () => {
    active = Math.max(0, active - 1);
    if (active === 0){ clearTimeout(timer); timer = null; const p = pill(); if (p) p.hidden = true; }
  };

  window.hideBoot = () => { const b = document.getElementById("boot"); if (b) b.hidden = true; };
  /* Safety net: if startup throws before the first screen is ready, do not leave the overlay up forever. */
  setTimeout(window.hideBoot, 15000);

  window.api = {
    async invoke(cmd, args){
      const track = !SILENT.has(cmd);
      if (track) busyStart();
      try{ return await T.core.invoke(cmd, args); }
      catch(e){ log("error", `invoke ${cmd} failed: ${typeof e === "string" ? e : JSON.stringify(e)}`); throw e; }
      finally{ if (track) busyEnd(); }
    },
    listen(event, cb){ return T.event.listen(event, e => cb(e.payload)); },
    log,
  };
  window.addEventListener("error", e => log("error", `${e.message} at ${e.filename}:${e.lineno}`));
  window.addEventListener("unhandledrejection", e => log("error", `unhandled: ${e.reason && (e.reason.stack || e.reason)}`));
})();
