# Projektplan

## Ziel

Eine kleine Hintergrund-App für macOS und Windows, die beim Ziehen einer Datei einen kontextbezogenen Ring einblendet und die Datei lokal umwandelt. Keine Cloud, kein Konto, keine Telemetrie. Die App funktioniert komplett offline.

### Nicht-Ziele (erst mal)

- Kein vollwertiger Editor (kein Zuschneiden per Timeline im MVP, nur einfache Presets).
- Kein Linux im ersten Schritt (Architektur hält es aber offen).
- Kein Mac App Store: die Sandbox erlaubt das globale Beobachten von Drags nicht zuverlässig. Vertrieb als signierter, notarisierter Download.

---

## Architektur

```
┌──────────────────────────── Orbit-App ─────────────────────────────┐
│                                                                      │
│  ┌──────────── Plattform-Schicht (je OS) ─────────────┐              │
│  │ Drag-Erkennung   Taste/Schütteln   Overlay-Fenster │              │
│  │ macOS: NSPasteboard(.drag) + NSEvent-Monitore      │              │
│  │ Windows: WH_MOUSE_LL-Hook + IDropTarget (OLE)      │              │
│  └───────────────┬────────────────────────────────────┘              │
│                  │ Dateipfade, Mausposition                           │
│  ┌───────────────▼──────────┐    ┌─────────────────────────────┐     │
│  │  Ring-UI (Webview)       │◄──►│  Kern (Rust)                │     │
│  │  vorgewärmt, transparent │    │  • Dateierkennung + Analyse │     │
│  │  Animationen, Hover,     │    │  • Vorschlags-Engine        │     │
│  │  Größenschätzung         │    │  • Job-Warteschlange        │     │
│  └──────────────────────────┘    │  • Engine-Adapter           │     │
│                                  │  • lokales Lernen (SQLite)  │     │
│  ┌──────────────────────────┐    └──────────────┬──────────────┘     │
│  │ Einstellungen, Onboarding│                   │                     │
│  │ Menüleiste / Tray        │                   ▼                     │
│  └──────────────────────────┘   ┌───────────── Engines ────────────┐  │
│                                 │ libvips · FFmpeg (LGPL) · libheif │  │
│                                 │ pdfium · qpdf · Pandoc · Tesseract│  │
│                                 │ whisper.cpp · ONNX · libarchive   │  │
│                                 │ + OS-eigene: ImageIO, AVFoundation│  │
│                                 │   Vision / WIC, Media Foundation  │  │
│                                 └───────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────┘
```

### Wie die Drag-Erkennung funktioniert

**macOS**
1. `NSEvent.addGlobalMonitorForEvents` für `leftMouseDragged` und `flagsChanged`.
2. Ändert sich der `changeCount` des Drag-Pasteboards (`NSPasteboard(name: .drag)`), läuft ein Datei-Drag. Pfade aus dem Pasteboard lesen (so arbeiten auch Dropover und Yoink).
3. Auslöser erkannt → vorgewärmtes, randloses `NSPanel` (nicht aktivierend, über allen Fenstern, alle Spaces) an der Mausposition einblenden. Das Panel ist selbst Drop-Ziel und bekommt die Dateien beim Loslassen.

**Windows**
1. Low-Level-Maus-Hook (`SetWindowsHookEx(WH_MOUSE_LL)`) plus `GetAsyncKeyState` für die Taste.
2. Gedrückte Maus + Bewegung über Schwelle + Auslöser → Overlay-Fenster (`WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE`) an der Maus einblenden.
3. Das Overlay ist per `RegisterDragDrop` OLE-Drop-Ziel. Bei `DragEnter` liefert `IDataObject` mit `CF_HDROP` die Pfade. So kennt der Ring die Datei schon **vor** dem Loslassen und kann passende Sektoren zeigen.

**Wichtig:** Beides ist in Phase 0 als technischer Spike zu beweisen, bevor UI gebaut wird. Das ist das größte Projektrisiko.

