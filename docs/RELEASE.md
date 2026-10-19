# Release und automatische Updates

Orbo aktualisiert sich selbst. Die installierte App fragt alle sechs Stunden (und 60 Sekunden nach dem Start) bei GitHub nach `latest.json`, lädt eine neuere Version im Hintergrund, prüft die Signatur und installiert sie, sobald gerade nichts passiert (kein Ring, kein Notch, kein Dialog, keine laufende Umwandlung). Danach startet Orbo neu und zeigt einmal „Orbo wurde auf Version X aktualisiert“. Ist „Automatisch aktualisieren“ in den Einstellungen (Über) aus, erscheint stattdessen „Update auf X installieren“ im Menü des Symbols und in Über.

## Aufbau

| Teil | Wo |
|---|---|
| Updater im Code | `core/crates/orbo-app/src/updates.rs` |
| Updater-Konfiguration (Endpunkt, öffentlicher Schlüssel, Windows `passive`) | `core/crates/orbo-app/tauri.conf.json`, Abschnitt `plugins.updater` |
| Signierte Update-Dateien erzeugen | `core/crates/orbo-app/tauri.updater.conf.json` (nur in der Pipeline) |
| FFmpeg-Sidecars für Windows (nur in der Pipeline) | `core/crates/orbo-app/tauri.windows.release.conf.json` |
| Kommandozeilen-Tool `orbo` als Sidecar (Mac: `bin/orbo-cli`, Windows: Ressource `cli/orbo.exe`) | `core/crates/orbo-app/tauri.conf.json`, `tauri.windows.release.conf.json` |
| Pipeline | `.github/workflows/release.yml` |
| Release-Text | Abschnitt `## X.Y.Z` in `CHANGELOG.md`, sonst die Nachricht des Tags |

`createUpdaterArtifacts`, `externalBin` und das Kommandozeilen-Tool für Windows stehen bewusst nicht in der normalen Konfiguration: Sonst verlangt jeder lokale `cargo tauri build` den Signaturschlüssel, und der lokale Windows-Cross-Build (`x86_64-pc-windows-gnu`) bräuchte FFmpeg-Dateien für diese Zielplattform.

## Kommandozeilen-Tool als Sidecar

Das Tool `orbo` (Crate `orbo-cli`) liegt im Bundle: auf dem Mac als `Orbo.app/Contents/MacOS/orbo-cli`, unter Windows als `cli\orbo.exe` im Installationsordner. Es heißt nicht einfach `orbo`, weil das Hauptprogramm `Orbo` heißt und beide Dateinamen auf macOS und Windows nicht auseinandergehalten werden. Der Befehl `orbo` entsteht erst durch die Verknüpfung (Mac) bzw. den PATH-Eintrag (Windows), den der Nutzer in den Einstellungen anlegt. Das Tool findet FFmpeg neben sich bzw. eine Ebene höher.

Tauri verlangt Sidecars mit Zielplattform im Dateinamen und prüft ihr Vorhandensein bei jedem Build der App. Die Pipeline baut das Tool deshalb vor `tauri-action`. Lokal (Apple Silicon):

```bash
cd core
cargo build --release -p orbo-cli --target aarch64-apple-darwin
cp target/aarch64-apple-darwin/release/orbo crates/orbo-app/bin/orbo-cli-aarch64-apple-darwin
```

Windows-Release (MSVC):

```powershell
cargo build --release -p orbo-cli --target x86_64-pc-windows-msvc
Copy-Item target/x86_64-pc-windows-msvc/release/orbo.exe crates/orbo-app/bin/orbo-x86_64-pc-windows-msvc.exe
```

Ohne diese Datei bricht `cargo build -p orbo-app` auf dem Mac ab; für reine Testläufe genügt eine leere Platzhalterdatei (`core.yml` legt sie an). Die normale Windows-Konfiguration (`tauri.windows.conf.json`) kennt keine Sidecars, das Tool kommt dort nur über `tauri.windows.release.conf.json` in den Installer. Nach dem Bauen die App einmal ohne Pipeline prüfen: `Orbo.app/Contents/MacOS/orbo-cli targets` muss die Ziele ausgeben.

## Einmalig einrichten

1. Repository `wl-lankin/orbo` auf GitHub anlegen und den Projektordner hineinpushen.
2. Unter Settings, Secrets and variables, Actions zwei Secrets anlegen:
   - `TAURI_SIGNING_PRIVATE_KEY`: kompletter Inhalt von `~/.tauri/orbo-updater.key`
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: Inhalt von `~/.tauri/orbo-updater.pass`
3. Unter Settings, Actions, General: „Workflow permissions“ auf „Read and write permissions“ stellen (die Pipeline legt das Release an).

## Wo der Schlüssel liegt

- Privat: `~/.tauri/orbo-updater.key` und das Passwort in `~/.tauri/orbo-updater.pass`, nur auf diesem Mac und in den GitHub-Secrets. Nie committen (`.gitignore` schließt `*.key` und `*.pass` aus), nie in Chats oder Logs einfügen.
- Öffentlich: `~/.tauri/orbo-updater.key.pub`, steht in `tauri.conf.json`.
- Geht der private Schlüssel verloren, können bereits installierte Apps keine Updates mehr annehmen. Dann hilft nur eine neue Installation mit neuem Schlüssel. Deshalb den Schlüssel und das Passwort zusätzlich sicher sichern (Passwortmanager).

