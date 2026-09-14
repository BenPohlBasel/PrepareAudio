'use strict';

/* Dictionary: merge. Keys merge.* — de, en, fr, it are all required. See docs/I18N.md. */
I18N.add({
  /* ---------- dialogs and messages ---------- */
  'merge.pickTitle': { de: 'Ordner mit Aufnahmen wählen', en: 'Choose folder with recordings', fr: 'Choisir le dossier des enregistrements', it: 'Scegli la cartella con le registrazioni' },
  'merge.noneFound': {
    de: 'Keine Aufnahmen gefunden. Gelesen werden nur WAV-Dateien; Aufnahme-Teile werden an der Aufnahmezeit (Metadaten, Dateiname oder Dateidatum) und per Audio-Prüfung an der Nahtstelle erkannt.',
    en: 'No recordings found. Only WAV files are read; recording chunks are recognised by their recording time (metadata, file name or file date) and an audio check at the seam.',
    fr: 'Aucun enregistrement trouvé. Seuls les fichiers WAV sont lus ; les fragments d’enregistrement sont reconnus à leur heure d’enregistrement (métadonnées, nom de fichier ou date du fichier) et par un contrôle audio au raccord.',
    it: 'Nessuna registrazione trovata. Vengono letti solo file WAV; i frammenti di registrazione sono riconosciuti dall’ora di registrazione (metadati, nome del file o data del file) e da un controllo audio al raccordo.',
  },
  'merge.whereToSave': { de: 'Wo sollen die Tracks gespeichert werden?', en: 'Where should the tracks be saved?', fr: 'Où enregistrer les pistes ?', it: 'Dove salvare le tracce?' },

  /* ---------- finished / partial ---------- */
  'merge.track.one': { de: 'Track', en: 'track', fr: 'piste', it: 'traccia' },
  'merge.track.other': { de: 'Tracks', en: 'tracks', fr: 'pistes', it: 'tracce' },
  'merge.finished.written.one': { de: '{n} Track geschrieben', en: '{n} track written', fr: '{n} piste écrite', it: '{n} traccia scritta' },
  'merge.finished.written.other': { de: '{n} Tracks geschrieben', en: '{n} tracks written', fr: '{n} pistes écrites', it: '{n} tracce scritte' },
  'merge.finished.open': { de: 'tracks öffnen', en: 'Open tracks', fr: 'Ouvrir tracks', it: 'Apri tracks' },
  'merge.finished.next': { de: 'Weiter: synchronisieren', en: 'Next: synchronise', fr: 'Suite : synchroniser', it: 'Avanti: sincronizzare' },
  'merge.finished.new': { de: 'Weitere Aufnahmen', en: 'More recordings', fr: 'Autres enregistrements', it: 'Altre registrazioni' },
  'merge.partialFailed.one': {
    de: '{n} Aufnahme nicht, siehe Liste.', en: '{n} recording failed, see list.',
    fr: '{n} enregistrement en échec, voir la liste.', it: '{n} registrazione non riuscita, vedi elenco.',
  },
  'merge.partialFailed.other': {
    de: '{n} Aufnahmen nicht, siehe Liste.', en: '{n} recordings failed, see list.',
    fr: '{n} enregistrements en échec, voir la liste.', it: '{n} registrazioni non riuscite, vedi elenco.',
  },

  /* ---------- summary ---------- */
  'merge.stat.split.one': { de: 'gestückelte Aufnahmen aus {n} Teil', en: 'split recordings from {n} chunk', fr: 'enregistrements fragmentés, issus de {n} fragment', it: 'registrazioni frammentate, da {n} frammento' },
  'merge.stat.split.other': { de: 'gestückelte Aufnahmen aus {n} Teilen', en: 'split recordings from {n} chunks', fr: 'enregistrements fragmentés, issus de {n} fragments', it: 'registrazioni frammentate, da {n} frammenti' },
  'merge.stat.singles': { de: 'Einzelaufnahmen', en: 'single recordings', fr: 'enregistrements uniques', it: 'registrazioni singole' },
  'merge.stat.files': { de: 'WAV-Dateien gefunden', en: 'WAV files found', fr: 'fichiers WAV trouvés', it: 'file WAV trovati' },
  'merge.stat.skipped.one': { de: 'übersprungen ({n} Kopie)', en: 'skipped ({n} copy)', fr: 'ignorés ({n} copie)', it: 'saltati ({n} copia)' },
  'merge.stat.skipped.other': { de: 'übersprungen ({n} Kopien)', en: 'skipped ({n} copies)', fr: 'ignorés ({n} copies)', it: 'saltati ({n} copie)' },
  'merge.stat.scanned': { de: 'Durchsucht: {roots}', en: 'Scanned: {roots}', fr: 'Analysé : {roots}', it: 'Scansionato: {roots}' },

  /* ---------- list ---------- */
  'merge.noRecordings': { de: 'Keine Aufnahmen gefunden.', en: 'No recordings found.', fr: 'Aucun enregistrement trouvé.', it: 'Nessuna registrazione trovata.' },
  'merge.skippedFiles.one': { de: '{n} Datei übersprungen', en: '{n} file skipped', fr: '{n} fichier ignoré', it: '{n} file saltato' },
  'merge.skippedFiles.other': { de: '{n} Dateien übersprungen', en: '{n} files skipped', fr: '{n} fichiers ignorés', it: '{n} file saltati' },
  'merge.duplicates': { de: 'Identische Kopien', en: 'Identical copies', fr: 'Copies identiques', it: 'Copie identiche' },
  'merge.unused': { de: 'Nicht verwendet', en: 'Not used', fr: 'Non utilisés', it: 'Non usati' },
  'merge.gap': { de: 'Anschluss {gap} s', en: 'gap {gap} s', fr: 'raccord {gap} s', it: 'raccordo {gap} s' },
  'merge.timeSource.bext': { de: 'Zeit aus Metadaten', en: 'time from metadata', fr: 'heure issue des métadonnées', it: 'ora dai metadati' },
  'merge.timeSource.name': { de: 'Zeit aus Dateiname', en: 'time from file name', fr: 'heure issue du nom de fichier', it: 'ora dal nome del file' },
  'merge.timeSource.file': { de: 'Zeit aus Dateidatum', en: 'time from file date', fr: 'heure issue de la date du fichier', it: 'ora dalla data del file' },
  'merge.confidence': { de: 'Verkettung: {level}', en: 'Chain confidence: {level}', fr: 'Fiabilité de l’enchaînement : {level}', it: 'Affidabilità del concatenamento: {level}' },
  'merge.confidence.high': { de: 'hoch', en: 'high', fr: 'élevée', it: 'alta' },
  'merge.confidence.medium': { de: 'mittel', en: 'medium', fr: 'moyenne', it: 'media' },
  'merge.confidence.low': { de: 'niedrig', en: 'low', fr: 'faible', it: 'bassa' },
  'merge.selectRecording': { de: 'Aufnahme auswählen', en: 'Select recording', fr: 'Sélectionner l’enregistrement', it: 'Seleziona registrazione' },
  'merge.partsBadge.one': { de: '{n} Teil', en: '{n} chunk', fr: '{n} fragment', it: '{n} frammento' },
  'merge.partsBadge.other': { de: '{n} Teile', en: '{n} chunks', fr: '{n} fragments', it: '{n} frammenti' },
  'merge.singleFile': { de: 'Einzeldatei', en: 'Single file', fr: 'Fichier unique', it: 'File singolo' },
  'merge.firstChunkFolder': { de: 'Ordner des ersten Teils', en: 'Folder of the first chunk', fr: 'Dossier du premier fragment', it: 'Cartella del primo frammento' },
  'merge.showParts': { de: 'Teile anzeigen', en: 'Show chunks', fr: 'Afficher les fragments', it: 'Mostra frammenti' },
  'merge.showFile': { de: 'Datei anzeigen', en: 'Show file', fr: 'Afficher le fichier', it: 'Mostra file' },
  'merge.selection.one': { de: '{n} Aufnahme · {size}', en: '{n} recording · {size}', fr: '{n} enregistrement · {size}', it: '{n} registrazione · {size}' },
  'merge.selection.other': { de: '{n} Aufnahmen · {size}', en: '{n} recordings · {size}', fr: '{n} enregistrements · {size}', it: '{n} registrazioni · {size}' },
});
