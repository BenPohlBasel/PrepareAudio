# PrepareAudio

Tauri-App (macOS), die die von der **DJI Mic 2** gestückelten WAV-Teile (338 MiB je Teil, ca. 30 min 46 s) wieder zu ganzen Aufnahmen zusammenfügt.

- Ordner ins Fenster ziehen oder „Ordner wählen…“. Alle Unterordner werden durchsucht, egal wie sortiert.
- Ergebnis: Ordner `tracks`. Bei einem Ordner liegt er darin, bei mehreren (z. B. `4` und `5` eines Tages) im gemeinsamen Elternordner.
- Ausgabe verlustfrei als WAV (Audiodaten bitidentisch kopiert, ab 4 GB automatisch RF64). Name: `yymmdd_SHHMMSS-EHHMMSS_DHHMMSS_<Ordner>.wav`.
- Originale werden nie verändert, vorhandene Dateien nie überschrieben. Schon vorhandene Ergebnisse werden erkannt und übersprungen.

## Wie die Teile zugeordnet werden

1. Dateiname `DJI_<Nr>_<JJJJMMTT>_<HHMMSS>` liefert Sequenznummer und Startzeit.
2. Nur ein voller Chunk kann einen Folgeteil haben. Der Folgeteil hat dieselbe Nummer und dasselbe Format und startet höchstens 3 s neben dem Ende des Vorgängers.
3. Passen mehrere Teile (zwei Sender, gleiche Dateinamen), entscheidet das Audio an der Nahtstelle: Ein linearer Prädiktor, trainiert auf der einen Seite, sagt nur beim echten Anschluss die andere Seite voraus (gemessen an 38 echten Nähten: echt −1,1…1,3, fremd 1,3…7,9).
4. Byte-identische Kopien, `._`-Dateien und frühere Ergebnisse werden ignoriert. Unsichere Zuordnungen, Zeitsprünge und abgebrochene Dateiköpfe erscheinen als Hinweis.

## Schritt 2: Synchronisieren

Zweiter Reiter der App. Ordner mit Tracks (oder rohe DJI-Ordner) hineinziehen. Ergebnis im Ordner `sync`: gemeinsame Abschnitte als Stereo-WAV (links der kleinere Sendername, z. B. `4`, rechts `5`), alles andere je Sender als Mono-WAV.

Gesucht werden **gemeinsame Ereignisse mit konstantem Zeitversatz**, nicht Klangähnlichkeit. Die lauteste Quelle ist bei zwei Ansteckmikros oft gegenläufig, verbindend sind Einsätze wie Silben, Stuhlrücken und Geschirr.

1. Einsatzstärke je Millisekunde: Pegelanstieg in dB in vier Bändern (150–400, 400–1000, 1000–2500, 2500–7000 Hz), je Band robust normiert.
2. Globaler Versatz per Kreuzkorrelation (±300 s um die Dateinamen-Zeit), fein in 60-s-Fenstern, robuste Gerade für Versatz und Uhrendrift.
3. Je 20-s-Fenster (alle 10 s) neue Laufzeitsuche (±300 ms). Treffer = scharfer Peak (Prominenz ≥ 6) höchstens ±20 ms neben der Geraden.
4. Trefferanteil über 50 s ergibt Phasen (gemeinsam ≥ 180 s, getrennt ≥ 120 s, Schnitt in der leisesten Sekunde). Gegenprobe: Kohärenz (Welch, 150–1200 Hz) nach Versatzausgleich, gemeinsame Phasen unter 0,02 werden Mono.

Getrennt wird ausschließlich, wenn beide Sender gleichzeitig verschiedene Gespräche aufnehmen. Läuft nur ein Sender (Vorlauf, Nachlauf oder Ausfall zwischen gemeinsamen Abschnitten), bleibt das in der Stereo-Datei, egal wie lange, und der fehlende Kanal ist still. Mono entstehen nur für getrennte parallele Gespräche und für Aufnahmen ohne jeden gemeinsamen Abschnitt.

Ohne Ausfall ist links bitgenau der linke Sender, rechts der andere, per Versatz und Drift verschoben (kubisch interpoliert). Mono-Abschnitte sind bitgenaue Ausschnitte.