### Tech-Stack (Empfehlung)

| Teil | Wahl | Begründung |
|---|---|---|
| App-Hülle | **Tauri 2** | Klein (ein paar MB statt 150 MB Electron), Rust-Kern, ein UI-Code für beide OS. |
| Kern | **Rust** | Sicher, schnell, gute Bindings zu C-Bibliotheken (libvips, pdfium, whisper.cpp), einfache Prozesssteuerung für FFmpeg. |
| Plattform-Code | Rust mit `objc2` (macOS) und `windows`-Crate (Windows) | Direkter Zugriff auf AppKit/Win32 ohne extra Sprache. |
| Ring-UI | Svelte oder Solid + Canvas/SVG, Web Animations API | Sehr flüssige Animationen, schnell iterierbar, klein. |
| Speicher | SQLite (lokal) | Nutzungsstatistik fürs Lernen, Rezepte, Verlauf für Rückgängig. |
| Tests | Rust-Unit-Tests, Golden-File-Tests pro Konvertierung, Playwright für die Ring-UI | Jede Umwandlung gegen Referenzdateien geprüft. |

**Plan B**, falls das transparente Webview-Overlay zu langsam erscheint oder Drops verschluckt: Ring nativ zeichnen (macOS Core Animation, Windows Composition/Direct2D), Kern bleibt gleich. Deshalb Kern und UI strikt über eine schmale Schnittstelle trennen (`analyze(paths) → RingModel`, `run(jobSpec) → JobHandle`).

### Engines und Lizenzen (Kurzfassung)

- **FFmpeg nur als LGPL-Build** und als separates Programm (Sidecar). H.264/HEVC über die **Hardware-Encoder des Systems** (VideoToolbox, Media Foundation, NVENC/QSV/AMF) statt x264/x265 (GPL). Spart auch Patentärger.
- **Kein Ghostscript** (AGPL). PDF über pdfium (BSD) und qpdf (Apache).
- Pandoc (GPL) und optional LibreOffice (MPL) nur als **separate Programme**, nicht eingelinkt.
- KI-Modelle (Whisper, Hintergrund entfernen) nur mit freier Lizenz (MIT/Apache), z. B. whisper.cpp + `ggml-base`, BiRefNet/U²-Net.
- Details pro Format in [KONVERTIERUNGEN.md](KONVERTIERUNGEN.md).

### Größe und Module

Grundpaket (Bilder, Audio, Video, PDF-Basics, Daten, Archive) Ziel **unter 80 MB**. Große Teile als **optionale Module**, die man in den Einstellungen per Klick installiert und die danach ebenfalls offline laufen:

| Modul | Inhalt | ca. Größe |
|---|---|---|
| Office | LibreOffice headless für DOCX/XLSX/PPTX → PDF in hoher Treue | 300 MB |
| Sprache | whisper.cpp + Modell für Transkripte/Untertitel | 150 bis 500 MB |
| Bild-KI | Hintergrund entfernen, Hochskalieren | 100 bis 200 MB |
| OCR-Sprachen | Tesseract-Sprachdaten (Deutsch und Englisch sind im Grundpaket) | je 5 bis 15 MB |

*Offene Entscheidung:* Module von eigener Webseite laden (einmaliger Download, Verarbeitung bleibt lokal) **oder** als „Komplett"-Installer anbieten. Beides verletzt „keine externen Dienste" nicht, weil nie Nutzerdateien das Gerät verlassen. Für strenge Umgebungen gibt es den Komplett-Installer.

---

## Phasen

Schätzung für eine Person mit Claude-Unterstützung. „W" = Wochen.

### Phase 0: Machbarkeit beweisen (2 W) · Go/No-Go

- Spike macOS: Drag aus Finder und Desktop erkennen, Taste und Schütteln erkennen, Panel an Maus einblenden, Drop empfangen, Pfade loggen.
- Spike Windows: dasselbe mit Explorer und Desktop.
- Messen: Zeit von Auslöser bis sichtbarem Ring.
- Prüfen: normales Drag-Verhalten bleibt intakt, wenn der Ring nicht genutzt wird.

