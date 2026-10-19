# Orbit-Kern (Phase 1)

Rust-Workspace mit dem plattformunabhängigen Kern und einer CLI zum Testen. Die späteren Apps (Mac, Windows) nutzen genau diesen Kern.

```
crates/orbo-core   Erkennung, Analyse, Katalog, Regeln, Warteschlange, Engines
crates/orbo-cli    `orbit` zum Ausprobieren
```

## Ausprobieren

```bash
cargo run -p orbo-cli -- targets                      # alle 83 Ziele
cargo run -p orbo-cli -- suggest Video.mov --explain  # was der Ring zeigen würde
cargo run -p orbo-cli -- convert Video.mov --to mp3
cargo run -p orbo-cli -- convert Foto.png --to crop --crop 0,0,800,800
cargo run -p orbo-cli -- convert Screenshot.png --to beautify --background ocean
cargo run -p orbo-cli -- convert Clip.mp4 --to trim_video --start 3 --end 12
```

## Aufbau

| Teil | Datei | Aufgabe |
|---|---|---|
| Erkennung | `src/format.rs` | 70+ Formate am Inhalt erkennen, falsche Endungen melden |
| Analyse | `src/analyze.rs` | Bildgröße, Transparenz, Animation, GPS, Kamera; Codecs, Dauer, Spuren (ffprobe) |
| Katalog | `catalog/targets.yaml` | alle Ziele mit Engine, Parametern, Vorlagen, Reglern fürs Optionsfenster |
| Regeln | `catalog/rules.yaml` | welche 6 Ziele der Ring zeigt, was in die Mitte kommt |
| Warteschlange | `src/jobs.rs` | parallele Jobs, Fortschritt, Abbrechen, nie überschreiben |
| Engines | `src/engines/` | `image` (reines Rust, HEIC am Mac über `sips`), `media` (ffmpeg, Hardware-Encoder zuerst, Umverpacken wenn möglich), `data` (CSV/Excel/JSON/YAML/TOML/Untertitel), `archive` (ZIP/TAR.GZ, Schutz vor Zip-Slip) |

## Tests

`cargo test --workspace`: 58 Tests (Kern 7, Bild 12, Medien 15, Daten 16, Archiv 8). Medien-Tests werden übersprungen, wenn kein ffmpeg da ist. CI für Mac und Windows: `.github/workflows/core.yml`.

## Abweichungen vom Plan

- **Bilder in reinem Rust** (`image`, `webp`, `ravif`, `resvg`) statt libvips: keine nativen Bibliotheken zum Mitliefern, Cross-Build für Windows ohne Zusatzaufwand.
- **Größenvorgaben** (Mail, WhatsApp, Discord) nutzen beim Video bevorzugt libx264, weil Hardware-Encoder die Grenze oft reißen. Das mitgelieferte LGPL-ffmpeg hat kein libx264, dann greift ein Hardware-Encoder mit gedeckelter Bitrate. Vor dem Release prüfen.

## Offen

- HEIC/AVIF lesen unter Windows (WIC) fehlt noch, am Mac geht es über das System.
- PNG-Kompression nur über bessere Deflate-Stufe, ohne Farbpalette (Quantisierung folgt).
- Dokumente (PDF, Word, EPUB, OCR) sind Phase 4.
