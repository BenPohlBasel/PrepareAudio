'use strict';

/* Third function: measure loudness, master to -16 LUFS, MP3 192 kbit/s.
   Shares esc, fmtDur, fmtBytes, plural, toast and setMode with app.js. */
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const q = (sel) => document.querySelector(sel);
  const E = {
    empty: q('#master-empty'), results: q('#master-results'), summary: q('#master-summary'), list: q('#master-list'),
    extras: q('#master-extras'), extrasSummary: q('#master-extras-summary'), extrasBody: q('#master-extras-body'),
    bottom: q('#master-bottom'), outDir: q('#master-out-dir'), progress: q('#master-progress'), bar: q('#master-bar'),
    ptext: q('#master-ptext'), done: q('#master-done'), sel: q('#master-sel'), go: q('#master-go'), cancel: q('#master-cancel'),
    analyzing: q('#master-analyzing'), abar: q('#master-abar'), atext: q('#master-atext'), topActions: q('#master-top-actions'),
  };
  const ms = { inputs: [], plan: null, outDir: '', selected: new Set(), outcomes: new Map(), busy: false, activeId: null };
  window.masterState = ms;

  const busyAny = () => ms.busy || !!(window.mergeState && window.mergeState.busy) || !!(window.syncState && window.syncState.busy);
  const dec = (v, d = 1) => v.toLocaleString('de-DE', { minimumFractionDigits: d, maximumFractionDigits: d });
  const signed = (v, d = 1) => `${v < 0 ? '−' : '+'}${dec(Math.abs(v), d)}`;
  const lufs = (v) => (Number.isFinite(v) ? `${signed(v)} LUFS` : '–');
  const dbtp = (v) => (Number.isFinite(v) ? `${signed(v)} dBTP` : '–');
  const METER_MIN = -60;
  const meterX = (v) => `${Math.max(0, Math.min(100, ((v - METER_MIN) / -METER_MIN) * 100)).toFixed(1)}%`;

  /* ---------- actions ---------- */

  async function choose() {
    if (busyAny()) return;
    try {
      const p = await invoke('pick_folder', { title: 'Ordner mit Audiodateien wählen' });
      if (p) analyze([p]);
    } catch (e) { toast(String(e), 'bad'); }
  }

  async function analyze(paths) {
    if (busyAny() || !paths || !paths.length) return;
    ms.inputs = paths;
    ms.busy = true;
    E.abar.style.width = '0%';
    E.atext.textContent = 'Suche Audiodateien';
    E.analyzing.hidden = false;
    try {
      const plan = await invoke('analyze_master', { paths });
      ms.plan = plan;
      ms.outDir = plan.default_out_dir;
      ms.outcomes.clear();
      const onlyParts = plan.files.every((f) => f.dji_part);
      ms.selected = new Set(plan.files.filter((f) => f.gain_db != null && (onlyParts || !f.dji_part)).map((f) => f.id));
      E.done.hidden = true;
    } catch (e) {
      if (!String(e).includes('Abgebrochen')) toast(String(e), 'bad');
    } finally {
      ms.busy = false;
      E.analyzing.hidden = true;
      render();
    }
  }
  window.masterAnalyze = analyze;

  async function chooseOutDir() {
    if (busyAny()) return;
    try {
      const p = await invoke('pick_folder', { title: 'Wo soll der Ordner „master“ liegen?' });
      if (!p) return;
      const clean = p.replace(/[\\/]$/, '');
      ms.outDir = /[\\/]master$/.test(clean) ? clean : `${clean}/master`;
      ms.outcomes.clear();
      E.done.hidden = true;
      render();
    } catch (e) { toast(String(e), 'bad'); }
  }

  async function runWrite() {
    const ids = ms.plan.files.filter((f) => ms.selected.has(f.id)).map((f) => f.id);
    if (!ids.length || busyAny()) return;
    ms.busy = true;
    setBusyUi(true);
    ms.outcomes.clear();
    E.done.hidden = true;
    E.bar.style.width = '0%';
    E.ptext.textContent = 'Vorbereiten…';
    render();
    try {
      const sum = await invoke('write_master', { ids, outDir: ms.outDir });
      for (const o of sum.outcomes) ms.outcomes.set(o.id, o);
      showDone(sum);
    } catch (e) {
      toast(String(e), 'bad');
    } finally {
      ms.busy = false;
      ms.activeId = null;
      setBusyUi(false);
      render();
    }
  }

  function setBusyUi(b) {
    E.progress.hidden = !b;
    E.cancel.hidden = !b;
    E.go.hidden = b;
  }

  function showDone(sum) {
    const c = { written: 0, existing: 0, failed: 0, cancelled: 0 };
    for (const o of sum.outcomes) c[o.status]++;
    const bits = [];
    if (c.written) bits.push(`${plural(c.written, 'MP3', 'MP3s')} geschrieben`);
    if (c.existing) bits.push(`${c.existing} schon vorhanden`);
    if (c.failed) bits.push(`${c.failed} fehlgeschlagen`);
    if (c.cancelled) bits.push(`${c.cancelled} abgebrochen`);
    const ok = !c.failed && !sum.cancelled;
    E.done.className = `done${ok ? '' : ' partial'}`;
    E.done.innerHTML = `<span>${ok ? 'Fertig' : sum.cancelled ? 'Abgebrochen' : 'Mit Fehlern beendet'}: ${esc(bits.join(', ') || 'nichts zu tun')}.</span>
      <button class="link" id="master-open-out">master öffnen</button>`;
    E.done.hidden = false;
    q('#master-open-out').onclick = () => invoke('reveal', { path: sum.out_dir, select: false }).catch((e) => toast(String(e), 'bad'));
  }

  /* ---------- rendering ---------- */

  function render() {
    const P = ms.plan;
    E.empty.hidden = !!P;
    E.results.hidden = !P;
    E.bottom.hidden = !P;
    E.topActions.hidden = !P;
    if (!P) return;

    const measured = P.files.filter((f) => f.loudness && Number.isFinite(f.loudness.lufs) && f.gain_db != null);
    const total = P.files.reduce((s, f) => s + f.duration, 0);
    const range = measured.length
      ? `${lufs(Math.min(...measured.map((f) => f.loudness.lufs)))} bis ${lufs(Math.max(...measured.map((f) => f.loudness.lufs)))}`
      : '–';
    E.summary.innerHTML = `
      <div class="stat lead"><div class="n">${signed(P.target_lufs, 0)}</div><div class="l">LUFS Ziel · MP3 ${P.bitrate_kbps} kbit/s · Spitzen ${signed(P.ceiling_dbtp)} dBTP</div></div>
      <div class="stat"><div class="n">${P.files.length}</div><div class="l">Dateien · ${fmtDur(total)}</div></div>
      <div class="stat"><div class="n" style="font-size:16px;line-height:32px">${range}</div><div class="l">gemessene Lautheit</div></div>
      <div class="roots" title="${esc(P.roots.join('\n'))}">Analysiert: ${esc(P.roots.join(' · '))}</div>`;

    let html = '';
    let folder = null;
    for (const f of P.files) {
      if (f.folder !== folder) { folder = f.folder; html += `<div class="day">${esc(folder || 'Dateien')}</div>`; }
      html += fileHtml(f);
    }
    E.list.innerHTML = html;

    if (P.ignored.length) {
      E.extras.hidden = false;
      E.extrasSummary.textContent = `${plural(P.ignored.length, 'Datei', 'Dateien')} nicht verwendet`;
      E.extrasBody.innerHTML = `<ul>${P.ignored.map((x) => `<li>${esc(x.path)} <span>– ${esc(x.reason)}</span></li>`).join('')}</ul>`;
    } else {
      E.extras.hidden = true;
    }
    E.outDir.innerHTML = `<bdi>${esc(ms.outDir)}</bdi>`;
    E.outDir.title = `${ms.outDir}\nKlicken zum Ändern`;
    updateSelection();
  }

  function fileHtml(f) {
    const P = ms.plan;
    const on = ms.selected.has(f.id);
    const o = ms.outcomes.get(f.id);
    const l = f.loudness;
    const label = { written: 'Fertig', existing: 'Schon vorhanden', failed: 'Fehler', cancelled: 'Abgebrochen' };
    let status = '';
    if (o) {
      status = o.path
        ? `<div class="status ${o.status}"><button data-reveal="${esc(o.path)}" title="Im Finder zeigen">${label[o.status]}</button></div>`
        : `<div class="status ${o.status}">${label[o.status]}</div>`;
    } else if (ms.busy && on) {
      status = '<div class="status existing">wartet…</div>';
    }
    const canMaster = f.gain_db != null;
    const measuredText = l && Number.isFinite(l.lufs)
      ? `Gemessen ${lufs(l.lufs)} · TP ${dbtp(l.true_peak)} · LRA ${dec(l.lra)} LU`
      : 'keine Lautheit messbar';
    const gainText = canMaster
      ? ` · Verstärkung ${signed(f.gain_db)} dB${f.limited_db >= 0.5 ? ` · Limiter fängt bis ${dec(f.limited_db)} dB ab` : ''}`
      : '';
    const result = o && o.result ? `<span class="result">Ergebnis ${lufs(o.result.lufs)} · TP ${dbtp(o.result.true_peak)}</span>` : '';
    const meter = l && Number.isFinite(l.lufs)
      ? `<div class="meter" title="Skala −60 bis 0 LUFS"><i class="m-target" style="left:${meterX(P.target_lufs)}"></i><i class="m-in" style="left:${meterX(l.lufs)}"></i>${o && o.result ? `<i class="m-out" style="left:${meterX(o.result.lufs)}"></i>` : ''}</div>`
      : '';
    return `
    <div class="rec${on ? '' : ' off'}" data-id="${f.id}">
      <input type="checkbox" data-mid="${f.id}" ${on ? 'checked' : ''} ${ms.busy || !canMaster ? 'disabled' : ''} aria-label="Datei auswählen">
      <div class="when">
        <span class="time">${esc(f.name)}</span>
        <span class="dur">${fmtDur(f.duration)}</span>
        <span class="parts-badge mono">${esc(f.format)}</span>
        ${f.dji_part ? '<span class="tag" title="Rohes DJI-Teilstück, meist schon in einem Track enthalten">DJI-Teil</span>' : ''}
      </div>
      ${status}
      <div class="meta"><span>${fmtBytes(f.size)}</span><span class="out">${esc(f.out_name)}</span></div>
      <div class="loud">${meter}<span>${measuredText}${gainText}</span>${result}</div>
      ${f.note ? `<div class="note">${esc(f.note)}</div>` : ''}
      ${o && o.message ? `<div class="errmsg">${esc(o.message)}</div>` : ''}
    </div>`;
  }

  function updateSelection() {
    const P = ms.plan;
    if (!P) return;
    const chosen = P.files.filter((f) => ms.selected.has(f.id));
    const secs = chosen.reduce((s, f) => s + f.duration, 0);
    E.sel.textContent = chosen.length ? `${plural(chosen.length, 'Datei', 'Dateien')} · ${fmtDur(secs)}` : 'nichts ausgewählt';
    E.go.disabled = !chosen.length || ms.busy;
  }

  /* ---------- events ---------- */

  q('#master-pick').addEventListener('click', choose);
  q('#master-pick-again').addEventListener('click', choose);
  q('#master-rescan').addEventListener('click', () => analyze(ms.inputs));
  q('#master-acancel').addEventListener('click', () => { E.atext.textContent = 'Breche ab…'; invoke('cancel_merge'); });
  E.outDir.addEventListener('click', chooseOutDir);
  E.go.addEventListener('click', runWrite);
  E.cancel.addEventListener('click', () => { E.ptext.textContent = 'Breche ab…'; invoke('cancel_merge'); });
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
    const frac = p.stage === 'measure' ? 0.05 + 0.95 * (p.total ? p.done / p.total : 1) : 0.03;
    E.abar.style.width = `${(frac * 100).toFixed(1)}%`;
    E.atext.textContent = p.stage === 'measure' ? `${p.text} · ${fmtDur(p.done / 1000)} von ${fmtDur(p.total / 1000)}` : p.text;
  });

  listen('master-write-progress', ({ payload: p }) => {
    const frac = p.total ? Math.min(1, p.done / p.total) : 1;
    E.bar.style.width = `${(frac * 100).toFixed(1)}%`;
    E.ptext.textContent = `${p.index} von ${p.count} fertig · ${Math.min(99, Math.floor(frac * 100))} % · ${p.name}`;
    E.ptext.title = E.ptext.textContent;
  });

  render();
})();
