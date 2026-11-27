(() => {
  if (window.lynkReader) { window.lynkReader.leave(); return "off"; }
  const STYLE = /*STYLE*/;
  let module;
  /*READABILITY*/
  ;
  // Every picture is resolved on the live page first: currentSrc is what the
  // page actually chose after srcset and <picture>, and a clone of the
  // markup alone brings back lazy placeholders instead.
  const late = ["data-src", "data-original", "data-lazy-src", "data-lazy", "data-full-src", "data-hi-res-src", "data-image"];
  const live = [...document.images].map((img) => {
    let src = img.currentSrc || img.getAttribute("src") || "";
    if (!src || src.startsWith("data:") || (img.complete && img.naturalWidth <= 2)) {
      src = "";
      for (const a of late) { const v = img.getAttribute(a); if (v) { src = v; break; } }
    }
    return src;
  });
  const liveAll = [...document.querySelectorAll("*")];
  const copy = document.cloneNode(true);
  // A clone leaves shadow roots behind, and some pages keep their words in
  // one (MDN draws its code blocks that way). An element with nothing of its
  // own to show brings its open shadow root's contents instead.
  const copyAll = [...copy.querySelectorAll("*")];
  liveAll.forEach((el, i) => {
    const into = copyAll[i];
    if (!el.shadowRoot || !into || into.textContent.trim()) return;
    into.replaceChildren(...[...el.shadowRoot.childNodes].map((n) => copy.importNode(n, true)));
  });
  // Words kept for screen readers only ("skip past newsletter promotion")
  // are a pixel square off to the side on the page; in the reader they would
  // be a line of prose.
  liveAll.forEach((el, i) => {
    if (!(el instanceof HTMLElement) || el.offsetWidth > 1 || el.offsetHeight > 1) return;
    if (getComputedStyle(el).position !== "absolute" || !el.textContent.trim()) return;
    copyAll[i]?.remove();
  });
  [...copy.images].forEach((img, i) => {
    if (live[i]) {
      img.setAttribute("src", live[i]);
      img.removeAttribute("srcset");
      img.removeAttribute("sizes");
    } else if (img.getAttribute("data-srcset")) {
      img.setAttribute("srcset", img.getAttribute("data-srcset"));
    }
    img.removeAttribute("loading");
  });
  const article = new Readability(copy, { serializer: (el) => el, charThreshold: 400 }).parse();
  if (!article || !article.content || (article.length || 0) < 400) return "none";

  const host = document.createElement("div");
      const root = host.attachShadow({ mode: "closed" });
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(STYLE);
  root.adoptedStyleSheets = [sheet];

  const scroll = document.createElement("div");
  scroll.className = "scroll";
  scroll.tabIndex = -1;
  const main = document.createElement("main");
  if (article.lang) main.lang = article.lang;
  if (article.dir) main.dir = article.dir;
  const title = document.createElement("h1");
  title.className = "title";
  // The page's own headline when Readability's title is it plus the site's
  // name ("... - Web APIs | MDN").
  const headline = (document.querySelector("h1")?.innerText || "").trim();
  title.textContent = headline && (article.title || "").startsWith(headline) ? headline : article.title || document.title;
  const from = document.createElement("p");
  from.className = "from";
  from.textContent = location.host.replace(/^www\./, "");
  // A table Readability judged to be layout (old sites set a whole essay in
  // one) is its cells' contents, not a grid with padding around the prose.
  for (const t of [...article.content.querySelectorAll("table")]) {
    if (t._readabilityDataTable) continue;
    const cells = [...t.querySelectorAll("td, th")].filter((c) => c.closest("table") === t);
    t.replaceWith(...cells.flatMap((c) => [...c.childNodes]));
  }
  // Readability turns some layout tables into divs and leaves their rows and
  // cells behind; outside a table those are wrappers, and a cell's padding
  // would push the prose off the title's edge.
  for (const el of [...article.content.querySelectorAll("tbody, thead, tfoot, tr, td, th")].reverse()) {
    if (!el.parentElement || !el.parentElement.closest("table")) el.replaceWith(...el.childNodes);
  }
  // A page's own custom elements would upgrade again inside the reader and
  // run the page's code there (MDN's code blocks empty themselves and grow a
  // Copy button): each one becomes the plain element it stands for.
  for (const el of [...article.content.querySelectorAll("*")].reverse()) {
    if (!el.localName.includes("-")) continue;
    const plain = copy.createElement("div");
    plain.append(...el.childNodes);
    el.replaceWith(plain);
  }
  // A reader shows everything: a disclosure is its contents, its summary a
  // line of its own.
  for (const d of [...article.content.querySelectorAll("details")].reverse()) {
    const summary = d.querySelector(":scope > summary");
    if (summary) {
      const line = copy.createElement("p");
      line.append(...summary.childNodes);
      summary.replaceWith(line);
      if (!line.textContent.trim()) line.remove();
    }
    d.replaceWith(...d.childNodes);
  }
  const body = document.createElement("article");
  body.append(...[...article.content.childNodes].map((n) => document.importNode(n, true)));
  // Nothing of the page's own code comes along: no handler attributes, no
  // scripts Readability let through, no picture with nothing behind it.
  for (const el of body.querySelectorAll("*")) {
    for (const a of [...el.attributes]) if (/^on/i.test(a.name)) el.removeAttribute(a.name);
  }
  for (const el of body.querySelectorAll("script, style, link, noscript, form, button, input, select, textarea")) el.remove();
  for (const img of body.querySelectorAll("img")) {
    const src = img.getAttribute("src") || "";
    if (!src || src.startsWith("data:")) img.remove();
    else img.loading = "eager";
  }
  // A heading, or a picture of one, that repeats the title is the title twice.
  const said = title.textContent.trim().toLowerCase();
  const first = body.querySelector("h1, h2");
  if (first && first.textContent.trim().toLowerCase() === said) first.remove();
  for (const img of body.querySelectorAll("img[alt]")) {
    if (img.alt.trim().toLowerCase() === said) img.remove();
  }
  main.append(title, from, body);
  scroll.append(main);
  root.append(scroll);

  // The page keeps playing underneath otherwise, with nothing on screen to
  // stop it from.
  for (const m of document.querySelectorAll("video, audio")) if (!m.paused) m.pause();
  const overflow = document.documentElement.style.overflow;
  document.documentElement.style.overflow = "hidden";
  host.style.cssText = "position:fixed;inset:0;z-index:2147483647;";
  document.documentElement.append(host);
  // Escape leaves, before the page's own key handlers can act on a page that is not on screen.
  const onKey = (e) => {
    if (e.key !== "Escape" || e.isComposing) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    leave();
    window.webkit?.messageHandlers?.lynkReader?.postMessage("left");
  };
  const leave = () => {
    window.removeEventListener("keydown", onKey, true);
    host.remove();
    document.documentElement.style.overflow = overflow;
    delete window.lynkReader;
  };
  window.addEventListener("keydown", onKey, true);
  window.lynkReader = { leave };
  scroll.focus({ preventScroll: true });
  return "on";
})()
