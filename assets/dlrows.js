/* DlRows (0.34) — the ONE downloads row renderer + command
   binder, spliced by core::assets into BOTH the chrome
   document (the downloads popover) and the downloads page.
   There is no second copy: assemble_chrome_html and
   assemble_downloads_html replace the same token with these
   exact bytes.
   Snapshot law: DlRows consumes THE downloads snapshot
     {version, tasks: [{id, name, host, status, total,
       downloaded, percent, size_text, detail_text,
       speed_bps, eta_secs,
       connections, error, url, added_ms, sha256, hint,
       retry_text, retry_pending, refreshing}],
      active_count, failed_unseen,
      settings: {default_connections, max_concurrent,
                 speed_limit_kbps},
      notice: {seq, kind, name, reason, open} | null}
   normalizeSnapshot also accepts the 0.33 legacy shapes
   (camelCase task fields / a bare task array under
   `downloads`) so older pushes keep rendering during the
   transition — the canonical field names win when both are
   present.
   Command law: every task gesture carries data-dl-action /
   data-dl-id attributes and is posted by the ONE delegated
   handler in bindCommands through the surface's own `post`
   function, which wraps the payload as the canonical body
   {"kind":"dl","action":"<verb>", ...args}. The verb set is
   frozen: open-panel, close-panel, open-page, pause, resume,
   cancel, restart, refresh, remove, delete-file, open-file,
   open-folder, copy-link (client-only — clipboard, never
   posted), add-url, set-connections, set-default-connections,
   set-max-concurrent, set-speed-limit, dismiss-notice.
   Escaping law (SECURITY.md): renderTaskList returns an HTML
   string, so EVERY server-supplied string interpolated into
   it passes escapeHtml first. Task names/URLs are network
   data; nothing reaches innerHTML raw. The only unescaped
   interpolations are numbers the renderer itself coerces
   (id, percent, connections) and the static icon constants
   below. */
