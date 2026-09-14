# Erkennung von Aufnahme-Teilen (Schritt 1)

Zusammengefügt werden nicht nur Dateien mit einem bestimmten Namensmuster, sondern
alle WAV-Dateien, die sich über Aufnahmezeit, Metadaten, Länge und die Nahtstelle
im Audio als Teile einer Aufnahme ausweisen. Der Code steht in
`src-tauri/src/scan.rs` (Zuordnung) und `src-tauri/src/wav.rs` (RIFF-Chunks
`fmt`, `data`, `ds64`, `bext`, `iXML`, direkt gelesen, ohne externe Werkzeuge).

## Recherche: wie Recorder lange Aufnahmen teilen

Stand 14.09.2026, Websuche plus eigene Messung. Nicht belegte Punkte sind markiert.

| Gerät | Teilung | Namen der Folgedateien | Metadaten |
|---|---|---|---|
| DJI Mic 2 (Sender) | alle ~30 min (Handbuch/FAQ: 30 min, 32-bit float 23 min); gemessen: 354 418 688 Byte = 1845,9 s bei 48 kHz/32f mono | `DJI_<nn>_<JJJJMMTT>_<HHMMSS>.WAV`, eigene Startzeit je Teil | nur `fmt` + `data`, kein bext/iXML (49 echte Dateien geprüft) |
| Zoom H4n/H5/H6/H2n | bei 2 GB, lückenlos | `ZOOMnnnn` oder `JJMMTT-HHMMSS`; laut Zach Poff haben Folgedateien (H1n) teils falsche Namen und Erstellungsdaten | bext (Inhalt der Folgedateien nicht belegt) |
| Zoom F6/F8n/F3 | bei 2 GB (FAT32-Karte), lückenlos | Name der Take plus angehängte laufende Nummer | bext + iXML; TimeReference der Folgedatei = Start der Folgedatei ist zu erwarten, **nicht verifiziert** |
| Tascam DR-40X/DR-05X, Portacapture | bei 2 GB, neue Datei ohne Lücke (Tascam Help Center) | `<Wort>_<nnnn>.wav`, kein Datum im Namen | bext **nicht verifiziert** → oft nur Dateidatum |
| Sound Devices MixPre II | bei 4 GB (BWF-Grenze), lückenlos | Namensschema der Teilung nicht belegt | bext + iXML, Timecode als „Samples seit Mitternacht“ |
| Rode Wireless PRO (Sender) | 1-Stunden-Segmente (Rode-Hilfe) | Sendername, Schema nicht belegt | Timecode im File, bext-TimeReference **nicht verifiziert** |
| Sony PCM-D100 / PCM-A10 | > 2 GB bzw. 4 GB (LPCM); A10: „Aufnahme um die Teilungsstelle kann verloren gehen“ | `JJMMTT_HHMM` (nur Minuten) | Lücke möglich → kein exakter Anschluss |

Folgerungen: (1) Teilungsgrenzen 2 GB/2 GiB/4 GB/4 GiB und 1 Stunde decken die
bekannten Geräte ab, dazu die feste DJI-Größe. (2) bext-TimeReference ist Samples
seit Mitternacht; bei lückenloser Teilung gilt TimeRef(n+1) = TimeRef(n) + Frames(n).
(3) Namen von Folgedateien sind nicht immer verlässlich (Zoom), Minuten-Namen (Sony)
reichen nicht. (4) Das iXML-`FILE_SET` (FAMILY_UID, FILE_SET_INDEX, TOTAL_FILES)
beschreibt die Dateien einer Take; laut Spezifikation meist die Mono-Dateien je
Kanal, die **gleichzeitig** beginnen. Ob Recorder es auch für Zeit-Teilungen
nutzen, ist nicht belegt; deshalb zählt es nur, wenn die Folgedatei später beginnt.

Quellen: Sound On Sound (Zoom F6), Zoom-Handbücher F6/F8n/H4n (ManualsLib),
zachpoff.com „Zoom Recorder Technical Details“, Tascam Help Center („Can recording
continue after the file … reaches its maximum size?“), Sound Devices MixPre II
User Guide, help.rode.com („Why does the Wireless PRO TX record in 1-hour
segments?“), Sony Help Guide PCM-A10/PCM-D100, DJI Mic 2 FAQ, iXML-Spezifikation.

## Regeln

1. **Kandidaten:** jede WAV-Datei außer `._*`, früheren Ergebnissen
   (`JJMMTT_S…-E…_D…_<label>.wav`) und byte-identischen Kopien. Dateien ohne
   Partner bleiben Einzelaufnahmen.
