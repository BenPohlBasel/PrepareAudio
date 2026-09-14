# PrepareAudio

Tauri-App (macOS) von **B/IAS – Basel Institut für angewandte Stadtforschung** (<https://bias.city/prepareaudio>). Sie fügt die WAV-Aufnahme-Teile, in die ein Audiorecorder lange Aufnahmen schneidet (338 MiB je Teil, bei 48 kHz / 32-bit float ca. 30 min 46 s), wieder zu ganzen Aufnahmen zusammen, synchronisiert zwei Audioquellen und mastert als MP3 auf −16 LUFS.

Die Oberfläche spricht Deutsch, Englisch, Französisch und Italienisch (`docs/I18N.md`); gewählt wird im Info-Feld, beim ersten Start gilt die Systemsprache.

- Ordner ins Fenster ziehen oder „Ordner wählen…“. Alle Unterordner werden durchsucht, egal wie sortiert.
- „Zusammenfügen…“ fragt, wo gespeichert werden soll, und legt dort den Ordner `tracks` an (wählt man einen Ordner namens `tracks`, wird er direkt benutzt). Genauso entstehen `sync` und `master` in den anderen Schritten.
- Während gerechnet wird, dreht sich ein Rad, solange das Backend Lebenszeichen sendet. Die Prozente zählen nur erledigte Arbeit. Nach dem Lauf leert sich die Liste; nur Fehlgeschlagene bleiben stehen, mit „Wiederholen“.
- Ausgabe verlustfrei als WAV (Audiodaten bitidentisch kopiert, ab 4 GB automatisch RF64). Name: `yymmdd_SHHMMSS-EHHMMSS_DHHMMSS_<Ordner>.wav`.
- Originale werden nie verändert, vorhandene Dateien nie überschrieben. Schon vorhandene Ergebnisse werden erkannt und übersprungen.

## Wie die Teile zugeordnet werden

Nicht nur ein Namensmuster: Jede WAV-Datei ist ein Kandidat. Details, Recherche zu den Recordern und Messwerte: `docs/CHUNK-ERKENNUNG.md`.

1. **Startzeit je Teil**, beste Quelle zuerst: Broadcast-WAV-Metadaten (`bext`: Datum, Uhrzeit, TimeReference in Samples seit Mitternacht), dann Datum und Uhrzeit im Dateinamen (`REC_01_20260906_112326.WAV`, `260906-112326.WAV`, `2026-09-06 11-23-26.wav` …), zuletzt das Dateidatum (Änderungszeit − Dauer). Die Liste zeigt die Quelle je Teil. Nicht gestellte Geräteuhren (vor 2010) werden behalten und markiert.
2. **Voller Teil:** feste Recorder-Größe, gleiche Größe oder Frame-Zahl bei mehreren Dateien (≥ 100 MiB), knapp unter 2 GB/2 GiB/4 GB/4 GiB oder genau 1 Stunde. Nur ein voller Teil hat einen Folgeteil; ein kurzer Teil beendet die Aufnahme.
3. **Anschluss** bei gleichem Format: iXML-File-Set (gleiche FAMILY_UID, nächster FILE_SET_INDEX) oder exakte TimeReference (TimeRef B = TimeRef A + Frames A, ±2 Samples) belegen ihn direkt; weicht die TimeReference ab, gibt es keinen Anschluss. Sonst muss B höchstens 3 s (Metadaten, Name) bzw. 5 s (Dateidatum) neben dem Ende von A starten.
4. Passen mehrere Teile (zwei Sender, gleiche Dateinamen), entscheidet das Audio an der Nahtstelle: Ein linearer Prädiktor, trainiert auf der einen Seite, sagt nur beim echten Anschluss die andere Seite voraus (26 echte Nähte: echt −1,13…1,26, fremd 1,28…7,87). Gleiche Sequenznummer im Namen und gleicher Ordner helfen leicht.
5. **Verkettung hoch/mittel/niedrig:** hoch = iXML oder exakte TimeReference ohne Widerspruch der Naht; mittel = Zeit passt, voller Teil, Naht bestätigt; niedrig = mehrdeutig, Naht widerspricht oder kein voller Teil. Niedrige Ketten sind nicht vorausgewählt und tragen einen Hinweis.
6. Byte-identische Kopien, `._`-Dateien und frühere Ergebnisse werden ignoriert. Unsichere Zuordnungen, Zeitsprünge, reine Dateidatum-Anschlüsse und abgebrochene Dateiköpfe erscheinen als Hinweis.

## Schritt 2: Synchronisieren

Zweiter Reiter der App. Ordner mit Tracks (oder rohe Recorder-Ordner) hineinziehen. Ergebnis im Ordner `sync`: gemeinsame Abschnitte als Stereo-WAV (links der kleinere Sendername, z. B. `4`, rechts `5`), alles andere je Sender als Mono-WAV.

**Eingaben:** WAV (Tracks aus Schritt 1 oder rohe Aufnahme-Teile) sowie MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF und OGG Vorbis, etwa Handy-Aufnahmen. Jede Nicht-WAV-Datei wird einmal mit Symphonia (eingebaut) in ein 32-bit-float-WAV mit Original-Abtastrate und -Kanälen dekodiert, lückenlos wo das Format Encoder-Verzögerung und Auffüllung angibt (MP3 mit LAME-Tag, OGG). Die Kopie liegt unter `~/Library/Caches/city.bias.prepareaudio/decoded/<hash>.wav`; der Hash umfasst Pfad, Größe und Änderungszeit der Quelle, eine unveränderte Datei wird also nicht erneut dekodiert. Vorher wird der freie Platz geprüft, Kopien, die 30 Tage nicht benutzt wurden, entfernt die nächste Analyse. Name, Sender (Ordnername), Tag und die Bearbeitungsdatei beziehen sich weiter auf die Originaldatei; Analyse, Vorschau, Wellenform und Ausgabe lesen die Kopie. Startzeit: Datum mit Sekunden im Dateinamen, sonst die Erstellungszeit einer M4A (`mvhd`), sonst das Dateidatum (Änderungszeit − Dauer) wie in Schritt 1. Solche Dateien sind nie Aufnahme-Teile, jede ist ein Track. Die MP3-Kopien aus Schritt 3 (Ordner `master`) werden übergangen.

**Mehrkanalige Quellen** (z. B. Stereo vom Handy) gehen als Mono-Mischung (Mittelwert der Kanäle) in Analyse, Vorschau und Stereo-Datei; auf der Mono-Seite entsteht eine Mono-WAV (32-bit float) derselben Mischung. Mono-Tracks bleiben bitgenaue Ausschnitte.

Gesucht werden **gemeinsame Ereignisse mit konstantem Zeitversatz**, nicht Klangähnlichkeit. Die lauteste Quelle ist bei zwei Ansteckmikros oft gegenläufig, verbindend sind Einsätze wie Silben, Stuhlrücken und Geschirr.

1. Einsatzstärke je Millisekunde: Pegelanstieg in dB in vier Bändern (150–400, 400–1000, 1000–2500, 2500–7000 Hz), je Band robust normiert.
2. Globaler Versatz per Kreuzkorrelation (±300 s um die Dateinamen-Zeit), fein in 60-s-Fenstern, robuste Gerade für Versatz und Uhrendrift.
3. Je 20-s-Fenster (alle 10 s) neue Laufzeitsuche (±300 ms). Treffer = scharfer Peak (Prominenz ≥ 6) höchstens ±20 ms neben der Geraden.
4. Trefferanteil über 50 s ergibt Phasen (gemeinsam ≥ 180 s, getrennt ≥ 120 s, Schnitt in der leisesten Sekunde). Gegenprobe: Kohärenz (Welch, 150–1200 Hz) nach Versatzausgleich, gemeinsame Phasen unter 0,02 werden Mono.

Getrennt wird ausschließlich, wenn beide Sender gleichzeitig verschiedene Gespräche aufnehmen. Läuft nur ein Sender (Vorlauf, Nachlauf oder Ausfall zwischen gemeinsamen Abschnitten), bleibt das in der Stereo-Datei, egal wie lange, und der fehlende Kanal ist still. Mono entstehen nur für getrennte parallele Gespräche und für Aufnahmen ohne jeden gemeinsamen Abschnitt.

Ohne Ausfall ist links bitgenau der linke Sender, rechts der andere, per Versatz und Drift verschoben (kubisch interpoliert). Mono-Abschnitte sind bitgenaue Ausschnitte (bei mehrkanaligen Quellen die Mono-Mischung, siehe oben).

### Timeline bearbeiten

Nach der Analyse zeigt eine Timeline den Vorschlag, eine Zeile je Sender und Stellung: Stereo-Clips liegen innen und treffen sich in der Mitte (Sender 4 oben, 5 unten, verbunden durch eine L/R-Leiste), Mono-Clips sind nach außen geschoben. Clips tragen die Wellenform in Senderfarbe. Gelöschte Clips und weggeschnittene Teile sind schraffiert, darunter liegen die gemeinsamen Ereignisse.

- Trimmen und Verlängern an den Clipkanten (Einrasten an Kanten und Playhead). Stoßen zwei Clips aneinander, verschiebt das Ziehen die gemeinsame Grenze.
- Trennen am Playhead (S; ohne Auswahl alle Spuren), Zusammenführen (J), Löschen und Wiederbringen (Entf), Stereo/Mono per Pfeil ↑/↓ oder durch senkrechtes Ziehen, Rückgängig ⌘Z / ⇧⌘Z, Kontextmenü.
- Zeitlich verschieben geht nicht: Die gemessene Platzierung jeder Spur bleibt fest, beide Sender bleiben synchron.
- Zoom mit ⌘ + Mausrad, Zwei-Finger-Geste oder +/−, „Tag“ zeigt alles; die Übersicht darüber verschiebt den Ausschnitt.
- Leertaste spielt ab dem Playhead die Vorschau über das Standard-Ausgabegerät (cpal): Stereo-Clips links/rechts wie in der Datei, Mono mittig, leise Aufnahmen angehoben. M und S schalten Sender stumm oder solo.
- Jede Änderung berechnet die Ausgabeliste sofort neu. Eine Datei endet nur dort, wo alle laufenden Stereo-Clips eine Grenze oder Lücke haben. Die Bearbeitung wird als `.prepareaudio-sync.json` neben den Quellen gespeichert und beim nächsten Analysieren wiederhergestellt; „Vorschlag wiederherstellen“ verwirft sie.

Kalibriert auf den Aufnahmen vom 6.–8.9.2026: Versatz auf ±3 ms gleich wie die Skill-Pipeline. Gegenproben (um 47 s versetzt, unabhängige Aufnahmen) ergaben 0 % Treffer und Kohärenz ≤ 0,005, echte gemeinsame Phasen 76–100 % Treffer und Kohärenz 0,11–0,32.

## Schritt 3: Mastern

Dritter Reiter. Audiodateien oder Ordner hineinziehen (WAV auch RF64, MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF, OGG Vorbis). Die App zeigt je Datei Format, Bittiefe, Abtastrate und Kanäle und misst die Lautheit nach EBU R128 (ITU-R BS.1770-4) mit True Peak und Lautheitsumfang.

- Feste Verstärkung auf −16 LUFS, danach ein Look-ahead-Limiter (5 ms, Release 50 ms) bei −1,5 dBTP. Keine Kompression, die Sprachdynamik bleibt.
- Ausgabe als MP3, CBR 192 kbit/s (LAME, Qualität 2), Mono bleibt Mono, 88,2/96/176,4/192 kHz werden auf 44,1 oder 48 kHz heruntergerechnet.
- Verstärkung und Limiter werden zuerst ohne Kodieren auf dem begrenzten Signal eingestellt (schnell, mehrere Durchgänge), dann wird einmal kodiert. Der Limiter startet 0,5 dB unter −1,5 dBTP, weil MP3 auf stark begrenzten Aufnahmen Spitzen hinzufügt.
- Das fertige MP3 wird nachgemessen. Liegt es mehr als 0,3 LU neben dem Ziel oder mit dem True Peak über −1,4 dBTP, wird nachgeregelt und neu kodiert.
- Der Fortschrittsbalken zählt alle Durchgänge und nennt den aktuellen Schritt (Pegel einstellen, MP3 kodieren, Nachmessen). Mit `PA_MASTER_DEBUG=1` protokolliert der Test `real_master` jeden Durchgang.
- Ergebnis im Ordner `master`. Vorhandene MP3s gleicher Länge werden erkannt und nicht neu geschrieben.

Die App braucht dafür keine installierten Programme: Symphonia liest die Formate, ebur128 misst, LAME 3.100 ist fest einkompiliert.

## Lizenz

PrepareAudio ist freie Software unter der GNU Affero General Public License, Version 3 oder später (`LICENSE`), wie LocalTranscript und PrepareMedia. Copyright © 2026 B/IAS – Basel Institut für angewandte Stadtforschung, <https://bias.city/prepareaudio>. Quellcode: <https://github.com/BenPohlBasel/PrepareAudio>.

Die App enthält Software Dritter unter MIT, Apache-2.0, BSD, Zlib, Unicode, MPL-2.0 (Symphonia, Teile von Tauri) und LGPL (LAME über mp3lame-sys/mp3lame-encoder); alle sind mit der AGPL vereinbar. Die Lizenztexte stehen in `THIRD_PARTY_LICENSES.md`, im App-Paket unter `Contents/Resources` und im Info-Feld der App. Wer die App weitergibt, gibt den Empfängern auch den Quellcode (oder den Zugang dazu).

Nach jeder Änderung an den Abhängigkeiten neu erzeugen:

```bash
python3 scripts/gen-licenses.py
```

## Entwickeln

```bash
npm install
npm run dev                      # App im Entwicklungsmodus
cd src-tauri && cargo test --lib # Unit-Tests
PA_REAL_DIR="/pfad/4|/pfad/5" cargo test --release --test real_data -- --ignored --nocapture
PA_SYNC_DIR="/pfad/tracks" cargo test --release --test real_sync -- --ignored --nocapture
npx tauri build --bundles app    # src-tauri/target/release/bundle/macos/PrepareAudio.app
```

## Release (signiert und notarisiert)

Wie bei LocalTranscript: `tauri build` signiert nur (Developer ID per SHA-1-Hash in `tauri.conf.json`, Hardened Runtime) und läuft ohne Apple-Zugangsdaten; die Notarisierung machen eigene Skripte, damit Apples Warteschlange nie den Build abbricht.

```bash
APPLE_KEYCHAIN_PROFILE=localtranscript npm run release
```

Ablauf: Lizenzliste neu erzeugen, App bauen und signieren, App notarisieren und Ticket anheften (`scripts/notarize-app.mjs`), DMG aus der gestapelten App bauen und signieren (`scripts/build-dmg.mjs`), DMG notarisieren und anheften (`scripts/notarize-dmg.mjs`). Ergebnis: `src-tauri/target/release/bundle/dmg/PrepareAudio_<version>_aarch64.dmg`. Statt des Profils gehen auch `APPLE_ID`, `APPLE_TEAM_ID` und `APPLE_PASSWORD` (app-spezifisches Passwort).
