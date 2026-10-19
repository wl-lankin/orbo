# Konvertierungs-Katalog

Legende Priorität: **MVP** = Phase 1/2 · **V1** = bis 1.0 · **V2** = danach / optionales Modul
⚡ = geht oft ohne Neukodierung (nur Umverpacken), dauert Sekunden statt Minuten.

Die Spalte „Ring-Vorschlag" zeigt, was der Ring typischerweise anbietet. Der volle Katalog ist nur über „Mehr …" erreichbar.

---

## 1. Bilder

**Lesen:** PNG, JPG/JPEG, WEBP, AVIF, HEIC/HEIF, GIF (auch animiert), BMP, TIFF (mehrseitig), ICO, ICNS, TGA, JPEG XL, QOI, PSD (zusammengeführt), SVG, RAW (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2), EXR/HDR (V2), PDF-Seite

| Von | Nach | Engine | Prio | Hinweis |
|---|---|---|---|---|
| HEIC/HEIF | JPG, PNG, WEBP | libheif / macOS ImageIO / Windows WIC | MVP | Häufigster Wunsch überhaupt (iPhone-Fotos) |
| PNG | JPG, WEBP, AVIF | libvips | MVP | Bei Transparenz: JPG mit Hintergrundfarbe, Hinweis im Ring |
| JPG | PNG, WEBP, AVIF | libvips | MVP | |
| WEBP / AVIF | JPG, PNG | libvips | MVP | „Bild aus Browser lässt sich nicht öffnen"-Fall |
| beliebig | JPEG XL | libjxl | V1 | |
| beliebig | HEIC | macOS ImageIO | V1 | Windows nur mit installierter HEVC-Erweiterung |
| beliebig | TIFF, BMP, TGA, QOI | libvips | V1 | |
| PNG/SVG | ICO (mehrere Größen) | eigen + libvips | V1 | 16 bis 256 px in einer Datei |
| PNG/SVG | ICNS / App-Icon-Set | eigen | V1 | Alle Größen für macOS/iOS/Android |
| PNG/SVG | Favicon-Paket | eigen | V1 | favicon.ico, apple-touch-icon, manifest-Icons + HTML-Schnipsel |
| SVG | PNG (1×/2×/3×) | resvg | MVP | |
| PNG/JPG | SVG (Vektorisieren) | vtracer (MIT) | V1 | Logos, Skizzen. Nicht potrace (GPL) |
| RAW | JPG, TIFF, DNG | LibRaw | V1 | |
| PSD | PNG, JPG | libvips / psd-Crate | V1 | Nur zusammengeführtes Bild |
| GIF (animiert) | MP4, WEBM, animiertes WEBP | FFmpeg | MVP | 90 % kleiner |
| Bilder (mehrere) | PDF | pdfium / eigen | MVP | Reihenfolge = Auswahl |
| Bilder (mehrere) | GIF / MP4 (Diashow) | FFmpeg | V1 | |
| Bilder (mehrere) | Kontaktabzug (ein Bild) | libvips | V2 | |

**Aktionen**

| Aktion | Engine | Prio |
|---|---|---|
| Verkleinern (50 %, 1920 px, 1080 px, eigene) | libvips | MVP |
| Komprimieren auf Zielgröße (< 500 KB, < 1 MB …) | libvips + Suche | MVP |
| Metadaten / GPS entfernen | libvips / exiftool-Ersatz in Rust | MVP |
| Nach EXIF drehen | libvips | MVP |
| Zuschneiden auf Seitenverhältnis (1:1, 4:5, 16:9, 9:16) mit Motiverkennung | libvips „attention" | V1 |
| In sRGB umwandeln | libvips + lcms | V1 |
| Text erkennen (OCR) → TXT / Zwischenablage | Apple Vision / Windows.Media.Ocr / Tesseract | V1 |
| Hintergrund entfernen → PNG | ONNX Runtime + BiRefNet/U²-Net | V2 (Modul Bild-KI) |
| Hochskalieren 2×/4× | Real-ESRGAN (ncnn) | V2 (Modul Bild-KI) |
| Wasserzeichen / Rahmen | libvips | V2 |
| Als Base64/Data-URI in Zwischenablage | eigen | V1 |

---

