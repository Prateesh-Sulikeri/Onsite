"use strict";
/* Thin wrapper over the Tauri globals (withGlobalTauri: true). Failures are logged to the app log.
   Named `api`, not `ipc`: Tauri owns a read-only window.ipc on Windows. */
(function(){
  const T = window.__TAURI__;
  const log = (level, message) => { try{ T.core.invoke("ui_log", {level, message: String(message).slice(0, 4000)}); }catch(e){} };
  window.api = {
    async invoke(cmd, args){
      try{ return await T.core.invoke(cmd, args); }
      catch(e){ log("error", `invoke ${cmd} failed: ${typeof e === "string" ? e : JSON.stringify(e)}`); throw e; }
    },
    listen(event, cb){ return T.event.listen(event, e => cb(e.payload)); },
    log,
  };
  window.addEventListener("error", e => log("error", `${e.message} at ${e.filename}:${e.lineno}`));
  window.addEventListener("unhandledrejection", e => log("error", `unhandled: ${e.reason && (e.reason.stack || e.reason)}`));
})();
