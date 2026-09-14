# Backlog — PrepareAudio

> **Stand 2026-09-14:** A1–A3 und B1–B4 sind in Version 0.2.0 gebaut (Plan: `docs/PLAN-0.2.md`). Die Einträge unten bleiben als Begründung stehen.

Offene Aufgaben, die nicht aus dem Code oder der Git-Historie hervorgehen.
Gesammelt 2026-09-14.

## A) Bedienung vereinfachen (alle drei Reiter)

### A1. „Wo speichern?“ beim Start statt Pfadzeile

- Die Pfadzeile unten links („Ziel …“) entfällt in allen drei Reitern.
- Die Start-Knöpfe unten rechts (Zusammenfügen, Erzeugen, Mastern) öffnen in jedem Schritt
  zuerst einen Ordner-Dialog „Wo speichern?“. Abbrechen startet nichts.
- Im gewählten Ordner legt die App immer den Unterordner des Schritts an und schreibt dorthin:
  `tracks` (Zusammenfügen), `sync` (Synchronisieren), `master` (Mastern). Entschieden 2026-09-14.
- Der Dialog startet im Ordner der Quellen, damit der bisherige Standard ein Klick bleibt.

### A2. Fortschritt ehrlich zeigen, Rechnen sichtbar machen

- Befund: Balken und Prozentzahl laufen dem tatsächlichen Stand voraus, vor allem beim
  Mastern (dort werden Durchgänge im Voraus geschätzt; nötige Zusatz-Durchgänge verlängern
  die Arbeit, der Balken steht dann lange nahe 100 %). Das wirkt wie ein Hänger.
- Eine Aktivitätsanzeige (Activity Wheel) dreht, solange das Backend nachweislich rechnet:
  das Backend schickt dazu einen Herzschlag (z. B. alle 0,5 s), bleibt er aus, zeigt die
  UI das an.
- Die Prozentzahl beruht nur auf abgeschlossener Arbeit (fertige Dateien und fertige
  Durchgänge), nie auf Schätzungen; im Zweifel „Datei 3 von 5 · MP3 kodieren“ ohne Prozent.
- Je Datei in der Liste der aktuelle Schritt (Pegel einstellen, Kodieren, Nachmessen).

### A3. Nach dem Lauf die Liste aufräumen

- Wurde alles erledigt: Liste leeren, nur „5 von 5 erledigt“ und „Ordner öffnen“ zeigen.
- Blieben Dateien übrig (Fehler, abgebrochen): nur diese stehen lassen, mit Grund und einem
  Knopf „Wiederholen“. „Schon vorhanden“ zählt als erledigt.

## B) Synchronisieren: Timeline als Schnittprogramm-light

### B1. Klarere Darstellung (Entwurf vom 2026-09-14)

- Je Sender eine Spur in eigener Farbe (4 magenta, 5 blau, weitere Sender weitere Farben).
- Jeder Abschnitt jeder Spur steht in einer von zwei Positionen (Schema, kein Design):
  - **Stereo:** beide Spuren treffen sich in der Mitte, die Abschnitte liegen genau übereinander.
  - **Mono:** die Spur ist nach außen geschoben (Sender 4 nach oben, Sender 5 nach unten).
- Das Aussehen folgt den Konventionen von Schnitt- und Audioprogrammen (Final Cut, Resolve,
  Audition, Logic), nicht dem Entwurf: Abschnitte als gefüllte Clips mit Wellenform in der Farbe
  des Senders; ein Stereo-Paar sichtbar verbunden (Klammer oder Verknüpfungssymbol, L/R-Kennung);
  Mono-Clips mit Mono-Kennung; weggeschnittene oder verworfene Teile abgedunkelt und schraffiert.
  Die genaue Gestaltung beim Bauen mit einem kurzen Entwurf festlegen.
- Die Ereignisspur bleibt darunter.

### B2. Bearbeiten wie in einem Schnittprogramm, Synchronisation bleibt fest

Entschieden 2026-09-14:
- Erlaubte Bearbeitungen je Abschnitt und Spur:
  - **Trimmen:** In- und Out-Punkt nach innen ziehen.
  - **Verlängern:** In- und Out-Punkt wieder nach außen ziehen, höchstens bis Anfang oder Ende
    der Aufnahme bzw. bis zum Nachbarabschnitt (verschiebt dabei die Phasengrenze).
  - **Trennen:** einen Abschnitt teilen, z. B. am Playhead.
  - **Zusammenführen:** benachbarte Abschnitte derselben Spur zu einem machen.
  - **Löschen und Wiederbringen:** Abschnitt verwerfen (bleibt schraffiert sichtbar) und wieder
    aufnehmen.
  - **Auf Stereo oder Mono schieben:** in die Mitte ziehen macht Stereo, nach außen ziehen macht
    Mono (zusätzlich per Knopf oder Taste).
  - Dazu Rückgängig und Wiederholen.
- Nie erlaubt: eine Spur oder einen Abschnitt zeitlich verschieben. Beide Spuren bleiben in der
  Timeline immer synchron (Versatz und Drift sind gemessen und fest).
- Abgeleitete Regel (prüfen): Stereo entsteht, wo beide Spuren in der Mitte liegen. Liegt nur eine
  Spur in der Mitte, entsteht Stereo mit stillem zweitem Kanal, wie heute bei Vor- und Nachlauf.