## 2. Video

**Lesen:** MP4, MOV, M4V, MKV, AVI, WMV, WEBM, FLV, MPG/MPEG, VOB, TS/MTS/M2TS (AVCHD), 3GP, OGV, GIF

| Von | Nach | Engine | Prio | Hinweis |
|---|---|---|---|---|
| MOV, MKV, AVI, WMV, FLV … | MP4 (H.264) | FFmpeg + Hardware-Encoder | MVP | „Überall abspielbar". ⚡ wenn schon H.264/AAC |
| beliebig | MP4 (HEVC) | Hardware-Encoder | MVP | Halbe Größe bei gleicher Qualität |
| beliebig | MP4 (AV1) | SVT-AV1 (BSD) | V1 | Sehr klein, langsamer |
| beliebig | WEBM (VP9/AV1) | libvpx / SVT-AV1 | V1 | Für Webseiten |
| beliebig | MOV (ProRes) | FFmpeg prores_ks | V1 | Zum Schneiden in Final Cut/Resolve |
| MKV ↔ MP4 ↔ MOV | Container wechseln | FFmpeg remux | MVP | ⚡ Sekunden statt Minuten |
| Video | GIF / animiertes WEBP | FFmpeg (palettegen) | MVP | Mit Ausschnitt-Wahl nach dem Drop |
| Video | MP3, M4A, WAV, FLAC (nur Ton) | FFmpeg | MVP | ⚡ wenn AAC → M4A |
| Video | Einzelbilder PNG/JPG | FFmpeg | V1 | Jede n-te Sekunde oder Szenenwechsel |
| Video | Vorschaubild / Poster | FFmpeg | MVP | Bestes Bild automatisch (nicht schwarz, scharf) |
| Video | Untertitel SRT/VTT | whisper.cpp | V2 (Modul Sprache) | Lokal, inklusive Deutsch |
| Video + SRT | Untertitel einbrennen | FFmpeg | V2 | Zwei Dateien zusammen ziehen |

**Aktionen**

| Aktion | Prio |
|---|---|
| Für E-Mail (< 25 MB) / für WhatsApp (< 100 MB) per Zielgröße (2-Pass) | MVP |
| Auflösung 4K → 1080p / 720p | MVP |
| Stummschalten (Ton entfernen) ⚡ | MVP |
| Drehen / falsche Ausrichtung korrigieren ⚡ | V1 |
| Hochformat für Reels/TikTok (9:16, Motiv mittig) | V1 |
| Kürzen (Mini-Zeitleiste nach Drop) ⚡ | V1 |
| Geschwindigkeit 0,5× / 2× / Zeitraffer | V2 |
| Lautstärke angleichen | V1 |
| Bildschirmaufnahme optimieren (niedrige fps, scharfer Text) | V1 |

---

## 3. Audio

**Lesen:** WAV, AIFF, MP3, M4A/AAC, ALAC, FLAC, OGG/Vorbis, OPUS, WMA, AMR, CAF, AC3

| Von | Nach | Engine | Prio | Hinweis |
|---|---|---|---|---|
| WAV/AIFF | MP3, M4A, FLAC | FFmpeg (LAME LGPL, System-AAC) | MVP | |
| beliebig | MP3 | FFmpeg | MVP | |
| beliebig | M4A (AAC) | AudioToolbox / Media Foundation | MVP | System-Encoder, gute Qualität |
| beliebig | FLAC / ALAC | FFmpeg | MVP | Verlustfrei |
| beliebig | WAV / AIFF | FFmpeg | MVP | |
| beliebig | OPUS, OGG | FFmpeg | V1 | |
| beliebig | M4R (iPhone-Klingelton) | FFmpeg | V1 | Automatisch auf 30 s gekürzt |
| AMR / WMA | MP3, M4A | FFmpeg | V1 | Alte Sprachnachrichten |
| Sprache | Transkript TXT / SRT / DOCX | whisper.cpp | V2 (Modul Sprache) | Interview, Sprachmemo |

**Aktionen:** Lautstärke angleichen (EBU R128) MVP · Stille am Anfang/Ende entfernen V1 · Mono/Stereo V1 · Samplerate ändern V1 · Kürzen V1 · Stimme isolieren V2 · Tags/Cover behalten (immer).

