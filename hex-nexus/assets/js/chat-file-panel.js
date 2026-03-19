/* File Browser Panel */
(function(H) {
"use strict";
var dom = H.dom;

var fpState = {
  open: false,
  projectId: null,
  pathStack: [],
  currentPath: "",
  viewingFile: false,
  projectsCache: null
};

function toggleFilePanel() {
  fpState.open = !fpState.open;
  if (fpState.open) {
    dom.filePanel.classList.add("open");
    document.body.classList.add("has-file-panel");
    if (!fpState.projectId) fetchProjects();
  } else {
    dom.filePanel.classList.remove("open");
    document.body.classList.remove("has-file-panel");
  }
}

dom.filePanelToggle.addEventListener("click", toggleFilePanel);
dom.fpClose.addEventListener("click", function() {
  fpState.open = false;
  dom.filePanel.classList.remove("open");
  document.body.classList.remove("has-file-panel");
});

dom.fpBack.addEventListener("click", function() {
  if (fpState.viewingFile) {
    fpState.viewingFile = false;
    fetchBrowse(fpState.projectId, fpState.currentPath);
    return;
  }
  if (fpState.pathStack.length > 0) {
    fpState.currentPath = fpState.pathStack.pop();
    fetchBrowse(fpState.projectId, fpState.currentPath);
  }
});

function updateBackBtn() {
  dom.fpBack.disabled = fpState.pathStack.length === 0 && !fpState.viewingFile;
}

function fpShowMsg(cls, text) {
  dom.fpBody.textContent = "";
  var el = document.createElement("div");
  el.className = cls;
  el.textContent = text;
  dom.fpBody.appendChild(el);
}

function fetchProjects() {
  fpShowMsg("fp-loading", "Loading projects\u2026");
  fetch("/api/projects").then(function(r) { return r.json(); }).then(function(data) {
    var projects = data.projects || data;
    dom.fpBody.textContent = "";
    if (!projects || projects.length === 0) {
      fpShowMsg("fp-loading", "No projects registered");
      return;
    }
    fpState.projectsCache = projects;
    if (projects.length === 1) {
      fpState.projectId = projects[0].id;
      dom.fpBreadcrumb.textContent = projects[0].name + " /";
      fetchBrowse(fpState.projectId, "");
      return;
    }
    var tree = document.createElement("div");
    tree.className = "file-tree";
    projects.forEach(function(p) {
      var item = document.createElement("div");
      item.className = "file-tree-item dir";
      var icon = document.createElement("span");
      icon.className = "ft-icon";
      icon.textContent = "\uD83D\uDCC1";
      var nameEl = document.createElement("span");
      nameEl.className = "ft-name";
      nameEl.textContent = p.name || p.id;
      item.appendChild(icon);
      item.appendChild(nameEl);
      item.addEventListener("click", function() {
        fpState.projectId = p.id;
        fpState.pathStack = [];
        fpState.currentPath = "";
        dom.fpBreadcrumb.textContent = (p.name || p.id) + " /";
        fetchBrowse(p.id, "");
      });
      tree.appendChild(item);
    });
    dom.fpBody.appendChild(tree);
  }).catch(function(err) {
    fpShowMsg("fp-error", "Failed to load projects: " + err.message);
  });
}

function fetchBrowse(projectId, path) {
  fpState.viewingFile = false;
  updateBackBtn();
  fpShowMsg("fp-loading", "Loading\u2026");
  var url = "/api/" + encodeURIComponent(projectId) + "/browse?path=" + encodeURIComponent(path);
  fetch(url).then(function(r) {
    if (!r.ok) return r.json().then(function(d) { throw new Error(d.error || r.statusText); });
    return r.json();
  }).then(function(data) {
    dom.fpBody.textContent = "";
    fpState.currentPath = path;
    updateBreadcrumb(path);
    updateBackBtn();
    if (!data.entries || data.entries.length === 0) {
      fpShowMsg("fp-loading", "Empty directory");
      return;
    }
    var tree = document.createElement("div");
    tree.className = "file-tree";
    data.entries.forEach(function(entry) {
      var item = document.createElement("div");
      item.className = "file-tree-item " + entry.kind;
      var icon = document.createElement("span");
      icon.className = "ft-icon";
      icon.textContent = entry.kind === "dir" ? "\uD83D\uDCC1" : "\uD83D\uDCC4";
      var nameEl = document.createElement("span");
      nameEl.className = "ft-name";
      nameEl.textContent = entry.name;
      item.appendChild(icon);
      item.appendChild(nameEl);
      if (entry.kind === "file" && entry.size > 0) {
        var sizeEl = document.createElement("span");
        sizeEl.className = "ft-size";
        sizeEl.textContent = H.fmt(entry.size);
        item.appendChild(sizeEl);
      }
      item.addEventListener("click", function() {
        var newPath = path ? path + "/" + entry.name : entry.name;
        if (entry.kind === "dir") {
          fpState.pathStack.push(fpState.currentPath);
          fetchBrowse(projectId, newPath);
        } else {
          fpState.pathStack.push(fpState.currentPath);
          fetchFile(projectId, newPath);
        }
      });
      tree.appendChild(item);
    });
    dom.fpBody.appendChild(tree);
  }).catch(function(err) {
    fpShowMsg("fp-error", err.message || "Failed to browse");
    updateBackBtn();
  });
}

function fetchFile(projectId, path) {
  fpState.viewingFile = true;
  updateBackBtn();
  fpShowMsg("fp-loading", "Loading file\u2026");
  updateBreadcrumb(path);
  var url = "/api/" + encodeURIComponent(projectId) + "/read/" + path.split("/").map(encodeURIComponent).join("/");
  fetch(url).then(function(r) {
    if (!r.ok) return r.json().then(function(d) { throw new Error(d.error || r.statusText); });
    return r.json();
  }).then(function(data) {
    dom.fpBody.textContent = "";
    var preview = document.createElement("div");
    preview.className = "fp-preview";
    if (data.language === "markdown") {
      H.setBodyContent(preview, data.content);
    } else {
      var copyBtn = document.createElement("button");
      copyBtn.className = "copy-btn";
      copyBtn.textContent = "copy";
      copyBtn.style.cssText = "opacity:1;position:relative;float:right;margin-bottom:8px";
      copyBtn.addEventListener("click", function() {
        navigator.clipboard.writeText(data.content).then(function() {
          copyBtn.textContent = "copied!";
          setTimeout(function() { copyBtn.textContent = "copy"; }, 1500);
        });
      });
      preview.appendChild(copyBtn);
      var pre = document.createElement("pre");
      var codeEl = document.createElement("code");
      codeEl.textContent = data.content;
      if (typeof hljs !== "undefined" && data.language && data.language !== "text") {
        try {
          if (hljs.getLanguage(data.language)) {
            var highlighted = hljs.highlight(data.content, { language: data.language });
            codeEl.textContent = "";
            codeEl.insertAdjacentHTML("afterbegin", highlighted.value);
          }
        } catch (_) { /* leave as textContent */ }
      }
      pre.appendChild(codeEl);
      preview.appendChild(pre);
    }
    dom.fpBody.appendChild(preview);
  }).catch(function(err) {
    fpShowMsg("fp-error", err.message || "Failed to read file");
  });
}

function updateBreadcrumb(path) {
  var projName = "";
  if (fpState.projectsCache) {
    for (var i = 0; i < fpState.projectsCache.length; i++) {
      if (fpState.projectsCache[i].id === fpState.projectId) {
        projName = fpState.projectsCache[i].name || fpState.projectId;
        break;
      }
    }
  }
  dom.fpBreadcrumb.textContent = (projName || fpState.projectId || "") + " / " + (path || "");
}

})(window.HexChat);
