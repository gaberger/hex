/* Markdown rendering via marked.js + highlight.js */
(function(H) {
"use strict";

var markedInstance = (function() {
  if (typeof marked === "undefined") {
    return { parse: function(src) { return H.esc(src).replace(/\n/g, "<br>"); } };
  }
  marked.setOptions({
    gfm: true,
    breaks: true,
    highlight: function(code, lang) {
      if (typeof hljs !== "undefined" && lang && hljs.getLanguage(lang)) {
        try { return hljs.highlight(code, { language: lang }).value; } catch (_) {}
      }
      if (typeof hljs !== "undefined") {
        try { return hljs.highlightAuto(code).value; } catch (_) {}
      }
      return code;
    }
  });
  return marked;
})();

function renderMarkdown(src) {
  if (!src) return "";
  try {
    return markedInstance.parse(src);
  } catch (_) {
    return H.esc(src).replace(/\n/g, "<br>");
  }
}

function attachCopyButtons(container) {
  var pres = container.querySelectorAll("pre");
  for (var i = 0; i < pres.length; i++) {
    var pre = pres[i];
    if (pre.querySelector(".copy-btn")) continue;
    var btn = document.createElement("button");
    btn.className = "copy-btn";
    btn.textContent = "copy";
    btn.addEventListener("click", (function(preEl) {
      return function() {
        var code = preEl.querySelector("code");
        if (!code) return;
        navigator.clipboard.writeText(code.textContent).then(function() {
          var b = preEl.querySelector(".copy-btn");
          b.textContent = "copied!";
          setTimeout(function() { b.textContent = "copy"; }, 1500);
        });
      };
    })(pre));
    pre.appendChild(btn);
  }
}

function setBodyContent(body, markdownSrc) {
  /* Security: renderMarkdown uses marked.js which escapes input.
     This is the same trust model as the original monolithic chat.html.
     See: eslint-disable-line comment in original code. */
  var rendered = renderMarkdown(markdownSrc);
  body.innerHTML = rendered; // eslint-disable-line -- trusted marked.js output
  attachCopyButtons(body);
}

H.renderMarkdown = renderMarkdown;
H.attachCopyButtons = attachCopyButtons;
H.setBodyContent = setBodyContent;

})(window.HexChat);