**Abnahme:** Auf beiden Systemen erscheint ein Test-Kreis in < 100 ms, Drop liefert korrekte Pfade, Drop außerhalb funktioniert wie immer. Wenn Windows hakt: Plan B (nativer Overlay) entscheiden.

**Stand 18.10.2026, macOS:** Automatisch bestanden (Ring in 2 bis 37 ms, Taste und Schütteln, echte Umwandlungen), **ohne** Sonderrechte. Manuelle Finder-Tests offen, siehe [spikes/macos-drag/README.md](../spikes/macos-drag/README.md). Windows: Handtest über RDP erfolgreich, Ring 17 bis 28 ms nach Auslöser, ebenfalls ohne Sonderrechte (siehe [spikes/windows-drag/README.md](../spikes/windows-drag/README.md)). **Phase 0 damit im Kern bestanden.**

### Phase 1: Kern (3 W)

- Dateierkennung (Magic Bytes, `infer`-Crate) plus Analyse (Bildmaße, Alpha, Animation, Video-Codecs per ffprobe, PDF-Textebene).
- Vorschlags-Engine mit Regeln (YAML-Dateien, leicht erweiterbar).
- Job-Warteschlange mit Fortschritt, Abbruch, paralleler Abarbeitung (CPU-Kerne beachten).
- Engines für das MVP: Bilder (libvips, libheif), Video/Audio (FFmpeg-Sidecar), Daten (CSV/JSON/XLSX in Rust).
- CLI zum Testen: `orbo analyze datei.heic`, `orbo convert datei.heic --to jpg`.

**Abnahme:** 30 MVP-Umwandlungen aus dem Katalog laufen per CLI, Golden-File-Tests grün auf Mac und Windows (CI mit GitHub Actions).

**Stand 18.10.2026:** Kern steht in `core/` (siehe [core/README.md](../core/README.md)). 83 Ziele statt 30, 58 Tests grün auf dem Mac, Cross-Build für Windows kompiliert. Abweichung: Bilder in reinem Rust statt libvips. Offen: Tests auf echtem Windows (CI braucht ein GitHub-Repo), HEIC unter Windows.

### Phase 2: Ring-MVP auf macOS (3 W)

- Vorgewärmtes Overlay-Fenster, Ring-UI aus dem Prototyp übernehmen.
- Hover, Verweilen mit Presets, Smart Default im Zentrum, Mehr-Palette.
- Ergebnis neben Original, Meldung mit Rückgängig, Menüleisten-Symbol mit Fortschritt.
- Einstellungen: Auslöser, Ausgabeort, Namensschema.

**Stand 18.10.2026:** `Orbit.app` läuft (siehe [core/crates/orbo-app/README.md](../core/crates/orbo-app/README.md)): Ring in ca. 1 ms, beide Stile mit echtem macOS-Hintergrund, Toast, Optionsfenster (Schick, Zuschneiden, Kürzen …), Einstellungen. Offen: ffmpeg mitliefern, Autostart, Onboarding, Notch-Portal.

**Abnahme:** Eigene Alltagsnutzung eine Woche lang ohne Absturz. Ring erscheint in < 80 ms. 10 Testpersonen finden ohne Erklärung die richtige Umwandlung (Ziel: 8 von 10 in < 5 Sekunden).

### Phase 3: Windows (3 W)

- Plattform-Schicht Windows, Tray-Symbol, Mica-Optik.
- HiDPI und mehrere Monitore mit unterschiedlicher Skalierung (häufige Fehlerquelle).
- Explorer, Desktop, Datei-Dialoge, Outlook-Anhänge testen.

**Abnahme:** Gleiche Kriterien wie Phase 2 auf Windows 10 und 11.

### Phase 4: Ausbau Konvertierungen (4 W)

