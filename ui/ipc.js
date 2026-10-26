"use strict";
/* Thin wrapper over the Tauri globals (withGlobalTauri: true). */
window.ipc = {
  invoke(cmd, args){ return window.__TAURI__.core.invoke(cmd, args); },
  listen(event, cb){ return window.__TAURI__.event.listen(event, e => cb(e.payload)); },
};