- Änderungen gelten für „Erzeugen“ und werden als kleine Bearbeitungsdatei neben den Quellen
  gespeichert, damit ein erneutes Analysieren sie nicht verwirft.

### B3. Zoom, Wellenform und Playhead

- Zoom vom ganzen Tag bis auf wenige Sekunden (Trackpad-Pinch, Ctrl/Cmd+Mausrad, Knöpfe),
  horizontal scrollen, Übersicht (Minimap) oben.
- Wellenform je Spur aus vorberechneten Pegelwerten (Rust), in mehreren Zoomstufen.
- Playhead: klicken zum Springen, Leertaste Play/Pause, Scrubbing; Vorhören genau so, wie
  die Datei entstehen würde (Stereo L/R mit Versatz und Drift, Mono einzeln, Trims beachtet).

### B4. Bibliotheken und Architektur (Recherche)

Web-Recherche vom 2026-09-14, noch nichts davon im Code ausprobiert.

**Nachtrag 2026-09-14:** PrepareAudio steht jetzt unter der AGPL-3.0-or-later. Damit sind LGPL- und
GPL-3-Pakete lizenzrechtlich möglich; die Tabelle „verworfen“ unten galt für MIT. Neu zu bewerten:
**peaks.js** (LGPL-3.0, 4.0.0) als fertige Timeline (Zoomview und Overview, Segmente mit Griffen,
mehrere Auflösungen aus `.dat`) statt einer eigenen; prüfen, ob sich die Wiedergabe über einen eigenen
Player an die Rust-Wiedergabe koppeln lässt. `waveform-data.js` (LGPL) darf dann mit. `audiowaveform`
(GPL-3.0) wäre erlaubt, ist aber ein C++-Programm und bräuchte ein mitgeliefertes Binary; die Peaks in
Rust zu erzeugen bleibt einfacher.

**Empfehlung (Stand MIT): native Wiedergabe in Rust, Peaks aus Rust, eigene Timeline im Frontend.**

- **Wiedergabe:** ein einziger `cpal`-Ausgabestrom (Apache-2.0, 0.18.2), der beide Spuren im
  selben Callback mischt. Lesen mit symphonia, Versatz als Sample-Offset, Drift per
  Resampling (`rubato`, MIT). Damit ist das Vorhören sample-genau synchron. Rust ist die Uhr,
  die Playhead-Position geht mit 30–60 Hz als Event an die UI, die dazwischen interpoliert.
- **Peaks:** Pyramide in Rust (z. B. 64, 256, 1024, 4096, 16384 Samples pro Pixel) im
  dokumentierten BBC-`.dat`-Format selbst schreiben (ca. 100 Zeilen), im Frontend selbst lesen
  (ca. 50 Zeilen). Das Frontend holt nur den sichtbaren Ausschnitt per `invoke`.
  2 h bei 256 Samples pro Pixel sind etwa 1,35 MB je Kanal (8 bit).
- **Timeline:** eigene Canvas2D-Zeichnung oder mit Konva (MIT, 10.5.0, aktiv, ca. 56 kB):
  feste Spuren, Abschnitte als Rechtecke mit zwei Griffen, Zuweisung umschalten, Playhead,
  Zoom um den Mauszeiger, Minimap als zweiter Canvas.
- **Schneller Prototyp als Alternative:** wavesurfer.js v7 (BSD-3, 7.12.12, sehr aktiv,
  Vanilla JS) mit einer Instanz je Spur, vorberechneten `peaks`, Regions mit `drag:false`
  (nur trimmbar). Nachteile: gemeinsame Zeitachse selbst koppeln, Zoomstufen selbst tauschen.

**Geprüft und verworfen:**

| Bibliothek | Lizenz | Grund |
|---|---|---|
| peaks.js (BBC) | LGPL-3.0 | technisch am passendsten, aber Lizenzfalle für die MIT-App |
| waveform-data.js | LGPL-3.0 | ebenso; steckt auch in @dawcore/components |
| audiowaveform (BBC) | GPL-3.0 | nicht mitliefern; nur das `.dat`-Format übernehmen |
| waveform-playlist | MIT | braucht React, dekodiert alles in den Speicher (1,5 GB geht nicht) |
| wavesurfer-multitrack | BSD-3 | nur ein Trim-Paar je Spur, kaum noch gepflegt |
| Tone.js | MIT | nur Audio-Engine im Speicher, keine Oberfläche |
| animation-timeline-js | MIT | für Keyframes, seit 2024 ohne Pflege |

**Risiken:**

- Aufwand der eigenen UI (Zoom-Rechnung, Treffer-Erkennung, viele Abschnitte flüssig).
- Falls doch über `<audio>` abgespielt wird: 32-bit-float-WAV spielt WKWebView womöglich nicht
  ab (dann live in 16-bit wandeln), zwei `<audio>`-Elemente laufen nicht sample-genau synchron,
  Tauri-Issue #6375 (Absturz beim Springen in Dateien über 3,5 GB) ist offen.
- Transitive LGPL-Abhängigkeiten bei jedem neuen Paket prüfen (`scripts/gen-licenses.py`).

Quellen: github.com/katspaugh/wavesurfer.js, github.com/bbc/peaks.js,
github.com/bbc/audiowaveform/blob/master/doc/DataFormat.md, github.com/rustaudio/cpal,
github.com/tauri-apps/tauri/issues/6375, github.com/tauri-apps/tauri/blob/dev/examples/streaming/main.rs