Kalibriert auf den Aufnahmen vom 6.–8.9.2026: Versatz auf ±3 ms gleich wie die Skill-Pipeline. Gegenproben (um 47 s versetzt, unabhängige Aufnahmen) ergaben 0 % Treffer und Kohärenz ≤ 0,005, echte gemeinsame Phasen 76–100 % Treffer und Kohärenz 0,11–0,32.

## Schritt 3: Mastern

Dritter Reiter. Audiodateien oder Ordner hineinziehen (WAV auch RF64, MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF, OGG Vorbis). Die App zeigt je Datei Format, Bittiefe, Abtastrate und Kanäle und misst die Lautheit nach EBU R128 (ITU-R BS.1770-4) mit True Peak und Lautheitsumfang.

- Feste Verstärkung auf −16 LUFS, danach ein Look-ahead-Limiter (5 ms, Release 50 ms) bei −1,5 dBTP. Keine Kompression, die Sprachdynamik bleibt.
- Ausgabe als MP3, CBR 192 kbit/s (LAME, Qualität 2), Mono bleibt Mono, 88,2/96/176,4/192 kHz werden auf 44,1 oder 48 kHz heruntergerechnet.
- Verstärkung und Limiter werden zuerst ohne Kodieren auf dem begrenzten Signal eingestellt (schnell, mehrere Durchgänge), dann wird einmal kodiert. Der Limiter startet 0,5 dB unter −1,5 dBTP, weil MP3 auf stark begrenzten Aufnahmen Spitzen hinzufügt.
- Das fertige MP3 wird nachgemessen. Liegt es mehr als 0,3 LU neben dem Ziel oder mit dem True Peak über −1,4 dBTP, wird nachgeregelt und neu kodiert.
- Der Fortschrittsbalken zählt alle Durchgänge und nennt den aktuellen Schritt (Pegel einstellen, MP3 kodieren, Nachmessen). Mit `DJI_MASTER_DEBUG=1` protokolliert der Test `real_master` jeden Durchgang.
- Ergebnis im Ordner `master`. Vorhandene MP3s gleicher Länge werden erkannt und nicht neu geschrieben.

Die App braucht dafür keine installierten Programme: Symphonia liest die Formate, ebur128 misst, LAME 3.100 ist fest einkompiliert.

## Lizenz

PrepareAudio steht unter der MIT-Lizenz (`LICENSE`). Die App enthält Software Dritter unter MIT, Apache-2.0, BSD, Zlib, Unicode, MPL-2.0 (Symphonia, Teile von Tauri) und LGPL (LAME über mp3lame-sys/mp3lame-encoder). Alle Lizenztexte stehen in `THIRD_PARTY_LICENSES.md`, im App-Paket unter `Contents/Resources` und im Info-Feld der App.

Nach jeder Änderung an den Abhängigkeiten neu erzeugen:

```bash
python3 scripts/gen-licenses.py
```

## Entwickeln

```bash
npm install
npm run dev                      # App im Entwicklungsmodus
cd src-tauri && cargo test --lib # Unit-Tests
DJI_REAL_DIR="/pfad/4|/pfad/5" cargo test --release --test real_data -- --ignored --nocapture
DJI_SYNC_DIR="/pfad/tracks" cargo test --release --test real_sync -- --ignored --nocapture
npx tauri build --bundles app    # src-tauri/target/release/bundle/macos/PrepareAudio.app
```

## Release (signiert und notarisiert)

Wie bei LocalTranscript: `tauri build` signiert nur (Developer ID per SHA-1-Hash in `tauri.conf.json`, Hardened Runtime) und läuft ohne Apple-Zugangsdaten; die Notarisierung machen eigene Skripte, damit Apples Warteschlange nie den Build abbricht.

```bash
APPLE_KEYCHAIN_PROFILE=localtranscript npm run release
```

Ablauf: Lizenzliste neu erzeugen, App bauen und signieren, App notarisieren und Ticket anheften (`scripts/notarize-app.mjs`), DMG aus der gestapelten App bauen und signieren (`scripts/build-dmg.mjs`), DMG notarisieren und anheften (`scripts/notarize-dmg.mjs`). Ergebnis: `src-tauri/target/release/bundle/dmg/PrepareAudio_<version>_aarch64.dmg`. Statt des Profils gehen auch `APPLE_ID`, `APPLE_TEAM_ID` und `APPLE_PASSWORD` (app-spezifisches Passwort).