---

## 4. Dokumente und Text

**Lesen:** DOCX, DOC, ODT, RTF, PAGES (Vorschau), TXT, MD, HTML, EPUB, TEX

| Von | Nach | Engine | Prio | Hinweis |
|---|---|---|---|---|
| DOCX, ODT, RTF | PDF | LibreOffice headless | V1 (Modul Office) | Höchste Layout-Treue |
| DOCX, ODT, RTF, HTML | PDF (einfach) | Pandoc + Webview-Druck | V1 | Ohne Office-Modul |
| DOCX ↔ ODT ↔ RTF | | Pandoc / macOS textutil | V1 | |
| DOCX / HTML | Markdown | Pandoc | V1 | Für Entwickler, Notizen, Wikis |
| Markdown | HTML, PDF, DOCX | Pandoc + Webview-Druck | V1 | Schöne Standardvorlage |
| HTML / Webseite (.webarchive, .mhtml) | PDF | WKWebView / WebView2 Druck | V1 | Keine extra Engine nötig |
| DOCX / MD / HTML | EPUB | Pandoc | V2 | |
| EPUB | PDF, TXT | Pandoc / Webview | V2 | |
| DOC (alt) | DOCX, PDF | LibreOffice / textutil | V1 | |
| PAGES | PDF | Eingebettete Vorschau-PDF auslesen | V2 | Ohne Pages-App |
| TXT | UTF-8 reparieren (Windows-1252, ISO-8859-1) | encoding_rs | V1 | „Umlaute kaputt"-Fall |
| TXT / Code | Zeilenenden LF ↔ CRLF | eigen | V2 | |

---

## 5. PDF

| Aktion / Ziel | Engine | Prio | Hinweis |
|---|---|---|---|
| Seiten → JPG / PNG | pdfium | MVP | Eine Datei je Seite oder ZIP |
| Text herausholen → TXT | pdfium | MVP | |
| Durchsuchbar machen (OCR, Textebene) | Tesseract (PDF-Ausgabe) / Apple Vision | V1 | Smart Default bei gescannten PDFs |
| Mehrere PDFs zusammenfügen | qpdf | MVP | Mehrere Dateien ziehen |
| In Einzelseiten trennen | qpdf | MVP | |
| Komprimieren (Bilder neu kodieren) | pdfium + libvips + qpdf | V1 | Stufen: Bildschirm / E-Mail / Druck |
| Seiten drehen | qpdf | V1 | |
| Passwort setzen / entfernen (wenn bekannt) | qpdf | V1 | |
| Metadaten entfernen | qpdf | V1 | |
| Bilder herausziehen | pdfium | V1 | |
| PDF → DOCX | LibreOffice (Modul) | V2 | Ergebnis je nach PDF begrenzt, im Ring ehrlich kennzeichnen |
| PDF/A (Archiv) | LibreOffice / eigene Lösung | V2 | Kein Ghostscript (AGPL) |

---

## 6. Tabellen und Präsentationen

| Von | Nach | Engine | Prio | Hinweis |
|---|---|---|---|---|
| XLSX, XLS, ODS | CSV (je Blatt) | calamine (Rust) | MVP | |
| CSV | XLSX | rust_xlsxwriter | MVP | Spaltentypen erkennen, Kopfzeile fett, Filter an |
| CSV | **CSV für deutsches Excel** (Semikolon, UTF-8 mit BOM, Dezimalkomma) | eigen | MVP | Klassiker im DACH-Raum |
| XLSX / CSV | JSON, HTML-Tabelle, Markdown-Tabelle | eigen | V1 | |
| XLSX, ODS | PDF | LibreOffice | V2 (Modul Office) | |
| PPTX, ODP, KEY | PDF | LibreOffice / eingebettete Vorschau | V2 (Modul Office) | |
| PPTX | Bilder je Folie | LibreOffice + pdfium | V2 | |

---

## 7. Daten und Entwickler-Formate

