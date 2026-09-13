'use strict';

/* Second function: find synchronous stretches, write stereo and mono files.
   Shares esc, fmtDur, fmtBytes, plural and toast with app.js. */
(() => {
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  const q = (sel) => document.querySelector(sel);
  const E = {
    empty: q('#sync-empty'), results: q('#sync-results'), summary: q('#sync-summary'), timeline: q('#sync-timeline'),
    pairs: q('#sync-pairs'), list: q('#sync-list'), extras: q('#sync-extras'), extrasSummary: q('#sync-extras-summary'),
    extrasBody: q('#sync-extras-body'), bottom: q('#sync-bottom'), outDir: q('#sync-out-dir'), progress: q('#sync-progress'),
    bar: q('#sync-bar'), ptext: q('#sync-ptext'), done: q('#sync-done'), sel: q('#sync-sel'), go: q('#sync-go'),
    cancel: q('#sync-cancel'), analyzing: q('#sync-analyzing'), abar: q('#sync-abar'), atext: q('#sync-atext'),
    topActions: q('#sync-top-actions'),
  };
  const sy = { inputs: [], plan: null, outDir: '', selected: new Set(), outcomes: new Map(), busy: false, activeId: null };
  window.syncState = sy;

  const busyAny = () => sy.busy || !!(window.mergeState && window.mergeState.busy) || !!(window.masterState && window.masterState.busy);
  const fixed = (v, d) => v.toLocaleString('de-DE', { minimumFractionDigits: d, maximumFractionDigits: d });
  const signed = (v, d) => `${v < 0 ? '−' : '+'}${fixed(Math.abs(v), d)}`;
  const percent = (v) => (v == null ? '–' : `${Math.round(v * 100)} %`);
  const clockText = (sec) => {
    const s = ((Math.round(sec) % 86400) + 86400) % 86400;
    return [Math.floor(s / 3600), Math.floor((s % 3600) / 60)].map((x) => String(x).padStart(2, '0')).join(':');
  };

  /* ---------- actions ---------- */

  async function choose() {
    if (busyAny()) return;
    try {
      const p = await invoke('pick_folder', { title: 'Ordner mit Tracks wählen' });
      if (p) analyze([p]);
    } catch (e) { toast(String(e), 'bad'); }
  }

  async function analyze(paths) {
    if (busyAny() || !paths || !paths.length) return;
    sy.inputs = paths;
    sy.busy = true;
    E.abar.style.width = '0%';
    E.atext.textContent = 'Suche Tracks';
    E.analyzing.hidden = false;
    try {
      const plan = await invoke('analyze_tracks', { paths });
      sy.plan = plan;
      sy.outDir = plan.default_out_dir;
      sy.outcomes.clear();
      sy.selected = new Set(plan.items.map((i) => i.id));
      E.done.hidden = true;
      if (plan.labels.length < 2) toast('Nur ein Sender gefunden, alles bleibt Mono.');
    } catch (e) {
      if (!String(e).includes('Abgebrochen')) toast(String(e), 'bad');
    } finally {
      sy.busy = false;
      E.analyzing.hidden = true;
      render();
    }
  }
  window.syncAnalyze = analyze;

  async function chooseOutDir() {
    if (busyAny()) return;
    try {
      const p = await invoke('pick_folder', { title: 'Wo soll der Ordner „sync“ liegen?' });
      if (!p) return;
      const clean = p.replace(/[\\/]$/, '');
      sy.outDir = /[\\/]sync$/.test(clean) ? clean : `${clean}/sync`;
      sy.outcomes.clear();
      E.done.hidden = true;
      render();
    } catch (e) { toast(String(e), 'bad'); }
  }

  async function runWrite() {
    const ids = sy.plan.items.filter((i) => sy.selected.has(i.id)).map((i) => i.id);
    if (!ids.length || busyAny()) return;
    sy.busy = true;
    setBusyUi(true);
    sy.outcomes.clear();
    E.done.hidden = true;
    E.bar.style.width = '0%';
    E.ptext.textContent = 'Vorbereiten…';
    render();
    try {
      const sum = await invoke('write_sync', { ids, outDir: sy.outDir });
      for (const o of sum.outcomes) sy.outcomes.set(o.id, o);
      showDone(sum);
    } catch (e) {
      toast(String(e), 'bad');
    } finally {
      sy.busy = false;
      sy.activeId = null;
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
    if (c.written) bits.push(`${plural(c.written, 'Datei', 'Dateien')} geschrieben`);
    if (c.existing) bits.push(`${c.existing} schon vorhanden`);
    if (c.failed) bits.push(`${c.failed} fehlgeschlagen`);
    if (c.cancelled) bits.push(`${c.cancelled} abgebrochen`);
    const ok = !c.failed && !sum.cancelled;
    E.done.className = `done${ok ? '' : ' partial'}`;
    E.done.innerHTML = `<span>${ok ? 'Fertig' : sum.cancelled ? 'Abgebrochen' : 'Mit Fehlern beendet'}: ${esc(bits.join(', ') || 'nichts zu tun')}.</span>
      <button class="link" id="sync-open-out">sync öffnen</button>
      <button class="link" id="sync-to-master">Weiter: mastern</button>`;
    E.done.hidden = false;
    q('#sync-open-out').onclick = () => invoke('reveal', { path: sum.out_dir, select: false }).catch((e) => toast(String(e), 'bad'));
    q('#sync-to-master').onclick = () => { setMode('master'); if (window.masterAnalyze) window.masterAnalyze([sum.out_dir]); };
  }

  /* ---------- rendering ---------- */

  function render() {
    const P = sy.plan;
    E.empty.hidden = !!P;
    E.results.hidden = !P;
    E.bottom.hidden = !P;
    E.topActions.hidden = !P;
    if (!P) return;

    const okPairs = P.pairs.filter((p) => p.ok);
    const stereo = P.items.filter((i) => i.kind === 'stereo');
    const mono = P.items.filter((i) => i.kind === 'mono');
    E.summary.innerHTML = `
      <div class="stat lead"><div class="n">${stereo.length}</div><div class="l">Stereo-Abschnitte · ${fmtDur(stereo.reduce((a, i) => a + i.duration, 0))}</div></div>
      <div class="stat"><div class="n">${mono.length}</div><div class="l">Mono-Abschnitte</div></div>
      <div class="stat"><div class="n">${okPairs.length}<span class="of"> / ${P.pairs.length}</span></div><div class="l">Track-Paare synchron</div></div>
      <div class="stat"><div class="n">${P.tracks.length}</div><div class="l">Tracks von ${plural(P.labels.length, 'Sender', 'Sendern')}${P.source === 'chunks' ? ', aus DJI-Teilen' : P.source === 'mixed' ? ', teils aus DJI-Teilen' : ''}</div></div>
      <div class="roots" title="${esc(P.roots.join('\n'))}">Analysiert: ${esc(P.roots.join(' · '))}</div>`;

    renderTimeline();
    E.pairs.innerHTML = P.pairs.length
      ? P.pairs.map(pairHtml).join('')
      : '<div class="pair"><div class="facts">Keine Tracks verschiedener Sender überlappen zeitlich.</div></div>';

    let html = '';
    let day = null;
    for (const it of P.items) {
      if (it.day !== day) { day = it.day; html += `<div class="day">${esc(day)}</div>`; }
      html += itemHtml(it);
    }
    E.list.innerHTML = html || '<p class="day">Nichts auszugeben.</p>';

    if (P.ignored.length) {
      E.extras.hidden = false;
      E.extrasSummary.textContent = `${plural(P.ignored.length, 'Datei', 'Dateien')} nicht verwendet`;
      E.extrasBody.innerHTML = `<ul>${P.ignored.map((x) => `<li>${esc(x.path)} <span>– ${esc(x.reason)}</span></li>`).join('')}</ul>`;
    } else {
      E.extras.hidden = true;
    }
    E.outDir.innerHTML = `<bdi>${esc(sy.outDir)}</bdi>`;
    E.outDir.title = `${sy.outDir}\nKlicken zum Ändern`;
    updateSelection();
  }

  function renderTimeline() {
    const P = sy.plan;
    const days = [...new Set(P.tracks.map((t) => t.day))];
    const legend = `<div class="legend">
      <span><i class="stereo"></i>Stereo: gemeinsames Geschehen</span>
      <span><i class="mono"></i>Mono: getrennt oder allein</span>
      <span><i class="hit"></i>gemeinsame Ereignisse, stabiler Versatz</span>
      <span><i class="miss"></i>keine gemeinsamen Ereignisse</span></div>`;
    E.timeline.innerHTML = days.map((day) => dayHtml(P, day)).join('') + legend;
  }

  function dayHtml(P, day) {
    const tracks = P.tracks.filter((t) => t.day === day);
    const ids = new Set(tracks.map((t) => t.id));
    let t0 = Math.min(...tracks.map((t) => t.clock0));
    let t1 = Math.max(...tracks.map((t) => t.clock0 + t.duration));
    const pad = Math.max(60, (t1 - t0) * 0.01);
    t0 -= pad;
    t1 += pad;
    const X = (c) => `${(((c - t0) / (t1 - t0)) * 100).toFixed(3)}%`;
    const W = (d) => `${Math.max(0.2, (d / (t1 - t0)) * 100).toFixed(3)}%`;
    let rows = '';
    for (const label of P.labels.filter((l) => tracks.some((t) => t.label === l))) {
      let lane = tracks
        .filter((t) => t.label === label)
        .map((t) => `<div class="tl-track" style="left:${X(t.clock0)};width:${W(t.duration)}" title="${esc(t.name)}"></div>`)
        .join('');
      for (const it of P.items) {
        if (!ids.has(it.left)) continue;
        const inLane = P.tracks[it.left].label === label || (it.right != null && P.tracks[it.right].label === label);
        if (!inLane) continue;
        const tip = `${it.kind === 'stereo' ? 'Stereo' : 'Mono'} ${it.start}–${it.end} (${fmtDur(it.duration)})`;
        lane += `<div class="tl-item ${it.kind}${sy.selected.has(it.id) ? '' : ' off'}" data-item="${it.id}" style="left:${X(it.clock0)};width:${W(it.duration)}" title="${esc(tip)}"></div>`;
      }
      rows += `<div class="tl-label" title="Sender ${esc(label)}">${esc(label)}</div><div class="tl-lane">${lane}</div>`;
    }
    let ticks = '';
    for (const p of P.pairs) {
      if (!p.ok || !ids.has(p.a)) continue;
      const base = P.tracks[p.a].clock0;
      for (const f of p.frames) {
        const cls = !f.active ? 'quiet' : f.hit ? 'hit' : 'miss';
        ticks += `<div class="tl-tick ${cls}" style="left:${X(base + f.t + 5)};width:${W(10)}"></div>`;
      }
    }
    if (ticks) rows += `<div class="tl-label" title="Gemeinsame Ereignisse je 20-s-Fenster">Ereignisse</div><div class="tl-lane hits">${ticks}</div>`;
    const span = t1 - t0;
    const step = [300, 600, 900, 1800, 3600, 7200, 10800].find((s) => span / s <= 8) || 14400;
    let axis = '';
    for (let c = Math.ceil(t0 / step) * step; c <= t1; c += step) axis += `<span style="left:${X(c)}">${clockText(c)}</span>`;
    rows += `<div></div><div class="tl-axis">${axis}</div>`;
    return `<div class="tl-day"><div class="tl-head">${esc(day)}</div><div class="tl-grid">${rows}</div></div>`;
  }

  function pairHtml(p) {
    const P = sy.plan;
    const a = P.tracks[p.a];
    const b = P.tracks[p.b];
    const who = `${esc(a.label)} ${a.start}–${a.end} <span class="with">mit</span> ${esc(b.label)} ${b.start}–${b.end}`;
    if (!p.ok) {
      return `<div class="pair"><div class="who">${who}</div><div class="verdict no">nicht synchron</div><div class="facts">${esc(p.note || '')}</div></div>`;
    }
    const act = p.frames.filter((f) => f.active);
    const hits = act.filter((f) => f.hit).length;
    const together = p.phases.filter((x) => x.kind === 'together').reduce((s, x) => s + x.end - x.start, 0);
    const total = p.phases.reduce((s, x) => s + x.end - x.start, 0);
    return `<div class="pair"><div class="who">${who}</div><div class="verdict ok">synchron</div>
      <div class="facts">Versatz ${signed(p.offset, 3)} s · Drift ${signed(p.drift_ppm, 1)} ppm · Streuung ${fixed(p.resid_ms, 1)} ms ·
      Treffer in ${act.length ? Math.round((hits / act.length) * 100) : 0} % der Fenster · gemeinsam ${fmtDur(together)} von ${fmtDur(total)}</div></div>`;
  }

  function itemHtml(it) {
    const P = sy.plan;
    const on = sy.selected.has(it.id);
    const o = sy.outcomes.get(it.id);
    const a = P.tracks[it.left];
    const badge = it.kind === 'stereo'
      ? `<span class="parts-badge">Stereo · L ${esc(a.label)} · R ${esc(P.tracks[it.right].label)}</span>`
      : `<span class="parts-badge mono">Mono · ${esc(a.label)}</span>`;
    const gap = (it.silent || []).filter((x) => x.seconds >= 1).map((x) => ` · Sender ${esc(x.label)} fehlt ${fmtDur(x.seconds)}`).join('');
    const why = it.kind === 'stereo'
      ? `Treffer ${percent(it.hit_share)} · Kohärenz ${it.msc == null ? '–' : fixed(it.msc, 2)}${gap}`
      : it.reason === 'getrennt' ? 'anderes Geschehen als der zweite Sender' : 'kein zweiter Sender zu dieser Zeit';
    const label = { written: 'Fertig', existing: 'Schon vorhanden', failed: 'Fehler', cancelled: 'Abgebrochen' };
    let status = '';
    if (o) {
      status = o.path
        ? `<div class="status ${o.status}"><button data-reveal="${esc(o.path)}" title="Im Finder zeigen">${label[o.status]}</button></div>`
        : `<div class="status ${o.status}">${label[o.status]}</div>`;
    } else if (sy.activeId === it.id) {
      status = '<div class="status existing">läuft…</div>';
    }
    return `
    <div class="rec${on ? '' : ' off'}${sy.activeId === it.id ? ' active' : ''}" data-id="${it.id}">
      <input type="checkbox" data-sid="${it.id}" ${on ? 'checked' : ''} ${sy.busy ? 'disabled' : ''} aria-label="Abschnitt auswählen">
      <div class="when">
        <span class="time">${esc(it.start)} – ${esc(it.end)}</span>
        <span class="dur">${fmtDur(it.duration)}</span>
        ${badge}
      </div>
      ${status}
      <div class="meta"><span>${esc(why)} · ${fmtBytes(it.bytes)}</span><span class="out">${esc(it.name)}</span></div>
      ${o && o.message ? `<div class="errmsg">${esc(o.message)}</div>` : ''}
    </div>`;
  }

  function updateSelection() {
    const P = sy.plan;
    if (!P) return;
    const chosen = P.items.filter((i) => sy.selected.has(i.id));
    const bytes = chosen.reduce((s, i) => s + i.bytes, 0);
    E.sel.textContent = chosen.length ? `${plural(chosen.length, 'Datei', 'Dateien')} · ${fmtBytes(bytes)}` : 'nichts ausgewählt';
    E.go.disabled = !chosen.length || sy.busy;
  }

  /* ---------- events ---------- */

  q('#sync-pick').addEventListener('click', choose);
  q('#sync-pick-again').addEventListener('click', choose);
  q('#sync-rescan').addEventListener('click', () => analyze(sy.inputs));
  q('#sync-acancel').addEventListener('click', () => { E.atext.textContent = 'Breche ab…'; invoke('cancel_merge'); });
  E.outDir.addEventListener('click', chooseOutDir);
  E.go.addEventListener('click', runWrite);
  E.cancel.addEventListener('click', () => { E.ptext.textContent = 'Breche ab…'; invoke('cancel_merge'); });
  q('#sync-all').addEventListener('click', () => { if (!sy.busy && sy.plan) { sy.plan.items.forEach((i) => sy.selected.add(i.id)); render(); } });
  q('#sync-none').addEventListener('click', () => { if (!sy.busy && sy.plan) { sy.selected.clear(); render(); } });

  E.list.addEventListener('change', (e) => {
    const id = e.target.dataset && e.target.dataset.sid;
    if (id === undefined || sy.busy) return;
    if (e.target.checked) sy.selected.add(Number(id)); else sy.selected.delete(Number(id));
    const row = e.target.closest('.rec');
    if (row) row.classList.toggle('off', !e.target.checked);
    renderTimeline();
    updateSelection();
  });
  E.timeline.addEventListener('click', (e) => {
    const bar = e.target.closest('[data-item]');
    if (!bar) return;
    const row = E.list.querySelector(`.rec[data-id="${bar.dataset.item}"]`);
    if (row) {
      row.scrollIntoView({ block: 'center', behavior: 'smooth' });
      row.classList.add('active');
      setTimeout(() => row.classList.remove('active'), 1200);
    }
  });

  listen('sync-progress', ({ payload: p }) => {
    let frac = 0.02;
    if (p.stage === 'envelope') frac = 0.05 + 0.65 * (p.total ? p.done / p.total : 1);
    else if (p.stage === 'pairs') frac = 0.7 + 0.3 * (p.total ? p.done / p.total : 1);
    E.abar.style.width = `${(frac * 100).toFixed(1)}%`;
    const detail = p.stage === 'envelope' ? `${fmtBytes(p.done)} von ${fmtBytes(p.total)}`
      : p.stage === 'pairs' ? `${p.done} von ${plural(p.total, 'Paar', 'Paaren')}` : '';
    E.atext.textContent = detail ? `${p.text} · ${detail}` : p.text;
  });

  listen('sync-write-progress', ({ payload: p }) => {
    const frac = p.total ? p.done / p.total : 1;
    E.bar.style.width = `${(frac * 100).toFixed(1)}%`;
    E.ptext.textContent = `${p.index + 1} von ${p.count} · ${p.name} · ${Math.floor(frac * 100)} %`;
    if (sy.activeId !== p.id) {
      sy.activeId = p.id;
      E.list.querySelectorAll('.rec.active').forEach((n) => n.classList.remove('active'));
      const row = E.list.querySelector(`.rec[data-id="${p.id}"]`);
      if (row) { row.classList.add('active'); row.scrollIntoView({ block: 'nearest', behavior: 'smooth' }); }
    }
  });

  render();
})();