2. **Startzeit je Teil**, beste Quelle zuerst, ausgegeben als `time_source`:
   - `bext`: OriginationDate + OriginationTime (lokal); stimmt TimeReference auf
     ±2 s mit der Uhrzeit überein, liefert sie den samplegenauen Start.
   - `name`: `<Präfix>_<Nr>_<JJJJMMTT>_<HHMMSS>` (mit Sequenznummer), sonst
     `JJJJMMTT[_- T]HHMMSS`, `JJMMTT[_-]HHMMSS`, `JJJJ-MM-TT[_- T]HHMMSS` oder
     `JJJJ-MM-TT[_- T]HH-MM-SS`/`HH.MM.SS`, jeweils von Nicht-Ziffern begrenzt,
     echtes Kalenderdatum, Jahr 1970–2099.
   - `file`: Änderungszeit − Dauer (Ende des Schreibens); die Erstellungszeit
     ersetzt das nur, wenn beide auf ±3 s übereinstimmen (Kopien bekommen eine neue).
   - Nicht gestellte Geräteuhren (Jahr vor 2010) werden **nicht** verworfen: Die
     Verkettung braucht nur Start B ≈ Ende A. Die Aufnahme bekommt einen Hinweis.
3. **Voller Teil** (kann einen Folgeteil haben): DJI-Größe 354 418 688 Byte, oder
   Datengröße ≥ 100 MiB und (Dateigröße oder Frame-Zahl kommt mindestens zweimal
   vor, oder die Datei endet höchstens 16 MiB unter 2 GB/2 GiB/4 GB/4 GiB, oder
   dauert 3600 ± 1 s). Ersatzweise der größte Teil eines Formats, wenn kein Teil
   dieses Formats die DJI-Größe hat. Ein kurzer Teil hat nie einen Nachfolger.
4. **Verbindung A → B** nur bei gleichem Format (Formatblock, also Abtastrate,
   Kanäle, Bittiefe, Formattyp), geprüft in dieser Reihenfolge:
   1. Beide haben TimeReference (≠ 0), OriginationDate höchstens 1 Tag auseinander:
      Differenz zu TimeRef(A) + Frames(A), modulo 1 Tag (Mitternacht). ≤ 2 Samples
      = exakt; ≤ 50 ms = Zeit-Treffer (Timecode auf Frames gerundet); sonst **keine**
      Verbindung, auch wenn Uhrzeit oder Name passen würden.
   2. iXML: gleiche FAMILY_UID, FILE_SET_INDEX von B folgt dem von A (1→2, A→B)
      und B beginnt nicht gleichzeitig mit A. Stärkster Beleg.
   3. Exakte TimeReference: gilt auch, wenn A nach Regel 3 kein voller Teil ist
      oder B die Uhrzeit von A trägt.
   4. Sonst muss A ein voller Teil sein und |Start B − Ende A| ≤ Toleranz der
      schlechteren Quelle: `bext`/`name` 3 s, `file` 5 s.
   5. Passt das nicht und hat keiner der beiden eine Sequenznummer im Namen,
      werden die Dateidaten verglichen (±5 s), weil manche Recorder Folgedateien
      mit dem Namen oder der Uhrzeit des ersten Teils versehen.
   Eine Verbindung, die nur auf Dateidaten beruht, braucht zusätzlich eine saubere
   Naht (Kosten ≤ 3,0) und bekommt einen Hinweis.
5. **Bewertung** (kleiner = besser): iXML −30, exakte TimeReference −20, Zeit-Treffer
   |Abweichung| · 0,5; jeweils plus Nahtkosten (lineare Prädiktion, −2…8, unbekannt
   1,5), +0,25 anderer Ordner, −0,25 gleiche Sequenznummer, +1,0 andere
   Sequenznummer. Gierig beste zuerst, jeder Teil höchstens ein Vorgänger und ein
   Nachfolger, keine Schleifen. Liegt ein Konkurrent weniger als 0,5 dahinter,
   gilt die Zuordnung als mehrdeutig.

### Warum diese Toleranzen

- **Name 3 s:** Namen haben Sekunden, abgeschnitten. Gemessen an 26 echten Nähten
  (6.–8.9.2026): Namens-Abweichung −1,93…+2,07 s.
