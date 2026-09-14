'use strict';

/* Dictionary: info. Keys info.* — de, en, fr, it are all required. See docs/I18N.md.
   The institute name stays German in every language. */
I18N.add({
  'info.version': {
    de: 'Version {version} · B/IAS · Lizenz {license}',
    en: 'Version {version} · B/IAS · Licence {license}',
    fr: 'Version {version} · B/IAS · Licence {license}',
    it: 'Versione {version} · B/IAS · Licenza {license}',
  },
  'info.about': {
    de: 'PrepareAudio ist eine App von B/IAS – Basel Institut für angewandte Stadtforschung.',
    en: 'PrepareAudio is made by B/IAS – Basel Institut für angewandte Stadtforschung.',
    fr: 'PrepareAudio est développée par le B/IAS – Basel Institut für angewandte Stadtforschung.',
    it: 'PrepareAudio è sviluppata dal B/IAS – Basel Institut für angewandte Stadtforschung.',
  },
  'info.link.site': {
    de: 'bias.city/prepareaudio',
    en: 'bias.city/prepareaudio',
    fr: 'bias.city/prepareaudio',
    it: 'bias.city/prepareaudio',
  },
  'info.link.source': {
    de: 'Quellcode auf GitHub',
    en: 'Source code on GitHub',
    fr: 'Code source sur GitHub',
    it: 'Codice sorgente su GitHub',
  },
  'info.lang.heading': {
    de: 'Sprache',
    en: 'Language',
    fr: 'Langue',
    it: 'Lingua',
  },
  'info.license': {
    de: 'PrepareAudio ist freie Software unter der GNU Affero General Public License, Version 3 oder später ({license}). Copyright © 2026 B/IAS – Basel Institut für angewandte Stadtforschung. Der Quellcode steht auf GitHub.',
    en: 'PrepareAudio is free software under the GNU Affero General Public License, version 3 or later ({license}). Copyright © 2026 B/IAS – Basel Institut für angewandte Stadtforschung. The source code is on GitHub.',
    fr: 'PrepareAudio est un logiciel libre sous GNU Affero General Public License, version 3 ou ultérieure ({license}). Copyright © 2026 B/IAS – Basel Institut für angewandte Stadtforschung. Le code source est disponible sur GitHub.',
    it: 'PrepareAudio è software libero sotto GNU Affero General Public License, versione 3 o successiva ({license}). Copyright © 2026 B/IAS – Basel Institut für angewandte Stadtforschung. Il codice sorgente è su GitHub.',
  },
  'info.loadError': {
    de: 'Lizenzangaben konnten nicht geladen werden: {error}',
    en: 'Licence information could not be loaded: {error}',
    fr: 'Impossible de charger les informations de licence : {error}',
    it: 'Impossibile caricare le informazioni sulle licenze: {error}',
  },
  'info.thirdIntro.one': {
    de: 'Die App enthält 1 Softwarepaket Dritter. Seine Lizenz ist mit der AGPL vereinbar; der Lizenztext steht unten und liegt als THIRD_PARTY_LICENSES.md im App-Paket. Die macOS-WebView stellt das System.',
    en: 'The app includes 1 third-party software package. Its licence is compatible with the AGPL; the licence text is below and ships as THIRD_PARTY_LICENSES.md inside the app bundle. The macOS WebView is provided by the system.',
    fr: 'L’app contient 1 paquet logiciel tiers. Sa licence est compatible avec l’AGPL ; le texte de licence figure ci-dessous et se trouve dans le paquet de l’app sous THIRD_PARTY_LICENSES.md. La WebView de macOS est fournie par le système.',
    it: 'L’app contiene 1 pacchetto software di terze parti. La sua licenza è compatibile con l’AGPL; il testo della licenza è qui sotto e si trova nel pacchetto dell’app come THIRD_PARTY_LICENSES.md. La WebView di macOS è fornita dal sistema.',
  },
  'info.thirdIntro.other': {
    de: 'Die App enthält {n} Softwarepakete Dritter. Alle ihre Lizenzen sind mit der AGPL vereinbar; die Lizenztexte stehen unten und liegen als THIRD_PARTY_LICENSES.md im App-Paket. Die macOS-WebView stellt das System.',
    en: 'The app includes {n} third-party software packages. All their licences are compatible with the AGPL; the licence texts are below and ship as THIRD_PARTY_LICENSES.md inside the app bundle. The macOS WebView is provided by the system.',
    fr: 'L’app contient {n} paquets logiciels tiers. Toutes leurs licences sont compatibles avec l’AGPL ; les textes de licence figurent ci-dessous et se trouvent dans le paquet de l’app sous THIRD_PARTY_LICENSES.md. La WebView de macOS est fournie par le système.',
    it: 'L’app contiene {n} pacchetti software di terze parti. Tutte le loro licenze sono compatibili con l’AGPL; i testi delle licenze sono qui sotto e si trovano nel pacchetto dell’app come THIRD_PARTY_LICENSES.md. La WebView di macOS è fornita dal sistema.',
  },
  'info.noHits': {
    de: 'Keine Treffer.',
    en: 'No matches.',
    fr: 'Aucun résultat.',
    it: 'Nessun risultato.',
  },
  'info.textsSummary.one': {
    de: 'Lizenztext (1)',
    en: 'Licence text (1)',
    fr: 'Texte de licence (1)',
    it: 'Testo di licenza (1)',
  },
  'info.textsSummary.other': {
    de: 'Alle Lizenztexte ({n} verschiedene)',
    en: 'All licence texts ({n} different)',
    fr: 'Tous les textes de licence ({n} différents)',
    it: 'Tutti i testi di licenza ({n} diversi)',
  },
  'info.textHeading': {
    de: 'Lizenztext {id}',
    en: 'Licence text {id}',
    fr: 'Texte de licence {id}',
    it: 'Testo di licenza {id}',
  },
  'info.appliesTo': {
    de: 'gilt für {crates}',
    en: 'applies to {crates}',
    fr: 's’applique à {crates}',
    it: 'vale per {crates}',
  },
  // Notes from licenses.json (generated in German) for the two packages the panel highlights.
  'info.note.mp3lame-sys': {
    de: 'Enthält die Quellen von LAME 3.100 (GNU LGPL), die beim Bauen statisch in die App übersetzt werden. Quellen: https://lame.sourceforge.io und das Paket selbst; die App lässt sich aus ihrem Quelltext mit einer geänderten LAME-Version neu bauen.',
    en: 'Contains the sources of LAME 3.100 (GNU LGPL), which are compiled statically into the app during the build. Sources: https://lame.sourceforge.io and the package itself; the app can be rebuilt from its source code with a modified LAME version.',
    fr: 'Contient les sources de LAME 3.100 (GNU LGPL), compilées statiquement dans l’app lors de la construction. Sources : https://lame.sourceforge.io et le paquet lui-même ; l’app peut être recompilée à partir de son code source avec une version modifiée de LAME.',
    it: 'Contiene i sorgenti di LAME 3.100 (GNU LGPL), compilati staticamente nell’app durante la build. Sorgenti: https://lame.sourceforge.io e il pacchetto stesso; l’app può essere ricompilata dal suo codice sorgente con una versione modificata di LAME.',
  },
  'info.note.symphonia': {
    de: 'Das Paket enthält keine eigene Lizenzdatei; aufgeführt ist der Standardtext der angegebenen Lizenz. Quellcode (MPL-2.0): https://github.com/pdeljanov/Symphonia',
    en: 'The package contains no licence file of its own; the standard text of the stated licence is listed. Source code (MPL-2.0): https://github.com/pdeljanov/Symphonia',
    fr: 'Le paquet ne contient pas de fichier de licence propre ; le texte standard de la licence indiquée est reproduit. Code source (MPL-2.0) : https://github.com/pdeljanov/Symphonia',
    it: 'Il pacchetto non contiene un proprio file di licenza; è riportato il testo standard della licenza indicata. Codice sorgente (MPL-2.0): https://github.com/pdeljanov/Symphonia',
  },
});
