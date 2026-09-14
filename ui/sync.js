'use strict';

/* Second function: synchronise tracks, edit the result in a timeline, write the files.
   Shares esc, fmtDur, fmtBytes, plural, toast, setMode, pickOutput, parentDir and
   finishedCard with app.js. The backend decides which files result from the clips. */
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const q = (sel) => document.querySelector(sel);
  const tr = (k, p) => I18N.t(k, p); // `t` is a time variable throughout this file
  const trn = (k, n, p) => I18N.tn(k, n, p);
  const E = {
    empty: q('#sync-empty'), results: q('#sync-results'), finished: q('#sync-finished'), summary: q('#sync-summary'),
    pairs: q('#sync-pairs'), list: q('#sync-list'), extras: q('#sync-extras'), extrasSummary: q('#sync-extras-summary'),
    extrasBody: q('#sync-extras-body'), bottom: q('#sync-bottom'), progress: q('#sync-progress'), bar: q('#sync-bar'),
    ptext: q('#sync-ptext'), done: q('#sync-done'), sel: q('#sync-sel'), go: q('#sync-go'), cancel: q('#sync-cancel'),
    retry: q('#sync-retry'), analyzing: q('#sync-analyzing'), abar: q('#sync-abar'), atext: q('#sync-atext'),
    topActions: q('#sync-top-actions'), editState: q('#sync-edit-state'),
    editor: q('#sync-editor'), days: q('#ed-days'), overview: q('#ed-overview'), canvas: q('#ed-canvas'),
    wrap: q('#ed-wrap'), legend: q('#ed-legend'), time: q('#ed-time'), play: q('#ed-play'), reset: q('#ed-reset'), menu: q('#ed-menu'),
  };
  const sy = { inputs: [], plan: null, outDir: '', outcomes: new Map(), busy: false, activeId: null, retryIds: null, decoding: false };
  window.syncState = sy;

  const LABEL_W = 124, RULER_H = 24, ROW_H = 46, GROUP_GAP = 10, EVENTS_H = 10;
  const COLORS = ['#3478f6', '#e0443e', '#1f9d55', '#d98b10', '#7c5cd6']; // blue = left, red = right, then further mono senders

  const MIN_CLIP = 0.05, EDGE_PX = 6, SNAP_PX = 7, MIN_SPP = 0.002;
  const ed = {
    clips: [], committed: [], history: [], future: [], sel: new Set(), dayIndex: 0, days: [],
    view: { t0: 0, spp: 1 }, width: 800, height: 200, rows: [], headerHits: [],
    playhead: 0, playing: false, posT: 0, posAt: 0, muted: new Set(), solo: new Set(),
    peaks: new Map(), drag: null, hatch: new Map(),
  };

  const busyAny = () => sy.busy || !!(window.mergeState && window.mergeState.busy) || !!(window.masterState && window.masterState.busy);
  const fixed = (v, d) => v.toLocaleString(I18N.locale(), { minimumFractionDigits: d, maximumFractionDigits: d });
  const signed = (v, d) => `${v < 0 ? '−' : '+'}${fixed(Math.abs(v), d)}`;
  const percent = (v) => (v == null ? '–' : `${Math.round(v * 100)} %`);
  const clone = (v) => JSON.parse(JSON.stringify(v));

  /* ---------- geometry helpers ---------- */

  const P = () => sy.plan;
  const place = (track) => P().places[track];
  const toTimeline = (track, tau) => place(track).p + tau / place(track).s;
  const toTrack = (track, t) => (t - place(track).p) * place(track).s;
  const labelOf = (track) => P().tracks[track].label;
  const colorOf = (label) => COLORS[Math.max(0, P().labels.indexOf(label)) % COLORS.length];
  const keyOf = (c) => `${c.track}:${c.t0.toFixed(3)}`;
  const day = () => ed.days[ed.dayIndex];
  const visibleSpan = () => (ed.width - LABEL_W) * ed.view.spp;
  const X = (t) => LABEL_W + (t - ed.view.t0) / ed.view.spp;
  const T = (x) => ed.view.t0 + (x - LABEL_W) * ed.view.spp;
  const playheadNow = () => (ed.playing ? ed.posT + (performance.now() - ed.posAt) / 1000 : ed.playhead);

  /** Colour mixed towards white: mono clips are the pale version of their sender colour. */
  function tint(hex, amount) {
    const n = parseInt(hex.slice(1), 16);
    const m = (c) => Math.round(c + (255 - c) * amount);
    return `rgb(${m((n >> 16) & 255)},${m((n >> 8) & 255)},${m(n & 255)})`;
  }
  function withAlpha(hex, a) {
    const n = parseInt(hex.slice(1), 16);
    return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`;
  }

  /** Seconds since midnight for an absolute timeline second, from an anchor track (clock rate 1) of the day. */
  function dayClock(t) {
    const d = day();
    if (!d) return 0;
    return t - d.midnight;
  }
  function clockText(t, withTenths) {
    let s = dayClock(t);
    s = ((s % 86400) + 86400) % 86400;
    const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), sec = s % 60;
    const ss = withTenths ? sec.toFixed(1).padStart(4, '0') : String(Math.floor(sec)).padStart(2, '0');
    return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}:${ss}`;
  }

  function buildDays() {
    const plan = P();
    const groups = new Map();
    plan.tracks.forEach((t) => {
      if (!groups.has(t.day)) groups.set(t.day, []);
      groups.get(t.day).push(t.id);
    });
    ed.days = [...groups.entries()].map(([key, ids]) => {
      const ext = ids.map((i) => [toTimeline(i, 0), toTimeline(i, plan.tracks[i].duration)]);
      const anchor = ids.find((i) => place(i).s === 1) ?? ids[0];
      return {
        key, tracks: ids,
        t0: Math.min(...ext.map((e) => e[0])), t1: Math.max(...ext.map((e) => e[1])),
        midnight: place(anchor).p - plan.tracks[anchor].clock0,
      };
    }).sort((a, b) => a.t0 - b.t0);
    ed.dayIndex = Math.min(ed.dayIndex, ed.days.length - 1);
  }

  function buildRows() {
    const plan = P();
    const d = day();
    const labels = plan.labels.filter((l) => d.tracks.some((t) => labelOf(t) === l));
    const stereoPossible = plan.labels.length >= 2;
    const rows = [];
    let y = RULER_H + 6;
    labels.forEach((label, i) => {
      const li = plan.labels.indexOf(label);
      const kinds = !stereoPossible || li > 1 ? ['mono'] : li === 0 ? ['mono', 'stereo'] : ['stereo', 'mono'];
      const joined = i > 0 && li === 1 && plan.labels.indexOf(labels[i - 1]) === 0;
      if (i > 0 && !joined) y += GROUP_GAP;
      kinds.forEach((kind) => { rows.push({ label, li, kind, y, h: ROW_H }); y += ROW_H; });
    });
    ed.rows = rows;
    ed.eventsY = y + 8;
    ed.height = ed.eventsY + EVENTS_H + 8;
  }

  const rowFor = (clip) => ed.rows.find((r) => r.label === labelOf(clip.track) && r.kind === clip.mode) || ed.rows.find((r) => r.label === labelOf(clip.track));

  function fitDay() {
    const d = day();
    if (!d) return;
    const pad = Math.max(30, (d.t1 - d.t0) * 0.01);
    ed.view.spp = Math.max(MIN_SPP, (d.t1 - d.t0 + 2 * pad) / Math.max(100, ed.width - LABEL_W));
    ed.view.t0 = d.t0 - pad;
  }

  function clampView() {
    const d = day();
    const span = d.t1 - d.t0;
    const maxSpp = (span * 1.1 + 60) / Math.max(100, ed.width - LABEL_W);
    ed.view.spp = Math.min(Math.max(ed.view.spp, MIN_SPP), maxSpp);
    const vis = visibleSpan();
    ed.view.t0 = Math.min(Math.max(ed.view.t0, d.t0 - vis * 0.5), d.t1 - vis * 0.5);
  }

  function zoomAt(factor, x) {
    const t = T(x);
    ed.view.spp *= factor;
    clampView();
    ed.view.t0 = t - (x - LABEL_W) * ed.view.spp;
    clampView();
    schedulePeaks();
    draw();
  }

  /* ---------- peaks ---------- */

  let peakTimer = 0;
  function schedulePeaks() {
    clearTimeout(peakTimer);
    peakTimer = setTimeout(fetchPeaks, 70);
  }
  async function fetchPeaks() {
    const plan = P();
    const d = day();
    if (!plan || !d) return;
    const vt0 = ed.view.t0, vt1 = ed.view.t0 + visibleSpan();
    const buckets = Math.max(50, Math.round(ed.width - LABEL_W));
    await Promise.all(d.tracks.map(async (t) => {
      const a = toTrack(t, vt0), b = toTrack(t, vt1);
      if (b < 0 || a > plan.tracks[t].duration) return;
      try {
        const data = await invoke('sync_peaks', { track: t, t0: a, t1: b, buckets });
        ed.peaks.set(t, { t0: a, t1: b, data });
      } catch (e) { /* peaks are decoration */ }
    }));
    draw();
  }

  /* ---------- drawing ---------- */

  function hatch(ctx, color) {
    const key = color;
    if (!ed.hatch.has(key)) {
      const c = document.createElement('canvas');
      c.width = 8; c.height = 8;
      const g = c.getContext('2d');
      g.strokeStyle = color; g.lineWidth = 1.5;
      g.beginPath(); g.moveTo(-2, 10); g.lineTo(10, -2); g.moveTo(-2, 2); g.lineTo(2, -2); g.moveTo(6, 10); g.lineTo(10, 6); g.stroke();
      ed.hatch.set(key, ctx.createPattern(c, 'repeat'));
    }
    return ed.hatch.get(key);
  }

  /** Header text in the fixed label column: shrink slightly, then cut with an ellipsis. */
  function fitText(ctx, text, x, y, maxW, size) {
    let s = size;
    const font = (px) => `600 ${px}px -apple-system, system-ui, sans-serif`;
    ctx.font = font(s);
    while (s > 9.5 && ctx.measureText(text).width > maxW) { s -= 0.5; ctx.font = font(s); }
    let out = text;
    if (ctx.measureText(out).width > maxW) {
      while (out.length > 1 && ctx.measureText(`${out}…`).width > maxW) out = out.slice(0, -1);
      out = `${out}…`;
    }
    ctx.fillText(out, x, y);
  }

  function roundRect(ctx, x, y, w, h, r) {
    const rr = Math.min(r, w / 2, h / 2);
    ctx.beginPath();
    ctx.moveTo(x + rr, y); ctx.arcTo(x + w, y, x + w, y + h, rr); ctx.arcTo(x + w, y + h, x, y + h, rr);
    ctx.arcTo(x, y + h, x, y, rr); ctx.arcTo(x, y, x + w, y, rr); ctx.closePath();
  }

  function draw() {
    const plan = P();
    if (!plan || !day() || E.results.hidden) return;
    const css = getComputedStyle(document.documentElement);
    const v = (name) => css.getPropertyValue(name).trim();
    const dpr = window.devicePixelRatio || 1;
    const c = E.canvas;
    ed.width = Math.max(320, E.wrap.clientWidth);
    buildRows();
    if (c.width !== Math.round(ed.width * dpr) || c.height !== Math.round(ed.height * dpr)) {
      c.width = Math.round(ed.width * dpr); c.height = Math.round(ed.height * dpr);
      c.style.height = `${ed.height}px`;
    }
    const ctx = c.getContext('2d');
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, ed.width, ed.height);
    const W = ed.width;
    const d = day();
    const inDay = new Set(d.tracks);

    // lanes
    ctx.fillStyle = v('--panel-2');
    for (const r of ed.rows) ctx.fillRect(LABEL_W, r.y, W - LABEL_W, r.h);
    ctx.strokeStyle = v('--line'); ctx.lineWidth = 1;
    for (const r of ed.rows) { ctx.beginPath(); ctx.moveTo(LABEL_W, r.y + r.h + 0.5); ctx.lineTo(W, r.y + r.h + 0.5); ctx.stroke(); }

    ctx.save();
    ctx.beginPath(); ctx.rect(LABEL_W, 0, W - LABEL_W, ed.height); ctx.clip();

    // cut-away parts of each recording: hatched in the row of the neighbouring clip
    for (const t of d.tracks) {
      const own = ed.clips.filter((cl) => cl.track === t).sort((x, y) => x.t0 - y.t0);
      const dur = plan.tracks[t].duration;
      const gaps = [];
      let from = 0;
      own.forEach((cl, i) => {
        if (cl.t0 > from + 0.01) gaps.push([from, cl.t0, i > 0 ? own[i - 1] : cl]);
        from = Math.max(from, cl.t1);
      });
      if (dur > from + 0.01) gaps.push([from, dur, own[own.length - 1] || { track: t, mode: P().labels.length >= 2 && P().labels.indexOf(labelOf(t)) < 2 ? 'stereo' : 'mono' }]);
      ctx.fillStyle = hatch(ctx, withAlpha(colorOf(labelOf(t)), 0.35));
      for (const [g0, g1, near] of gaps) {
        const a = X(toTimeline(t, g0)), b = X(toTimeline(t, g1));
        const r = rowFor(near);
        if (!r || b < LABEL_W || a > W) continue;
        ctx.fillRect(a, r.y + 5, b - a, r.h - 10);
      }
    }

    // stereo link between the two inner rows
    const stereoRows = ed.rows.filter((r) => r.kind === 'stereo');
    if (stereoRows.length === 2) {
      const spans = (li) => ed.clips.filter((cl) => inDay.has(cl.track) && !cl.deleted && cl.mode === 'stereo' && plan.labels.indexOf(labelOf(cl.track)) === li)
        .map((cl) => [toTimeline(cl.track, cl.t0), toTimeline(cl.track, cl.t1)]);
      const seamY = stereoRows[1].y;
      ctx.fillStyle = withAlpha(v('--ink') || '#222', 0.55);
      for (const [a0, a1] of spans(0)) for (const [b0, b1] of spans(1)) {
        const s0 = Math.max(a0, b0), s1 = Math.min(a1, b1);
        if (s1 > s0) ctx.fillRect(X(s0), seamY - 1.5, X(s1) - X(s0), 3);
      }
    }

    // clips
    for (const clip of ed.clips) {
      if (!inDay.has(clip.track)) continue;
      const row = rowFor(clip);
      if (!row) continue;
      const a = X(toTimeline(clip.track, clip.t0)), b = X(toTimeline(clip.track, clip.t1));
      if (b < LABEL_W - 2 || a > W + 2) continue;
      const color = colorOf(labelOf(clip.track));
      const y = row.y + 4, h = row.h - 8, w = Math.max(1.5, b - a);
      const selected = ed.sel.has(keyOf(clip));
      roundRect(ctx, a, y, w, h, 5);
      if (clip.deleted) {
        ctx.fillStyle = v('--panel');
        ctx.fill();
        ctx.fillStyle = hatch(ctx, withAlpha(color, 0.55));
        ctx.fill();
        ctx.setLineDash([4, 3]); ctx.strokeStyle = withAlpha(color, 0.8); ctx.lineWidth = 1; ctx.stroke(); ctx.setLineDash([]);
      } else {
        const mono = clip.mode === 'mono';
        ctx.fillStyle = mono ? tint(color, 0.72) : withAlpha(color, 0.92);
        ctx.fill();
        drawWave(ctx, clip, Math.max(a, LABEL_W), Math.min(b, W), y, h, mono ? tint(color, 0.38) : 'rgba(255,255,255,0.6)');
      }
      if (w > 26) {
        ctx.font = '600 10px -apple-system, system-ui, sans-serif';
        const tag = clip.deleted ? tr('sync.clip.deleted') : clip.mode === 'mono' ? 'M' : plan.labels.indexOf(labelOf(clip.track)) === 0 ? 'L' : 'R';
        const tx = Math.max(a, LABEL_W) + 5;
        ctx.fillStyle = clip.deleted || clip.mode === 'mono' ? withAlpha(color, 0.95) : 'rgba(255,255,255,0.95)';
        ctx.fillText(tag, tx, y + 12);
      }
      if (selected) {
        roundRect(ctx, a, y, w, h, 5);
        ctx.lineWidth = 2; ctx.strokeStyle = v('--ink'); ctx.stroke();
        ctx.fillStyle = v('--ink');
        ctx.fillRect(a - 1, y + h / 2 - 9, 3, 18);
        ctx.fillRect(b - 2, y + h / 2 - 9, 3, 18);
      }
    }

    // drag preview for moving between stereo and mono
    if (ed.drag && ed.drag.type === 'move' && ed.drag.targetRow) {
      const clip = ed.clips[ed.drag.idx];
      const r = ed.drag.targetRow;
      const a = X(toTimeline(clip.track, clip.t0)), b = X(toTimeline(clip.track, clip.t1));
      roundRect(ctx, a, r.y + 4, Math.max(2, b - a), r.h - 8, 5);
      ctx.setLineDash([5, 4]); ctx.lineWidth = 2; ctx.strokeStyle = colorOf(labelOf(clip.track)); ctx.stroke(); ctx.setLineDash([]);
    }

    // shared events
    for (const p of plan.pairs) {
      if (!p.ok || !inDay.has(p.a)) continue;
      for (const f of p.frames) {
        const t = toTimeline(p.a, f.t + 5);
        const x0 = X(t), x1 = X(t + 10);
        if (x1 < LABEL_W || x0 > W) continue;
        ctx.fillStyle = !f.active ? v('--line') : f.hit ? v('--accent') : withAlpha('#c8901e', 0.7);
        ctx.fillRect(x0, ed.eventsY, Math.max(1, x1 - x0), EVENTS_H);
      }
    }
    ctx.restore();

    // ruler
    ctx.fillStyle = v('--panel');
    ctx.fillRect(LABEL_W, 0, W - LABEL_W, RULER_H);
    const steps = [0.1, 0.5, 1, 2, 5, 10, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200];
    const step = steps.find((s) => s / ed.view.spp >= 90) || 10800;
    ctx.fillStyle = v('--faint'); ctx.strokeStyle = v('--line');
    ctx.font = '10.5px -apple-system, system-ui, sans-serif';
    const mid = day().midnight;
    for (let t = Math.ceil((ed.view.t0 - mid) / step) * step + mid; t < ed.view.t0 + visibleSpan(); t += step) {
      const x = X(t);
      ctx.beginPath(); ctx.moveTo(x + 0.5, RULER_H - 7); ctx.lineTo(x + 0.5, RULER_H); ctx.stroke();
      ctx.fillText(clockText(t, step < 1), x + 3, 13);
    }

    // headers
    ctx.fillStyle = v('--panel');
    ctx.fillRect(0, 0, LABEL_W, ed.height);
    ed.headerHits = [];
    const seen = new Set();
    for (const r of ed.rows) {
      ctx.fillStyle = v('--faint');
      ctx.font = '10.5px -apple-system, system-ui, sans-serif';
      ctx.textAlign = 'right';
      ctx.fillText(tr(r.kind === 'stereo' ? 'sync.row.stereo' : 'sync.row.mono'), LABEL_W - 8, r.y + r.h - 8);
      ctx.textAlign = 'left';
      if (seen.has(r.label)) continue;
      seen.add(r.label);
      const top = r.li === 1 && r.kind === 'stereo' ? r.y + r.h : r.y;
      ctx.fillStyle = colorOf(r.label);
      ctx.fillRect(6, top + 6, 4, 34);
      ctx.fillStyle = v('--ink');
      fitText(ctx, tr('sync.canvas.sender', { label: r.label }), 16, top + 17, LABEL_W - 22, 12);
      [['M', ed.muted], ['S', ed.solo]].forEach(([txt, set], i) => {
        const bx = 16 + i * 24, by = top + 24;
        roundRect(ctx, bx, by, 20, 16, 4);
        ctx.fillStyle = set.has(r.label) ? (txt === 'M' ? '#c8901e' : v('--accent')) : v('--panel-2');
        ctx.fill(); ctx.strokeStyle = v('--line'); ctx.stroke();
        ctx.fillStyle = set.has(r.label) ? '#fff' : v('--muted');
        ctx.font = '600 10px -apple-system, system-ui, sans-serif';
        ctx.fillText(txt, bx + 6, by + 12);
        ed.headerHits.push({ x: bx, y: by, w: 20, h: 16, label: r.label, kind: txt });
      });
    }
    ctx.fillStyle = v('--faint');
    ctx.font = '10.5px -apple-system, system-ui, sans-serif';
    ctx.fillText(tr('sync.canvas.events'), 16, ed.eventsY + 9);

    // playhead
    const ph = playheadNow();
    const px = X(ph);
    if (px >= LABEL_W && px <= W) {
      ctx.strokeStyle = v('--ink'); ctx.lineWidth = 1.5;
      ctx.beginPath(); ctx.moveTo(px, 6); ctx.lineTo(px, ed.height); ctx.stroke();
      ctx.fillStyle = v('--ink');
      ctx.beginPath(); ctx.moveTo(px - 6, 0); ctx.lineTo(px + 6, 0); ctx.lineTo(px, 8); ctx.closePath(); ctx.fill();
    }
    E.time.textContent = clockText(ph, true);
    drawOverview(v);
  }

  function drawWave(ctx, clip, x0, x1, y, h, fill) {
    const pk = ed.peaks.get(clip.track);
    if (!pk || !pk.data.length || pk.t1 <= pk.t0) return;
    ctx.fillStyle = fill;
    const mid = y + h / 2;
    const n = pk.data.length;
    for (let x = Math.floor(x0); x < x1; x += 1) {
      const tau = toTrack(clip.track, T(x + 0.5));
      if (tau < clip.t0 || tau > clip.t1) continue;
      const i = Math.floor(((tau - pk.t0) / (pk.t1 - pk.t0)) * n);
      if (i < 0 || i >= n) continue;
      const amp = (pk.data[i] / 255) * (h / 2 - 3);
      if (amp > 0.3) ctx.fillRect(x, mid - amp, 1, amp * 2);
    }
  }

  function drawOverview(v) {
    const c = E.overview;
    const d = day();
    const dpr = window.devicePixelRatio || 1;
    const W = Math.max(320, c.clientWidth), H = 30;
    if (c.width !== Math.round(W * dpr)) { c.width = Math.round(W * dpr); c.height = Math.round(H * dpr); }
    const ctx = c.getContext('2d');
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, W, H);
    const pad = (d.t1 - d.t0) * 0.02 + 30;
    const o0 = d.t0 - pad, o1 = d.t1 + pad;
    const ox = (t) => ((t - o0) / (o1 - o0)) * W;
    ed.overviewMap = { o0, o1, W };
    const labels = P().labels.filter((l) => d.tracks.some((t) => labelOf(t) === l));
    const laneH = (H - 6) / Math.max(1, labels.length);
    for (const clip of ed.clips) {
      if (!d.tracks.includes(clip.track) || clip.deleted) continue;
      const li = labels.indexOf(labelOf(clip.track));
      ctx.fillStyle = clip.mode === 'stereo' ? withAlpha(colorOf(labelOf(clip.track)), 0.9) : tint(colorOf(labelOf(clip.track)), 0.72);
      const a = ox(toTimeline(clip.track, clip.t0)), b = ox(toTimeline(clip.track, clip.t1));
      ctx.fillRect(a, 3 + li * laneH, Math.max(1, b - a), laneH - 1);
    }
    ctx.strokeStyle = v('--ink'); ctx.lineWidth = 1.5;
    const va = ox(ed.view.t0), vb = ox(ed.view.t0 + visibleSpan());
    ctx.strokeRect(Math.max(0.75, va), 0.75, Math.min(W - 1.5, vb - va), H - 1.5);
    const p = ox(playheadNow());
    ctx.fillStyle = v('--ink');
    ctx.fillRect(p - 0.75, 0, 1.5, H);
  }

  /* ---------- hit testing and gestures ---------- */

  function hitAt(x, y) {
    for (const hh of ed.headerHits) if (x >= hh.x && x <= hh.x + hh.w && y >= hh.y && y <= hh.y + hh.h) return { type: 'header', ...hh };
    if (x < LABEL_W) return { type: 'none' };
    if (y < RULER_H) return { type: 'ruler' };
    const inDay = new Set(day().tracks);
    let best = null;
    ed.clips.forEach((clip, idx) => {
      if (!inDay.has(clip.track)) return;
      const row = rowFor(clip);
      if (!row || y < row.y || y > row.y + row.h) return;
      const a = X(toTimeline(clip.track, clip.t0)), b = X(toTimeline(clip.track, clip.t1));
      const edge = b - a > 14 ? EDGE_PX : 3;
      if (Math.abs(x - a) <= edge) best = { type: 'edge', idx, edge: 'l' };
      else if (Math.abs(x - b) <= edge) best = best || { type: 'edge', idx, edge: 'r' };
      else if (x > a && x < b && !best) best = { type: 'clip', idx };
    });
    return best || { type: 'lane' };
  }

  function snap(t, exclude) {
    const candidates = [ed.playhead];
    const inDay = new Set(day().tracks);
    ed.clips.forEach((c, i) => {
      if (!inDay.has(c.track)) return;
      if (!(exclude && exclude.idx === i && exclude.edge === 'l')) candidates.push(toTimeline(c.track, c.t0));
      if (!(exclude && exclude.idx === i && exclude.edge === 'r')) candidates.push(toTimeline(c.track, c.t1));
    });
    let best = t, dist = SNAP_PX;
    for (const c of candidates) {
      const dx = Math.abs(X(c) - X(t));
      if (dx < dist) { best = c; dist = dx; }
    }
    return best;
  }

  function trackNeighbours(idx, clips) {
    const c = clips[idx];
    const list = clips.map((cl, i) => [cl, i]).filter(([cl]) => cl.track === c.track).sort((a, b) => a[0].t0 - b[0].t0);
    const pos = list.findIndex(([, i]) => i === idx);
    return { prev: pos > 0 ? list[pos - 1][1] : null, next: pos < list.length - 1 ? list[pos + 1][1] : null };
  }

  function trimTo(t) {
    const { idx, edge, orig } = ed.drag;
    const c = ed.clips[idx];
    const o = orig[idx];
    const dur = P().tracks[c.track].duration;
    const { prev, next } = trackNeighbours(idx, orig);
    let tau = toTrack(c.track, t);
    if (edge === 'l') {
      const rolling = prev != null && Math.abs(orig[prev].t1 - o.t0) < 0.01;
      const lo = rolling ? orig[prev].t0 + MIN_CLIP : prev != null ? orig[prev].t1 : 0;
      tau = Math.min(Math.max(tau, lo), c.t1 - MIN_CLIP);
      c.t0 = tau;
      if (rolling) ed.clips[prev].t1 = tau;
    } else {
      const rolling = next != null && Math.abs(orig[next].t0 - o.t1) < 0.01;
      const hi = rolling ? orig[next].t1 - MIN_CLIP : next != null ? orig[next].t0 : dur;
      tau = Math.max(Math.min(tau, hi), c.t0 + MIN_CLIP);
      c.t1 = tau;
      if (rolling) ed.clips[next].t0 = tau;
    }
  }

  function pointerPos(e) {
    const r = E.canvas.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  E.canvas.addEventListener('pointerdown', (e) => {
    if (!P() || e.button === 2) return;
    hideMenu();
    const { x, y } = pointerPos(e);
    E.canvas.focus({ preventScroll: true });
    const hit = hitAt(x, y);
    if (hit.type === 'header') {
      const set = hit.kind === 'M' ? ed.muted : ed.solo;
      if (set.has(hit.label)) set.delete(hit.label); else set.add(hit.label);
      invoke('player_solo', { muted: [...ed.muted], solo: [...ed.solo] }).catch(() => {});
      draw();
      return;
    }
    try { E.canvas.setPointerCapture(e.pointerId); } catch (err) { /* synthetic pointer */ }
    if (hit.type === 'edge') {
      const key = keyOf(ed.clips[hit.idx]);
      if (!ed.sel.has(key)) { ed.sel = new Set([key]); }
      ed.drag = { type: 'trim', idx: hit.idx, edge: hit.edge, orig: clone(ed.clips) };
    } else if (hit.type === 'clip') {
      const key = keyOf(ed.clips[hit.idx]);
      if (e.shiftKey || e.metaKey) { if (ed.sel.has(key)) ed.sel.delete(key); else ed.sel.add(key); }
      else if (!ed.sel.has(key)) ed.sel = new Set([key]);
      ed.drag = { type: 'move', idx: hit.idx, y0: y, targetRow: null, modifier: e.shiftKey || e.metaKey };
    } else {
      if (hit.type === 'lane' && !e.shiftKey) ed.sel.clear();
      ed.drag = { type: 'scrub' };
      seek(Math.max(day().t0, Math.min(day().t1, T(x))));
    }
    updateToolbar();
    draw();
  });

  let lastScrub = 0;
  E.canvas.addEventListener('pointermove', (e) => {
    if (!P()) return;
    const { x, y } = pointerPos(e);
    if (!ed.drag) {
      const hit = hitAt(x, y);
      E.canvas.style.cursor = hit.type === 'edge' ? 'ew-resize' : hit.type === 'clip' ? 'grab' : hit.type === 'header' ? 'pointer' : hit.type === 'ruler' || hit.type === 'lane' ? 'text' : 'default';
      return;
    }
    if (ed.drag.type === 'trim') {
      trimTo(snap(T(x), { idx: ed.drag.idx, edge: ed.drag.edge }));
      draw();
    } else if (ed.drag.type === 'move') {
      const clip = ed.clips[ed.drag.idx];
      const row = ed.rows.find((r) => y >= r.y && y <= r.y + r.h && r.label === labelOf(clip.track));
      ed.drag.targetRow = row && row.kind !== clip.mode && Math.abs(y - ed.drag.y0) > 10 ? row : null;
      E.canvas.style.cursor = 'grabbing';
      draw();
    } else if (ed.drag.type === 'scrub') {
      ed.playhead = Math.max(day().t0, Math.min(day().t1, T(x)));
      if (performance.now() - lastScrub > 60) { lastScrub = performance.now(); seek(ed.playhead); }
      draw();
    }
  });

  E.canvas.addEventListener('pointerup', (e) => {
    const drag = ed.drag;
    ed.drag = null;
    if (!drag || !P()) return;
    if (drag.type === 'trim') {
      const c = ed.clips[drag.idx];
      ed.sel = new Set([keyOf(c)]);
      if (JSON.stringify(drag.orig) !== JSON.stringify(ed.clips)) commit(ed.clips);
    } else if (drag.type === 'move' && !drag.targetRow) {
      if (!drag.modifier) { ed.sel = new Set([keyOf(ed.clips[drag.idx])]); updateToolbar(); }
    } else if (drag.type === 'move' && drag.targetRow) {
      const mode = drag.targetRow.kind;
      const next = clone(ed.clips);
      const keys = ed.sel.size ? ed.sel : new Set([keyOf(ed.clips[drag.idx])]);
      next.forEach((c) => { if (keys.has(keyOf(c)) && labelOf(c.track) === drag.targetRow.label) c.mode = mode; });
      commit(next);
    } else if (drag.type === 'scrub') {
      seek(ed.playhead);
    }
    draw();
  });

  E.canvas.addEventListener('wheel', (e) => {
    if (!P()) return;
    const { x } = pointerPos(e);
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      zoomAt(Math.exp(e.deltaY * 0.01), Math.max(LABEL_W, x));
    } else if (Math.abs(e.deltaX) > Math.abs(e.deltaY) || e.shiftKey) {
      e.preventDefault();
      ed.view.t0 += (e.shiftKey && !e.deltaX ? e.deltaY : e.deltaX) * ed.view.spp;
      clampView();
      schedulePeaks();
      draw();
    }
  }, { passive: false });

  E.canvas.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    if (!P()) return;
    const { x, y } = pointerPos(e);
    const hit = hitAt(x, y);
    if (hit.type === 'clip' || hit.type === 'edge') {
      const key = keyOf(ed.clips[hit.idx]);
      if (!ed.sel.has(key)) ed.sel = new Set([key]);
    }
    showMenu(x, y);
    updateToolbar();
    draw();
  });

  let overviewDrag = false;
  const overviewTo = (e) => {
    const m = ed.overviewMap;
    if (!m) return;
    const r = E.overview.getBoundingClientRect();
    const t = m.o0 + ((e.clientX - r.left) / m.W) * (m.o1 - m.o0);
    ed.view.t0 = t - visibleSpan() / 2;
    clampView();
    schedulePeaks();
    draw();
  };
  E.overview.addEventListener('pointerdown', (e) => { overviewDrag = true; try { E.overview.setPointerCapture(e.pointerId); } catch (err) { /* synthetic */ } overviewTo(e); });
  E.overview.addEventListener('pointermove', (e) => { if (overviewDrag) overviewTo(e); });
  E.overview.addEventListener('pointerup', () => { overviewDrag = false; });

  function showMenu(x, y) {
    const n = ed.sel.size;
    const items = [
      ['split', tr('sync.menu.split'), 'S'], ['merge', tr('sync.menu.join'), 'J'],
      ['delete', tr(selectionDeleted() ? 'sync.op.restore' : 'sync.op.delete'), tr('sync.kbd.delete')],
      ['stereo', tr('sync.menu.toStereo'), '↑'], ['mono', tr('sync.menu.toMono'), '↓'],
      ['play', tr(ed.playing ? 'sync.menu.pause' : 'sync.menu.playHere'), '␣'],
    ];
    E.menu.innerHTML = items.map(([op, label, kbd]) => `<button data-op="${op}" ${!n && op !== 'split' && op !== 'play' ? 'disabled' : ''}>${esc(label)}<kbd>${kbd}</kbd></button>`).join('');
    E.menu.style.left = `${Math.min(x, ed.width - 200)}px`;
    E.menu.style.top = `${y}px`;
    E.menu.hidden = false;
  }
  function hideMenu() { E.menu.hidden = true; }
  E.menu.addEventListener('click', (e) => {
    const b = e.target.closest('[data-op]');
    if (!b) return;
    hideMenu();
    if (b.dataset.op === 'play') { const t = T(parseFloat(E.menu.style.left)); seek(t); togglePlay(); return; }
    runOp(b.dataset.op);
  });
  document.addEventListener('pointerdown', (e) => { if (!E.menu.hidden && !E.menu.contains(e.target)) hideMenu(); });

  /* ---------- edit operations ---------- */

  const selectedIndices = () => ed.clips.map((c, i) => [c, i]).filter(([c]) => ed.sel.has(keyOf(c))).map(([, i]) => i);
  const selectionDeleted = () => { const s = selectedIndices(); return s.length > 0 && s.every((i) => ed.clips[i].deleted); };

  function opSplit() {
    const t = ed.playhead;
    const inDay = new Set(day().tracks);
    const targets = selectedIndices().length ? new Set(selectedIndices()) : null;
    const next = [];
    const newSel = new Set();
    ed.clips.forEach((c, i) => {
      const tau = toTrack(c.track, t);
      const hit = inDay.has(c.track) && (!targets || targets.has(i)) && tau > c.t0 + MIN_CLIP && tau < c.t1 - MIN_CLIP;
      if (!hit) { next.push(c); return; }
      next.push({ ...c, t1: tau }, { ...c, t0: tau });
      newSel.add(keyOf({ ...c, t0: tau }));
    });
    if (next.length === ed.clips.length) { toast(tr('sync.toast.noSplit')); return; }
    ed.sel = newSel;
    commit(next);
  }

  function opMerge() {
    const sel = selectedIndices();
    if (!sel.length) return;
    const next = clone(ed.clips);
    const remove = new Set();
    const byTrack = new Map();
    sel.forEach((i) => { const c = next[i]; if (!byTrack.has(c.track)) byTrack.set(c.track, []); byTrack.get(c.track).push(i); });
    let merged = 0;
    for (const [track, idxs] of byTrack) {
      const list = next.map((c, i) => [c, i]).filter(([c]) => c.track === track).sort((a, b) => a[0].t0 - b[0].t0);
      const positions = idxs.map((i) => list.findIndex(([, j]) => j === i)).sort((a, b) => a - b);
      const first = positions[0];
      const last = positions.length > 1 ? positions[positions.length - 1] : first + 1;
      if (last >= list.length) continue;
      const base = list[first][0];
      base.t1 = list[last][0].t1;
      base.deleted = list.slice(first, last + 1).every(([c]) => c.deleted);
      for (let p = first + 1; p <= last; p++) remove.add(list[p][1]);
      merged++;
    }
    if (!merged) { toast(tr('sync.toast.noJoin')); return; }
    ed.sel = new Set(sel.filter((i) => !remove.has(i)).map((i) => keyOf(next[i])));
    commit(next.filter((_, i) => !remove.has(i)));
  }

  function opDelete() {
    const sel = selectedIndices();
    if (!sel.length) return;
    const restore = sel.every((i) => ed.clips[i].deleted);
    const next = clone(ed.clips);
    sel.forEach((i) => { next[i].deleted = !restore; });
    commit(next);
  }

  function opMode(mode) {
    const sel = selectedIndices();
    if (!sel.length) return;
    if (mode === 'stereo' && P().labels.length < 2) { toast(tr('sync.toast.stereoNeedsTwo')); return; }
    const next = clone(ed.clips);
    sel.forEach((i) => { if (P().labels.indexOf(labelOf(next[i].track)) < 2) next[i].mode = mode; });
    commit(next);
  }

  function runOp(op) {
    if (!P() || sy.busy) return;
    if (op === 'split') opSplit();
    else if (op === 'merge') opMerge();
    else if (op === 'delete') opDelete();
    else if (op === 'stereo' || op === 'mono') opMode(op);
    else if (op === 'undo') undo();
    else if (op === 'redo') redo();
    else if (op === 'zoom-in') zoomAt(1 / 1.6, LABEL_W + (ed.width - LABEL_W) / 2);
    else if (op === 'zoom-out') zoomAt(1.6, LABEL_W + (ed.width - LABEL_W) / 2);
    else if (op === 'fit') { fitDay(); clampView(); schedulePeaks(); draw(); }
  }

  async function commit(next) {
    ed.history.push(clone(ed.committed));
    if (ed.history.length > 200) ed.history.shift();
    ed.future = [];
    await sendClips(next);
  }
  function undo() {
    if (!ed.history.length) return;
    ed.future.push(clone(ed.committed));
    sendClips(ed.history.pop());
  }
  function redo() {
    if (!ed.future.length) return;
    ed.history.push(clone(ed.committed));
    sendClips(ed.future.pop());
  }

  async function sendClips(clips) {
    const refs = selectedIndices().map((i) => ({ track: ed.clips[i].track, t0: ed.clips[i].t0 }));
    const selKeys = new Set(ed.sel);
    try {
      const upd = await invoke('sync_set_clips', { clips });
      ed.clips = upd.clips;
      ed.committed = clone(upd.clips);
      sy.plan.clips = upd.clips;
      sy.plan.items = upd.items;
      sy.plan.edited = upd.edited;
      ed.sel = new Set(upd.clips.filter((c) => selKeys.has(keyOf(c)) || refs.some((r) => r.track === c.track && Math.abs(r.t0 - c.t0) < 0.002)).map(keyOf));
      sy.outcomes.clear();
      E.done.hidden = true;
      E.retry.hidden = true;
      renderOutputs();
    } catch (e) {
      toast(String(e), 'bad');
      ed.clips = clone(ed.committed);
    }
    updateToolbar();
    draw();
  }

  function updateToolbar() {
    const n = selectedIndices().length;
    E.editor.querySelectorAll('[data-op]').forEach((b) => {
      const op = b.dataset.op;
      if (['merge', 'delete', 'stereo', 'mono'].includes(op)) b.disabled = !n || sy.busy;
      if (op === 'undo') b.disabled = !ed.history.length || sy.busy;
      if (op === 'redo') b.disabled = !ed.future.length || sy.busy;
    });
    const del = E.editor.querySelector('[data-op="delete"]');
    if (del) del.textContent = tr(selectionDeleted() ? 'sync.op.restore' : 'sync.op.delete');
    E.reset.hidden = !(P() && P().edited);
  }

  /* ---------- playback ---------- */

  function seek(t) {
    ed.playhead = t;
    ed.posT = t;
    ed.posAt = performance.now();
    invoke('player_seek', { t }).catch(() => {});
    draw();
  }
  function togglePlay() {
    if (!P()) return;
    if (ed.playing) invoke('player_pause').catch((e) => toast(String(e), 'bad'));
    else invoke('player_play', { t: ed.playhead }).catch((e) => toast(String(e), 'bad'));
  }
  function followPlayhead() {
    const x = X(playheadNow());
    if (x > ed.width - 30 || x < LABEL_W) {
      ed.view.t0 = playheadNow() - visibleSpan() * 0.1;
      clampView();
      schedulePeaks();
    }
  }
  function loop() {
    if (!ed.playing) return;
    followPlayhead();
    draw();
    requestAnimationFrame(loop);
  }
  listen('player-position', ({ payload: p }) => {
    if (p.error) toast(p.error, 'bad');
    const was = ed.playing;
    ed.playing = p.playing;
    ed.posT = p.t;
    ed.posAt = performance.now();
    ed.playhead = p.t;
    E.play.textContent = p.playing ? '❚❚' : '▶';
    if (p.playing && !was) requestAnimationFrame(loop);
    if (!p.playing) draw();
  });
  E.play.addEventListener('click', togglePlay);

  document.addEventListener('keydown', (e) => {
    if (document.body.dataset.mode !== 'sync' || !P() || E.results.hidden) return;
    if ((e.target.closest && e.target.closest('input, textarea, select, [contenteditable]')) || !q('#info').hidden) return;
    const cmd = e.metaKey || e.ctrlKey;
    let handled = true;
    if (e.key === ' ') togglePlay();
    else if (cmd && e.key.toLowerCase() === 'z') (e.shiftKey ? redo : undo)();
    else if (cmd && (e.key === '+' || e.key === '=')) runOp('zoom-in');
    else if (cmd && e.key === '-') runOp('zoom-out');
    else if (cmd) handled = false;
    else if (e.key === 's' || e.key === 'S') runOp('split');
    else if (e.key === 'j' || e.key === 'J') runOp('merge');
    else if (e.key === 'Delete' || e.key === 'Backspace') runOp('delete');
    else if (e.key === 'ArrowUp') runOp('stereo');
    else if (e.key === 'ArrowDown') runOp('mono');
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') seek(ed.playhead + (e.key === 'ArrowLeft' ? -1 : 1) * (e.shiftKey ? 10 : 1));
    else if (e.key === '0') runOp('fit');
    else if (e.key === 'Escape') { ed.sel.clear(); hideMenu(); updateToolbar(); draw(); }
    else handled = false;
    if (handled) e.preventDefault();
  });

  E.editor.querySelector('.ed-toolbar').addEventListener('click', (e) => {
    const b = e.target.closest('[data-op]');
    if (b && !b.disabled) runOp(b.dataset.op);
  });
  E.reset.addEventListener('click', async () => {
    if (!P() || sy.busy) return;
    ed.history.push(clone(ed.committed));
    ed.future = [];
    try {
      const upd = await invoke('sync_reset_clips');
      ed.clips = upd.clips; ed.committed = clone(upd.clips);
      Object.assign(sy.plan, { clips: upd.clips, items: upd.items, edited: upd.edited });
      ed.sel.clear();
      renderOutputs();
    } catch (err) { toast(String(err), 'bad'); }
    updateToolbar();
    draw();
  });

  new ResizeObserver(() => { if (P()) { const keep = ed.view.t0; draw(); ed.view.t0 = keep; schedulePeaks(); } }).observe(E.wrap);
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => { ed.hatch.clear(); draw(); });

  /* ---------- analysis and writing ---------- */

  async function choose() {
    if (busyAny()) return;
    try {
      const p = await invoke('pick_folder', { title: tr('sync.pickTitle') });
      if (p) analyze([p]);
    } catch (e) { toast(String(e), 'bad'); }
  }

  /** The backend's cancel message, in whichever language it was sent. */
  function isCancelled(e) {
    const entry = I18N.entry('sync.cancelledError') || {};
    return I18N.LANGS.some((l) => entry[l] && String(e).includes(entry[l]));
  }

  async function analyze(paths) {
    if (busyAny() || !paths || !paths.length) return;
    sy.inputs = paths;
    sy.busy = true;
    sy.decoding = false;
    window.beat.last = Date.now();
    E.abar.style.width = '0%';
    E.atext.textContent = tr('sync.searching');
    E.analyzing.hidden = false;
    try {
      const plan = await invoke('analyze_tracks', { paths });
      sy.plan = plan;
      sy.outcomes.clear();
      E.done.hidden = true;
      E.retry.hidden = true;
      E.finished.hidden = true;
      ed.clips = plan.clips;
      ed.committed = clone(plan.clips);
      ed.history = []; ed.future = []; ed.sel.clear(); ed.peaks.clear(); ed.dayIndex = 0;
      ed.playing = false;
      buildDays();
      if (plan.labels.length < 2) toast(tr('sync.toast.oneSender'));
      if (plan.edited) toast(tr('sync.toast.editRestored'));
    } catch (e) {
      if (!isCancelled(e)) toast(String(e), 'bad');
    } finally {
      sy.busy = false;
      E.analyzing.hidden = true;
      render();
      if (sy.plan && day()) {
        ed.width = Math.max(320, E.wrap.clientWidth);
        fitDay();
        clampView();
        ed.playhead = day().t0;
        seek(ed.playhead);
        schedulePeaks();
        draw();
      }
    }
  }
  window.syncAnalyze = analyze;

  async function runWrite(retryIds) {
    if (!P() || busyAny()) return;
    const ids = retryIds || P().items.map((i) => i.id);
    if (!ids.length) { toast(tr('sync.toast.nothingToCreate')); return; }
    if (!retryIds) {
      let out = null;
      try {
        out = await window.pickOutput(tr('sync.outputTitle'), window.parentDir(P().default_out_dir), 'sync');
      } catch (e) { toast(String(e), 'bad'); return; }
      if (!out) return;
      sy.outDir = out;
    }
    if (ed.playing) invoke('player_pause').catch(() => {});
    sy.busy = true;
    window.beat.last = Date.now();
    setBusyUi(true);
    sy.outcomes.clear();
    E.done.hidden = true;
    E.retry.hidden = true;
    E.bar.style.width = '0%';
    E.ptext.textContent = tr('sync.preparing');
    renderOutputs();
    try {
      const sum = await invoke('write_sync', { ids, outDir: sy.outDir });
      for (const o of sum.outcomes) sy.outcomes.set(o.id, o);
      finishRun(sum);
    } catch (e) {
      toast(String(e), 'bad');
    } finally {
      sy.busy = false;
      sy.activeId = null;
      setBusyUi(false);
      render();
    }
  }

  function finishRun(sum) {
    const bad = sum.outcomes.filter((o) => o.status === 'failed' || o.status === 'cancelled');
    sy.lastSum = sum;
    if (!bad.length) {
      sy.outcomes.clear();
      showFinished(sum);
      return;
    }
    const left = [...new Set(bad.map((o) => o.id))];
    E.done.className = 'done partial';
    renderPartial();
    E.done.hidden = false;
    E.retry.hidden = false;
    E.retry.onclick = () => runWrite(left);
  }

  function showFinished(sum) {
    window.finishedCard(E.finished, sum, {
      noun: [tr('sync.noun.file.one'), tr('sync.noun.file.other')],
      openLabel: tr('sync.finished.open'), nextLabel: tr('sync.finished.next'), newLabel: tr('sync.finished.back'),
      onNext: () => { setMode('master'); if (window.masterAnalyze) window.masterAnalyze([sum.out_dir]); },
      onNew: () => { render(); draw(); },
    });
  }

  function renderPartial() {
    const sum = sy.lastSum;
    if (!sum) return;
    const bad = sum.outcomes.filter((o) => o.status === 'failed' || o.status === 'cancelled');
    const done = sum.outcomes.length - bad.length;
    E.done.innerHTML = `<span>${esc(trn('sync.partial', bad.length, { done, total: sum.outcomes.length }))}</span>`;
  }

  function setBusyUi(b) {
    E.progress.hidden = !b;
    E.cancel.hidden = !b;
    E.go.hidden = b;
    updateToolbar();
  }

  /* ---------- rendering of the page ---------- */

  function render() {
    const plan = P();
    const finished = !E.finished.hidden;
    E.empty.hidden = !!plan || finished;
    E.results.hidden = !plan || finished;
    E.bottom.hidden = !plan || finished;
    E.topActions.hidden = !plan;
    if (!plan) return;

    const sw = (bg, extra = '') => `<i style="background:${bg};${extra}"></i>`;
    const senders = plan.labels.map((l, i) => {
      const c = COLORS[i % COLORS.length];
      const role = tr(plan.labels.length >= 2 && i < 2 ? (i === 0 ? 'sync.legend.left' : 'sync.legend.right') : 'sync.legend.monoOnly');
      return `<span>${sw(c)}${esc(tr('sync.legend.sender', { label: l, role }))}</span>`;
    }).join('');
    E.legend.innerHTML = `${senders}
      <span>${sw(tint(COLORS[0], 0.72))}${sw(tint(COLORS[1], 0.72))}${esc(tr('sync.legend.mono'))}</span>
      <span><i class="lg-cut"></i>${esc(tr('sync.legend.cut'))}</span>
      <span><i class="lg-hit"></i>${esc(tr('sync.legend.events'))}</span>`;
    E.days.innerHTML = ed.days.length > 1
      ? ed.days.map((d, i) => `<button data-day="${i}" class="${i === ed.dayIndex ? 'on' : ''}">${esc(d.key)}</button>`).join('')
      : '';
    renderOutputs();
    updateToolbar();
  }

  function renderOutputs() {
    const plan = P();
    if (!plan) return;
    const okPairs = plan.pairs.filter((p) => p.ok);
    const failedIds = new Set([...sy.outcomes.values()].filter((o) => o.status === 'failed' || o.status === 'cancelled').map((o) => o.id));
    const items = failedIds.size && !sy.busy ? plan.items.filter((i) => failedIds.has(i.id)) : plan.items;
    const stereo = plan.items.filter((i) => i.kind === 'stereo');
    const mono = plan.items.filter((i) => i.kind === 'mono');
    E.summary.innerHTML = `
      <div class="stat lead"><div class="n">${stereo.length}</div><div class="l">${esc(trn('sync.stat.stereo', stereo.length, { dur: fmtDur(stereo.reduce((a, i) => a + i.duration, 0)) }))}</div></div>
      <div class="stat"><div class="n">${mono.length}</div><div class="l">${esc(trn('sync.stat.mono', mono.length))}</div></div>
      <div class="stat"><div class="n">${okPairs.length}<span class="of"> / ${plan.pairs.length}</span></div><div class="l">${esc(tr('sync.stat.pairs'))}</div></div>
      <div class="stat"><div class="n">${plan.tracks.length}</div><div class="l">${esc(trn('sync.stat.tracks', plan.labels.length, { source: plan.source === 'chunks' ? tr('sync.stat.fromChunks') : plan.source === 'mixed' ? tr('sync.stat.partlyFromChunks') : '' }))}</div></div>
      <div class="roots" title="${esc(plan.roots.join('\n'))}">${esc(tr('sync.analysed', { roots: plan.roots.join(' · ') }))}</div>`;
    E.pairs.innerHTML = plan.pairs.length ? plan.pairs.map(pairHtml).join('') : `<div class="pair"><div class="facts">${esc(tr('sync.pairs.none'))}</div></div>`;
    let html = '';
    let dayKey = null;
    for (const it of items) {
      if (it.day !== dayKey) { dayKey = it.day; html += `<div class="day">${esc(dayKey)}</div>`; }
      html += itemHtml(it);
    }
    E.list.innerHTML = html || `<p class="day">${esc(tr('sync.list.empty'))}</p>`;
    E.editState.textContent = tr(plan.edited ? 'sync.editState.edited' : 'sync.editState.proposal');
    if (plan.ignored.length) {
      E.extras.hidden = false;
      E.extrasSummary.textContent = trn('sync.unused', plan.ignored.length);
      E.extrasBody.innerHTML = `<ul>${plan.ignored.map((x) => `<li>${esc(x.path)} <span>– ${esc(x.reason)}</span></li>`).join('')}</ul>`;
    } else {
      E.extras.hidden = true;
    }
    const bytes = plan.items.reduce((s, i) => s + i.bytes, 0);
    E.sel.textContent = plan.items.length ? trn('sync.files', plan.items.length, { size: fmtBytes(bytes) }) : tr('sync.nothingToCreate');
    E.go.disabled = !plan.items.length || sy.busy;
  }

  function pairHtml(p) {
    const plan = P();
    const a = plan.tracks[p.a];
    const b = plan.tracks[p.b];
    const who = `${esc(a.label)} ${a.start}–${a.end} <span class="with">${esc(tr('sync.pair.with'))}</span> ${esc(b.label)} ${b.start}–${b.end}`;
    if (!p.ok) return `<div class="pair"><div class="who">${who}</div><div class="verdict no">${esc(tr('sync.pair.notInSync'))}</div><div class="facts">${esc(p.note || '')}</div></div>`;
    const act = p.frames.filter((f) => f.active);
    const hits = act.filter((f) => f.hit).length;
    return `<div class="pair"><div class="who">${who}</div><div class="verdict ok">${esc(tr('sync.pair.inSync'))}</div>
      <div class="facts">${esc(tr('sync.pair.facts', { offset: signed(p.offset, 3), drift: signed(p.drift_ppm, 1), spread: fixed(p.resid_ms, 1), hits: act.length ? Math.round((hits / act.length) * 100) : 0 }))}</div></div>`;
  }

  function itemHtml(it) {
    const plan = P();
    const o = sy.outcomes.get(it.id);
    const a = plan.tracks[it.left];
    const badge = it.kind === 'stereo'
      ? `<span class="parts-badge">${esc(tr('sync.badge.stereo', { left: plan.labels[0], right: plan.labels[1] || '' }))}</span>`
      : `<span class="parts-badge mono">${esc(tr('sync.badge.mono', { label: a.label }))}</span>`;
    const gap = (it.silent || []).filter((x) => x.seconds >= 1).map((x) => ` · ${esc(tr('sync.item.missing', { label: x.label, dur: fmtDur(x.seconds) }))}`).join('');
    const why = it.kind === 'stereo'
      ? `${esc(tr('sync.item.why', { hits: percent(it.hit_share), coh: it.msc == null ? '–' : fixed(it.msc, 2) }))}${gap}`
      : esc(tr(it.reason === 'getrennt' ? 'sync.item.apart' : 'sync.item.alone'));
    const srcTracks = it.kind === 'stereo' ? [...it.left_sources, ...it.right_sources].map((s) => s.track) : [it.left];
    const formats = [...new Set(srcTracks.map((i) => plan.tracks[i] && plan.tracks[i].decoded_from).filter(Boolean))];
    const decoded = formats.length ? ` · ${esc(tr('sync.item.decoded', { format: formats.join(', ') }))}` : '';
    const label = {
      written: esc(tr('sync.status.written')), existing: esc(tr('sync.status.existing')),
      failed: esc(tr('sync.status.failed')), cancelled: esc(tr('sync.status.cancelled')),
    };
    let status = '';
    if (o) {
      status = o.path
        ? `<div class="status ${o.status}"><button data-reveal="${esc(o.path)}" title="${esc(tr('sync.item.reveal'))}">${label[o.status]}</button></div>`
        : `<div class="status ${o.status}">${label[o.status]}</div>`;
    } else if (sy.activeId === it.id) {
      status = `<div class="status running">${esc(tr('sync.item.writing'))}</div>`;
    }
    return `
    <div class="rec clickable${sy.activeId === it.id ? ' active' : ''}" data-item="${it.id}" title="${esc(tr('sync.item.show'))}">
      <span></span>
      <div class="when">
        <span class="time">${esc(it.start)} – ${esc(it.end)}</span>
        <span class="dur">${fmtDur(it.duration)}</span>
        ${badge}
      </div>
      ${status}
      <div class="meta"><span>${why}${decoded} · ${fmtBytes(it.bytes)}</span><span class="out">${esc(it.name)}</span></div>
      ${o && o.message ? `<div class="errmsg">${esc(o.message)}</div>` : ''}
    </div>`;
  }

  /* ---------- page events ---------- */

  q('#sync-pick').addEventListener('click', choose);
  q('#sync-pick-again').addEventListener('click', choose);
  q('#sync-rescan').addEventListener('click', () => analyze(sy.inputs));
  q('#sync-acancel').addEventListener('click', () => { E.atext.textContent = tr('sync.cancelling'); invoke('cancel_merge'); });
  E.go.addEventListener('click', () => runWrite());
  E.cancel.addEventListener('click', () => { E.ptext.textContent = tr('sync.cancelling'); invoke('cancel_merge'); });
  E.days.addEventListener('click', (e) => {
    const b = e.target.closest('[data-day]');
    if (!b) return;
    ed.dayIndex = Number(b.dataset.day);
    ed.peaks.clear();
    render();
    fitDay(); clampView(); seek(day().t0); schedulePeaks(); draw();
  });
  E.list.addEventListener('click', (e) => {
    if (e.target.closest('[data-reveal]')) return;
    const row = e.target.closest('[data-item]');
    if (!row) return;
    const it = P().items.find((i) => i.id === Number(row.dataset.item));
    if (!it) return;
    const t = it.kind === 'stereo' ? it.t0 : toTimeline(it.left, it.t0);
    const di = ed.days.findIndex((d) => t >= d.t0 - 1 && t <= d.t1 + 1);
    if (di >= 0 && di !== ed.dayIndex) { ed.dayIndex = di; ed.peaks.clear(); render(); fitDay(); }
    const span = it.kind === 'stereo' ? it.t1 - it.t0 : it.duration;
    ed.view.spp = Math.max(MIN_SPP, (span * 1.2) / Math.max(100, ed.width - LABEL_W));
    ed.view.t0 = t - span * 0.1;
    clampView();
    seek(t);
    schedulePeaks();
    E.editor.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    draw();
  });

  listen('sync-progress', ({ payload: p }) => {
    let frac = 0.02;
    const part = p.total ? p.done / p.total : 1;
    // Decoding non-WAV files (only when there are any) takes the part of the bar before the frequency analysis.
    if (p.stage === 'decode') { sy.decoding = true; frac = 0.02 + 0.28 * part; }
    else if (p.stage === 'envelope') { const base = sy.decoding ? 0.3 : 0.05; frac = base + (0.7 - base) * part; }
    else if (p.stage === 'pairs') frac = 0.7 + 0.3 * part;
    E.abar.style.width = `${(frac * 100).toFixed(1)}%`;
    const bytes = p.stage === 'envelope' || p.stage === 'decode';
    const detail = bytes ? tr('sync.progress.bytes', { done: fmtBytes(p.done), total: fmtBytes(p.total) }) : p.stage === 'pairs' ? trn('sync.progress.pairs', p.total, { done: p.done }) : '';
    E.atext.textContent = detail ? `${p.text} · ${detail}` : p.text;
  });

  listen('sync-write-progress', ({ payload: p }) => {
    const frac = p.total ? p.done / p.total : 0;
    E.bar.style.width = `${(frac * 100).toFixed(1)}%`;
    E.ptext.textContent = tr('sync.progress.write', { index: p.index + 1, count: p.count, name: p.name, pct: Math.floor(frac * 100) });
    if (sy.activeId !== p.id) {
      sy.activeId = p.id;
      E.list.querySelectorAll('.rec.active').forEach((n) => n.classList.remove('active'));
      const row = E.list.querySelector(`.rec[data-item="${p.id}"]`);
      if (row) row.classList.add('active');
    }
  });

  window.addEventListener('resize', () => draw());
  I18N.onChange(() => {
    hideMenu();
    if (!E.finished.hidden && sy.lastSum) showFinished(sy.lastSum);
    if (!E.done.hidden) renderPartial();
    render();
    draw();
  });
  render();
})();
