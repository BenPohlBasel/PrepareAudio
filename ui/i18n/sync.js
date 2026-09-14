'use strict';

/* Dictionary: sync. Keys sync.* — de, en, fr, it are all required. See docs/I18N.md. */
I18N.add({
  // timeline canvas
  'sync.clip.deleted': { de: 'gelöscht', en: 'deleted', fr: 'supprimé', it: 'eliminato' },
  'sync.row.stereo': { de: 'Stereo', en: 'Stereo', fr: 'Stéréo', it: 'Stereo' },
  'sync.row.mono': { de: 'Mono', en: 'Mono', fr: 'Mono', it: 'Mono' },
  'sync.canvas.sender': { de: 'Sender {label}', en: 'Transmitter {label}', fr: 'Émetteur {label}', it: 'Trasmettitore {label}' },
  'sync.canvas.events': { de: 'Ereignisse', en: 'Events', fr: 'Événements', it: 'Eventi' },

  // context menu and toolbar state
  'sync.menu.split': { de: 'Am Playhead trennen', en: 'Split at playhead', fr: 'Couper à la tête de lecture', it: 'Dividi alla testina' },
  'sync.menu.join': { de: 'Zusammenführen', en: 'Join', fr: 'Joindre', it: 'Congiungi' },
  'sync.menu.toStereo': { de: 'Auf Stereo schieben', en: 'Move to stereo', fr: 'Passer en stéréo', it: 'Sposta su stereo' },
  'sync.menu.toMono': { de: 'Auf Mono schieben', en: 'Move to mono', fr: 'Passer en mono', it: 'Sposta su mono' },
  'sync.menu.pause': { de: 'Anhalten', en: 'Pause', fr: 'Pause', it: 'Pausa' },
  'sync.menu.playHere': { de: 'Ab hier abspielen', en: 'Play from here', fr: 'Lire à partir d’ici', it: 'Riproduci da qui' },
  'sync.op.delete': { de: 'Löschen', en: 'Delete', fr: 'Supprimer', it: 'Elimina' },
  'sync.op.restore': { de: 'Wiederbringen', en: 'Restore', fr: 'Rétablir', it: 'Ripristina' },
  'sync.kbd.delete': { de: 'Entf', en: '⌫', fr: '⌫', it: '⌫' },

  // toasts and dialogs
  'sync.toast.noSplit': { de: 'Am Playhead liegt kein Clip zum Trennen.', en: 'No clip to split at the playhead.', fr: 'Aucun clip à couper à la tête de lecture.', it: 'Nessuna clip da dividere alla testina.' },
  'sync.toast.noJoin': { de: 'Kein folgender Clip auf dieser Spur.', en: 'No following clip on this track.', fr: 'Aucun clip suivant sur cette piste.', it: 'Nessuna clip successiva su questa traccia.' },
  'sync.toast.stereoNeedsTwo': { de: 'Stereo braucht zwei Sender.', en: 'Stereo needs two transmitters.', fr: 'La stéréo nécessite deux émetteurs.', it: 'Lo stereo richiede due trasmettitori.' },
  'sync.toast.oneSender': { de: 'Nur ein Sender gefunden, alles bleibt Mono.', en: 'Only one transmitter found, everything stays mono.', fr: 'Un seul émetteur trouvé, tout reste en mono.', it: 'Trovato un solo trasmettitore, tutto resta mono.' },
  'sync.toast.editRestored': { de: 'Frühere Bearbeitung dieser Tracks wiederhergestellt.', en: 'Earlier edits of these tracks restored.', fr: 'Modifications précédentes de ces pistes rétablies.', it: 'Modifiche precedenti di queste tracce ripristinate.' },
  'sync.toast.nothingToCreate': { de: 'Nichts zu erzeugen: alle Clips sind gelöscht.', en: 'Nothing to create: all clips are deleted.', fr: 'Rien à créer : tous les clips sont supprimés.', it: 'Niente da creare: tutte le clip sono eliminate.' },
  'sync.pickTitle': { de: 'Ordner mit Tracks wählen', en: 'Choose a folder with tracks', fr: 'Choisir un dossier de pistes', it: 'Scegli una cartella con tracce' },
  'sync.outputTitle': { de: 'Wo sollen die synchronisierten Dateien gespeichert werden?', en: 'Where should the synchronised files be saved?', fr: 'Où enregistrer les fichiers synchronisés ?', it: 'Dove salvare i file sincronizzati?' },
  // Matched against backend errors in all four languages to recognise a cancelled analysis.
  'sync.cancelledError': { de: 'Abgebrochen', en: 'Cancelled', fr: 'Annulé', it: 'Annullato' },

  // progress
  'sync.searching': { de: 'Suche Tracks', en: 'Looking for tracks', fr: 'Recherche des pistes', it: 'Ricerca delle tracce' },
  'sync.preparing': { de: 'Vorbereiten…', en: 'Preparing…', fr: 'Préparation…', it: 'Preparazione…' },
  'sync.cancelling': { de: 'Breche ab…', en: 'Cancelling…', fr: 'Annulation…', it: 'Annullamento…' },
  'sync.progress.bytes': { de: '{done} von {total}', en: '{done} of {total}', fr: '{done} sur {total}', it: '{done} di {total}' },
  'sync.progress.pairs.one': { de: '{done} von {n} Paar', en: '{done} of {n} pair', fr: '{done} sur {n} paire', it: '{done} di {n} coppia' },
  'sync.progress.pairs.other': { de: '{done} von {n} Paaren', en: '{done} of {n} pairs', fr: '{done} sur {n} paires', it: '{done} di {n} coppie' },
  'sync.progress.write': { de: '{index} von {count} · {name} · {pct} %', en: '{index} of {count} · {name} · {pct} %', fr: '{index} sur {count} · {name} · {pct} %', it: '{index} di {count} · {name} · {pct} %' },

  // finished and partial run
  'sync.noun.file.one': { de: 'Datei', en: 'file', fr: 'fichier', it: 'file' },
  'sync.noun.file.other': { de: 'Dateien', en: 'files', fr: 'fichiers', it: 'file' },
  'sync.finished.open': { de: 'sync öffnen', en: 'Open sync', fr: 'Ouvrir sync', it: 'Apri sync' },
  'sync.finished.next': { de: 'Weiter: mastern', en: 'Next: master', fr: 'Suivant : masteriser', it: 'Avanti: masterizza' },
  'sync.finished.back': { de: 'Zurück zur Bearbeitung', en: 'Back to editing', fr: 'Retour à l’édition', it: 'Torna alla modifica' },
  'sync.partial.one': { de: '{done} von {total} erledigt. {n} Datei nicht, siehe Liste.', en: '{done} of {total} done. {n} file failed, see list.', fr: '{done} sur {total} terminés. {n} fichier en échec, voir la liste.', it: '{done} di {total} completati. {n} file non riuscito, vedi elenco.' },
  'sync.partial.other': { de: '{done} von {total} erledigt. {n} Dateien nicht, siehe Liste.', en: '{done} of {total} done. {n} files failed, see list.', fr: '{done} sur {total} terminés. {n} fichiers en échec, voir la liste.', it: '{done} di {total} completati. {n} file non riusciti, vedi elenco.' },

  // legend
  'sync.legend.sender': { de: 'Sender {label}: {role}', en: 'Transmitter {label}: {role}', fr: 'Émetteur {label} : {role}', it: 'Trasmettitore {label}: {role}' },
  'sync.legend.left': { de: 'links', en: 'left', fr: 'gauche', it: 'sinistra' },
  'sync.legend.right': { de: 'rechts', en: 'right', fr: 'droite', it: 'destra' },
  'sync.legend.monoOnly': { de: 'nur Mono', en: 'mono only', fr: 'mono uniquement', it: 'solo mono' },
  'sync.legend.mono': { de: 'heller: Mono, nach außen geschoben', en: 'lighter: mono, moved outwards', fr: 'plus clair : mono, déplacé vers l’extérieur', it: 'più chiaro: mono, spostato verso l’esterno' },
  'sync.legend.cut': { de: 'gelöscht oder weggeschnitten', en: 'deleted or cut away', fr: 'supprimé ou coupé', it: 'eliminato o tagliato' },
  'sync.legend.events': { de: 'gemeinsame Ereignisse', en: 'shared events', fr: 'événements communs', it: 'eventi comuni' },

  // summary
  'sync.stat.stereo.one': { de: 'Stereo-Datei · {dur}', en: 'Stereo file · {dur}', fr: 'Fichier stéréo · {dur}', it: 'File stereo · {dur}' },
  'sync.stat.stereo.other': { de: 'Stereo-Dateien · {dur}', en: 'Stereo files · {dur}', fr: 'Fichiers stéréo · {dur}', it: 'File stereo · {dur}' },
  'sync.stat.mono.one': { de: 'Mono-Datei', en: 'Mono file', fr: 'Fichier mono', it: 'File mono' },
  'sync.stat.mono.other': { de: 'Mono-Dateien', en: 'Mono files', fr: 'Fichiers mono', it: 'File mono' },
  'sync.stat.pairs': { de: 'Track-Paare synchron', en: 'Track pairs in sync', fr: 'Paires de pistes synchrones', it: 'Coppie di tracce sincrone' },
  'sync.stat.tracks.one': { de: 'Tracks von {n} Sender{source}', en: 'Tracks from {n} transmitter{source}', fr: 'Pistes de {n} émetteur{source}', it: 'Tracce da {n} trasmettitore{source}' },
  'sync.stat.tracks.other': { de: 'Tracks von {n} Sendern{source}', en: 'Tracks from {n} transmitters{source}', fr: 'Pistes de {n} émetteurs{source}', it: 'Tracce da {n} trasmettitori{source}' },
  'sync.stat.fromChunks': { de: ', aus Aufnahme-Teilen', en: ', from recording chunks', fr: ', issues de fragments d’enregistrement', it: ', da frammenti di registrazione' },
  'sync.stat.partlyFromChunks': { de: ', teils aus Aufnahme-Teilen', en: ', partly from recording chunks', fr: ', en partie issues de fragments d’enregistrement', it: ', in parte da frammenti di registrazione' },
  'sync.analysed': { de: 'Analysiert: {roots}', en: 'Analysed: {roots}', fr: 'Analysé : {roots}', it: 'Analizzato: {roots}' },

  // track pairs
  'sync.pairs.none': { de: 'Keine Tracks verschiedener Sender überlappen zeitlich.', en: 'No tracks from different transmitters overlap in time.', fr: 'Aucune piste d’émetteurs différents ne se chevauche dans le temps.', it: 'Nessuna traccia di trasmettitori diversi si sovrappone nel tempo.' },
  'sync.pair.with': { de: 'mit', en: 'with', fr: 'avec', it: 'con' },
  'sync.pair.inSync': { de: 'synchron', en: 'in sync', fr: 'synchrone', it: 'sincrona' },
  'sync.pair.notInSync': { de: 'nicht synchron', en: 'not in sync', fr: 'non synchrone', it: 'non sincrona' },
  'sync.pair.facts': {
    de: 'Versatz {offset} s · Drift {drift} ppm · Streuung {spread} ms · Treffer in {hits} % der Fenster',
    en: 'Offset {offset} s · Drift {drift} ppm · Spread {spread} ms · Hits in {hits} % of windows',
    fr: 'Décalage {offset} s · Dérive {drift} ppm · Dispersion {spread} ms · Concordances dans {hits} % des fenêtres',
    it: 'Sfasamento {offset} s · Deriva {drift} ppm · Dispersione {spread} ms · Corrispondenze nel {hits} % delle finestre',
  },

  // output list
  'sync.badge.stereo': { de: 'Stereo · L {left} · R {right}', en: 'Stereo · L {left} · R {right}', fr: 'Stéréo · L {left} · R {right}', it: 'Stereo · L {left} · R {right}' },
  'sync.badge.mono': { de: 'Mono · {label}', en: 'Mono · {label}', fr: 'Mono · {label}', it: 'Mono · {label}' },
  'sync.item.why': { de: 'Treffer {hits} · Kohärenz {coh}', en: 'Hits {hits} · Coherence {coh}', fr: 'Concordances {hits} · Cohérence {coh}', it: 'Corrispondenze {hits} · Coerenza {coh}' },
  'sync.item.missing': { de: 'Sender {label} fehlt {dur}', en: 'Transmitter {label} missing for {dur}', fr: 'Émetteur {label} absent pendant {dur}', it: 'Trasmettitore {label} assente per {dur}' },
  'sync.item.apart': { de: 'anderes Geschehen als der zweite Sender', en: 'different situation from the second transmitter', fr: 'situation différente de celle du second émetteur', it: 'situazione diversa dal secondo trasmettitore' },
  'sync.item.alone': { de: 'kein zweiter Sender zu dieser Zeit', en: 'no second transmitter at this time', fr: 'pas de second émetteur à ce moment', it: 'nessun secondo trasmettitore in questo momento' },
  'sync.item.decoded': { de: 'aus {format} dekodiert', en: 'decoded from {format}', fr: 'décodé depuis {format}', it: 'decodificato da {format}' },
  'sync.item.reveal': { de: 'Im Finder zeigen', en: 'Show in Finder', fr: 'Afficher dans le Finder', it: 'Mostra nel Finder' },
  'sync.item.show': { de: 'In der Timeline zeigen', en: 'Show in timeline', fr: 'Afficher dans la timeline', it: 'Mostra nella timeline' },
  'sync.item.writing': { de: 'wird geschrieben…', en: 'writing…', fr: 'écriture…', it: 'scrittura…' },
  'sync.status.written': { de: 'Fertig', en: 'Done', fr: 'Terminé', it: 'Fatto' },
  'sync.status.existing': { de: 'Schon vorhanden', en: 'Already exists', fr: 'Déjà présent', it: 'Già presente' },
  'sync.status.failed': { de: 'Fehler', en: 'Error', fr: 'Erreur', it: 'Errore' },
  'sync.status.cancelled': { de: 'Abgebrochen', en: 'Cancelled', fr: 'Annulé', it: 'Annullato' },
  'sync.list.empty': { de: 'Nichts auszugeben: alle Clips sind gelöscht.', en: 'Nothing to output: all clips are deleted.', fr: 'Rien à produire : tous les clips sont supprimés.', it: 'Niente da produrre: tutte le clip sono eliminate.' },
  'sync.editState.edited': { de: 'bearbeitet · wird neben den Quellen gespeichert', en: 'edited · saved next to the sources', fr: 'modifié · enregistré à côté des sources', it: 'modificato · salvato accanto alle sorgenti' },
  'sync.editState.proposal': { de: 'Vorschlag der Analyse', en: 'Analysis proposal', fr: 'Proposition de l’analyse', it: 'Proposta dell’analisi' },
  'sync.unused.one': { de: '{n} Datei nicht verwendet', en: '{n} file not used', fr: '{n} fichier non utilisé', it: '{n} file non usato' },
  'sync.unused.other': { de: '{n} Dateien nicht verwendet', en: '{n} files not used', fr: '{n} fichiers non utilisés', it: '{n} file non usati' },
  'sync.files.one': { de: '{n} Datei · {size}', en: '{n} file · {size}', fr: '{n} fichier · {size}', it: '{n} file · {size}' },
  'sync.files.other': { de: '{n} Dateien · {size}', en: '{n} files · {size}', fr: '{n} fichiers · {size}', it: '{n} file · {size}' },
  'sync.nothingToCreate': { de: 'nichts zu erzeugen', en: 'nothing to create', fr: 'rien à créer', it: 'niente da creare' },
});
