# Machbarkeitstest Windows (Phase 0)

Konsolen-Programm in **Rust** (`windows`-Crate), auf dem Mac cross-kompiliert. Es beweist dasselbe wie der macOS-Test: Datei-Drag systemweit erkennen, Auslöser per Taste oder Schütteln, Ring an der Maus, Drop empfangen, echte Umwandlung.

## Bauen (auf dem Mac)

```bash
cargo build --release          # Ziel x86_64-pc-windows-gnu ist in .cargo/config.toml gesetzt
cp target/x86_64-pc-windows-gnu/release/orbit-spike.exe dist/
```

Voraussetzungen: `rustup target add x86_64-pc-windows-gnu`, `brew install mingw-w64`.

Ring-Layout ohne Windows ansehen (rendert PNGs, braucht eine TTF-Schrift unter `$WINDIR\Fonts\segoeui.ttf`):

```bash
cargo run --release --example preview --target aarch64-apple-darwin -- <ausgabeordner>
```

## Auf Windows ausführen

`dist/orbit-spike.exe` auf den PC kopieren (ein Ordner ohne Sonderrechte reicht, z. B. Desktop), dann in PowerShell oder per Doppelklick:

```powershell
.\orbit-spike.exe                     # normal: Datei im Explorer ziehen, Alt drücken oder schütteln
.\orbit-spike.exe --selbsttest        # spielt 4 Fälle selbst durch, ca. 30 s, Maus nicht bewegen
.\orbit-spike.exe --stil glas         # Glas-Optik (ohne echten Unschärfe-Effekt, siehe unten)
.\orbit-spike.exe --taste strg        # Auslöser Strg statt Alt (auch: shift)
.\orbit-spike.exe --ohne-schuetteln
```

Log: Konsole und `%LOCALAPPDATA%\OrbitSpike\log.txt`. Bitte nach dem Test diese Datei schicken.

Windows SmartScreen meldet sich beim ersten Start, weil die Datei nicht signiert ist: „Weitere Informationen" → „Trotzdem ausführen".

## Wie es funktioniert

| Baustein | Umsetzung | Rechte |
|---|---|---|
| Maustaste, Taste | `GetAsyncKeyState` alle ~10 ms (kein Hook) | keine |
| Drag erkannt | Taste gedrückt + Bewegung über `SM_CXDRAG` | keine |
| Datei-Drag? | Bei Auslöser erscheint eine **unsichtbare Sonde** (Alpha 1/255) unter der Maus. Läuft ein Explorer-Drag, schickt OLE sofort `DragEnter` mit den Dateien (`CF_HDROP`). Erst dann wird der Ring sichtbar. Kein `DragEnter` = kein Datei-Drag, Sonde verschwindet. | keine |
| Ring | Layered Window (`UpdateLayeredWindow`, Alpha pro Pixel), gezeichnet mit `tiny-skia`, Schrift Segoe UI über `ab_glyph`, pro Monitor DPI-skaliert | - |
| Drop | `IDropTarget` per `RegisterDragDrop`, `IDropTargetHelper` damit das Explorer-Ziehbild sichtbar bleibt | - |
| Umwandlung | PowerShell + `System.Drawing` (JPG, PNG, BMP, TIFF, GIF, 1600 px), `tar.exe` für ZIP | - |

Unterschied zu macOS: Windows hat keine globale Drag-Zwischenablage. Deshalb die Sonde. Vorteil: Wenn der Ring sichtbar wird, sind die Dateien schon bekannt.

## Selbsttest

Eine Test-Quelle im selben Programm startet einen echten OLE-Drag (`DoDragDrop` mit Shell-Datenobjekt, wie der Explorer), ein zweiter Thread steuert Maus und Tastatur per `SendInput`:

1. Alt → Feld oben (JPG) → erwartet `test.jpg`
2. Schütteln → Feld rechts oben (1600 px) → erwartet `test (1600 px).jpg`
3. Alt → Mitte (Vorschlag) → erwartet `test 2.jpg`
4. Alt → weit wegziehen → zurück zur Quelle → loslassen → erwartet **keine** neue Datei

Testdateien liegen in `%TEMP%\OrbitSpikeTest`.

## Ergebnisse

| Test | Ergebnis |
|---|---|
| Cross-Build auf dem Mac | ✅ 2 MB, nur System-DLLs (UCRT, ab Windows 10) |
| Ring-Rendering inkl. Textzentrierung | ✅ per Vorschau-PNG geprüft |
| Handtest auf Windows (über RDP, 18.10.2026) | ✅ „funktioniert schon sehr geil" |
| Selbsttest Lauf 1 (über RDP) | Erkennung ✅ (Ring 17 bis 28 ms nach Auslöser, Dateien sofort bekannt), Drops ❌ 3 von 4. Vermutete Ursache: die Mac-Maus über dem RDP-Fenster hat die Test-Maus gestört (falsches „Schütteln", falsches „weggezogen"). Lauf 2 mit Kalibrierung und mehr Diagnose steht aus. |
| Manuelle Tests (Liste unten) | teilweise |

## Von Hand zu testen

| # | Test | Erwartung | Ergebnis |
|---|---|---|---|
| 1 | Datei aus Explorer ziehen, Alt drücken | Ring erscheint, Felder passen | |
| 2 | Datei vom Desktop | wie 1 | |
| 3 | mehrere Bilder | alle umgewandelt | |
| 4 | Ordner | nur ZIP | |
| 5 | Ring offen, in die **Ecke** des Ring-Fensters loslassen | Datei landet beim Fenster darunter | |
| 6 | Alt loslassen nach dem Drop | Explorer-Menüleiste springt **nicht** an (bekanntes Alt-Risiko) | |
| 7 | Fenster mit gedrückter Maus verschieben + Alt | **kein** Ring (Sonde verschwindet nach 2,5 s) | |
| 8 | Text in Word/Browser ziehen + Alt | kein Ring, Log „DragEnter ohne Dateien" | |
| 9 | zwei Monitore mit 100 % und 150 % | Ring scharf und richtig groß auf beiden | |
| 10 | Datei aus Outlook-Anhang | dokumentieren | |
| 11 | Explorer als Administrator, Orbit normal | dokumentieren (UIPI kann Drops sperren) | |
| 12 | Task-Manager: CPU im Leerlauf | < 0,5 % | |

## Bekannte Grenzen des Tests

- **Glas-Stil ohne Unschärfe:** Layered Windows mit Alpha pro Pixel können Mica/Acrylic nicht nutzen. Für echte Unschärfe im Produkt: runde Fensterregion + `DwmSetWindowAttribute(DWMWA_SYSTEMBACKDROP_TYPE)` oder Composition-API prüfen.
- **Abfrage im Leerlauf (~100 Hz):** Für das Produkt auf `WH_MOUSE_LL`-Hook oder Ereignis-gesteuert umstellen und Stromverbrauch messen.
- **Esc zum Abbrechen** fehlt noch (wie am Mac).
