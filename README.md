# Orbo

Dateien umwandeln, ohne ein Programm zu öffnen: Datei ziehen, Taste halten, ein Ring erscheint um das Icon. Der Ring zeigt nur das, was mit **genau dieser Datei** sinnvoll ist. Datei auf ein Feld fallen lassen, fertig. Alles läuft lokal, ohne Cloud, ohne Konto, ohne Upload.

Läuft auf macOS und Windows.

## In einem Satz

> „Das Kontextmenü, das man nicht suchen muss: es kennt die Datei und bietet die 5 Dinge an, die man wirklich will."

## Inhalt

| Datei | Worum es geht |
|---|---|
| [docs/UI-UX-KONZEPT.md](docs/UI-UX-KONZEPT.md) | Der Ring im Detail, bessere Ideen, Auslöser, Gesten, Feedback |
| [docs/PROJEKTPLAN.md](docs/PROJEKTPLAN.md) | Architektur, Tech-Stack, Phasen mit Abnahmekriterien, Risiken |
| [docs/KONVERTIERUNGEN.md](docs/KONVERTIERUNGEN.md) | Katalog aller Umwandlungen und Aktionen, Engines, Lizenzen |
| [docs/RELEASE.md](docs/RELEASE.md) | Release erstellen, automatische Updates, Signaturschlüssel |
| [prototype/ring-prototype.html](prototype/ring-prototype.html) | Klickbarer Prototyp des Rings im Browser |

## Prototyp ansehen

```bash
open prototype/ring-prototype.html
```

Eine Datei auf dem simulierten Desktop ziehen, dann **⌥ / Alt** halten oder die Datei kurz **schütteln**. Mit dem Schalter oben rechts erscheint der Ring auch ohne Taste.

## Name und Marke (18.10.2026)

- Name: **Orbo** (Arbeitstitel war Orbit, danach kurz Rondo). Orbit ist als Mac-Datei-App belegt, RONDO ist beim DPMA als Wort-/Bildmarke für Computersoftware (Kl. 9/42, Prüftech GmbH, bis 2032) eingetragen. ORBO: DPMA und EUIPO in Kl. 9/42 ohne Treffer (geprüft 18.10.2026), Websuche nur Orbo.ai (Indien, Bild-KI, B2B).
- Wortmarke: „orb◉“, Nunito Extrabold, letztes O als Ring aus sechs Farbsegmenten (`core/crates/orbo-app/ui/shared.css`, `.wordmark`).
- App-Icon: dunkles Squircle mit Farbring und Datei (Higgsfield, Entwurf A), `core/crates/orbo-app/icons/`.
- Schriften: UI = Systemschrift, Ring und Kacheln = SF Pro Rounded, Marke und große Überschriften = Nunito (SIL OFL).

Projektordner `Projects/Orbo`, Crates `orbo-core`, `orbo-cli` (Befehl `orbo`), `orbo-app`.

## Kommandozeile

Die App bringt das Kommandozeilen-Tool `orbo` mit. In den Einstellungen (Allgemein, „Kommandozeile und KI-Programme“) richtest du es mit einem Klick ein. Auf dem Mac legt Orbo dafür die Verknüpfung `~/.local/bin/orbo` an und zeigt dir die Zeile für `~/.zprofile`, falls der Ordner noch nicht im PATH liegt. Unter Windows kommt der Ordner mit dem Tool in den Benutzer-PATH. Danach funktioniert in einem neuen Terminal:

```bash
orbo targets                          # alle Ziele, auch eigene Abläufe und aktivierte KI-Ziele
orbo analyze foto.heic                # Format und Eigenschaften
orbo suggest video.mov --explain      # was der Ring anbieten würde
orbo convert foto.heic --to jpg --quality 85
orbo convert *.png --to webp --out ~/Desktop/web
orbo convert clip.mov --to mail_video
orbo convert bericht.pdf --to chain_mein_ablauf
orbo convert "Seite 1.pdf" "Seite 2.pdf" "Seite 10.pdf" --to pdf_merge   # eine PDF aus allen
orbo convert scans/*.jpg --to images_pdf --preset "A4 hoch"
orbo convert teil1.mp3 teil2.mp3 --to audio_concat
```

Ziele mit „(mehrere Dateien)“ in `orbo targets` machen aus mindestens zwei Dateien eine: `pdf_merge` (PDFs, Bilder werden Seiten), `images_pdf` (ein Bild pro Seite, Voreinstellungen Originalgröße, A4 hoch, A4 quer) und `audio_concat` (Audiodateien aneinanderhängen, Voreinstellungen MP3, M4A, WAV). Die Reihenfolge ergibt sich immer aus dem Dateinamen in natürlicher Sortierung („Seite 2“ vor „Seite 10“). Das Ergebnis liegt neben der ersten Datei und heißt „<erste Datei> (zusammengeführt).<Endung>“; Bestehendes wird nie überschrieben. Im Ring erscheinen sie, sobald du mehrere passende Dateien gleichzeitig ziehst.

Das Tool liest die Einstellungen der App (Abläufe und KI-Ziele). Mit `--no-settings` ignoriert es sie. Bestehende Dateien überschreibt es nie.

### MCP-Server für Claude und Codex

`orbo mcp` startet einen MCP-Server über stdio. Claude Code, Claude Desktop und Codex können damit Dateien analysieren und umwandeln lassen. In den Einstellungen gibt es dafür je einen Knopf („In Claude Code einrichten“, „In Claude Desktop einrichten“, „In Codex einrichten“), jeweils mit „Entfernen“. Orbo trägt dabei nur den Eintrag `orbo` ein und lässt alles andere in den Konfigurationsdateien unverändert; von der Datei wird vorher eine Sicherung `.bak` angelegt.

Werkzeuge:

| Name | Zweck |
|---|---|
| `orbo_suggest(paths)` | Analyse und Vorschläge: empfohlenes Ziel, bis zu sechs Ring-Vorschläge, alle kompatiblen Ziele; bei mehreren Pfaden zusätzlich `merge_targets` (Dateien zu einer zusammenführen) |
| `orbo_list_targets(kind?, format?)` | Ziele, optional nach Dateiart oder Eingabeformat gefiltert |
| `orbo_convert(paths, target, preset?, quality?, max_edge?, out_dir?)` | Umwandlung nacheinander, Ergebnis neben dem Original, nie überschreibend; Merge-Ziele (`pdf_merge`, `images_pdf`, `audio_concat`) machen aus allen Pfaden eine Datei |
| `orbo_analyze(path)` | Format und Eigenschaften einer Datei |

Manuell einrichten, falls du die Knöpfe nicht nutzen willst (Pfad zur eingebauten `orbo` anpassen):

```bash
claude mcp add --scope user orbo -- "/Applications/Orbo.app/Contents/MacOS/orbo-cli" mcp
```

```toml
# ~/.codex/config.toml
[mcp_servers.orbo]
command = "/Applications/Orbo.app/Contents/MacOS/orbo-cli"
args = ["mcp"]
```

Es gibt keine Werkzeuge zum Löschen und keine freie Shell. Pfade müssen absolut sein und existieren.

## Modell

Orbo ist kostenlos, ohne Werbung, ohne Konto. Freiwillige Unterstützung über „Orbo unterstützen“ in den Einstellungen (Über). Hintergrund und verworfenes Pro-Modell: [docs/LIZENZEN.md](docs/LIZENZEN.md). Vergleichbares Projekt: Filee (filee.sh, MIT, Windows-Schwerpunkt).