| Von | Nach | Prio | Hinweis |
|---|---|---|---|
| JSON ↔ YAML ↔ TOML ↔ XML | | MVP | serde |
| JSON ↔ CSV | | MVP | Verschachtelte Felder flach machen |
| CSV/JSON ↔ Parquet | | V2 | Für Datenleute |
| SQLite | CSV je Tabelle, XLSX | V1 | |
| JSON | Schön formatieren / minimieren | MVP | |
| PLIST binär ↔ XML ↔ JSON | | V1 | Mac-Nutzer |
| VCF (Kontakte) ↔ CSV | | V1 | Kontakte zwischen Handy/Outlook umziehen |
| ICS (Kalender) | CSV | V2 | |
| EML / MSG (E-Mail) | PDF | V2 | Mit Anhängen als ZIP |
| .url ↔ .webloc (Verknüpfung) | | V2 | Mac ↔ Windows |

---

## 8. Archive

| Aktion | Engine | Prio |
|---|---|---|
| Entpacken: ZIP, 7Z, TAR, GZ, BZ2, XZ, ZST, RAR, CAB, ISO | libarchive (BSD) / unrar (nur Entpacken) | MVP |
| Packen als ZIP / 7Z / TAR.GZ | libarchive / 7-Zip-SDK (LGPL) | MVP |
| Mit Passwort packen (AES-256) | 7-Zip-SDK | V1 |
| Umpacken RAR/7Z → ZIP | libarchive | V1 |
| DMG erstellen / entpacken | macOS hdiutil | V2 |
| Für Mac/Win säubern (.DS_Store, __MACOSX, Thumbs.db entfernen) | eigen | V1 |

---

## 9. Schriften

| Von | Nach | Engine | Prio |
|---|---|---|---|
| TTF / OTF | WOFF2, WOFF | woff2 (MIT) | V1 |
| WOFF / WOFF2 | TTF | woff2 | V1 |
| TTF/OTF | Webfont-Paket (WOFF2 + CSS @font-face) | eigen | V1 |
| TTF/OTF | Teilmenge (nur Deutsch/Latein) | hb-subset (MIT) | V2 |

---

## 10. 3D

| Von | Nach | Engine | Prio | Hinweis |
|---|---|---|---|---|
| OBJ, FBX, DAE, PLY, 3DS | GLB / glTF | assimp (BSD) | V2 | Web und Unreal/Unity |
| STL ↔ 3MF ↔ OBJ | | assimp | V2 | 3D-Druck |
| GLB / OBJ | USDZ | macOS ModelIO | V2 | AR Quick Look am iPhone |
| Modell | Vorschaubild PNG | eigener Renderer | V2 | |

---

## 11. Untertitel und E-Books

| Von | Nach | Prio | Hinweis |
|---|---|---|---|
| SRT ↔ VTT ↔ ASS ↔ SBV | | V1 | eigen |
| SRT | Zeitversatz verschieben | V1 | Nach Drop kleiner Regler |
| SRT | TXT (reiner Text) | V1 | |
| EPUB ↔ AZW3 / MOBI | | V2 | Calibre als separates Programm (GPL), Modul |

---

## 12. Absicht-Aktionen (dateiübergreifend)

Diese tauchen im Ring auf, wenn sie zur Datei passen. Sie sind oft wertvoller als ein reiner Formatwechsel.

| Absicht | Was passiert | Für |
|---|---|---|
| **Für E-Mail** | Auf Zielgröße bringen (Bild < 1 MB, Video < 25 MB, PDF komprimiert, mehrere Dateien → ZIP) | alle |
| **Für Web** | WEBP/AVIF, max. 1920 px, Metadaten weg | Bilder |
| **Für Social** | 1:1 / 4:5 / 9:16 zuschneiden, Größen passend zur Plattform | Bilder, Video |
| **Privat machen** | GPS, Kamera, Autor, Bearbeitungsverlauf entfernen | Bilder, PDF, Office |
| **Text herausholen** | OCR oder Transkript, Ergebnis in Zwischenablage oder TXT | Bilder, PDF, Audio, Video |
| **Zusammenfügen** | Mehrere PDFs/Bilder → ein PDF, mehrere Videos → eines | mehrere Dateien |
| **Archivieren** | Verlustfreie, platzsparende Fassung (FLAC, PNG optimiert, PDF/A, 7Z) | alle |
| **Teilen in Zwischenablage** | Ergebnis nicht als Datei, sondern direkt kopieren | alle |
| **Rezept** | Eigene Kette, z. B. „Shop-Bild: 1200×1200, weißer Hintergrund, WEBP" | frei |

