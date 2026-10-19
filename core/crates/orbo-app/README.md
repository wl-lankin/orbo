# Orbo-App (Phase 2, macOS)

Tauri-2-App mit dem Orbo-Kern. Läuft als Menüleisten-App ohne Dock-Symbol.

## Bauen und starten

```bash
cd core/crates/orbo-app
cargo tauri build                       # → core/target/release/bundle/macos/Orbo.app
open ../../target/release/bundle/macos/Orbo.app
```

Zum Entwickeln: `cargo run -p orbo-app` (UI-Dateien werden beim Bauen eingebettet, nach Änderungen in `ui/` neu bauen).
Hilfen: `--settings` öffnet die Einstellungen, `--dialog <ziel> <datei>` öffnet direkt ein Optionsfenster (z. B. `--dialog beautify ~/Desktop/shot.png`).

## Aufbau

| Teil | Datei |
|---|---|
| Drag-Erkennung macOS (ohne Sonderrechte) | `src/platform/macos.rs` |
| Fenster, Menüleiste, nativer Ring-Hintergrund (HUD-Vibrancy / Liquid Glass) | `src/windows.rs` |
| Befehle für die Oberfläche | `src/commands.rs`, Vertrag in `IPC.md` |
| Einstellungen (`~/Library/Application Support/Orbo/settings.json`) | `src/settings.rs` |
| Oberfläche | `ui/ring.*`, `ui/toast.*`, `ui/dialog.*`, `ui/settings.*`, Design-Basis `ui/shared.css` |

Im Browser ohne Tauri lassen sich die Seiten mit `ui/dev-mock.js` bzw. `ui/dev-mock-dialogs.js` ansehen (`python3 -m http.server` in `ui/`).

## Stand 18.10.2026

Getestet in der echten App (synthetischer Drag wie aus dem Finder):
- Ring erscheint in ca. 1 ms nach dem Auslöser, Vorschläge aus dem Kern, Stil Klassisch und Glas mit echtem macOS-Hintergrund
- Ablegen wandelt um (PNG→AVIF, MOV→MP4 ohne Neukodierung, MOV→Mail, WAV→MP3), Toast mit Fortschritt, Zeigen und Rückgängig
- Optionsfenster Schick, Zuschneiden (auch HEIC), Kürzen mit Filmstreifen; Speichern erzeugt die Datei
- Einstellungen mit fünf Bereichen, Über-Seite mit Wolfgang Linz / wolfgang-linz.de

Nachgezogen (18.10.2026 abends):
- ffmpeg/ffprobe als LGPL-Build mitgeliefert (`vendor/build-ffmpeg-macos.sh`, nur Systembibliotheken, VideoToolbox, AudioToolbox, LAME). Ziele, deren Encoder fehlen (z. B. VP9/WEBM), werden ausgeblendet.
- Dokumente: PDF → Bilder/Text/Trennen/Drehen/Privat/Komprimieren, TXT/MD/HTML/RTF/DOC/DOCX/ODT untereinander und nach PDF (macOS: textutil, cupsfilter)
- Entpacken mit Vorlagen „Neuer Ordner“, „Hier“, „Ordner wählen …“
- Einführung beim ersten Start (Ordnerfreigaben, Taste, Stil, Autostart), Bei Anmeldung starten (Standard an), Notch-Portal (Standard aus), „…“-Feld für Optionen im Vorlagenbogen
- Geschützte Ordner ohne Freigabe: Erkennung fällt auf die Dateiendung zurück statt auf „Text“ (vorher nur ZIP im Ring)
- Ring schließt sich 256 pt um die Mitte, dadurch nehmen die Fensterecken keine Drops mehr an

Bekannt / offen:
- Release-Build braucht `strip = "none"` (macOS-27-Werkzeug beschädigt sonst Proc-Macro-Bibliotheken)
- App nicht signiert/notarisiert (Phase 6)

## Windows (Phase 3, Cross-Build auf dem Mac)

Voraussetzungen: `rustup target add x86_64-pc-windows-gnu`, `brew install mingw-w64`. Der Linker steht in `core/.cargo/config.toml`, Windows-Abweichungen der Konfiguration in `tauri.windows.conf.json` (kein mitgeliefertes ffmpeg, Symbol `icons/icon.ico`).

```bash
cd core
CARGO_TARGET_DIR=target/win cargo build --release -p orbo-app --target x86_64-pc-windows-gnu
D=../dist/windows/Orbo
cp target/win/x86_64-pc-windows-gnu/release/orbo-app.exe $D/Orbo.exe && x86_64-w64-mingw32-strip $D/Orbo.exe
cp target/win/x86_64-pc-windows-gnu/release/WebView2Loader.dll $D/
```

Eigenes `CARGO_TARGET_DIR`, damit der Windows-Build den macOS-Build nicht blockiert. `WebView2Loader.dll` wird beim GNU-Ziel dynamisch gebunden und muss neben `Orbo.exe` liegen.

| Teil | Datei |
|---|---|
| Drag-Erkennung Windows (unsichtbare Sonde mit `IDropTarget`, `GetAsyncKeyState`), Platzierung pro Monitor-DPI, Log-Datei | `src/platform/windows.rs` |
| Windows-Anpassungen der Oberfläche (Ring-Hintergrund, Tastennamen, kein Notch-Portal) | Init-Skripte in `src/windows.rs` |

Log: `%APPDATA%\Orbo\orbo.log` (vorheriger Start: `orbo.old.log`). Anleitung für den Test: `dist/windows/Orbo/README-WINDOWS.txt`.