var DlRows = (function () {
  "use strict";

  var CONN_CHOICES = [1, 2, 4, 8, 16, 32];
  /* Row-action icons: STATIC markup only (constants, like the
     tab close glyph) — never built from task data. */
  var ICON_PAUSE = '<svg viewBox="0 0 24 24" class="ic"><path d="M8 5v14M16 5v14"/></svg>';
  var ICON_PLAY = '<svg viewBox="0 0 24 24" class="ic"><path d="M8 5l11 7-11 7z"/></svg>';
  var ICON_FOLDER = '<svg viewBox="0 0 24 24" class="ic"><path d="M3 8a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>';
  var ICON_OPEN = '<svg viewBox="0 0 24 24" class="ic"><path d="M6 3h7l5 5v13H6z"/><path d="M13 3v5h5"/></svg>';

  function escapeHtml(v) {
    return String(v === null || v === undefined ? "" : v)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");
  }

  function pick(obj, snake, camel) {
    if (!obj) return undefined;
    if (obj[snake] !== undefined) return obj[snake];
    return obj[camel];
  }

  function normalizeTask(t) {
    t = t || {};
    return {
      id: t.id,
      name: t.name || "",
      host: t.host || "",
      status: t.status || "",
      total: pick(t, "total", "totalBytes"),
      downloaded: pick(t, "downloaded", "downloadedBytes"),
      percent: t.percent,
      size_text: pick(t, "size_text", "sizeText") || "",
      detail_text: pick(t, "detail_text", "detailText") || "",
      connections: t.connections,
      error: t.error || null,
      url: t.url || "",
      added_ms: pick(t, "added_ms", "addedMs"),
      sha256: t.sha256 || null,
      hint: t.hint || null,
      retry_text: pick(t, "retry_text", "retryText") || null,
      retry_pending: !!pick(t, "retry_pending", "retryPending"),
      refreshing: !!t.refreshing,
      /* 0.35: engine-meter speed + ETA. Kept EXACTLY as
         picked (a number, null, or undefined when absent):
         undefined-on-both is the legacy-snapshot marker
         progressText uses for its detail_text fallback —
         coercing absent to null here would erase it. */
      speed_bps: pick(t, "speed_bps", "speedBps"),
      eta_secs: pick(t, "eta_secs", "etaSecs")
    };
  }

  /* --- Formatters (0.35) --------------------------------
     Ported byte-for-byte from assets/start.html (which
     mirrors Rust core::stats): same math, same outputs —
     there is no third variant. humanizeBytes(12250) is
     "12.3 KB" (the HALF-UP tie the Rust audit pinned). */
  function sigFigs(v) {
    if (v >= 99.95) return String(Math.round(v));
    var s = v >= 9.995 ? v.toFixed(1) : v.toFixed(2);
    if (s.indexOf(".") >= 0) s = s.replace(/0+$/, "").replace(/\.$/, "");
    return s;
  }

  function humanizeBytes(n) {
    var num = Number(n);
    if (!isFinite(num) || num < 0) return "";
    if (num < 1000) return num + " B";
    var units = ["KB", "MB", "GB", "TB", "PB", "EB"], v = num, u = 0;
    while (v >= 1000 && u < units.length) { v /= 1000; u++; }
    if (v >= 999.95 && u < units.length) { v /= 1000; u++; }
    return sigFigs(v) + " " + units[u - 1];
  }

  function humanizeDuration(ms) {
    var num = Number(ms);
    if (!isFinite(num) || num < 0) return "";
    var s = num / 1000;
    if (s < 60) return sigFigs(s) + " s";
    var m = s / 60;
    if (m < 60) return sigFigs(m) + " min";
    var h = m / 60;
    if (h < 24) return sigFigs(h) + " h";
    var d = sigFigs(h / 24);
    return d === "1" ? "1 day" : d + " days";
  }

  function fmtSpeed(bps) {
    var num = Number(bps);
    if (bps === null || bps === undefined || !isFinite(num)) return "";
    return humanizeBytes(num) + "/s";
  }

  function fmtEta(secs) {
    var num = Number(secs);
    if (secs === null || secs === undefined || !isFinite(num)) return "";
    return humanizeDuration(num * 1000);
  }

  function numOrNull(v) {
    if (v === null || v === undefined) return null;
    var n = Number(v);
    return isFinite(n) ? n : null;
  }

  /* progressText(task, tasks) — the ONE progress line,
     shared by the popover (.dl-prog) and the page
     (.dlp-detail) so both surfaces print the identical
     string for the same task. Composed from the NUMERIC
     fields (sizes via humanizeBytes, never size_text —
     except the queued zero-byte branch, which the spec
     defines on size_text). Speed is keyed off the STATUS,
     not field presence: a paused task never shows one,
     even if a stale speed_bps rides along. Fallback law:
     a legacy snapshot (speed_bps AND eta_secs both absent)
     or any field the composition needs being absent
     returns detail_text (or "") — never a throw, never
     "undefined"/"NaN"/"null" in the output. `tasks` is
     the surface's FULL snapshot list (newest-first); the
     queued position counts QUEUED tasks OLDER than this
     one, i.e. appearing AFTER it in that array, because
     the scheduler starts the oldest queued first. */
  function progressText(rawTask, tasks) {
    var task = normalizeTask(rawTask || {});
    var fallback = task.detail_text || "";
    if (task.speed_bps === undefined && task.eta_secs === undefined) {
      return fallback;
    }
    var st = task.status;
    var dl = numOrNull(task.downloaded);
    var total = numOrNull(task.total);
    var pct = numOrNull(task.percent);
    function ofTotal() {
      return humanizeBytes(dl) + " of " + humanizeBytes(total);
    }
    if (st === "downloading") {
      if (dl === null) return fallback;
      var outDl;
      if (total !== null) {
        if (pct === null) return fallback;
        outDl = ofTotal() + " · " + Math.round(pct) + "%";
      } else {
        outDl = humanizeBytes(dl);
      }
      var spd = numOrNull(task.speed_bps);
      if (spd !== null && spd > 0) outDl += " · " + fmtSpeed(spd);
      var eta = numOrNull(task.eta_secs);
      if (eta !== null) outDl += " · " + fmtEta(eta) + " left";
      return outDl;
    }
    if (st === "paused") {
      if (dl === null) return fallback;
      if (total !== null) {
        if (pct === null) return fallback;
        return ofTotal() + " · " + Math.round(pct) + "%";
      }
      return humanizeBytes(dl);
    }
    if (st === "queued") {
      if (dl === null) return fallback;
      var list = Array.isArray(tasks) ? tasks : [];
      var idx = -1;
      for (var i = 0; i < list.length; i++) {
        var cand = list[i] || {};
        if (cand === rawTask || (cand.id !== undefined && task.id !== undefined && cand.id === task.id)) { idx = i; break; }
      }
      var ahead = 0;
      if (idx >= 0) {
        for (var j = idx + 1; j < list.length; j++) {
          if ((list[j] || {}).status === "queued") ahead++;
        }
      }
      var waiting = ahead > 0 ? "Waiting · " + ahead + " ahead" : "Waiting · next up";
      if (dl > 0) {
        if (total === null || pct === null) return fallback;
        return ofTotal() + " · " + Math.round(pct) + "% · " + waiting;
      }
      if (!task.size_text) return fallback;
      return task.size_text + " · " + waiting;
    }
    if (st === "failed") {
      if (dl === null) return fallback;
      if (total === null) return humanizeBytes(dl);
      var outF = ofTotal();
      if (pct !== null) outF += " · " + Math.round(pct) + "%";
      return outF;
    }
    if (st === "done") {
      var sz = total !== null ? total : dl;
      if (sz === null) return fallback;
      return humanizeBytes(sz);
    }
    return fallback;
  }

  /* Accept THE snapshot, a legacy page payload
     ({downloads: [...], settings: {camelCase}}), a legacy
     chrome fragment (bare task array + sibling count
     fields, assembled by the chrome's syncDownloads), or a
     bare task array. Always returns the canonical shape. */
  function normalizeSnapshot(raw) {
    if (!raw) raw = {};
    var tasksRaw = raw.tasks || raw.downloads || (Array.isArray(raw) ? raw : []);
    if (!Array.isArray(tasksRaw)) tasksRaw = [];
    var tasks = tasksRaw.map(normalizeTask);
    var settingsRaw = raw.settings || {};
    var settings = {
      default_connections: pick(settingsRaw, "default_connections", "defaultConnections"),
      max_concurrent: pick(settingsRaw, "max_concurrent", "maxConcurrent"),
      speed_limit_kbps: pick(settingsRaw, "speed_limit_kbps", "speedLimitKbps")
    };
    if (settings.default_connections === undefined &&
        raw.downloadsDefaultConn !== undefined) {
      settings.default_connections = raw.downloadsDefaultConn;
    }
    var active = pick(raw, "active_count", "downloadsActive");
    if (active === undefined || active === null) {
      active = 0;
      tasks.forEach(function (t) {
        if (t.status === "downloading" || t.status === "queued") active++;
      });
    }
    var failed = pick(raw, "failed_unseen", "downloadsFailedBadge");
    if (failed === undefined || failed === null) failed = 0;
    return {
      version: raw.version,
      tasks: tasks,
      active_count: active,
      failed_unseen: failed,
      settings: settings,
      notice: raw.notice || raw.downloadsNotice || null
    };
  }

  /* Local UI state for the page surface only (the popover
     never shows the refresh editor or Restart): the armed
     Restart confirm (two-click, 3 s window) and the open
     refresh-link editor with its draft. Lives here because
     the rows themselves are rendered strings — the state
     must survive re-renders, exactly as the 0.33 page's
     module vars did. */
  var ui = {
    armedRestartId: null,
    armedTimer: null,
    refreshOpenId: null,
    refreshDraft: ""
  };

  /* Tasks from the most recent render, by id — bindCommands
     resolves copy-link's URL and the refresh editor's
     prefill from here, so URLs never sit in HTML
     attributes. */
  var tasksById = {};

  function percentWidth(task) {
    if (task.percent === null || task.percent === undefined) return null;
    var n = Number(task.percent);
    if (!isFinite(n)) return null;
    if (n < 0) n = 0;
    if (n > 100) n = 100;
    return n;
  }

  function connOptionsHtml(task) {
    var want = parseInt(task.connections, 10);
    var choices = CONN_CHOICES.slice();
    if (isFinite(want) && choices.indexOf(want) === -1) choices.push(want);
    var out = "";
    choices.forEach(function (n) {
      out += '<option value="' + n + '"' + (n === want ? " selected" : "") + ">" + n + "</option>";
    });
    return out;
  }

  /* Popover row anatomy (0.35): line 1 (.dl-name-line,
     24px) = name + status word + the state icon buttons
     (pause/resume toggle; done only: open-file + folder;
     then cancel/remove × — NO hidden placeholder buttons,
     a row renders only what its state needs); line 2
     (.dl-bar, 4px); line 3 (.dl-prog, 22px) = the
     progressText span + the per-task conn select. The
     host leaves the popover row (the page keeps it in
     meta). Row pricing: padding 16 + 24 + gap 5 + 4 +
     gap 5 + 22 = 76 (DL_ROW_H on the Rust side); the
     error line below adds its net +15 as before. */
  function renderPopoverRow(task, tasks) {
    var st = task.status;
    var resume = st === "paused" || st === "failed";
    var w = percentWidth(task);
    /* A done row with no percent (a legacy record whose
       total was unknown) is a FINISHED file: full bar, not
       the indeterminate wash (AUDIT 0.34.2 G3 — the
       producer also repairs the total on load now). */
    if (w === null && st === "done") w = 100;
    var fill = w === null
      ? '<div class="dl-fill indet" style="width:100%"></div>'
      : '<div class="dl-fill" style="width:' + w + '%"></div>';
    var buttons = "";
    if (st === "done") {
      buttons += '<button class="dl-icon-btn dl-open-file" data-dl-action="open-file" data-dl-id="' + task.id +
        '" aria-label="Open file" title="Open file">' + ICON_OPEN + "</button>";
      buttons += '<button class="dl-icon-btn dl-folder" data-dl-action="open-folder" data-dl-id="' + task.id +
        '" aria-label="Open folder" title="Open folder">' + ICON_FOLDER + "</button>";
    } else {
      buttons += '<button class="dl-icon-btn dl-act" data-dl-action="' + (resume ? "resume" : "pause") +
        '" data-dl-id="' + task.id + '" aria-label="' + (resume ? "Resume" : "Pause") +
        '" title="' + (resume ? "Resume" : "Pause") + '">' + (resume ? ICON_PLAY : ICON_PAUSE) + "</button>";
    }
    var cancelVerb = st === "done" ? "remove" : "cancel";
    var cancelTitle = st === "done" ? "Remove from list" : "Cancel";
    buttons += '<button class="dl-icon-btn dl-cancel" data-dl-action="' + cancelVerb + '" data-dl-id="' + task.id +
      '" aria-label="' + cancelTitle + '" title="' + cancelTitle + '">×</button>';
    /* Failed + retry_pending: the countdown owns line 3
       INSTEAD of the progress line (the popover rule). */
    var prog = (st === "failed" && task.retry_pending && task.retry_text)
      ? task.retry_text
      : progressText(task, tasks);
    var err = task.error
      ? '<div class="dl-error">' + escapeHtml(task.error) + "</div>"
      : '<div class="dl-error" style="display:none"></div>';
    return '<div class="dl-row" data-dl-id="' + task.id + '" data-status="' + escapeHtml(st) + '">' +
      '<div class="dl-name-line"><span class="dl-name">' + escapeHtml(task.name) +
      '</span><span class="dl-status">' + escapeHtml(st) + "</span>" + buttons + "</div>" +
      '<div class="dl-bar">' + fill + "</div>" +
      '<div class="dl-prog"><span class="dl-prog-text">' + escapeHtml(prog) + "</span>" +
      '<select class="dl-conn" data-dl-action="set-connections" data-dl-id="' + task.id +
      '" aria-label="Connections for this download"' + (st === "done" ? " disabled" : "") + ">" +
      connOptionsHtml(task) + "</select>" +
      "</div>" + err + "</div>";
  }

  function pageBtn(label, action, id, extraClass) {
    return '<button type="button" class="dl-btn' + (extraClass ? " " + extraClass : "") +
      '" data-dl-action="' + action + '" data-dl-id="' + id + '">' + label + "</button>";
  }

  function renderPageRow(task, tasks) {
    var st = task.status;
    var w = percentWidth(task);
    /* Done with no percent: full bar (AUDIT 0.34.2 G3,
       same rule as the popover row). */
    if (w === null && st === "done") w = 100;
    var fill;
    if (w === null) {
      fill = st === "downloading"
        ? '<div class="dlp-fill indet" style="width:100%"></div>'
        : '<div class="dlp-fill" style="width:0%"></div>';
    } else {
      fill = '<div class="dlp-fill" style="width:' + w + '%"></div>';
    }
    var out = '<div class="dlp-row" data-dl-id="' + task.id + '" data-status="' + escapeHtml(st) + '">' +
      '<div class="dlp-name-line"><span class="dlp-name">' + escapeHtml(task.name) +
      '</span><span class="dlp-status">' + escapeHtml(st) + "</span></div>";
    var metaParts = [];
    if (task.host) metaParts.push(escapeHtml(task.host));
    if (task.size_text) metaParts.push(escapeHtml(task.size_text));
    if (task.added_ms) {
      /* A stamp outside the JS Date range (crafted state)
         must never print the literal "Invalid Date"
         (AUDIT 0.34.2 G5 — the producer also clamps it). */
      var addedDate = new Date(task.added_ms);
      if (isFinite(addedDate.getTime())) {
        metaParts.push(escapeHtml(addedDate.toLocaleString()));
      }
    }
    if (metaParts.length) out += '<div class="dlp-meta">' + metaParts.join(" · ") + "</div>";
    out += '<div class="dlp-bar">' + fill + "</div>";
    if (task.refreshing) {
      out += '<div class="dlp-detail">Refreshing the link…</div>';
    } else if (st === "downloading" || st === "paused" || st === "queued" || st === "failed") {
      /* 0.35 parity: the detail line IS progressText —
         the identical string the popover prints for the
         same task (legacy snapshots fall back to
         detail_text inside progressText itself). */
      var progText = progressText(task, tasks);
      if (progText) {
        out += '<div class="dlp-detail">' + escapeHtml(progText) + "</div>";
      }
    } else if (task.detail_text) {
      out += '<div class="dlp-detail">' + escapeHtml(task.detail_text) + "</div>";
    }
    /* The error line follows the ERROR, not just the
       failed status: a paused/queued task can carry one
       (a failed link-refresh keeps the task's status,
       AUDIT 0.34.2 G2) — the popover already showed it;
       the page used to hide it. The hint stays
       failure-only (it is recovery advice). */
    if (task.error && (st === "failed" || st === "paused" || st === "queued")) {
      out += '<div class="dlp-error">' + escapeHtml(task.error) + "</div>";
    }
    if (st === "failed") {
      if (task.hint) out += '<div class="dlp-hint">' + escapeHtml(task.hint) + "</div>";
    }
    if (task.retry_text) {
      out += '<div class="dlp-retry">' + escapeHtml(task.retry_text) + "</div>";
    }
    if (st === "done" && task.sha256) {
      out += '<div class="dlp-sha"><span>SHA-256</span><span class="sha-val" data-dl-sha="' +
        escapeHtml(task.sha256) + '" title="Copy the SHA-256 hash">' +
        escapeHtml(task.sha256) + "</span></div>";
    }
    var actions = "";
    if (st === "downloading") {
      actions += pageBtn("Pause", "pause", task.id);
    } else if (st === "queued") {
      actions += pageBtn("Pause", "pause", task.id);
      actions += pageBtn("Remove", "remove", task.id);
    } else if (st === "paused" || st === "failed") {
      actions += pageBtn("Resume", "resume", task.id);
      var armed = ui.armedRestartId === task.id || ui.armedRestartId === String(task.id);
      actions += pageBtn(armed ? "Sure?" : "Restart", "restart", task.id,
        "dl-restart" + (armed ? " armed" : ""));
      if (st === "failed") {
        actions += '<button type="button" class="dl-btn" data-dl-local="refresh-toggle" data-dl-id="' +
          task.id + '">Refresh link…</button>';
      }
      actions += pageBtn("Remove", "remove", task.id);
      actions += pageBtn("Delete file", "delete-file", task.id);
    } else if (st === "done") {
      actions += pageBtn("Open file", "open-file", task.id);
      actions += pageBtn("Open folder", "open-folder", task.id);
      actions += pageBtn("Remove", "remove", task.id);
      actions += pageBtn("Delete file", "delete-file", task.id);
    }
    /* Copy link is client-only (the task URL to the
       clipboard) and rides every row, whatever its status. */
    actions += pageBtn("Copy link", "copy-link", task.id, "dl-copy-link");
    out += '<div class="dlp-actions">' + actions + "</div>";
    if (ui.refreshOpenId === task.id || ui.refreshOpenId === String(task.id)) {
      out += '<div class="dlp-refresh"><input type="text" class="well dl-refresh-input" data-dl-id="' +
        task.id + '" spellcheck="false" autocomplete="off" aria-label="New link for this download" value="' +
        escapeHtml(ui.refreshDraft) + '">' +
        '<button type="button" class="dl-btn dl-refresh-apply" data-dl-action="refresh" data-dl-id="' +
        task.id + '">Apply</button>' +
        '<button type="button" class="dl-btn dl-refresh-cancel" data-dl-local="refresh-cancel" data-dl-id="' +
        task.id + '">Cancel</button></div>';
    }
    out += "</div>";
    return out;
  }

  /* renderTaskList(snapshot, surface, allTasks?) → HTML
     string for the task list. surface "popover" = the
     compact row set; surface "page" = the full row.
     Queue-position seam (0.35): progressText's queued
     math must run against the surface's FULL snapshot,
     not a filtered subset — the page renders a filtered
     list, so it passes its full (normalized) task list as
     `allTasks` (an array, or a snapshot carrying one);
     when omitted, the rendered list itself is the
     context, which is correct for the popover (it always
     renders the full snapshot). */
  function renderTaskList(snapshot, surface, allTasks) {
    var snap = normalizeSnapshot(snapshot);
    tasksById = {};
    snap.tasks.forEach(function (t) { tasksById[t.id] = t; });
    var context = snap.tasks;
    if (allTasks !== undefined && allTasks !== null) {
      var ctxSnap = normalizeSnapshot(allTasks);
      if (ctxSnap.tasks.length || !snap.tasks.length) context = ctxSnap.tasks;
    }
    return snap.tasks.map(function (t) {
      return surface === "page" ? renderPageRow(t, context) : renderPopoverRow(t, context);
    }).join("");
  }

  /* Clipboard with the fallback shape both surfaces used in
     0.33: async clipboard first, a temporary textarea +
     execCommand second. `done(ok)` reports honestly. */
  function copyText(text, done) {
    function fallback() {
      var ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      var ok = false;
      try {
        ok = typeof document.execCommand === "function" && document.execCommand("copy");
      } catch (e) {}
      ta.remove();
      done(!!ok);
    }
    if (typeof navigator !== "undefined" && navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(function () { done(true); }, fallback);
    } else {
      fallback();
    }
  }

  function disarmRestart() {
    ui.armedRestartId = null;
    if (ui.armedTimer !== null) {
      clearTimeout(ui.armedTimer);
      ui.armedTimer = null;
    }
  }

  /* bindCommands(root, post, opts) — the ONE delegated
     click/change/input/keydown handler for task verbs.
     Elements carry data-dl-action + data-dl-id (+ the
     data-dl-local gestures that never leave the page: the
     refresh editor toggle/cancel, and data-dl-sha
     click-to-copy). `post(payload)` is the surface's
     transport: it wraps the payload into the canonical
     {"kind":"dl", ...} body. opts.rerender() is called
     after a local-state change (arm / editor toggle) so
     the surface can repaint from its snapshot + ui state. */
  function bindCommands(root, post, opts) {
    opts = opts || {};
    function rerender() { if (typeof opts.rerender === "function") opts.rerender(); }

    root.addEventListener("click", function (ev) {
      var t = ev.target;
      if (!t || !t.closest) return;
      var shaEl = t.closest("[data-dl-sha]");
      if (shaEl) {
        var hash = shaEl.getAttribute("data-dl-sha");
        var line = shaEl.closest(".dlp-sha");
        var label = line ? line.querySelector("span") : null;
        copyText(hash, function (ok) {
          if (label) {
            label.textContent = ok ? "Copied" : "SHA-256";
            setTimeout(function () { label.textContent = "SHA-256"; }, 1200);
          }
        });
        return;
      }
      var local = t.closest("[data-dl-local]");
      if (local) {
        var lid = local.getAttribute("data-dl-id");
        var ltask = tasksById[lid];
        if (local.getAttribute("data-dl-local") === "refresh-toggle") {
          if (ui.refreshOpenId === lid) {
            ui.refreshOpenId = null;
            ui.refreshDraft = "";
          } else {
            ui.refreshOpenId = lid;
            ui.refreshDraft = ltask ? (ltask.url || "") : "";
          }
          rerender();
        } else if (local.getAttribute("data-dl-local") === "refresh-cancel") {
          ui.refreshOpenId = null;
          ui.refreshDraft = "";
          rerender();
        }
        return;
      }
      var el = t.closest("[data-dl-action]");
      if (!el || el.tagName === "SELECT") return;
      var action = el.getAttribute("data-dl-action");
      var id = el.getAttribute("data-dl-id");
      var numId = id === null ? undefined : parseInt(id, 10);
      if (action === "copy-link") {
        /* Client-only verb: clipboard, never posted. */
        var ctask = tasksById[id];
        var btn = el;
        copyText(ctask ? String(ctask.url || "") : "", function (ok) {
          btn.textContent = ok ? "Copied" : "Copy link";
          setTimeout(function () { btn.textContent = "Copy link"; }, 1200);
        });
        return;
      }
      if (action === "restart") {
        /* Two-click inline confirm (0.33): the first click
           only arms; the second posts. */
        if (ui.armedRestartId === id) {
          disarmRestart();
          post({ action: "restart", id: numId });
        } else {
          disarmRestart();
          ui.armedRestartId = id;
          el.textContent = "Sure?";
          el.classList.add("armed");
          ui.armedTimer = setTimeout(function () {
            ui.armedRestartId = null;
            ui.armedTimer = null;
            rerender();
          }, 3000);
        }
        return;
      }
      if (action === "refresh") {
        var row = el.closest("[data-dl-id]");
        var input = row ? row.querySelector(".dl-refresh-input") : null;
        var url = input ? input.value.trim() : ui.refreshDraft.trim();
        ui.refreshOpenId = null;
        ui.refreshDraft = "";
        if (url) post({ action: "refresh", id: numId, url: url });
        rerender();
        return;
      }
      var payload = { action: action };
      if (numId !== undefined && isFinite(numId)) payload.id = numId;
      post(payload);
    });

    root.addEventListener("change", function (ev) {
      var el = ev.target;
      if (!el || !el.getAttribute) return;
      if (el.getAttribute("data-dl-action") === "set-connections") {
        post({
          action: "set-connections",
          id: parseInt(el.getAttribute("data-dl-id"), 10),
          conn: parseInt(el.value, 10)
        });
      }
    });

    root.addEventListener("input", function (ev) {
      var el = ev.target;
      if (el && el.classList && el.classList.contains("dl-refresh-input")) {
        ui.refreshDraft = el.value;
      }
    });

    root.addEventListener("keydown", function (ev) {
      var el = ev.target;
      if (!el || !el.classList || !el.classList.contains("dl-refresh-input")) return;
      if (ev.key === "Enter") {
        ev.preventDefault();
        var id = el.getAttribute("data-dl-id");
        var url = el.value.trim();
        ui.refreshOpenId = null;
        ui.refreshDraft = "";
        if (url) post({ action: "refresh", id: parseInt(id, 10), url: url });
        rerender();
      } else if (ev.key === "Escape") {
        ev.preventDefault();
        ui.refreshOpenId = null;
        ui.refreshDraft = "";
        rerender();
      }
    });
  }

  return {
    escapeHtml: escapeHtml,
    normalizeSnapshot: normalizeSnapshot,
    normalizeTask: normalizeTask,
    humanizeBytes: humanizeBytes,
    humanizeDuration: humanizeDuration,
    fmtSpeed: fmtSpeed,
    fmtEta: fmtEta,
    progressText: progressText,
    renderTaskList: renderTaskList,
    bindCommands: bindCommands,
    copyText: copyText,
    _ui: ui,
    CONN_CHOICES: CONN_CHOICES
  };
})();
