"use strict";
/* Applies the saved theme before first paint (loaded in <head>; the CSP allows no inline script).
   Saved in localStorage as {family, mode, image, dim}: family is onsite | catppuccin | rosepine |
   gruvbox, mode is system | light | dark. The custom image itself lives in Rust (app data). */
(function(){
  const KEY = "onsite.theme";
  const FAMILIES = ["onsite", "catppuccin", "rosepine", "gruvbox"];
  const sys = window.matchMedia("(prefers-color-scheme: dark)");
  let image = null;   // data URL of the custom background, once loaded
  function read(){
    let t = {};
    try{ t = JSON.parse(localStorage.getItem(KEY)) || {}; }catch(e){}
    return {
      family: FAMILIES.includes(t.family) ? t.family : "onsite",
      // Dark by default: it's how Onsite looked before themes existed.
      mode: ["system", "light", "dark"].includes(t.mode) ? t.mode : "dark",
      image: !!t.image,
      dim: Number.isFinite(t.dim) ? Math.min(85, Math.max(0, t.dim)) : 35,
    };
  }
  function apply(t){
    t = t || read();
    const mode = t.mode === "system" ? (sys.matches ? "dark" : "light") : t.mode;
    const r = document.documentElement;
    r.dataset.theme = `${t.family}-${mode}`;
    r.dataset.mode = mode;
    if (t.image && image){
      const veil = `color-mix(in srgb, var(--bg) ${t.dim}%, transparent)`;
      r.style.setProperty("--backdrop", `linear-gradient(${veil}, ${veil}), url("${image}")`);
      r.style.setProperty("--backdrop-pos", "center");
    } else if (t.image){
      // The image is still loading: show nothing rather than flash the theme's own.
      r.style.setProperty("--backdrop", "none");
    } else {
      r.style.removeProperty("--backdrop");
      r.style.removeProperty("--backdrop-pos");
    }
    return t;
  }
  apply();
  sys.addEventListener("change", () => apply());
  window.OnsiteTheme = {
    FAMILIES, read, apply,
    save(t){ try{ localStorage.setItem(KEY, JSON.stringify(t)); }catch(e){} return apply(t); },
    setImage(dataUrl){ image = dataUrl || null; return apply(); },
  };
})();
