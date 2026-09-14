'use strict';

/* Third function: measure loudness, master to -16 LUFS, MP3 192 kbit/s.
   Shares esc, fmtDur, fmtBytes, toast and setMode with app.js. Texts: ui/i18n/master.js. */
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const tr = (k, p) => I18N.t(k, p);
  const trn = (k, n, p) => I18N.tn(k, n, p);
  const q = (sel) => document.querySelector(sel);
  const E = {
    empty: q('#master-empty'), results: q('#master-results'), summary: q('#master-summary'), list: q('#master-list'),
    extras: q('#master-extras'), extrasSummary: q('#master-extras-summary'), extrasBody: q('#master-extras-body'),
    bottom: q('#master-bottom'), retry: q('#master-retry'), finished: q('#master-finished'), progress: q('#master-progress'), bar: q('#master-bar'),
    ptext: q('#master-ptext'), done: q('#master-done'), sel: q('#master-sel'), go: q('#master-go'), cancel: q('#master-cancel'),
    analyzing: q('#master-analyzing'), abar: q('#master-abar'), atext: q('#master-atext'), topActions: q('#master-top-actions'),
  };
  const ms = { inputs: [], plan: null, outDir: '', selected: new Set(), outcomes: new Map(), busy: false, activeId: null, stages: new Map() };
  window.masterState = ms;
  // Remembered so a language switch can redraw texts that are not part of render().
  const ui = { finishedSum: null, partial: null, analyzeP: null, writeP: null, cancelling: false };

  const busyAny = () => ms.busy || !!(window.mergeState && window.mergeState.busy) || !!(window.syncState && window.syncState.busy);
  const dec = (v, d = 1) => v.toLocaleString(I18N.locale(), { minimumFractionDigits: d, maximumFractionDigits: d });
  const signed = (v, d = 1) => `${v < 0 ? '−' : '+'}${dec(Math.abs(v), d)}`;
  const lufs = (v) => (Number.isFinite(v) ? `${signed(v)} LUFS` : '–');
  const dbtp = (v) => (Number.isFinite(v) ? `${signed(v)} dBTP` : '–');
  const METER_MIN = -60;
  const meterX = (v) => `${Math.max(0, Math.min(100, ((v - METER_MIN) / -METER_MIN) * 100)).toFixed(1)}%`;
  // The backend answers a cancel with its (translated) word for "cancelled"; accept it in any language.
  const isCancel = (e) => {
    const s = String(e).toLowerCase();
    const entry = I18N.entry('master.cancelledError') || {};
    return Object.values(entry).some((w) => s.includes(String(w).toLowerCase()));
  };

  /* ---------- actions ---------- */

  async function choose() {
    if (busyAny()) return;
    try {
      const p = await invoke('pick_folder', { title: tr('master.pickTitle') });
      if (p) analyze([p]);
    } catch (e) { toast(String(e), 'bad'); }
  }

  async function analyze(paths) {
    if (busyAny() || !paths || !paths.length) return;
    ms.inputs = paths;
    ms.busy = true;
    ui.analyzeP = null;
    ui.cancelling = false;
    E.abar.style.width = '0%';
    E.atext.textContent = tr('master.searching');
    E.analyzing.hidden = false;
    try {
      const plan = await invoke('analyze_master', { paths });
      ms.plan = plan;
      ms.outDir = plan.default_out_dir;
      ms.outcomes.clear();
      // `dji_part` is the backend's field name for a raw recording chunk; only the label is shown.
      const onlyParts = plan.files.every((f) => f.dji_part);
      ms.selected = new Set(plan.files.filter((f) => f.gain_db != null && (onlyParts || !f.dji_part)).map((f) => f.id));
      E.done.hidden = true;
      ui.partial = null;
      E.retry.hidden = true;
      E.finished.hidden = true;
      ui.finishedSum = null;
    } catch (e) {
      if (!isCancel(e)) toast(String(e), 'bad');
    } finally {
      ms.busy = false;
      ui.analyzeP = null;
      ui.cancelling = false;
      E.analyzing.hidden = true;
      render();
    }
  }
  window.masterAnalyze = analyze;

  async function runWrite(retryIds) {
    if (!ms.plan || busyAny()) return;
    const ids = retryIds || ms.plan.files.filter((f) => ms.selected.has(f.id)).map((f) => f.id);
    if (!ids.length) return;
    if (!retryIds) {
      let out = null;
      try {
        out = await window.pickOutput(tr('master.outTitle'), window.parentDir(ms.plan.default_out_dir), 'master');
      } catch (e) { toast(String(e), 'bad'); return; }
      if (!out) return;
      ms.outDir = out;
    }
    ms.busy = true;
    window.beat.last = Date.now();
    setBusyUi(true);
    ms.outcomes.clear();
    ms.stages.clear();
    E.done.hidden = true;
    ui.partial = null;
    E.retry.hidden = true;
    E.bar.style.width = '0%';
    ui.writeP = null;
    ui.cancelling = false;
    E.ptext.textContent = tr('master.preparing');
    render();
    try {
      const sum = await invoke('write_master', { ids, outDir: ms.outDir });
      for (const o of sum.outcomes) ms.outcomes.set(o.id, o);
      finishRun(sum);
    } catch (e) {
      toast(String(e), 'bad');
    } finally {
      ms.busy = false;
      ms.stages.clear();
      ui.writeP = null;
      ui.cancelling = false;
      setBusyUi(false);
      render();
    }
  }

  function showFinished(sum) {
    window.finishedCard(E.finished, sum, {
      noun: [tr('master.noun.one'), tr('master.noun.other')],
      openLabel: tr('master.openLabel'),
      newLabel: tr('master.newLabel'),
      onNew: () => { ui.finishedSum = null; render(); },
    });
  }

  function renderPartial() {
    const p = ui.partial;
    if (!p) return;
    E.done.innerHTML = `<span>${esc(trn('master.partial', p.bad, { done: p.done, total: p.total }))}</span>`;
  }

  function finishRun(sum) {
    const bad = sum.outcomes.filter((o) => o.status === 'failed' || o.status === 'cancelled');
    if (!bad.length) {
      ms.plan = null;
      ms.selected.clear();
      ms.outcomes.clear();
      ui.finishedSum = sum;
      showFinished(sum);
      return;
    }
    const left = new Set(bad.map((o) => o.id));
    ms.plan.files = ms.plan.files.filter((f) => left.has(f.id));
    ms.selected = new Set(left);
    ui.partial = { done: sum.outcomes.length - bad.length, total: sum.outcomes.length, bad: bad.length };
    E.done.className = 'done partial';
    renderPartial();
    E.done.hidden = false;
    E.retry.hidden = false;
    E.retry.onclick = () => runWrite([...left]);
  }

  function setBusyUi(b) {
    E.progress.hidden = !b;
    E.cancel.hidden = !b;
    E.go.hidden = b;
  }

  /* ---------- rendering ---------- */

  function render() {
    const P = ms.plan;
    E.empty.hidden = !!P || !E.finished.hidden;
    E.results.hidden = !P;
    E.bottom.hidden = !P;
    E.topActions.hidden = !P;
    if (!P) return;

    const measured = P.files.filter((f) => f.loudness && Number.isFinite(f.loudness.lufs) && f.gain_db != null);
    const total = P.files.reduce((s, f) => s + f.duration, 0);
    const range = measured.length
      ? tr('master.rangeTo', {
        from: lufs(Math.min(...measured.map((f) => f.loudness.lufs))),
        to: lufs(Math.max(...measured.map((f) => f.loudness.lufs))),
      })
      : '–';
    E.summary.innerHTML = `
      <div class="stat lead"><div class="n">${signed(P.target_lufs, 0)}</div><div class="l">${esc(tr('master.stat.target', { kbps: P.bitrate_kbps, peak: signed(P.ceiling_dbtp) }))}</div></div>
      <div class="stat"><div class="n">${P.files.length}</div><div class="l">${esc(trn('master.stat.files', P.files.length, { dur: fmtDur(total) }))}</div></div>
      <div class="stat"><div class="n" style="font-size:16px;line-height:32px">${esc(range)}</div><div class="l">${esc(tr('master.stat.range'))}</div></div>
      <div class="roots" title="${esc(P.roots.join('\n'))}">${esc(tr('master.analysed', { roots: P.roots.join(' · ') }))}</div>`;

    let html = '';
    let folder = null;
    for (const f of P.files) {
      if (f.folder !== folder) { folder = f.folder; html += `<div class="day">${esc(folder || tr('master.folderFallback'))}</div>`; }
      html += fileHtml(f);
    }
    E.list.innerHTML = html;

    if (P.ignored.length) {
      E.extras.hidden = false;
      E.extrasSummary.textContent = trn('master.ignored', P.ignored.length);
      E.extrasBody.innerHTML = `<ul>${P.ignored.map((x) => `<li>${esc(x.path)} <span>– ${esc(x.reason)}</span></li>`).join('')}</ul>`;
    } else {
      E.extras.hidden = true;
    }
    updateSelection();
  }

  function fileHtml(f) {
    const P = ms.plan;
    const on = ms.selected.has(f.id);
    const o = ms.outcomes.get(f.id);
    const l = f.loudness;
    const label = (s) => esc(tr(`master.status.${s}`));
    let status = '';
    if (o) {
      status = o.path
        ? `<div class="status ${o.status}"><button data-reveal="${esc(o.path)}" title="${esc(tr('master.revealTitle'))}">${label(o.status)}</button></div>`
        : `<div class="status ${o.status}">${label(o.status)}</div>`;
    } else if (ms.busy && ms.stages.has(f.id)) {
      status = `<div class="status running">${esc(ms.stages.get(f.id))}</div>`;
    } else if (ms.busy && on) {
      status = `<div class="status existing">${esc(tr('master.waiting'))}</div>`;
    }
    const canMaster = f.gain_db != null;
    const measuredText = l && Number.isFinite(l.lufs)
      ? tr('master.measured', { lufs: lufs(l.lufs), tp: dbtp(l.true_peak), lra: dec(l.lra) })
      : tr('master.noLoudness');
    const gainText = canMaster
      ? ` · ${tr('master.gain', { gain: signed(f.gain_db) })}${f.limited_db >= 0.5 ? ` · ${tr('master.limiter', { db: dec(f.limited_db) })}` : ''}`
      : '';
    const result = o && o.result
      ? `<span class="result">${esc(tr('master.result', { lufs: lufs(o.result.lufs), tp: dbtp(o.result.true_peak) }))}</span>`
      : '';
    const meter = l && Number.isFinite(l.lufs)
      ? `<div class="meter" title="${esc(tr('master.meterTitle'))}"><i class="m-target" style="left:${meterX(P.target_lufs)}"></i><i class="m-in" style="left:${meterX(l.lufs)}"></i>${o && o.result ? `<i class="m-out" style="left:${meterX(o.result.lufs)}"></i>` : ''}</div>`
      : '';
    return `
    <div class="rec${on ? '' : ' off'}" data-id="${f.id}">
      <input type="checkbox" data-mid="${f.id}" ${on ? 'checked' : ''} ${ms.busy || !canMaster ? 'disabled' : ''} aria-label="${esc(tr('master.selectFile'))}">
      <div class="when">
        <span class="time">${esc(f.name)}</span>
        <span class="dur">${fmtDur(f.duration)}</span>
        <span class="parts-badge mono">${esc(f.format)}</span>
        ${f.dji_part ? `<span class="tag" title="${esc(tr('master.partTitle'))}">${esc(tr('master.partTag'))}</span>` : ''}
      </div>
      ${status}
      <div class="meta"><span>${fmtBytes(f.size)}</span><span class="out">${esc(f.out_name)}</span></div>
      <div class="loud">${meter}<span>${esc(measuredText + gainText)}</span>${result}</div>
      ${f.note ? `<div class="note">${esc(f.note)}</div>` : ''}
      ${o && o.message ? `<div class="errmsg">${esc(o.message)}</div>` : ''}
    </div>`;
  }

  function updateSelection() {
    const P = ms.plan;
    if (!P) return;
    const chosen = P.files.filter((f) => ms.selected.has(f.id));
    const secs = chosen.reduce((s, f) => s + f.duration, 0);
    E.sel.textContent = chosen.length ? trn('master.sel', chosen.length, { dur: fmtDur(secs) }) : tr('master.nothingSelected');
    E.go.disabled = !chosen.length || ms.busy;
  }

  function analyzeText(p) {
    return p.stage === 'measure'
      ? tr('master.measureProgress', { text: p.text, done: fmtDur(p.done / 1000), total: fmtDur(p.total / 1000) })
      : p.text;
  }

  function writeText(p) {
    const frac = p.total ? Math.min(1, p.done / p.total) : 0;
    const stagesNow = [...new Set((p.active || []).map((a) => a.stage))].join(', ');
    const head = tr('master.writeProgress', { index: p.index, count: p.count, pct: Math.floor(frac * 100) });
    return stagesNow ? `${head} · ${stagesNow}` : head;
  }

  /* ---------- events ---------- */

  q('#master-pick').addEventListener('click', choose);
  q('#master-pick-again').addEventListener('click', choose);
  q('#master-rescan').addEventListener('click', () => analyze(ms.inputs));
  q('#master-acancel').addEventListener('click', () => { ui.cancelling = true; E.atext.textContent = tr('master.cancelling'); invoke('cancel_merge'); });
  E.go.addEventListener('click', () => runWrite());
  E.cancel.addEventListener('click', () => { ui.cancelling = true; E.ptext.textContent = tr('master.cancelling'); invoke('cancel_merge'); });
  q('#master-all').addEventListener('click', () => {
    if (!ms.busy && ms.plan) { ms.plan.files.filter((f) => f.gain_db != null).forEach((f) => ms.selected.add(f.id)); render(); }
  });
  q('#master-none').addEventListener('click', () => { if (!ms.busy && ms.plan) { ms.selected.clear(); render(); } });

  E.list.addEventListener('change', (e) => {
    const id = e.target.dataset && e.target.dataset.mid;
    if (id === undefined || ms.busy) return;
    if (e.target.checked) ms.selected.add(Number(id)); else ms.selected.delete(Number(id));
    const row = e.target.closest('.rec');
    if (row) row.classList.toggle('off', !e.target.checked);
    updateSelection();
  });

  listen('master-progress', ({ payload: p }) => {
    ui.analyzeP = p;
    const frac = p.stage === 'measure' ? 0.05 + 0.95 * (p.total ? p.done / p.total : 1) : 0.03;
    E.abar.style.width = `${(frac * 100).toFixed(1)}%`;
    E.atext.textContent = analyzeText(p);
  });

  listen('master-write-progress', ({ payload: p }) => {
    ui.writeP = p;
    // Counts only finished files: the bar never runs ahead of the work.
    const frac = p.total ? Math.min(1, p.done / p.total) : 0;
    E.bar.style.width = `${(frac * 100).toFixed(1)}%`;
    const active = p.active || [];
    E.ptext.textContent = writeText(p);
    E.ptext.title = E.ptext.textContent;
    ms.stages = new Map(active.map((a) => [a.id, a.stage]));
    E.list.querySelectorAll('.rec').forEach((row) => {
      const id = Number(row.dataset.id);
      if (ms.outcomes.has(id)) return;
      let badge = row.querySelector('.status');
      const text = ms.stages.get(id) || (ms.selected.has(id) ? tr('master.waiting') : '');
      if (!text) { if (badge) badge.remove(); return; }
      if (!badge) { badge = document.createElement('div'); row.insertBefore(badge, row.querySelector('.meta')); }
      badge.className = `status ${ms.stages.has(id) ? 'running' : 'existing'}`;
      badge.textContent = text;
    });
  });

  // Language switch: redraw everything this tab wrote itself.
  I18N.onChange(() => {
    render();
    if (ui.finishedSum && !E.finished.hidden) showFinished(ui.finishedSum);
    if (ui.partial && !E.done.hidden) renderPartial();
    if (!E.analyzing.hidden) {
      E.atext.textContent = ui.cancelling ? tr('master.cancelling') : ui.analyzeP ? analyzeText(ui.analyzeP) : tr('master.searching');
    }
    if (ms.busy && !E.progress.hidden) {
      E.ptext.textContent = ui.cancelling ? tr('master.cancelling') : ui.writeP ? writeText(ui.writeP) : tr('master.preparing');
      E.ptext.title = E.ptext.textContent;
    }
  });

  render();
})();
