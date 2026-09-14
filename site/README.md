# PrepareAudio — statische Website

Die Seite für <https://bias.city/prepareaudio>, aufgebaut wie die Seite von
LocalTranscript (`~/Claude/enrich-transcript/site`): ein Ordner, drei Sachen
darin. Zum Veröffentlichen den Inhalt auf einen beliebigen Webserver legen;
zum Anschauen `index.html` doppelklicken, die Seite läuft auch von `file://`.

```
index.html          alles: Auszeichnung, Stil, Übersetzungen
fonts/              Recursive Variable (woff2) + OFL-Lizenztext
img/                Bildschirmfotos der App, je Motiv in vier Sprachen
```

## Was die Seite bewusst NICHT tut

- **Keine Cookies, kein Storage, keine Messung.**
- **Keine fremden Server.** Schrift und Logo liegen im Ordner bzw. in der
  Seite. Die einzigen ausgehenden Adressen sind Links, die der Besucher
  anklickt: GitHub, LinkedIn, bias.city.
- **Keine Markennamen.** Die Seite spricht von Aufnahme-Teilen von
  Audiorecordern, nicht von einem Gerätehersteller, und sagt klar, dass
  Zusammenfügen und Synchronisieren nur WAV annehmen.

## Raster, Schrift, Farbe

Raster und Typografie sind von LocalTranscript übernommen (Label 17 rem,
Inhalt mindestens 30 rem, Recursive, Schrift schwarz oder weiss). Der Ton ist
die Aubergine aus dem App-Zeichen: `--aub-600` (#56107a) für Knöpfe und
Unterstriche, `--aub-700` für den Zeigezustand, `--aub-300` für Links auf dem
dunklen Datenschutz-Abschnitt. Dieselbe Farbe trägt die App.

Das Zeichen in der Überschrift und das Favicon sind das App-Logo
(`src-tauri/icons/icon-source.svg`) als Inline-SVG.

## Sprachen

Deutsch, Englisch, Französisch, Italienisch — dieselben vier wie in der App.
Umgeschaltet wird oben rechts, die Wahl landet in `?lang=xx`; ohne Angabe
entscheidet die Browsersprache, sonst Deutsch. Alle Texte stehen im Objekt `T`
am Ende von `index.html`.

## Bildschirmfotos

`img/<motiv>-<sprache>.png`, sieben Motive mal vier Sprachen. Beim
Sprachwechsel tauscht das Skript die Quelle jedes `img[data-img]` aus, die
Seite zeigt die App also immer in der gewählten Sprache.

Aufgenommen aus der echten Oberfläche (`ui/`) im Browser mit einem
nachgebauten Backend, gefüttert mit den Analysedaten der Aufnahmen vom
6.9.2026 in Basel. Pfade sind auf `/Users/demo/Aufnahmen` umgeschrieben. Die
Bilder enthalten nur Dateinamen, Zeiten, Pegel und Wellenformen, keine
Inhalte. Hero 1600 × 1000, Anleitung 1200 × 750, als PNG-8 (256 Farben ohne
Dithering). Die Analysedaten erzeugt der ignorierte Test `dump_plan_for_ui`
in `src-tauri/src/sync.rs`.

## Version

`const VERSION` am Anfang des Skripts in `index.html`. Bei jedem Release
nachziehen, Quelle ist `src-tauri/tauri.conf.json`.

## Schrift

Recursive Variable Font, **SIL OFL 1.1**, Lizenztext in `fonts/OFL.txt`. Die
Nennung steht in der Fusszeile der Seite und hier.

## Veröffentlichen

Die Seite liegt auf dem Webspace von bias.city unter `bias.city/prepareaudio`
und wird per FTP gespiegelt (den Inhalt dieses Ordners ohne `README.md`). Die
Zugangsdaten stehen nicht im Repository; sie liegen im Schlüsselbund der
Entwickler und dürfen nie in Ausgaben oder Protokollen erscheinen.

Danach mit `curl -s https://bias.city/prepareaudio/ | grep VERSION` prüfen.
Der Download-Knopf zeigt auf die Release-Übersicht im öffentlichen Repository
<https://github.com/BenPohlBasel/PrepareAudio/releases>.
