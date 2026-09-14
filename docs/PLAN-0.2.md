# Plan PrepareAudio 0.2 — Backlog A und B umsetzen

Stand 2026-09-14. Grundlage: `BACKLOG.md`. Lizenz AGPL-3.0-or-later.

## Leitentscheidungen

1. **Rust bleibt die Wahrheit.** Welche Dateien entstehen, berechnet nur das Backend. Die
   Oberfläche schickt Bearbeitungen (Clips) und bekommt die Ausgabeliste zurück. So erzeugt
   „Erzeugen“ genau das, was die Liste zeigt.
2. **Eine Zeitachse je Tag mit festen Spurpositionen.** Jede Spur bekommt eine Platzierung
   (Startposition p und Taktfaktor s): Zeitachse t ↦ Spurzeit τ = (t − p) · s. Die
   Referenzspur (frühester Track des ersten Senders) liegt auf ihrer Uhr (s = 1); alle anderen
   werden über die gemessenen Paare (Versatz, Drift) erreicht (Breitensuche). Nicht verbundene
   Spuren liegen auf ihrer Dateinamen-Uhr. Bearbeitungen ändern nie p oder s.
3. **Bearbeitungsmodell: Clips je Spur.** Ein Clip ist ein Intervall in Spurzeit mit Zustand
   `stereo`, `mono` oder `deleted`. Lücken zwischen Clips sind weggeschnitten. Clips derselben
   Spur überlappen nie. Operationen: trimmen, verlängern (bis Aufnahmegrenze oder Nachbar;
   am Nachbarn Rollschnitt), trennen, zusammenführen (benachbart, gleicher Zustand wird
   übernommen vom ersten), löschen/wiederbringen, Stereo/Mono umschalten, Undo/Redo.
4. **Ausgaberegeln aus Clips** (ersetzen `plan_items`, `extend_alone`, `merge_dropouts`):
   - Mono-Clip → eine Mono-Datei (bitgenauer Ausschnitt).
   - Stereo: das Senderpaar (erster Sender links, zweiter rechts) bildet Stereo-Dateien aus der
     Vereinigung seiner Stereo-Clips. Eine Datei endet nur dort, wo **jede** zu diesem Zeitpunkt
     aktive Stereo-Spur eine Clipgrenze hat, oder an einer Lücke ohne Stereo-Clip. Damit bleibt
     ein Ausfall eines Senders eine durchgehende Datei (heutiges Verhalten), Trennen auf beiden
     Spuren teilt die Datei, Trennen auf nur einer nicht.
   - Liegt nur eine Spur auf Stereo, entsteht Stereo mit stillem zweitem Kanal.
   - Weitere Sender (3+) sind nur Mono.
   - Die Anfangsclips aus der Analyse ergeben exakt die heutigen Dateien (Regressionstest).
5. **Einheitlicher Stereo-Schreiber.** Ausgaberaster = Zeitachse bei der Abtastrate des Tages.
   Je Kanal eine Liste von Quellen (Spur, Zeitachsen-Intervall). Liegt die Quelle auf dem Raster
   (s = 1 und p ganzzahlig in Samples), wird bitgenau kopiert, sonst kubisch interpoliert.
6. **Bearbeitungsdatei** `.prepareaudio-sync.json` im ersten Quellordner: Clips je Spur,
   verknüpft über Dateiname, Startzeit und Länge. Beim Analysieren wiederhergestellt, wenn alle
   Spuren passen; „Vorschlag der Analyse wiederherstellen“ verwirft sie.
7. **Peaks in Rust.** Im Hüllkurven-Durchgang je Spur (jetzt für alle Spuren) Spitzenwert je
   1 ms als u8 in dB (−60…0 dBFS), Pyramide 1 ms / 10 ms / 100 ms / 1 s. Befehl liefert für
   einen sichtbaren Ausschnitt genau so viele Werte wie Pixel.
8. **Vorhören nativ mit cpal** (Apache-2.0, geprüft: CoreAudio 48 kHz F32). Eigener Player-
   Thread besitzt den Stream; ein Renderer-Thread füllt einen Puffer (100-ms-Blöcke, ~300 ms
   Vorlauf). Stereo-Clips auf L/R nach Sender, Mono-Clips mittig; Stumm/Solo je Sender.
   Vorhörpegel je Spur aus der gemessenen Lautheit, weiche Begrenzung. Position als Event mit
   30 Hz; die Oberfläche interpoliert dazwischen.
9. **Timeline ohne Bibliothek** (Canvas2D, Vanilla JS): Lineal, Übersicht, je Sender eine
   Stereo-Zeile (innen) und eine Mono-Zeile (außen), Ereignisspur, Playhead. Aussehen nach
   Schnittprogramm-Konvention: gefüllte Clips mit Wellenform in Senderfarbe, Stereo-Paar
   verbunden mit L/R-Kennung, Mono-Kennung, gelöschte und weggeschnittene Teile abgedunkelt und
   schraffiert. peaks.js wurde geprüft (seit AGPL zulässig), passt aber nicht zum Modell
   „mehrere versetzte Spuren mit Stereo/Mono-Position“.
10. **Fortschritt ehrlich (A2).** Das Backend sendet während jedes Vorgangs alle 0,5 s einen
    Herzschlag. Das Aktivitätsrad dreht nur, solange Herzschläge kommen. Prozente zählen nur
    abgeschlossene Arbeit (Mastern: fertige Dateien), dazu der Schritt je Datei.
11. **Speichern (A1) und Aufräumen (A3).** Pfadzeilen entfallen. Start-Knopf öffnet „Wo
    speichern?“ im Quellordner; die App schreibt in den Unterordner `tracks`/`sync`/`master`
    (heißt der gewählte Ordner schon so, direkt dorthin). Nach dem Lauf: alles erledigt →
    Liste weg, Zusammenfassung mit „Ordner öffnen“ und „Weiter“; Rest → nur diese Dateien
    mit Grund und „Wiederholen“.

## Umsetzungsschritte

| # | Schritt | Prüfung |
|---|---|---|
| 1 | A1–A3 in allen drei Reitern (Dialog, Herzschlag, Aktivitätsrad, ehrlicher Master-Fortschritt, Aufräumen, Wiederholen) | Unit-Tests Fortschritt, Browser-Mock-Screenshots |
| 2 | Sync-Modell: Platzierung, Clips aus Analyse, Ausgaberegeln, einheitlicher Stereo-Schreiber, Bearbeitungsoperationen, Bearbeitungsdatei | Unit-Tests je Regel; Regression: Anfangsclips = heutige Dateien (Basel, Zürich) |
| 3 | Peaks-Pyramide und Befehle | Unit-Tests Pooling |
| 4 | Player (cpal) mit Renderer, Stumm/Solo, Events | Unit-Test Renderer offline gegen erwartete Samples; Gerät öffnen mit Stille |
| 5 | Timeline-Oberfläche mit allen Operationen, Tastatur, Kontextmenü | Browser-Mock mit simulierten Gesten; App-Screenshot |
| 6 | Integration, Lizenzliste (cpal), README/BACKLOG/Info, Version 0.2.0, App bauen | alle Tests, echte Daten, App starten |

Tastatur: Leertaste Play/Pause, S trennen am Playhead (ausgewählte Spur, mit Umschalt alle),
Entf löschen/wiederbringen, J zusammenführen mit dem nächsten Clip, ↑/↓ Stereo/Mono,
Cmd+Z/Cmd+Umschalt+Z, Cmd+Plus/Minus Zoom, 0 Tag einpassen, ←/→ Playhead in Sekunden.