---

## Engine-Übersicht mit Lizenzen

| Engine | Lizenz | Einbindung | Wofür |
|---|---|---|---|
| libvips | LGPL-2.1 | dynamisch | Bildverarbeitung |
| libheif | LGPL-3 | dynamisch | HEIC/AVIF lesen |
| libjxl, libavif, libwebp | BSD | statisch | Bildformate |
| resvg | Apache/MIT | statisch | SVG rendern |
| vtracer | MIT | statisch | Vektorisieren |
| LibRaw | LGPL/CDDL | dynamisch | RAW-Fotos |
| FFmpeg (LGPL-Build) | LGPL-2.1 | Sidecar-Programm | Video, Audio |
| SVT-AV1, libvpx | BSD | in FFmpeg | AV1, VP9 |
| LAME | LGPL | in FFmpeg | MP3 |
| pdfium | BSD | dynamisch | PDF rendern, Text |
| qpdf | Apache-2.0 | dynamisch | PDF-Struktur |
| Tesseract | Apache-2.0 | dynamisch | OCR (Fallback) |
| Pandoc | GPL-2 | Sidecar-Programm | Dokumente |
| LibreOffice | MPL-2.0 | Sidecar, Modul | Office → PDF |
| whisper.cpp | MIT | statisch, Modul | Sprache → Text |
| ONNX Runtime | MIT | dynamisch, Modul | Bild-KI |
| libarchive | BSD | dynamisch | Archive |
| 7-Zip-SDK | LGPL | dynamisch | 7Z, AES-ZIP |
| assimp | BSD | dynamisch | 3D |
| woff2, HarfBuzz | MIT | statisch | Schriften |
| **Systemeigen macOS** | | | ImageIO, AVFoundation, VideoToolbox, Vision (OCR), ModelIO, textutil |
| **Systemeigen Windows** | | | WIC, Media Foundation, Windows.Media.Ocr, WebView2 |

**Bewusst ausgeschlossen:** Ghostscript (AGPL), x264/x265 (GPL, Patente), potrace (GPL), PyMuPDF (AGPL), alle Cloud-APIs.

---

## Marktvergleich Online-Konverter (Recherche 18.10.2026)

Angesehen: CloudConvert, Convertio, FreeConvert, Zamzar, online-convert.com.

**Meistbeworbene Umwandlungen** (alle über die Cloud, Orbit macht sie lokal):
MP4→MP3 · MOV→MP4 · M4A→MP3 · MKV→MP4 · WAV→MP3 · WEBM→MP4 · Video→GIF · HEIC→JPG · PNG→JPG · PNG→ICO · JPG→PDF · PDF→JPG · PDF↔Word · DOCX→PDF · EPUB→PDF/MOBI · PDF→Excel

**Werkzeuge neben dem Formatwechsel**, die dort beliebt sind: Video-/Bild-/Audio-/PDF-Kompressor, Kürzen, Zuschneiden, Drehen, Spiegeln, Stumm schalten, Tempo, Lauter, Schwarzweiß, Ton aus Video, GIF-Maker, PDF zusammenfügen/teilen, OCR, Vorlagen für Plattformen (WhatsApp, Instagram, Discord, TikTok) und Geräte (iPhone, Android, Klingelton).

**Einstellungen**, die Nutzer dort setzen können: Qualität, Auflösung, Codec, Bitrate, Abtastrate, Kanäle, fps, Seitenverhältnis, Start/Ende.

**Was Orbit anders macht:** keine Uploads und Größengrenzen (dort 50 MB bis 1 GB), Vorschläge passend zur Datei statt Formatliste, Ziel „Für Mail/WhatsApp/Discord“ mit garantierter Dateigröße, Umverpacken ohne Neukodierung (Sekunden statt Minuten).

**Stand Kern (Phase 1):** 83 Ziele in `core/crates/orbo-core/catalog/targets.yaml`, darunter alle Medien- und Bild-Umwandlungen der Liste oben. PDF↔Word, EPUB, PDF→Bild und OCR folgen in Phase 4 (pdfium, Pandoc, LibreOffice, Tesseract).
