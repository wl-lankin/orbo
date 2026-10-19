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

## Modell

Orbo ist kostenlos, ohne Werbung, ohne Konto. Freiwillige Unterstützung über „Orbo unterstützen“ in den Einstellungen (Über). Hintergrund und verworfenes Pro-Modell: [docs/LIZENZEN.md](docs/LIZENZEN.md). Vergleichbares Projekt: Filee (filee.sh, MIT, Windows-Schwerpunkt).
