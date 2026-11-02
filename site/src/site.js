// Points the main Download button at the file for the visitor's system. Without JS it stays on Windows,
// and "Other systems" lists every file.
(() => {
  const btn = document.getElementById("get");
  const label = document.getElementById("get-label");
  if (!btn || !label) return;
  const ua = navigator.userAgent;
  const platform = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.platform || "";
  const mobile = /Android|iPhone|iPad|iPod/i.test(ua);
  let os = null;
  if (mobile) os = null;
  else if (/Mac/i.test(platform) || /Macintosh/i.test(ua)) os = "mac";
  else if (/Win/i.test(platform) || /Windows/i.test(ua)) os = "win";
  else if (/Linux|X11/i.test(platform) || /Linux|X11/i.test(ua)) os = "linux";

  const names = { win: "Windows", mac: "macOS", linux: "Linux" };
  if (os) {
    btn.href = btn.dataset[os];
    label.textContent = "Download for " + names[os];
    const card = document.querySelector('.dl[data-os="' + (os === "linux" ? "appimage" : os) + '"]');
    if (card) card.classList.add("yours");
  } else {
    // Phones and unknown systems: Onsite is a desktop app, so show the list instead of guessing.
    btn.href = "#download";
    label.textContent = "Get it for your computer";
  }
})();

// Until the app is code-signed: explain the system warning and how to check the file before a release download
// starts. Only plain clicks on release files; Ctrl/middle click and no-JS visitors download directly.
(() => {
  const dlg = document.getElementById("dlg");
  if (!dlg || typeof dlg.showModal !== "function") return;
  const $ = id => document.getElementById(id);
  const osOf = name => /\.exe$/i.test(name) ? "win" : /\.dmg$/i.test(name) ? "mac" : "linux";
  const command = (os, name) => os === "win" ? `(Get-FileHash "$HOME\\Downloads\\${name}").Hash.ToLower()`
    : os === "mac" ? `shasum -a 256 ~/Downloads/${name}` : `sha256sum ~/Downloads/${name}`;

  document.addEventListener("click", e => {
    const a = e.target.closest && e.target.closest("a[href]");
    if (!a || dlg.contains(a) || !/^https:\/\/github\.com\/[^/]+\/[^/]+\/releases\/download\/.+\.(exe|dmg|AppImage|deb)$/i.test(a.href)) return;
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    const name = decodeURIComponent(a.href.split("/").pop());
    const os = osOf(name);
    $("dlgName").textContent = name;
    $("dlgCmd").textContent = command(os, name);
    $("dlgCopy").textContent = "Copy";
    $("dlgGo").href = a.href;
    if (a.dataset.sums) $("dlgSums").href = a.dataset.sums; // that release's checksums
    for (const p of dlg.querySelectorAll(".dlg-os")) p.hidden = p.dataset.os !== os;
    dlg.showModal();
    $("dlgGo").focus();
  });

  for (const b of dlg.querySelectorAll("[data-close]")) b.addEventListener("click", () => dlg.close());
  $("dlgGo").addEventListener("click", () => dlg.close()); // the link itself starts the download
  dlg.addEventListener("click", e => { if (e.target === dlg) dlg.close(); }); // backdrop
  $("dlgCopy").addEventListener("click", async () => {
    try { await navigator.clipboard.writeText($("dlgCmd").textContent); $("dlgCopy").textContent = "Copied"; }
    catch { $("dlgCopy").textContent = "Select and copy"; }
  });
})();