## Ein Release erstellen

1. Version erhöhen, an beiden Stellen gleich:
   - `core/Cargo.toml`, Abschnitt `[workspace.package]`, `version`
   - `core/crates/orbo-app/tauri.conf.json`, `version`
2. Abschnitt `## X.Y.Z` oben in `CHANGELOG.md` ergänzen (er wird zum Release-Text und erscheint in `latest.json`).
3. Committen und pushen. Die Tests (`core.yml`) müssen grün sein.
4. Tag setzen und pushen:
   ```bash
   git tag -a v1.0.1 -m "Orbo 1.0.1"
   git push origin v1.0.1
   ```
   Ein Tag mit Bindestrich (zum Beispiel `v0.2.0-beta.1`) wird als Vorabversion markiert. Die Version in den beiden Dateien oben muss trotzdem rein numerisch sein (NSIS), also `0.2.0`.
5. Unter Actions den Lauf „Release“ abwarten. Der macOS-Job läuft zuerst (beim ersten Mal baut er FFmpeg, etwa 10 bis 20 Minuten, danach aus dem Cache), danach der Windows-Job. Sie laufen nacheinander, weil beide dieselbe `latest.json` schreiben.
6. Unter Releases den Entwurf prüfen. Er muss enthalten:
   - `Orbo_X.Y.Z_aarch64.dmg`
   - `Orbo.app.tar.gz` und `Orbo.app.tar.gz.sig`
   - `Orbo_X.Y.Z_x64-setup.exe` und `Orbo_X.Y.Z_x64-setup.exe.sig`
   - `latest.json` mit den Einträgen `darwin-aarch64` und `windows-x86_64`, jeweils mit `signature` und `url`
7. Release veröffentlichen („Publish release“). Erst jetzt sehen installierte Apps das Update, denn `releases/latest/download/latest.json` zeigt nur auf veröffentlichte Releases.

## Updater lokal testen

Der Updater lädt nur signierte Dateien von GitHub. Ein Test braucht deshalb zwei echte Releases:

1. Version `1.0.0` veröffentlichen und die App daraus installieren (DMG bzw. Setup.exe).
2. Version auf `1.0.1` erhöhen, Tag pushen, Entwurf prüfen und veröffentlichen.
3. Die installierte `1.0.0` starten. Nach 60 Sekunden prüft sie selbst. Schneller geht es mit Einstellungen, Über, „Jetzt prüfen“: Die Zeile zeigt „Version 1.0.1 wird geladen …“, dann startet Orbo neu und meldet die neue Version.
4. Auch testen: „Automatisch aktualisieren“ aus, dann „Jetzt prüfen“. Es soll „Version 1.0.1 ist verfügbar“ mit Schaltfläche und der Eintrag „Update auf 1.0.1 installieren“ im Menü des Symbols erscheinen.
5. Fehlerfall: Ohne Netz prüfen. Im Hintergrund bleibt es still (nur Zeile im Log), „Jetzt prüfen“ zeigt „Fehler: …“.

Eine lokal gebaute App (`cargo run`) kann sich nicht ersetzen und zeigt dann nur einen Fehler. Das Log liegt auf Windows in `%APPDATA%\Orbo\orbo.log`; auf dem Mac erscheinen die Zeilen mit `[orbo] Update` auf der Konsole, wenn Orbo aus dem Terminal gestartet wird.

## Einschränkungen und offene Punkte

- TODO Apple-Notarisierung: Das DMG ist weder signiert noch notarisiert. Beim ersten Öffnen blockiert Gatekeeper (Systemeinstellungen, Datenschutz und Sicherheit, „Trotzdem öffnen“, oder `xattr -dr com.apple.quarantine /Applications/Orbo.app`). Spätere Updates über den Updater sind davon nicht betroffen.
- TODO Windows-Signierung: Der Installer ist nicht signiert, SmartScreen meldet sich beim ersten Start („Weitere Informationen“, „Trotzdem ausführen“).
- Nur Apple Silicon (`aarch64-apple-darwin`). Intel-Macs bekommen weder Installer noch Updates. Ein Intel-Build bräuchte einen zweiten FFmpeg-Build und einen weiteren Job.
- Die Windows-Pipeline lädt FFmpeg aus den rollenden BtbN-Builds (Tag `latest`) und prüft die Prüfsumme. BtbN ersetzt diese Dateien regelmäßig. Verschwindet der Dateiname `ffmpeg-n8.1-latest-win64-lgpl-8.1.zip`, bricht der Job mit einer klaren Meldung ab. Dann Dateinamen im Workflow anpassen oder die ZIP selbst als Release-Asset ablegen.
- TODO FFmpeg-Lizenztext für macOS: Der Windows-Installer legt `FFmpeg-LICENSE.txt` bei, das macOS-Bundle noch nicht.
- Der Windows-Plattformcode wurde bisher nur mit `x86_64-pc-windows-gnu` übersetzt. Die Pipeline baut erstmals mit MSVC. Schlägt dort etwas fehl, steht der Fehler im Log des Jobs „windows“.
