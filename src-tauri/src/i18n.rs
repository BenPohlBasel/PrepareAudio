//! Backend texts in German, English, French and Italian.
//!
//! The interface tells the backend its language with `set_language`; every
//! user-visible message (errors, scan warnings, skip reasons, pair notes,
//! progress texts, stage names, format labels) is looked up here. The pure
//! functions take the language explicitly (tests use those); `t`/`tf` and the
//! format helpers without `_in` use the process-wide current language.
//! Machine values the interface compares against (item kinds, reasons, stage
//! ids, status values, the cancel sentinel) are not translated.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    De,
    En,
    Fr,
    It,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::De, Lang::En, Lang::Fr, Lang::It];

    /// Accepts "de", "en", "fr", "it" and locale tags such as "en-GB" or "fr_CH".
    pub fn parse(code: &str) -> Option<Lang> {
        let c = code.trim().to_ascii_lowercase();
        match c.get(..2)? {
            "de" => Some(Lang::De),
            "en" => Some(Lang::En),
            "fr" => Some(Lang::Fr),
            "it" => Some(Lang::It),
            _ => None,
        }
        .filter(|_| c.len() == 2 || matches!(c.as_bytes()[2], b'-' | b'_'))
    }

    pub fn code(self) -> &'static str {
        ["de", "en", "fr", "it"][self as usize]
    }

    /// Decimal separator: point in English, comma otherwise.
    pub fn decimal_sep(self) -> char {
        if self == Lang::En {
            '.'
        } else {
            ','
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(Lang::De as u8);

/// Sets the language for all following messages. Unknown codes are ignored (returns false).
pub fn set(lang: &str) -> bool {
    match Lang::parse(lang) {
        Some(l) => {
            CURRENT.store(l as u8, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

pub fn current() -> Lang {
    Lang::ALL[(CURRENT.load(Ordering::Relaxed) as usize).min(3)]
}

macro_rules! messages {
    ($($name:ident => [$de:expr, $en:expr, $fr:expr, $it:expr $(,)?],)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Msg {
            $($name,)*
        }

        impl Msg {
            pub const ALL: &'static [Msg] = &[$(Msg::$name,)*];

            fn table(self) -> [&'static str; 4] {
                match self {
                    $(Msg::$name => [$de, $en, $fr, $it],)*
                }
            }
        }
    };
}

messages! {
    // ---- commands (lib.rs)
    ScanFirst => ["Bitte zuerst einen Ordner scannen.", "Please scan a folder first.", "Veuillez d'abord analyser un dossier.", "Prima scansiona una cartella."],
    NoRecordingsSelected => ["Keine Aufnahmen ausgewählt.", "No recordings selected.", "Aucun enregistrement sélectionné.", "Nessuna registrazione selezionata."],
    NoOutDir => ["Kein Zielordner angegeben.", "No destination folder given.", "Aucun dossier de destination indiqué.", "Nessuna cartella di destinazione indicata."],
    Busy => ["Es läuft bereits ein Vorgang.", "Another task is already running.", "Une opération est déjà en cours.", "È già in corso un'operazione."],
    AnalyzeTracksFirst => ["Bitte zuerst Tracks analysieren.", "Please analyse tracks first.", "Veuillez d'abord analyser des pistes.", "Prima analizza le tracce."],
    NoSectionsSelected => ["Keine Abschnitte ausgewählt.", "No sections selected.", "Aucune section sélectionnée.", "Nessuna sezione selezionata."],
    AnalyzeFilesFirst => ["Bitte zuerst Dateien analysieren.", "Please analyse files first.", "Veuillez d'abord analyser des fichiers.", "Prima analizza i file."],
    NoFilesSelected => ["Keine Dateien ausgewählt.", "No files selected.", "Aucun fichier sélectionné.", "Nessun file selezionato."],
    InvalidLink => ["Ungültiger Link", "Invalid link", "Lien non valide", "Link non valido"],
    // The interface recognises a cancelled run by these words; keep them in sync with the UI.
    Cancelled => ["Abgebrochen.", "Cancelled.", "Annulé.", "Annullato."],

    // ---- scanning and reading files
    NotWav => ["keine WAV-Datei", "not a WAV file", "pas un fichier WAV", "non è un file WAV"],
    NoFoldersOrWavs => ["Keine Ordner oder WAV-Dateien angegeben.", "No folders or WAV files given.", "Aucun dossier ni fichier WAV indiqué.", "Nessuna cartella o file WAV indicato."],
    UnreadableWith => ["nicht lesbar: {e}", "unreadable: {e}", "illisible : {e}", "non leggibile: {e}"],
    Unreadable => ["nicht lesbar", "unreadable", "illisible", "non leggibile"],
    NoAudioData => ["enthält keine Audiodaten", "contains no audio data", "ne contient aucune donnée audio", "non contiene dati audio"],
    IdenticalCopy => ["identische Kopie von {path}", "identical copy of {path}", "copie identique de {path}", "copia identica di {path}"],
    WarnAmbiguous => [
        "Zuordnung der Teile nicht eindeutig (zwei Sender mit fast gleicher Startzeit?). Bitte die Teile prüfen.",
        "Chunk assignment is ambiguous (two transmitters with almost the same start time?). Please check the chunks.",
        "Attribution des fragments ambiguë (deux émetteurs avec presque la même heure de début ?). Veuillez vérifier les fragments.",
        "Assegnazione dei frammenti non univoca (due trasmettitori con quasi la stessa ora di inizio?). Controlla i frammenti.",
    ],
    WarnRepaired => [
        "Teil {n}: Dateikopf unvollständig (Aufnahme vermutlich abgebrochen), Länge aus der Dateigröße bestimmt.",
        "Chunk {n}: incomplete file header (recording probably interrupted), length taken from the file size.",
        "Fragment {n} : en-tête incomplet (enregistrement probablement interrompu), durée déduite de la taille du fichier.",
        "Frammento {n}: intestazione incompleta (registrazione probabilmente interrotta), durata ricavata dalla dimensione del file.",
    ],
    WarnGap => [
        "Zwischen Teil {a} und {b} weichen die Zeitstempel um {gap} s ab.",
        "The timestamps of chunks {a} and {b} differ by {gap} s.",
        "Les horodatages des fragments {a} et {b} diffèrent de {gap} s.",
        "I timestamp dei frammenti {a} e {b} differiscono di {gap} s.",
    ],
    WarnFileTimes => [
        "Teil {a} und {b} sind nur über das Dateidatum und die Audio-Prüfung an der Nahtstelle verbunden. Bitte prüfen.",
        "Chunks {a} and {b} are linked only by their file dates and the audio check at the seam. Please check.",
        "Les fragments {a} et {b} ne sont reliés que par la date des fichiers et le contrôle audio au raccord. Veuillez vérifier.",
        "I frammenti {a} e {b} sono collegati solo dalla data dei file e dal controllo audio al raccordo. Controlla.",
    ],
    WarnLowConfidence => [
        "Verkettung unsicher: Die Teile passen zeitlich oder an der Nahtstelle nicht eindeutig. Bitte prüfen, bevor du sie zusammenfügst.",
        "Uncertain chain: the chunks do not clearly fit by time or at the seam. Please check them before merging.",
        "Enchaînement incertain : les fragments ne concordent pas clairement dans le temps ou au raccord. Veuillez les vérifier avant d'assembler.",
        "Concatenamento incerto: i frammenti non combaciano chiaramente nel tempo o al raccordo. Controllali prima di unirli.",
    ],
    WarnClockNotSet => [
        "Startzeit {date}: Die Uhr des Aufnahmegeräts war vermutlich nicht gestellt. Die Reihenfolge der Teile stimmt trotzdem.",
        "Start time {date}: the recorder's clock was probably not set. The order of the chunks is still right.",
        "Heure de début {date} : l'horloge de l'enregistreur n'était probablement pas réglée. L'ordre des fragments reste correct.",
        "Ora di inizio {date}: l'orologio del registratore probabilmente non era impostato. L'ordine dei frammenti è comunque corretto.",
    ],
    WarnShortWithNext => [
        "Teil {n} ist kürzer als ein voller Teil, hat aber einen Folgeteil.",
        "Chunk {n} is shorter than a full chunk but has a following chunk.",
        "Le fragment {n} est plus court qu'un fragment complet, mais il a une suite.",
        "Il frammento {n} è più corto di un frammento completo, ma ha un seguito.",
    ],
    WarnLastFull => [
        "Der letzte Teil hat volle Größe. Möglicherweise fehlt ein Folgeteil.",
        "The last chunk has full size. A following chunk may be missing.",
        "Le dernier fragment a la taille complète. Il manque peut-être une suite.",
        "L'ultimo frammento ha la dimensione piena. Forse manca un frammento successivo.",
    ],
    FileTooShort => ["Datei zu kurz", "file too short", "fichier trop court", "file troppo corto"],
    InvalidFmtChunk => ["ungültiger fmt-Chunk", "invalid fmt chunk", "bloc fmt non valide", "chunk fmt non valido"],
    DataBeforeFmt => ["data-Chunk vor fmt-Chunk", "data chunk before fmt chunk", "bloc data avant le bloc fmt", "chunk data prima del chunk fmt"],
    InvalidAudioFormat => ["ungültiges Audioformat", "invalid audio format", "format audio non valide", "formato audio non valido"],
    NoDataChunk => ["kein data-Chunk gefunden", "no data chunk found", "aucun bloc data trouvé", "nessun chunk data trovato"],

    // ---- format labels
    Mono => ["Mono", "Mono", "Mono", "Mono"],
    Stereo => ["Stereo", "Stereo", "Stéréo", "Stereo"],
    Channels => ["{n} Kanäle", "{n} channels", "{n} canaux", "{n} canali"],

    // ---- writing files (merge, sync, master)
    CannotCreateDir => [
        "Zielordner {path} kann nicht angelegt werden: {e}",
        "Cannot create the destination folder {path}: {e}",
        "Impossible de créer le dossier de destination {path} : {e}",
        "Impossibile creare la cartella di destinazione {path}: {e}",
    ],
    LowSpace => [
        "Zu wenig Speicherplatz im Zielordner: benötigt {need}, frei {free}.",
        "Not enough space in the destination folder: {need} needed, {free} free.",
        "Espace insuffisant dans le dossier de destination : {need} requis, {free} disponibles.",
        "Spazio insufficiente nella cartella di destinazione: servono {need}, liberi {free}.",
    ],
    NoFreeNameInDir => ["kein freier Dateiname im Zielordner", "no free file name in the destination folder", "aucun nom de fichier libre dans le dossier de destination", "nessun nome di file libero nella cartella di destinazione"],
    NoFreeName => ["kein freier Dateiname", "no free file name", "aucun nom de fichier libre", "nessun nome di file libero"],
    ExistsMeanwhile => ["{path} existiert inzwischen bereits", "{path} has been created in the meantime", "{path} existe désormais déjà", "{path} esiste già nel frattempo"],
    ChunkChanged => ["Teil {n} wurde seit dem Scan verändert: {path}", "Chunk {n} has changed since the scan: {path}", "Le fragment {n} a été modifié depuis l'analyse : {path}", "Il frammento {n} è cambiato dopo la scansione: {path}"],
    VerifyFailed => ["Kontrolle der geschriebenen Datei fehlgeschlagen", "Verification of the written file failed", "Échec de la vérification du fichier écrit", "Verifica del file scritto non riuscita"],

    // ---- preview player
    NoAudioOutput => ["Kein Audioausgang gefunden.", "No audio output found.", "Aucune sortie audio trouvée.", "Nessuna uscita audio trovata."],
    AudioOutputError => ["Audioausgang: {e}", "Audio output: {e}", "Sortie audio : {e}", "Uscita audio: {e}"],
    AudioOutputFormat => [
        "Audioausgang mit Format {format} wird nicht unterstützt.",
        "Audio output format {format} is not supported.",
        "Le format de sortie audio {format} n'est pas pris en charge.",
        "Il formato di uscita audio {format} non è supportato.",
    ],

    // ---- synchronise
    NoFoldersOrFiles => ["Keine Ordner oder Dateien angegeben.", "No folders or files given.", "Aucun dossier ni fichier indiqué.", "Nessuna cartella o file indicato."],
    AudioFormatUnsupported => ["Audioformat wird nicht unterstützt", "Audio format not supported", "Format audio non pris en charge", "Formato audio non supportato"],
    NoTracksOrChunks => [
        "Keine Tracks, Aufnahme-Teile oder Audiodateien gefunden (WAV, MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF, OGG).",
        "No tracks, recording chunks or audio files found (WAV, MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF, OGG).",
        "Aucune piste, aucun fragment d'enregistrement ni fichier audio trouvé (WAV, MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF, OGG).",
        "Nessuna traccia, frammento di registrazione o file audio trovato (WAV, MP3, M4A/AAC, FLAC, ALAC, AIFF, CAF, OGG).",
    ],
    PairTooLittleOverlap => ["zu wenig gemeinsame Aufnahmezeit", "too little shared recording time", "trop peu de temps d'enregistrement commun", "troppo poco tempo di registrazione in comune"],
    PairNoStableEvents => ["keine gemeinsamen Ereignisse mit stabilem Versatz", "no shared events with a stable offset", "aucun événement commun avec un décalage stable", "nessun evento comune con sfasamento stabile"],
    ProgressFindTracks => ["Suche Tracks", "Looking for tracks", "Recherche des pistes", "Ricerca delle tracce"],
    ProgressDecode => ["Dekodiere Audiodateien", "Decoding audio files", "Décodage des fichiers audio", "Decodifica dei file audio"],
    LowSpaceCache => [
        "Zu wenig Speicherplatz für dekodierte Audiodateien in {path}: benötigt {need}, frei {free}.",
        "Not enough space for decoded audio files in {path}: {need} needed, {free} free.",
        "Espace insuffisant pour les fichiers audio décodés dans {path} : {need} requis, {free} disponibles.",
        "Spazio insufficiente per i file audio decodificati in {path}: servono {need}, liberi {free}.",
    ],
    CacheWriteFailed => [
        "Dekodierte Audiodatei kann nicht in {path} gespeichert werden: {e}",
        "Cannot store the decoded audio file in {path}: {e}",
        "Impossible d'enregistrer le fichier audio décodé dans {path} : {e}",
        "Impossibile salvare il file audio decodificato in {path}: {e}",
    ],
    StreamLayoutChanged => [
        "Kanalzahl oder Abtastrate ändert sich innerhalb der Datei",
        "Channel count or sample rate changes within the file",
        "Le nombre de canaux ou la fréquence d'échantillonnage change dans le fichier",
        "Il numero di canali o la frequenza di campionamento cambia all'interno del file",
    ],
    ProgressEnvelope => ["Frequenzanalyse der Tracks", "Frequency analysis of the tracks", "Analyse fréquentielle des pistes", "Analisi in frequenza delle tracce"],
    ProgressPairs => ["Suche gemeinsame Ereignisse", "Looking for shared events", "Recherche d'événements communs", "Ricerca di eventi comuni"],
    AnalysisFailed => ["Analyse fehlgeschlagen.", "Analysis failed.", "Échec de l'analyse.", "Analisi non riuscita."],
    SaveEditFailed => ["Bearbeitung konnte nicht gespeichert werden: {e}", "Could not save the edit: {e}", "Impossible d'enregistrer les modifications : {e}", "Impossibile salvare le modifiche: {e}"],

    // ---- master
    WavFormatUnsupported => ["WAV-Format wird nicht unterstützt", "WAV format not supported", "Format WAV non pris en charge", "Formato WAV non supportato"],
    ReadError => ["Lesefehler: {e}", "Read error: {e}", "Erreur de lecture : {e}", "Errore di lettura: {e}"],
    FormatUnsupported => ["Format wird nicht unterstützt", "Format not supported", "Format non pris en charge", "Formato non supportato"],
    NoAudioTrack => ["keine Audiospur", "no audio track", "aucune piste audio", "nessuna traccia audio"],
    CodecUnsupported => ["Codec wird nicht unterstützt", "Codec not supported", "Codec non pris en charge", "Codec non supportato"],
    DecodeError => ["Dekodierfehler: {e}", "Decoding error: {e}", "Erreur de décodage : {e}", "Errore di decodifica: {e}"],
    LoudnessMeasurement => ["Lautheitsmessung: {e}", "Loudness measurement: {e}", "Mesure de loudness : {e}", "Misura della loudness: {e}"],
    RateUnsupportedMp3 => [
        "Abtastrate {rate} Hz wird für MP3 nicht unterstützt",
        "Sample rate {rate} Hz is not supported for MP3",
        "Fréquence d'échantillonnage {rate} Hz non prise en charge pour le MP3",
        "Frequenza di campionamento {rate} Hz non supportata per MP3",
    ],
    ProgressFindAudio => ["Suche Audiodateien", "Looking for audio files", "Recherche des fichiers audio", "Ricerca dei file audio"],
    ProgressMeasure => ["Lautheit messen (EBU R128)", "Measuring loudness (EBU R128)", "Mesure de la loudness (EBU R128)", "Misura della loudness (EBU R128)"],
    NotSupportedAudioFile => ["keine unterstützte Audiodatei", "not a supported audio file", "fichier audio non pris en charge", "file audio non supportato"],
    NoAudioFilesFound => [
        "Keine Audiodateien gefunden (WAV, MP3, M4A, AAC, FLAC, AIFF, CAF, OGG).",
        "No audio files found (WAV, MP3, M4A, AAC, FLAC, AIFF, CAF, OGG).",
        "Aucun fichier audio trouvé (WAV, MP3, M4A, AAC, FLAC, AIFF, CAF, OGG).",
        "Nessun file audio trovato (WAV, MP3, M4A, AAC, FLAC, AIFF, CAF, OGG).",
    ],
    NoteVeryQuiet => ["sehr leise, Verstärkung auf +{max} dB begrenzt", "very quiet, gain limited to +{max} dB", "très faible, gain limité à +{max} dB", "molto basso, guadagno limitato a +{max} dB"],
    NoteSilence => ["Stille, keine Lautheit messbar", "silence, no measurable loudness", "silence, loudness non mesurable", "silenzio, loudness non misurabile"],
    NoReadableAudio => ["Keine lesbaren Audiodateien gefunden.", "No readable audio files found.", "Aucun fichier audio lisible trouvé.", "Nessun file audio leggibile trovato."],
    EncoderSetting => ["MP3-Encoder: Einstellung nicht möglich", "MP3 encoder: setting not possible", "Encodeur MP3 : réglage impossible", "Encoder MP3: impostazione non possibile"],
    EncoderUnavailable => ["MP3-Encoder nicht verfügbar", "MP3 encoder not available", "Encodeur MP3 indisponible", "Encoder MP3 non disponibile"],
    EncodingFailed => ["MP3-Kodierung fehlgeschlagen", "MP3 encoding failed", "Échec de l'encodage MP3", "Codifica MP3 non riuscita"],
    StageLevel => ["Pegel einstellen", "Setting level", "Réglage du niveau", "Regolazione del livello"],
    StageEncode => ["MP3 kodieren", "Encoding MP3", "Encodage MP3", "Codifica MP3"],
    StageCheck => ["Nachmessen", "Measuring again", "Nouvelle mesure", "Misura di controllo"],
    NoMeasurableLoudness => ["keine messbare Lautheit", "no measurable loudness", "loudness non mesurable", "loudness non misurabile"],
    NoMeasurement => ["keine Messung", "no measurement", "aucune mesure", "nessuna misura"],
    StoppedInternal => ["abgebrochen (interner Fehler)", "stopped (internal error)", "interrompu (erreur interne)", "interrotto (errore interno)"],
}

/// The text of `msg` in `lang` (placeholders left in place).
pub fn text(lang: Lang, msg: Msg) -> &'static str {
    msg.table()[lang as usize]
}

/// The text of `msg` in `lang` with every `{name}` replaced by its value.
pub fn format(lang: Lang, msg: Msg, args: &[(&str, &dyn Display)]) -> String {
    let mut s = text(lang, msg).to_string();
    for (name, value) in args {
        s = s.replace(&format!("{{{name}}}"), &value.to_string());
    }
    s
}

/// `text` in the current language.
pub fn t(msg: Msg) -> &'static str {
    text(current(), msg)
}

/// `format` in the current language.
pub fn tf(msg: Msg, args: &[(&str, &dyn Display)]) -> String {
    format(current(), msg, args)
}

/// The cancel error in the current language.
pub fn cancelled() -> String {
    t(Msg::Cancelled).to_string()
}

/// Whether an error is the cancel error, in whatever language it was created.
pub fn is_cancelled(err: &str) -> bool {
    Lang::ALL.iter().any(|&l| err == text(l, Msg::Cancelled))
}

/// Replaces the decimal point of a formatted number with the language's separator.
pub fn decimal(lang: Lang, number: &str) -> String {
    number.replace('.', &lang.decimal_sep().to_string())
}

/// "Mono", "Stereo" or "n channels".
pub fn channels(lang: Lang, n: u32) -> String {
    match n {
        1 => text(lang, Msg::Mono).to_string(),
        2 => text(lang, Msg::Stereo).to_string(),
        n => format(lang, Msg::Channels, &[("n", &n)]),
    }
}

/// "48 kHz", "44,1 kHz" (en "44.1 kHz").
pub fn khz(lang: Lang, rate: u32) -> String {
    let k = rate as f64 / 1000.0;
    if k.fract() == 0.0 {
        format!("{} kHz", k as u64)
    } else {
        format!("{} kHz", decimal(lang, &format!("{k:.1}")))
    }
}

/// Byte count with binary units ("1,5 GB"; French "1,5 Go").
pub fn bytes(lang: Lang, b: u64) -> String {
    let units: [&str; 5] = if lang == Lang::Fr { ["o", "Ko", "Mo", "Go", "To"] } else { ["B", "KB", "MB", "GB", "TB"] };
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i >= 2 {
        format!("{} {}", decimal(lang, &format!("{v:.1}")), units[i])
    } else {
        format!("{v:.0} {}", units[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(s: &str) -> Vec<String> {
        let mut out: Vec<String> = s
            .split('{')
            .skip(1)
            .filter_map(|rest| rest.split_once('}').map(|(name, _)| name.to_string()))
            .collect();
        out.sort();
        out
    }

    #[test]
    fn every_message_exists_in_all_languages_with_the_same_placeholders() {
        for &m in Msg::ALL {
            let de = placeholders(text(Lang::De, m));
            for l in Lang::ALL {
                let s = text(l, m);
                assert!(!s.trim().is_empty(), "{m:?} empty in {}", l.code());
                assert_eq!(placeholders(s), de, "{m:?} placeholders in {}", l.code());
                assert!(!s.contains("DJI"), "{m:?} names a brand in {}", l.code());
            }
        }
    }

    #[test]
    fn parses_language_codes() {
        assert_eq!(Lang::parse("de"), Some(Lang::De));
        assert_eq!(Lang::parse("EN"), Some(Lang::En));
        assert_eq!(Lang::parse("fr-CH"), Some(Lang::Fr));
        assert_eq!(Lang::parse("it_IT"), Some(Lang::It));
        assert_eq!(Lang::parse("es"), None);
        assert_eq!(Lang::parse("deu"), None);
        assert_eq!(Lang::parse(""), None);
        for l in Lang::ALL {
            assert_eq!(Lang::parse(l.code()), Some(l));
        }
    }

    #[test]
    fn formats_numbers_and_labels_per_language() {
        assert_eq!(khz(Lang::De, 44_100), "44,1 kHz");
        assert_eq!(khz(Lang::En, 44_100), "44.1 kHz");
        assert_eq!(khz(Lang::Fr, 88_200), "88,2 kHz");
        assert_eq!(khz(Lang::It, 48_000), "48 kHz");
        assert_eq!(channels(Lang::De, 6), "6 Kanäle");
        assert_eq!(channels(Lang::En, 6), "6 channels");
        assert_eq!(channels(Lang::Fr, 2), "Stéréo");
        assert_eq!(channels(Lang::Fr, 4), "4 canaux");
        assert_eq!(channels(Lang::It, 3), "3 canali");
        assert_eq!(channels(Lang::It, 1), "Mono");
        assert_eq!(bytes(Lang::De, 3 << 29), "1,5 GB");
        assert_eq!(bytes(Lang::En, 3 << 29), "1.5 GB");
        assert_eq!(bytes(Lang::Fr, 3 << 29), "1,5 Go");
        assert_eq!(bytes(Lang::It, 512), "512 B");
        assert_eq!(
            format(Lang::En, Msg::LowSpace, &[("need", &"2.0 GB"), ("free", &"1.0 GB")]),
            "Not enough space in the destination folder: 2.0 GB needed, 1.0 GB free."
        );
        assert_eq!(format(Lang::It, Msg::WarnGap, &[("a", &1), ("b", &2), ("gap", &"+3,5")]), "I timestamp dei frammenti 1 e 2 differiscono di +3,5 s.");
    }
}
