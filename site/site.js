// Fills the download links with the installer of the latest published release, and shows its version, date and size.
// Without JavaScript or when GitHub does not answer, the links stay on the releases page.
(function () {
  "use strict";

  var REPO = "Gronck2/rgb-tray";
  var lang = document.documentElement.lang;

  // The language menu closes on Esc and on a click elsewhere.
  var langs = document.querySelector(".langs");
  if (langs) {
    document.addEventListener("click", function (e) {
      if (langs.open && !langs.contains(e.target)) langs.open = false;
    });
    langs.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && langs.open) {
        langs.open = false;
        langs.querySelector("summary").focus();
      }
    });
  }

  fetch("https://api.github.com/repos/" + REPO + "/releases/latest", { headers: { Accept: "application/vnd.github+json" } })
    .then(function (r) {
      return r.ok ? r.json() : null;
    })
    .then(function (rel) {
      if (!rel || !rel.assets) return;
      var setup = null;
      for (var i = 0; i < rel.assets.length; i++) {
        var n = rel.assets[i].name.toLowerCase();
        if (n.indexOf("setup") >= 0 && n.slice(-4) === ".exe") {
          setup = rel.assets[i];
          break;
        }
      }
      if (!setup) return;
      document.querySelectorAll('[data-asset="setup"]').forEach(function (a) {
        a.href = setup.browser_download_url;
      });
      var version = (rel.tag_name || "").replace(/^v/, "");
      var date = rel.published_at ? new Date(rel.published_at) : null;
      var mb = setup.size ? (setup.size / 1048576).toLocaleString(lang, { maximumFractionDigits: 1 }) : null;
      document.querySelectorAll("[data-release]").forEach(function (el) {
        var parts = [el.getAttribute("data-release").replace("{version}", version)];
        if (date) parts.push(date.toLocaleDateString(lang, { year: "numeric", month: "long", day: "numeric" }));
        if (mb) parts.push(el.getAttribute("data-size").replace("{mb}", mb));
        el.textContent = parts.join(" · ");
        el.hidden = false;
      });
    })
    .catch(function () {});

  // Screenshots open large in a dialog; arrows, the arrow keys and a swipe go through them, Esc,
  // the close button or a click beside the picture close it. Without JavaScript the links open the PNG.
  var dialog = document.querySelector(".lightbox");
  var shots = Array.prototype.slice.call(document.querySelectorAll("a.zoom"));
  if (!dialog || !dialog.showModal || !shots.length) return;
  var big = dialog.querySelector("img");
  var caption = dialog.querySelector("figcaption");
  var count = dialog.querySelector(".lb-count");
  var current = 0;

  function show(i) {
    current = (i + shots.length) % shots.length;
    var a = shots[current];
    var img = a.querySelector("img");
    var fig = a.closest("figure");
    var cap = fig && fig.querySelector("figcaption");
    big.src = a.href;
    big.alt = img.alt;
    caption.textContent = cap ? cap.textContent : img.alt;
    count.textContent = current + 1 + " / " + shots.length;
  }

  shots.forEach(function (a, i) {
    a.addEventListener("click", function (e) {
      e.preventDefault();
      show(i);
      dialog.showModal();
    });
  });
  dialog.querySelector(".lb-prev").addEventListener("click", function () {
    show(current - 1);
  });
  dialog.querySelector(".lb-next").addEventListener("click", function () {
    show(current + 1);
  });
  dialog.querySelector(".lb-close").addEventListener("click", function () {
    dialog.close();
  });
  dialog.addEventListener("click", function (e) {
    if (e.target === dialog || e.target.tagName === "FIGURE") dialog.close();
  });
  dialog.addEventListener("keydown", function (e) {
    if (e.key === "ArrowLeft") show(current - 1);
    else if (e.key === "ArrowRight") show(current + 1);
  });
  dialog.addEventListener("close", function () {
    shots[current].focus();
  });
  var startX = null;
  dialog.addEventListener(
    "touchstart",
    function (e) {
      startX = e.touches.length === 1 ? e.touches[0].clientX : null;
    },
    { passive: true }
  );
  dialog.addEventListener("touchend", function (e) {
    if (startX === null) return;
    var dx = e.changedTouches[0].clientX - startX;
    startX = null;
    if (Math.abs(dx) > 50) show(dx < 0 ? current + 1 : current - 1);
  });
})();