- PDF (OCR, zusammenfügen, trennen, komprimieren), Dokumente (Pandoc), Archive, Schriften, Untertitel.
- Absicht-Aktionen: *Für E-Mail*, *Zielgröße*, *Metadaten entfernen*.
- Optionale Module: Office, Sprache (Whisper), Bild-KI.
- Rezepte (Ketten) und lokales Lernen.
- Optionale KI über lokales Ollama: Umbenennen, Alt-Text, Zusammenfassen, Übersetzen, Befehl in eigenen Worten (siehe UI-UX-Konzept).

### Phase 5: Feinschliff (3 W)

- Onboarding mit Beispieldatei, Notch-Portal, Vorher/Nachher-Lupe.
- Barrierefreiheit (Hotkey-Modus, VoiceOver/Narrator).
- Mehrsprachig (Deutsch, Englisch).
- App-Icon (macOS `.icns`, Windows `.ico`), Tray-/Menüleisten-Symbol und Landingpage-Grafiken über Higgsfield erzeugen, sobald der Name feststeht.
- Performance: Ring-Animationen auf 120 fps, Größenschätzung im Hintergrund.

### Phase 6: Veröffentlichung (2 W)

- macOS: Developer-ID-Signatur, Notarisierung, DMG.
- Windows: Codesignatur (SignPath-Antrag wie bei Session Buddy, oder EV-Zertifikat), MSIX und winget.
- Updates nur auf Wunsch prüfen (abschaltbar), damit „offline" ehrlich bleibt.
- Landingpage mit Demo-Video, Lizenzhinweise aller Engines (Pflicht bei LGPL/MPL).

**Gesamt: ca. 20 Wochen** bis 1.0. Ein nutzbares Mac-MVP gibt es nach ca. 8 Wochen.

---

## Risiken

| Risiko | Wahrscheinlichkeit | Gegenmaßnahme |
|---|---|---|
| Drag-Erkennung unter Windows unzuverlässig (z. B. in Apps mit eigener Drag-Logik) | mittel | Phase 0 zuerst. Fallback: Hotkey-Modus und Top-Portal funktionieren ohne Hook. |
| Transparentes Webview-Overlay zu träge | mittel | Vorwärmen, Plan B nativ. Schnittstelle Kern/UI schmal halten. |
| macOS-Rechte (Eingabeüberwachung/Bedienungshilfen) schrecken ab | mittel | Gutes Onboarding mit Begründung. Prüfen, welches Recht wirklich nötig ist (Maus-Monitore brauchen weniger als Tastatur). |
| Virenscanner schlagen wegen globalem Hook an | mittel | Signieren, bei Microsoft zur Prüfung einreichen, Hook minimal halten. |
| Lizenzfehler (GPL/AGPL eingeschleppt) | niedrig, aber teuer | Lizenzliste pflegen, `cargo-deny` in CI, FFmpeg-Build-Flags festschreiben. |
| Patente (HEVC, AAC) | niedrig | Nur System-Encoder nutzen, die das Betriebssystem lizenziert. |
| Installer zu groß | mittel | Module optional, Grundpaket < 80 MB. |
| Konvertierung zerstört Daten | niedrig | Original nie anfassen, Ergebnis erst nach Erfolg sichtbar (temporär schreiben, dann umbenennen). |

## Offene Entscheidungen

1. Name und Marke.
2. Geschäftsmodell: kostenlos/Open Source, einmal kaufen (z. B. 19 €), oder Grundfunktion frei plus Pro-Module.
3. Module per Download oder Komplett-Installer.
4. Standard-Taste nach Tests in Phase 0 festlegen.
5. Linux später ja/nein.

## Nächste Schritte

1. Prototyp `prototype/ring-prototype.html` durchspielen, Feedback zu Ring, Sektoren, Animation.
2. Name festlegen, Git-Repo anlegen.
3. Phase 0 starten: Spike macOS (Rust + objc2), dann Windows.