- **Dateidatum 5 s:** FAT speichert die Änderungszeit in 2-s-Schritten, dazu kommt
  die Verzögerung beim Schließen der Datei, an beiden Enden. Gemessen an denselben
  26 Nähten (Kopien auf der Nextcloud, Änderungszeit erhalten): −1,93…+2,07 s.
  5 s lassen Reserve und bleiben weit unter echten Pausen (eine neue Aufnahme
  braucht am Gerät mehrere Sekunden).
- **Erstellungszeit:** auf der Nextcloud-Kopie liegt sie beim Ende der Aufnahme
  (= Änderungszeit), wäre als Start also falsch; deshalb nur bei Übereinstimmung.
- **Nahtkosten:** an 38 DJI-Nähten echt −1,1…1,3, fremd 1,3…7,9; neue Messung an
  26 Nähten echt −1,13…1,26, Konkurrenten 1,28…7,87.
- **Rauschboden** (RMS des leisesten Teils der letzten/ersten 2 s) wurde gemessen,
  entscheidet aber nichts: echte Nähte sprangen um bis zu 33 dB (Ansteckmikro,
  Sprechpause gegen durchgehendes Gespräch), fremde lagen teils nur 0,2 dB
  auseinander. Die Werte stehen in der Ausgabe von `tests/real_data.rs`.

## Verlässlichkeit der Verkettung

Jede Verbindung bekommt `link_confidence` (am Folgeteil), jede gestückelte
Aufnahme `confidence` = schwächste Verbindung. Die Oberfläche zeigt „Verkettung:
hoch/mittel/niedrig“.

| Stufe | Bedingung |
|---|---|
| `high` | iXML-File-Set oder exakte TimeReference, Naht widerspricht nicht (Kosten ≤ 3,0 oder nicht messbar) |
| `medium` | Zeit-Treffer (Name, Dateidatum, bext-Uhrzeit, TimeReference ≤ 50 ms), A ist ein voller Teil nach Regel 3, Naht bestätigt (Kosten ≤ 2,0) |
| `low` | mehrdeutig, Naht widerspricht (Kosten > 3,0), Naht nicht messbar bei reinem Zeit-Treffer, oder A nur als größter Teil eingestuft |

Aufnahmen mit `low` werden angezeigt, aber **nicht vorausgewählt**, und tragen den
Hinweis „Verkettung unsicher … Bitte prüfen“. Die DJI-Aufnahmen vom 6.–8.9.2026
sind alle `medium` und vorausgewählt.

Nicht umgesetzt (außerhalb des Rahmens): Zuordnung nur über den Audioinhalt ohne
Zeitbeleg, Netzbrummen (ENF), Transkription, Sprecher-Erkennung. Jede Datei hat
mindestens ein Dateidatum, eine reine Inhalts-Reparatur ist daher nie nötig.

## Auch in Schritt 2 und 3

- **Synchronisieren:** Eigene Tracks (`JJMMTT_S…-E…_D…_<label>.wav`) werden wie
  bisher direkt geladen. Alle übrigen WAV-Dateien laufen durch dieselbe Erkennung:
  Ketten werden ein Track, einzelne Dateien ein eigener Track. Eine Aufnahme, die
  schon als Track vorliegt, wird nicht doppelt geladen. MP3, M4A/AAC, FLAC, ALAC,
  AIFF, CAF und OGG sind nie Teile: Jede Datei wird ein Track (einmal in ein
  zwischengespeichertes WAV dekodiert). Startzeit wie oben, nur ohne bext:
  `name` (Teile-Muster, Track-Name aus Schritt 1 oder freies Datum mit Sekunden),
  dann bei M4A die Erstellungszeit aus `mvhd` (gesetzt, ab 2010, nicht nach der
  Änderungszeit), sonst `file` (Änderungszeit − Dauer). Sender = Ordnername, bei
  einem umgewandelten Track aus Schritt 1 das Label im Namen.
- **Mastern:** Eine WAV-Datei gilt als roher Aufnahme-Teil, wenn die Erkennung sie
  mit anderen zu einer Aufnahme verbindet (nicht mehr am Namensmuster). Einzelne
  Aufnahmen und fertige Tracks sind keine Teile.

## Prüfen

- `cargo test --release`: synthetische Fälle (bext exakt/versetzt/Mitternacht,
  iXML-Set und Kanal-Set, Dateidaten, freie Namensmuster, 60-s-Lücke, andere
  Abtastrate, nicht gestellte Uhr, schwache Kette).
- `PA_REAL_DIR="…/260906_basel/4|…/260906_basel/5" cargo test --release --test real_data -- --ignored --nocapture`
  zeigt Zuordnung, Quelle, Verlässlichkeit, Nahtkosten und Rauschboden je Naht.
