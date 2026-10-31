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
