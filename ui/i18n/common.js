'use strict';

/* Dictionary: common. Keys common.* — de, en, fr, it are all required. See docs/I18N.md. */
I18N.add({
  /* ---------- shared buttons and words ---------- */
  'common.chooseFolder': { de: 'Ordner wählen…', en: 'Choose folder…', fr: 'Choisir un dossier…', it: 'Scegli cartella…' },
  'common.otherFolder': { de: 'Anderer Ordner…', en: 'Other folder…', fr: 'Autre dossier…', it: 'Altra cartella…' },
  'common.all': { de: 'Alle', en: 'All', fr: 'Tout', it: 'Tutti' },
  'common.none': { de: 'Keine', en: 'None', fr: 'Aucun', it: 'Nessuno' },
  'common.cancel': { de: 'Abbrechen', en: 'Cancel', fr: 'Annuler', it: 'Annulla' },
  'common.retry': { de: 'Wiederholen', en: 'Retry', fr: 'Réessayer', it: 'Riprova' },
  'common.close': { de: 'Schließen', en: 'Close', fr: 'Fermer', it: 'Chiudi' },
  'common.create': { de: 'Erzeugen…', en: 'Create…', fr: 'Créer…', it: 'Crea…' },
  'common.reveal': { de: 'Im Finder zeigen', en: 'Show in Finder', fr: 'Afficher dans le Finder', it: 'Mostra nel Finder' },

  /* ---------- activity ---------- */
  'common.working': { de: 'rechnet', en: 'working', fr: 'calcul en cours', it: 'elaborazione in corso' },
  'common.noResponse': { de: 'Keine Rückmeldung seit {s} s', en: 'No response for {s} s', fr: 'Aucune réponse depuis {s} s', it: 'Nessuna risposta da {s} s' },
  'common.preparing': { de: 'Vorbereiten…', en: 'Preparing…', fr: 'Préparation…', it: 'Preparazione…' },
  'common.cancelling': { de: 'Breche ab…', en: 'Cancelling…', fr: 'Annulation…', it: 'Annullamento…' },
  'common.running': { de: 'läuft…', en: 'running…', fr: 'en cours…', it: 'in corso…' },
  'common.progress': { de: '{i} von {count} · {name} · {pct} %', en: '{i} of {count} · {name} · {pct} %', fr: '{i} sur {count} · {name} · {pct} %', it: '{i} di {count} · {name} · {pct} %' },
  'common.waitBusy': {
    de: 'Bitte warten, bis der laufende Vorgang fertig ist.',
    en: 'Please wait until the current task has finished.',
    fr: 'Veuillez attendre la fin de l’opération en cours.',
    it: 'Attendi che l’operazione in corso sia terminata.',
  },
  'common.nothingSelected': { de: 'nichts ausgewählt', en: 'nothing selected', fr: 'rien de sélectionné', it: 'nessuna selezione' },

  /* ---------- results ---------- */
  'common.doneOf': { de: '{done} von {total} erledigt', en: '{done} of {total} done', fr: '{done} sur {total} terminés', it: '{done} di {total} completati' },
  'common.status.written': { de: 'Fertig', en: 'Done', fr: 'Terminé', it: 'Fatto' },
  'common.status.existing': { de: 'Schon vorhanden', en: 'Already there', fr: 'Déjà présent', it: 'Già presente' },
  'common.status.failed': { de: 'Fehler', en: 'Error', fr: 'Erreur', it: 'Errore' },
  'common.status.cancelled': { de: 'Abgebrochen', en: 'Cancelled', fr: 'Annulé', it: 'Annullato' },
  /* finishedCard fallback when the caller passes only opts.noun: {items} is "3 Dateien" etc. */
  'common.finished.written': { de: '{items} geschrieben', en: '{items} written', fr: '{items} écrits', it: '{items} scritti' },
  'common.finished.existing.one': { de: '{n} schon vorhanden', en: '{n} already there', fr: '{n} déjà présent', it: '{n} già presente' },
  'common.finished.existing.other': { de: '{n} schon vorhanden', en: '{n} already there', fr: '{n} déjà présents', it: '{n} già presenti' },
  'common.finished.nothing': { de: 'nichts zu tun', en: 'nothing to do', fr: 'rien à faire', it: 'niente da fare' },

  /* ---------- formats ---------- */
  'common.bytesUnits': { de: 'B|KB|MB|GB|TB', en: 'B|KB|MB|GB|TB', fr: 'o|Ko|Mo|Go|To', it: 'B|KB|MB|GB|TB' },

  /* ================= page.*: static text of index.html ================= */

  /* ---------- header ---------- */
  'page.subtitle': {
    de: 'Aufnahme-Teile zusammenfügen, Audioquellen synchronisieren, als MP3 auf −16 LUFS mastern',
    en: 'Merge recording chunks, synchronise audio sources, master to MP3 at −16 LUFS',
    fr: 'Assembler des fragments d’enregistrement, synchroniser des sources audio, masteriser en MP3 à −16 LUFS',
    it: 'Unire frammenti di registrazione, sincronizzare sorgenti audio, masterizzare in MP3 a −16 LUFS',
  },
  'page.tab.merge': { de: 'Zusammenfügen', en: 'Merge', fr: 'Assembler', it: 'Unire' },
  'page.tab.sync': { de: 'Synchronisieren', en: 'Synchronise', fr: 'Synchroniser', it: 'Sincronizzare' },
  'page.tab.master': { de: 'Mastern', en: 'Master', fr: 'Masteriser', it: 'Masterizzare' },
  'page.scanAgain': { de: 'Neu scannen', en: 'Scan again', fr: 'Réanalyser', it: 'Scansiona di nuovo' },
  'page.scanAgain.title': { de: 'Dieselben Ordner erneut durchsuchen', en: 'Scan the same folders again', fr: 'Réanalyser les mêmes dossiers', it: 'Scansiona di nuovo le stesse cartelle' },
  'page.analyseAgain': { de: 'Neu analysieren', en: 'Analyse again', fr: 'Réanalyser', it: 'Analizza di nuovo' },
  'page.analyseAgain.title': { de: 'Dieselben Ordner erneut analysieren', en: 'Analyse the same folders again', fr: 'Réanalyser les mêmes dossiers', it: 'Analizza di nuovo le stesse cartelle' },
  'page.measureAgain': { de: 'Neu messen', en: 'Measure again', fr: 'Remesurer', it: 'Misura di nuovo' },
  'page.measureAgain.title': { de: 'Dieselben Ordner erneut messen', en: 'Measure the same folders again', fr: 'Remesurer les mêmes dossiers', it: 'Misura di nuovo le stesse cartelle' },
  'page.info': { de: 'Info und Lizenzen', en: 'Info and licences', fr: 'Informations et licences', it: 'Info e licenze' },

  /* ---------- 1 merge ---------- */
  'page.merge.drop': { de: 'Ordner mit Aufnahmen hierher ziehen', en: 'Drag a folder with recordings here', fr: 'Glissez ici un dossier d’enregistrements', it: 'Trascina qui una cartella con registrazioni' },
  'page.merge.text': {
    de: 'Nur WAV. Aufnahme-Teile, die ein Audiorecorder bei langen Aufnahmen schreibt, werden an der Aufnahmezeit erkannt (aus Metadaten, Dateiname oder Dateidatum) und an der Nahtstelle per Audio geprüft. Sauber sortiert oder durcheinander: alle Unterordner werden durchsucht, Kopien erkannt, die Teile jeder Aufnahme wieder zusammengesetzt.',
    en: 'WAV only. The recording chunks an audio recorder writes during long recordings are recognised by their recording time (from metadata, file name or file date) and checked by an audio test at the seam. Neatly sorted or jumbled: every subfolder is scanned, copies are detected and the chunks of each recording are put back together.',
    fr: 'WAV uniquement. Les fragments d’enregistrement qu’un enregistreur audio crée lors de longs enregistrements sont reconnus à leur heure d’enregistrement (issue des métadonnées, du nom de fichier ou de la date du fichier) et vérifiés par un contrôle audio au raccord. Bien rangés ou en vrac : tous les sous-dossiers sont analysés, les copies détectées et les fragments de chaque enregistrement réassemblés.',
    it: 'Solo WAV. I frammenti di registrazione che un registratore audio crea durante le registrazioni lunghe vengono riconosciuti dall’ora di registrazione (dai metadati, dal nome del file o dalla data del file) e verificati con un controllo audio al raccordo. In ordine o alla rinfusa: tutte le sottocartelle vengono scansionate, le copie riconosciute e i frammenti di ogni registrazione ricomposti.',
  },
  'page.merge.fine': {
    de: 'Die Originale bleiben unangetastet. Beim Start fragt die App, wo der Ordner <b>tracks</b> entstehen soll, verlustfrei als WAV.',
    en: 'The originals stay untouched. When you start, the app asks where to create the <b>tracks</b> folder, lossless as WAV.',
    fr: 'Les originaux restent intacts. Au démarrage, l’app demande où créer le dossier <b>tracks</b>, sans perte en WAV.',
    it: 'Gli originali restano intatti. All’avvio l’app chiede dove creare la cartella <b>tracks</b>, senza perdita in WAV.',
  },
  'page.merge.singles': {
    de: 'Einzelaufnahmen (nicht gestückelt) ebenfalls nach tracks kopieren',
    en: 'Also copy single recordings (not split) to tracks',
    fr: 'Copier aussi les enregistrements uniques (non fragmentés) dans tracks',
    it: 'Copia in tracks anche le registrazioni singole (non frammentate)',
  },
  'page.merge.go': { de: 'Zusammenfügen…', en: 'Merge…', fr: 'Assembler…', it: 'Unisci…' },

  /* ---------- 2 sync ---------- */
  'page.sync.drop': { de: 'Tracks hierher ziehen', en: 'Drag tracks here', fr: 'Glissez des pistes ici', it: 'Trascina qui le tracce' },
  'page.sync.text': {
    de: 'Die App sucht Ereignisse, die beide Mikrofone mit gleichbleibendem Zeitversatz hören: Silben, Stuhlrücken, Geschirr. Wo zwei Sender dasselbe Geschehen aufgenommen haben, entsteht eine Stereo-Datei. Wo die Mikros in verschiedenen Situationen waren, bleiben zwei Mono-Spuren.',
    en: 'The app looks for events that both microphones hear with a constant time offset: syllables, a chair scraping, dishes. Where two transmitters recorded the same scene, a stereo file is created. Where the mics were in different situations, two mono tracks remain.',
    fr: 'L’app cherche des événements que les deux micros entendent avec un décalage constant : syllabes, chaise qu’on déplace, vaisselle. Là où deux émetteurs ont enregistré la même scène, un fichier stéréo est créé. Là où les micros étaient dans des situations différentes, deux pistes mono restent.',
    it: 'L’app cerca eventi che entrambi i microfoni sentono con uno sfasamento costante: sillabe, una sedia spostata, stoviglie. Dove due trasmettitori hanno registrato la stessa scena nasce un file stereo. Dove i microfoni erano in situazioni diverse restano due tracce mono.',
  },
  'page.sync.fine': {
    de: 'Am besten den Ordner <b>tracks</b> aus Schritt 1; rohe Aufnahme-Teile vom Recorder gehen auch. Neben WAV nimmt die App MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF und OGG an: Diese Dateien werden einmal dekodiert und zwischengespeichert, erzeugt wird immer WAV. Danach lässt sich alles in der Timeline bearbeiten; beim Erzeugen fragt die App, wo der Ordner <b>sync</b> entstehen soll.',
    en: 'Ideally the <b>tracks</b> folder from step 1; raw recording chunks from the recorder work too. Besides WAV the app accepts MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF and OGG: these files are decoded once and cached, the output is always WAV. Afterwards everything can be edited in the timeline; when creating, the app asks where to create the <b>sync</b> folder.',
    fr: 'Idéalement le dossier <b>tracks</b> de l’étape 1 ; les fragments bruts de l’enregistreur fonctionnent aussi. Outre le WAV, l’app accepte MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF et OGG : ces fichiers sont décodés une fois et mis en cache, la sortie est toujours en WAV. Tout se modifie ensuite dans la timeline ; à la création, l’app demande où créer le dossier <b>sync</b>.',
    it: 'Meglio la cartella <b>tracks</b> del passo 1, ma vanno bene anche i frammenti grezzi del registratore. Oltre al WAV l’app accetta MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF e OGG: questi file vengono decodificati una volta e messi in cache, l’uscita è sempre WAV. Poi tutto si modifica nella timeline; alla creazione l’app chiede dove creare la cartella <b>sync</b>.',
  },
  'page.sync.pairs': { de: 'Track-Paare', en: 'Track pairs', fr: 'Paires de pistes', it: 'Coppie di tracce' },
  'page.sync.output': { de: 'Ausgabe', en: 'Output', fr: 'Sortie', it: 'Uscita' },
  'page.sync.analysing': { de: 'Analysiere Tracks…', en: 'Analysing tracks…', fr: 'Analyse des pistes…', it: 'Analisi delle tracce…' },
  'page.sync.searching': { de: 'Suche Tracks', en: 'Looking for tracks', fr: 'Recherche de pistes', it: 'Ricerca delle tracce' },

  /* ---------- 2 sync: timeline toolbar ---------- */
  'page.ed.play.title': { de: 'Abspielen oder anhalten (Leertaste)', en: 'Play or pause (Space)', fr: 'Lire ou mettre en pause (Espace)', it: 'Riproduci o metti in pausa (Spazio)' },
  'page.ed.split': { de: 'Trennen', en: 'Split', fr: 'Couper', it: 'Dividi' },
  'page.ed.split.title': { de: 'Am Playhead trennen (S)', en: 'Split at the playhead (S)', fr: 'Couper à la tête de lecture (S)', it: 'Dividi alla testina (S)' },
  'page.ed.join': { de: 'Zusammenführen', en: 'Join', fr: 'Joindre', it: 'Congiungi' },
  'page.ed.join.title': { de: 'Mit dem nächsten Clip zusammenführen (J)', en: 'Join with the next clip (J)', fr: 'Joindre au clip suivant (J)', it: 'Congiungi con la clip successiva (J)' },
  'page.ed.delete': { de: 'Löschen', en: 'Delete', fr: 'Supprimer', it: 'Elimina' },
  'page.ed.delete.title': { de: 'Löschen oder wiederbringen (Entf)', en: 'Delete or restore (Delete)', fr: 'Supprimer ou rétablir (Suppr)', it: 'Elimina o ripristina (Canc)' },
  'page.ed.stereo': { de: 'Stereo', en: 'Stereo', fr: 'Stéréo', it: 'Stereo' },
  'page.ed.stereo.title': { de: 'Auf Stereo schieben (↑)', en: 'Move to stereo (↑)', fr: 'Passer en stéréo (↑)', it: 'Sposta su stereo (↑)' },
  'page.ed.mono': { de: 'Mono', en: 'Mono', fr: 'Mono', it: 'Mono' },
  'page.ed.mono.title': { de: 'Auf Mono schieben (↓)', en: 'Move to mono (↓)', fr: 'Passer en mono (↓)', it: 'Sposta su mono (↓)' },
  'page.ed.undo.title': { de: 'Rückgängig (⌘Z)', en: 'Undo (⌘Z)', fr: 'Annuler (⌘Z)', it: 'Annulla (⌘Z)' },
  'page.ed.redo.title': { de: 'Wiederholen (⇧⌘Z)', en: 'Redo (⇧⌘Z)', fr: 'Rétablir (⇧⌘Z)', it: 'Ripeti (⇧⌘Z)' },
  'page.ed.reset': { de: 'Vorschlag wiederherstellen', en: 'Restore proposal', fr: 'Rétablir la proposition', it: 'Ripristina la proposta' },
  'page.ed.reset.title': { de: 'Alle Bearbeitungen verwerfen', en: 'Discard all edits', fr: 'Abandonner toutes les modifications', it: 'Scarta tutte le modifiche' },
  'page.ed.zoomOut.title': { de: 'Verkleinern (⌘−)', en: 'Zoom out (⌘−)', fr: 'Zoom arrière (⌘−)', it: 'Riduci (⌘−)' },
  'page.ed.zoomIn.title': { de: 'Vergrößern (⌘+)', en: 'Zoom in (⌘+)', fr: 'Zoom avant (⌘+)', it: 'Ingrandisci (⌘+)' },
  'page.ed.fit': { de: 'Tag', en: 'Day', fr: 'Jour', it: 'Giorno' },
  'page.ed.fit.title': { de: 'Ganzen Tag zeigen (0)', en: 'Show the whole day (0)', fr: 'Afficher toute la journée (0)', it: 'Mostra l’intera giornata (0)' },
  'page.ed.overview.title': {
    de: 'Übersicht: klicken oder ziehen zum Verschieben',
    en: 'Overview: click or drag to move',
    fr: 'Vue d’ensemble : cliquez ou glissez pour vous déplacer',
    it: 'Panoramica: clicca o trascina per spostarti',
  },
  'page.ed.hint': {
    de: 'Kanten ziehen zum Trimmen oder Verlängern, Clip nach innen oder außen ziehen für Stereo oder Mono. Leertaste spielt, S trennt, J führt zusammen, Entf löscht, ⌘Z macht rückgängig. ⌘ + Mausrad oder Zwei-Finger-Zoom zoomt.',
    en: 'Drag edges to trim or extend, drag a clip inwards or outwards for stereo or mono. Space plays, S splits, J joins, Delete deletes, ⌘Z undoes. ⌘ + scroll wheel or two-finger pinch zooms.',
    fr: 'Glissez les bords pour rogner ou prolonger, glissez un clip vers l’intérieur ou l’extérieur pour stéréo ou mono. Espace lit, S coupe, J joint, Suppr supprime, ⌘Z annule. ⌘ + molette ou pincement à deux doigts pour zoomer.',
    it: 'Trascina i bordi per tagliare o allungare, trascina una clip verso l’interno o l’esterno per stereo o mono. Spazio riproduce, S divide, J congiunge, Canc elimina, ⌘Z annulla. ⌘ + rotellina o pizzico con due dita per lo zoom.',
  },

  /* ---------- 3 master ---------- */
  'page.master.drop': { de: 'Audiodateien hierher ziehen', en: 'Drag audio files here', fr: 'Glissez des fichiers audio ici', it: 'Trascina qui i file audio' },
  'page.master.text': {
    de: 'Die App erkennt Format und Bittiefe jeder Datei, misst die Lautheit nach EBU R128 und bringt sie mit einer festen Verstärkung auf −16 LUFS. Ein Limiter fängt nur die Spitzen bei −1,5 dBTP ab, die Sprachdynamik bleibt.',
    en: 'The app detects the format and bit depth of every file, measures loudness according to EBU R128 and brings it to −16 LUFS with a fixed gain. A limiter only catches the peaks at −1.5 dBTP; the dynamics of speech remain.',
    fr: 'L’app reconnaît le format et la profondeur de bits de chaque fichier, mesure la loudness selon EBU R128 et l’amène à −16 LUFS avec un gain fixe. Un limiteur n’intercepte que les crêtes à −1,5 dBTP, la dynamique de la parole est préservée.',
    it: 'L’app riconosce formato e profondità di bit di ogni file, misura la loudness secondo EBU R128 e la porta a −16 LUFS con un guadagno fisso. Un limiter intercetta solo i picchi a −1,5 dBTP, la dinamica del parlato resta.',
  },
  'page.master.fine': {
    de: 'Nimmt WAV (auch RF64 und 32-bit float), MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF und OGG Vorbis, zum Beispiel den Ordner <b>sync</b> aus Schritt 2. Beim Start fragt die App, wo der Ordner <b>master</b> entstehen soll, MP3 mit 192 kbit/s.',
    en: 'Accepts WAV (also RF64 and 32-bit float), MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF and OGG Vorbis, for example the <b>sync</b> folder from step 2. When you start, the app asks where to create the <b>master</b> folder, MP3 at 192 kbit/s.',
    fr: 'Accepte WAV (y compris RF64 et 32 bits flottant), MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF et OGG Vorbis, par exemple le dossier <b>sync</b> de l’étape 2. Au démarrage, l’app demande où créer le dossier <b>master</b>, en MP3 à 192 kbit/s.',
    it: 'Accetta WAV (anche RF64 e 32 bit float), MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF e OGG Vorbis, per esempio la cartella <b>sync</b> del passo 2. All’avvio l’app chiede dove creare la cartella <b>master</b>, MP3 a 192 kbit/s.',
  },
  'page.master.files': { de: 'Dateien', en: 'Files', fr: 'Fichiers', it: 'File' },
  'page.master.go': { de: 'Mastern…', en: 'Master…', fr: 'Masteriser…', it: 'Masterizza…' },
  'page.master.measuring': { de: 'Messe Lautheit…', en: 'Measuring loudness…', fr: 'Mesure de la loudness…', it: 'Misurazione della loudness…' },
  'page.master.searching': { de: 'Suche Audiodateien', en: 'Looking for audio files', fr: 'Recherche de fichiers audio', it: 'Ricerca dei file audio' },

  /* ---------- overlays ---------- */
  'page.dropToScan': { de: 'Loslassen zum Durchsuchen', en: 'Release to scan', fr: 'Relâchez pour analyser', it: 'Rilascia per scansionare' },
  'page.scanning': { de: 'Durchsuche Ordner…', en: 'Scanning folders…', fr: 'Analyse des dossiers…', it: 'Scansione delle cartelle…' },

  /* ---------- info modal (static parts) ---------- */
  'page.info.what': { de: 'Was die App macht', en: 'What the app does', fr: 'Ce que fait l’app', it: 'Cosa fa l’app' },
  'page.info.step1': {
    de: '<b>Zusammenfügen:</b> setzt die Aufnahme-Teile eines Audiorecorders bitgenau zu ganzen WAV-Aufnahmen zusammen.',
    en: '<b>Merge:</b> puts the recording chunks of an audio recorder back together into whole WAV recordings, bit for bit.',
    fr: '<b>Assembler :</b> réassemble au bit près les fragments d’enregistrement d’un enregistreur audio en enregistrements WAV complets.',
    it: '<b>Unire:</b> ricompone bit per bit i frammenti di registrazione di un registratore audio in registrazioni WAV complete.',
  },
  'page.info.step2': {
    de: '<b>Synchronisieren:</b> findet gemeinsame Ereignisse mit gleichbleibendem Zeitversatz und schreibt gemeinsame Abschnitte als Stereo, parallele verschiedene Gespräche als Mono.',
    en: '<b>Synchronise:</b> finds shared events with a constant time offset and writes shared sections as stereo, separate conversations running in parallel as mono.',
    fr: '<b>Synchroniser :</b> trouve des événements communs avec un décalage constant et écrit les passages communs en stéréo, les conversations distinctes en parallèle en mono.',
    it: '<b>Sincronizzare:</b> trova eventi comuni con uno sfasamento costante e scrive le parti comuni in stereo, le conversazioni diverse in parallelo in mono.',
  },
  'page.info.step3': {
    de: '<b>Mastern:</b> misst die Lautheit nach EBU R128 (ITU-R BS.1770-4), bringt sie mit fester Verstärkung auf −16 LUFS, begrenzt Spitzen bei −1,5 dBTP und schreibt MP3 mit 192 kbit/s.',
    en: '<b>Master:</b> measures loudness according to EBU R128 (ITU-R BS.1770-4), brings it to −16 LUFS with a fixed gain, limits peaks at −1.5 dBTP and writes MP3 at 192 kbit/s.',
    fr: '<b>Masteriser :</b> mesure la loudness selon EBU R128 (ITU-R BS.1770-4), l’amène à −16 LUFS avec un gain fixe, limite les crêtes à −1,5 dBTP et écrit du MP3 à 192 kbit/s.',
    it: '<b>Masterizzare:</b> misura la loudness secondo EBU R128 (ITU-R BS.1770-4), la porta a −16 LUFS con un guadagno fisso, limita i picchi a −1,5 dBTP e scrive MP3 a 192 kbit/s.',
  },
  'page.info.selfContained': {
    de: 'Alles läuft in der App selbst, ohne installierte Zusatzprogramme. Originaldateien werden nie verändert oder überschrieben.',
    en: 'Everything runs inside the app itself, without any additional programs installed. Original files are never changed or overwritten.',
    fr: 'Tout s’exécute dans l’app elle-même, sans programme supplémentaire installé. Les fichiers originaux ne sont jamais modifiés ni écrasés.',
    it: 'Tutto avviene nell’app stessa, senza programmi aggiuntivi installati. I file originali non vengono mai modificati né sovrascritti.',
  },
  'page.info.licence': { de: 'Lizenz', en: 'Licence', fr: 'Licence', it: 'Licenza' },
  /* Placeholder until info.js fills #info-license with version details. */
  'page.info.licenceText': {
    de: 'PrepareAudio ist freie Software unter der GNU Affero General Public License, Version 3 oder später.',
    en: 'PrepareAudio is free software under the GNU Affero General Public License, version 3 or later.',
    fr: 'PrepareAudio est un logiciel libre sous licence GNU Affero General Public License, version 3 ou ultérieure.',
    it: 'PrepareAudio è software libero sotto la GNU Affero General Public License, versione 3 o successiva.',
  },
  'page.info.showLicence': { de: 'Lizenztext anzeigen', en: 'Show licence text', fr: 'Afficher le texte de la licence', it: 'Mostra il testo della licenza' },
  'page.info.included': { de: 'Enthaltene Software', en: 'Included software', fr: 'Logiciels inclus', it: 'Software incluso' },
  'page.info.loading': { de: 'Lade Lizenzangaben…', en: 'Loading licence information…', fr: 'Chargement des informations de licence…', it: 'Caricamento delle informazioni sulle licenze…' },
  'page.info.search': { de: 'Paket oder Lizenz suchen', en: 'Search package or licence', fr: 'Rechercher un paquet ou une licence', it: 'Cerca pacchetto o licenza' },
  'page.info.allTexts': { de: 'Alle Lizenztexte', en: 'All licence texts', fr: 'Tous les textes de licence', it: 'Tutti i testi delle licenze' },
});
