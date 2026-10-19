# Orbo-App: Schnittstelle Rust ↔ Oberfläche

Alle Fenster sind statische HTML-Seiten in `ui/` (kein Bundler). Tauri ist global verfügbar (`window.__TAURI__`):

```js
const { invoke } = window.__TAURI__.core;
const { listen, emit } = window.__TAURI__.event;
const { getCurrentWebview } = window.__TAURI__.webview;
const { convertFileSrc } = window.__TAURI__.core;   // lokale Datei als <img>/<video>-Quelle
```

## Fenster

| Label | Datei | Art |
|---|---|---|
| `ring` | `ui/ring.html` | transparent, rahmenlos, 520×520 pt, über allem, aktiviert die App nicht. Hinter dem Ring liegt **nativ** ein runder Hintergrund (Radius 180 pt um die Fenstermitte): Klassisch = dunkles HUD-Vibrancy, Glas = macOS Liquid Glass. Die Seite selbst hat **transparenten** Hintergrund. |
| `toast` | `ui/toast.html` | transparent, rahmenlos, 460×260 pt, unten mittig über dem Dock, aktiviert die App nicht, Klicks gehen. |
| `dialog` | `ui/dialog.html` | normales Fenster mit Vibrancy-Hintergrund, Titelleiste transparent (Ampel sichtbar, kein Titel), 760×620 pt. |
| `settings` | `ui/settings.html` | wie `dialog`, 720×520 pt. |

## Ring (`ring.html`)

Ablauf: Benutzer zieht eine Datei und drückt die Taste (oder schüttelt) → Rust positioniert das Fenster so, dass seine Mitte am Mauszeiger liegt (am Bildschirmrand verschoben) → Event `ring:open` → die Seite baut den Ring mit Animation auf. Ab jetzt liefert Tauri die Drag-Ereignisse über dem Fenster an die Seite.

**Event `ring:open`** (Rust → Seite):
```json
{
  "files": ["/Users/x/Bild.png"],          // kann leer sein, dann kommen die Pfade mit dem ersten Drag-Ereignis
  "suggestions": { ... wie suggest() ... } , // oder null, wenn files leer
  "style": "classic" | "glass",
  "sound": true,
  "reduce_motion": false
}
```
**Event `ring:close`** (Rust → Seite): `{ "reason": "released" | "cancelled" }`. Rust blendet das Fenster nach ca. 200 ms selbst aus, die Seite spielt nur die Ausblend-Animation.

**Drag-Ereignisse** (Tauri): `getCurrentWebview().onDragDropEvent(e => ...)`, `e.payload.type` ist `enter` (mit `paths`, `position`), `over` (`position`), `drop` (`paths`, `position`), `leave`. `position` ist in **physischen** Pixeln relativ zum Fenster, also durch `window.devicePixelRatio` teilen.

Beim Drop entscheidet die Seite anhand des getroffenen Felds:
- Ziel mit `dialog: true` → `invoke('open_dialog', { paths, target, preset })`
- sonst → `invoke('convert', { paths, target, preset, options: {} })`
- danach immer `invoke('ring_hide')`. Drop außerhalb eines Felds: nur `ring_hide`.

**Mausrad** über einem Feld mit Regler `quality`: Qualität 10 bis 100 in 5er-Schritten, Größe in der Mitte live schätzen (Schätzung: `size_estimate` aus `suggest`, skaliert mit Qualität). Beim Drop als `options.quality` mitgeben.

## Befehle (`invoke`)

| Befehl | Argumente | Rückgabe |
|---|---|---|
| `suggest` | `{ paths: string[] }` | `Suggest` (unten) |
| `convert` | `{ paths, target, preset?: string, options?: Options }` | `number[]` Job-IDs |
| `cancel_job` | `{ id }` | – |
| `ring_hide` | – | – |
| `open_dialog` | `{ paths, target, preset? }` | – |
| `dialog_context` | – | `DialogContext` (unten), für das Dialogfenster |
| `dialog_close` | – | – |
| `reveal` | `{ path }` | zeigt die Datei im Finder |
| `undo` | `{ path }` | legt die Datei in den Papierkorb |
| `toast_hide` | – | – |
| `settings_get` | – | `Settings` |
| `settings_set` | `{ settings: Settings }` | – (sendet Event `settings` an alle Fenster) |
| `open_settings` | – | – |
| `catalog` | – | alle Ziele mit `id, title, sub, group, controls, dialog, presets[]` |

**`Suggest`**:
```json
{
  "files": [{ "path": "...", "name": "Bild.png", "format": "png", "kind": "image", "size": 182345,
              "meta": "2400 × 1600 · Transparenz", "width": 2400, "height": 1600, "duration_s": null }],
  "default": "webp",
  "ring": [ { "id": "webp", "title": "WEBP", "sub": "behält Transparenz", "ext": "webp", "intent": false,
              "group": "Bild", "kind": "image", "dialog": false, "controls": ["quality","resize"],
              "presets": ["Web","Ausgewogen","Verlustfrei"], "size_estimate": 21034 } ],
  "more": [ ... gleiche Form ... ]
}
```
`kind` eines Ziels ist die Art des Ergebnisses (image, video, audio, data, archive, folder) und bestimmt die Farbe. `size_estimate` ist eine grobe Schätzung in Bytes oder `null`.

**`Options`** (alle Felder optional, Namen wie im Kern): `quality, max_edge, width, height, crop {x,y,width,height}, aspect, rotate, flip_horizontal, start_s, end_s, target_mb, audio_bitrate_k, sample_rate, channels, fps, background, padding_px, corner_px, shadow_px`.

**`DialogContext`**: `{ paths, target: { id, title, sub, controls, presets }, file: <wie Suggest.files[0]>, preview: <convertFileSrc-URL oder null>, preset }`. Für Bilder ist `preview` die Datei selbst (HEIC wird vorher in ein PNG gewandelt), für Video/Audio die Datei für `<video>`/`<audio>`.

**`Settings`**:
```json
{
  "style": "classic", "trigger_key": "option", "shake": true, "notch": false, "sound": false, "reduce_motion": false,
  "output": { "mode": "beside", "folder": null, "naming": "suffix" },
  "ollama": { "enabled": false, "url": "http://localhost:11434", "model": null,
              "actions": ["rename", "alt_text", "summary"] }
}
```
`trigger_key`: `option | control | shift | command | fn`.

## Job-Ereignisse (Rust → alle Fenster)

Event `job` mit `payload`:
```json
{ "event": "started",  "id": 3, "input": "/…/Video.mov", "target": "mp4", "title": "MP4", "count": 1 }
{ "event": "progress", "id": 3, "fraction": 0.42 }
{ "event": "finished", "id": 3, "output": "/…/Video.mp4", "bytes": 1234567, "millis": 812, "input_bytes": 9876543 }
{ "event": "failed",   "id": 3, "error": "…" }
{ "event": "cancelled","id": 3 }
```
`count` ist 1, bei einer Zusammenführung die Zahl der Dateien (`input` ist dann die erste Datei, `input_bytes` die Summe). Rust blendet das Toast-Fenster bei `started` ein. Die Seite ruft `toast_hide`, wenn nichts mehr anzuzeigen ist.

## Gestaltung (für alle Seiten)

- Farben pro Art: image `#b197fc`, video `#ff8094`, audio `#5be08f`, document `#74b0ff`, data `#ffd84d`, archive `#ffb878`, folder `#9cc8ff`, intent: Farbe der Quelle.
- Schrift: `-apple-system, "SF Pro Text", system-ui`. Texte deutsch mit echten Umlauten, keine Gedankenstriche.
- Zwei Stile, Klasse am `<body>`: `style-classic` (dunkel, kontrastreich) und `style-glass` (hell-transparente Flächen, Spiegelkante). Umschalten live über Event `settings`.
- `prefers-reduced-motion` und Einstellung `reduce_motion` respektieren.

## Ergänzungen Phase 2 (Einführung, Notch, Optionen)

### Neue Fenster

| Label | Datei | Art |
|---|---|---|
| `onboarding` | `ui/onboarding.html` | wie `settings`, 720×560 pt, erscheint beim ersten Start und über Einstellungen → Allgemein → „Einführung zeigen“ |
| `notch` | `ui/notch.html` | transparent, rahmenlos, 720×180 pt, oben mittig am Bildschirmrand, aktiviert die App nicht |

### Neue Befehle

| Befehl | Argumente | Rückgabe |
|---|---|---|
| `request_folder_access` | – | `{ desktop: bool, documents: bool, downloads: bool }`. Liest die drei Ordner einmal, dadurch fragt macOS nach der Freigabe. Erneut aufrufen zeigt den aktuellen Stand. |
| `open_privacy_settings` | – | öffnet Systemeinstellungen → Datenschutz → Dateien und Ordner |
| `onboarding_sample` | – | Pfad einer Beispieldatei (`Orbo Beispiel.png`), die Rust anlegt und im Finder zeigt |
| `onboarding_done` | – | merkt sich, dass die Einführung fertig ist, schließt das Fenster |
| `open_onboarding` | – | öffnet die Einführung erneut |
| `notch_hide` | – | blendet das Notch-Fenster aus |

### Neue Einstellungen

`Settings.autostart: bool` (Bei Anmeldung starten, Rust trägt Orbo als Anmeldeobjekt ein bzw. aus), `Settings.onboarded: bool`.

### Notch-Portal (`notch.html`)

Wenn während eines Datei-Drags (ohne Taste) der Mauszeiger oben in die Mitte des Bildschirms kommt (näher als 60 pt am oberen Rand, höchstens 260 pt links/rechts der Mitte) und `Settings.notch` an ist, zeigt Rust das Fenster und sendet **`notch:open`** mit derselben Nutzlast wie `ring:open`. Verlässt der Zeiger den Bereich (mehr als 200 pt unter dem Rand) oder wird losgelassen: **`notch:close`** `{ reason }`, Rust blendet nach 250 ms aus. Drag-Ereignisse kommen wie beim Ring über `onDragDropEvent` (auf macOS Punkte, nicht teilen). Drop-Logik wie beim Ring, danach `notch_hide`.

Gestaltung: oben bündig eine schwarze Form wie die MacBook-Notch (200×32 pt im Ruhezustand), die wie Dynamic Island zu einem breiten, abgerundeten Panel aufklappt (Feder-Animation) und die bis zu 6 Ziele als Kacheln zeigt (Titel, Untertitel, Größe, Farbe nach Art), Hover hebt die Kachel an.

### Ring-Ergänzungen

- **„…“ im Vorlagen-Bogen:** Für Ziele mit nicht leerem `controls` und `dialog: false` hängt der Bogen hinter die Vorlagen ein zusätzliches Feld „…“ (Untertitel „Optionen“) an. Drop darauf → `open_dialog({ paths, target })`. Ziele ohne Vorlagen, aber mit `controls`, bekommen einen Bogen nur mit „…“.
- **Fehlende Ordnerfreigabe:** `Suggest.files[i].readable === false` → die Linse zeigt im Ruhezustand unter dem Dateinamen klein „Ordnerfreigabe fehlt“ (Farbe warn). Die Felder funktionieren trotzdem, macOS fragt beim Umwandeln nach.

## Lizenz: Gratis, Testphase, Orbo Pro (entfernt am 18.10.2026)

> Orbo ist kostenlos. Die folgenden Befehle und Felder gibt es nicht mehr; neu ist nur `support` (öffnet die Unterstützen-Seite).

Modell: Gratis für alle Format-Umwandlungen. Pro (Einmalkauf, 1 Jahr Updates inklusive) für Werkzeuge mit Optionen, Vorlagen für Plattformen, PDF- und Audio-Werkzeuge, mehrere Dateien gleichzeitig, KI. Beim ersten Start 14 Tage Testphase mit allen Pro-Funktionen. Lizenzschlüssel werden offline geprüft (Ed25519), kein Konto, kein Server.

### Daten

**`LicenseStatus`**:
```json
{ "state": "free" | "trial" | "pro" | "updates_expired",
  "entitled": true,                 // Pro-Funktionen nutzbar (pro oder trial)
  "trial_days_left": 9,             // nur bei trial sinnvoll, sonst 0
  "trial_total": 14,
  "name": "Wolfgang Linz", "email": "...", "updates_until": "2027-10-18",   // nur mit Lizenz
  "key_hint": "ORBO-…Xk3f",         // gekürzte Anzeige
  "release_date": "2026-10-18",     // Datum dieses Builds
  "buy_url": "https://…",
  "price": "24,99 €" }
```
`updates_expired`: Lizenz echt, aber dieser Build ist neuer als `updates_until`. Pro ist dann aus; Hinweis „Update-Zeitraum verlängern“.

`Suggest.ring[i]` und `more[i]` bekommen zusätzlich `pro: bool` (Pro-Funktion) und `locked: bool` (Pro-Funktion und aktuell nicht freigeschaltet).

### Befehle

| Befehl | Argumente | Rückgabe |
|---|---|---|
| `license_status` | – | `LicenseStatus` |
| `license_activate` | `{ key }` | `LicenseStatus` oder Fehlertext (deutsch) |
| `license_remove` | – | `LicenseStatus` |
| `open_upgrade` | `{ feature?: string }` | öffnet das Fenster `upgrade` (ui/upgrade.html, 560×620, Stil wie settings) |
| `upgrade_context` | – | `{ feature: string \| null, status: LicenseStatus }` für das Upgrade-Fenster |
| `buy` | – | öffnet die Kaufseite im Browser |

Event `license` mit `LicenseStatus` an alle Fenster, wenn sich etwas ändert.

`convert` und `open_dialog` geben den Fehler `"pro"` zurück, wenn eine gesperrte Funktion genutzt wird (Pro-Ziel, Optionsfenster für normale Formate, mehrere Dateien auf einmal); Rust öffnet dann selbst das Upgrade-Fenster. Die Oberfläche soll den Fehler `"pro"` still schlucken.

### Oberfläche

- **Ring und Notch:** `locked`-Ziele normal anzeigen, aber mit kleinem „PRO“-Abzeichen (Pill, Kategorie-Farbe gedämpft, Schloss-Symbol optional) und leicht gedämpft. In der Linse beim Hover: Untertitel „Orbo Pro“. Drop darauf geht wie immer an `convert`/`open_dialog` (Rust leitet ins Upgrade). Das „…“-Optionsfeld ist ebenfalls gesperrt, wenn `entitled` false ist.
- **Einstellungen:** neuer Bereich **„Orbo Pro“** (eigenes Symbol, ganz oben in der Sidebar) mit Status-Karte (Gratis / Testphase noch N Tage mit Fortschrittsbalken / Pro für Name, Updates bis Datum), Schlüsselfeld + „Aktivieren“, „Lizenz entfernen“, Kaufen-Knopf mit Preis, Liste der Pro-Funktionen.
- **Upgrade-Fenster `upgrade.html`:** hochwertig, verkaufsstark aber nicht aufdringlich: Wortmarke „orb◉“ + „Pro“, Satz zur gesperrten Funktion („Schick ist Teil von Orbo Pro“), Vergleich Gratis/Pro in 6 bis 8 Zeilen, Preis groß „24,99 € einmalig, 1 Jahr Updates inklusive“, primär „Orbo Pro kaufen“ (`buy`), sekundär „Lizenzschlüssel eingeben“ (klappt Feld auf, `license_activate`, Erfolg mit Häkchen-Animation und schließt nach 1,2 s via `window.close()`), Hinweis „Kein Abo. Alles läuft lokal.“
- **Einführung, Schritt 4:** kleine Zeile/Chip „14 Tage alle Pro-Funktionen gratis testen“.

## Rechtsklick-Eintrag und Klick-Ring

Der Ring lässt sich auch ohne Ziehen öffnen: über den Eintrag „Mit Orbo umwandeln“ im Finder (macOS, Schnellaktion) bzw. Explorer (Windows). Die Einrichtung geschieht nur auf Knopfdruck in den Einstellungen (Allgemein).

### Aufruf

- `orbo://open?f=<Pfad>&f=<Pfad>` (Pfade URL-codiert, absolut). Schema in `tauri.conf.json` unter `plugins.deep-link`.
- `Orbo.exe --open <Pfad>...` (Explorer-Eintrag oder zweiter Start, den `tauri-plugin-single-instance` an die laufende Instanz weitergibt). Pfade, die innerhalb von 300 ms eintreffen, landen in einem Ring (Mehrfachauswahl im Explorer startet einen Prozess pro Datei).

Rust öffnet den Ring mit der Mitte am Mauszeiger (gleiche Platzierung wie beim Ziehen) und sendet `ring:open` mit **`mode: "click"`** (beim Ziehen `"drag"`; fehlt das Feld, gilt `"drag"`). Die übrigen Felder sind unverändert. Wie beim Ziehen kommt `ring:open` zweimal, zuerst mit `suggestions: null`.

### Klick-Modus (`mode: "click"`)

Es läuft kein Drag, das Fenster bekommt Mausereignisse nicht verlässlich (aktiviert die App nicht). Rust fragt deshalb Zeiger, linke Taste und Esc alle 10 ms ab und sendet:

| Event | Nutzlast | Bedeutung |
|---|---|---|
| `ring:pointer` | `{ x, y }` | Zeiger in CSS-Pixeln relativ zur linken oberen Fensterecke; Seite ruft dieselbe Hover-Logik wie bei `over` |
| `ring:click` | `{ x, y }` | Linke Taste gedrückt innerhalb des Rings; Treffer auf Feld, Vorlage oder Mitte führt wie `drop` zu `convert` bzw. `open_dialog` mit den Pfaden aus `ring:open`, danach `ring_hide`. Klick ins Leere im Ring wird ignoriert |

Ein Klick weiter als 260 px (`R.pout + 16`) von der Mitte, Esc oder Fensterfokusverlust schließt den Ring (Rust: `ring:close` mit `reason: "dismissed"`). Solange ein Klick-Ring offen ist, ignoriert Rust die Signale `released` und `cancelled` des Drag-Beobachters. Mausrad funktioniert wie beim Ziehen. `ring_hide` beendet den Klick-Modus.

### Befehle

| Befehl | Argumente | Rückgabe |
|---|---|---|
| `context_menu_status` | – | `{ supported: bool, installed: bool, kind: "finder" \| "explorer" }` |
| `context_menu_install` | – | neuer Status, oder Fehlertext. macOS: schreibt `~/Library/Services/Mit Orbo umwandeln.workflow` (Vorlage in `resources/macos/`) und ruft `pbs -update` auf. Windows: schreibt `HKCU\Software\Classes\*\shell\Orbo` und `...\Directory\shell\Orbo` |
| `context_menu_remove` | – | neuer Status; entfernt Workflow bzw. Registrierungsschlüssel |

## Kommandozeile und KI-Programme

Wird nur durch Klick in den Einstellungen (Allgemein) ausgelöst, nie beim Start. `id` ist `cli`, `claude_code`, `claude_desktop` oder `codex`. Alle drei Befehle laufen asynchron (Login-Shell, Kindprozesse, Dateizugriffe).

| Befehl | Argumente | Rückgabe |
|---|---|---|
| `integration_status` | `{ id }` | `{ id, supported, available, installed, stale, manual?, note? }`. `available`: das mitgelieferte Tool wurde neben der App gefunden. `stale`: eingerichtet, zeigt aber auf einen anderen Pfad. `manual`: Text zum Kopieren (Terminalbefehl bzw. PATH-Zeile), `note`: Erklärung dazu |
| `integration_install` | `{ id }` | neuer Status, oder Fehlertext. `cli`: macOS Verknüpfung `~/.local/bin/orbo` (prüft per Login-Shell, ob der Ordner im PATH liegt, und liefert sonst die Zeile in `manual`), Windows Eintrag `<Installationsordner>\cli` in `HKCU\Environment\Path`. `claude_code`: `claude mcp add --scope user`, sonst Befehl in `manual`. `claude_desktop`: `mcpServers.orbo` in `claude_desktop_config.json`. `codex`: `[mcp_servers.orbo]` in `~/.codex/config.toml`. Bei den Dateien zuerst Sicherung `.bak` |
| `integration_remove` | `{ id }` | neuer Status; entfernt nur den Eintrag von Orbo |

## Überwachte Ordner

Neue Dateien in überwachten Ordnern wandelt Rust automatisch um (`src/watch.rs`, Logik in `orbo-core/src/watch_rules.rs`, Mechanismus: `notify`, FSEvents bzw. ReadDirectoryChangesW).

### Einstellungen

`Settings.watch: WatchRule[]` (Standard `[]`, fehlt das Feld in einer alten `settings.json`, bleibt die Liste leer):
```json
{ "id": "lq3k9x2a", "folder": "/Users/x/Bildschirmfotos", "target": "webp", "preset": "Web", "include_subfolders": false, "enabled": true }
```
`target` ist eine Katalog-ID, `preset` der Name einer Vorlage des Ziels oder `null`. `id` vergibt die Oberfläche. Rust baut die Beobachter bei jedem `settings_set` und beim Start neu auf (nur, wenn sich Regeln oder Ausgabe-Einstellungen geändert haben).

### Befehle

| Befehl | Argumente | Rückgabe |
|---|---|---|
| `watch_targets` | – | Ziele, die eine Regel nutzen kann (ohne KI und ohne „Ordner wählen …“): `[{ id, title, sub, group, presets: string[] }]`, in Katalogreihenfolge, die Oberfläche gruppiert nach `group` |
| `watch_status` | – | je Regel `[{ id, exists, readable, protected }]`. `protected`: unter Schreibtisch, Dokumente oder Downloads (nur macOS). Das Lesen löst beim ersten Mal die Ordnerfreigabe-Abfrage aus. Bei `protected && !readable` bietet die Oberfläche `request_folder_access` und `open_privacy_settings` an. |

### Verhalten

- Reagiert auf neu angelegte oder hineinverschobene Dateien. Ignoriert werden Ordner, versteckte Dateien, `.crdownload`, `.part`, `.download`, `.tmp`, `.partial`, `~$`-Dateien, `.orbo-*`, Dateien mit dem Suffix des Ziels und alles, was Orbo selbst erzeugt hat (Merkliste aus den `finished`-Ereignissen, auch für Ordner-Ergebnisse).
- Wartet, bis die Dateigröße 1,5 s unverändert ist (Abfrage alle 0,5 s, höchstens 10 min), prüft dann wie der Ring mit `Target::accepts`, ob das Ziel passt, und reicht den Auftrag über dieselben Ausgabe-Einstellungen ein wie `convert` (`commands::apply_output_settings`, `commands::submit_jobs`). Das Toast zeigt ihn wie jeden anderen Auftrag.
- Pausiert: nichts wird umgewandelt. Liegt der Ausgabeordner in einem überwachten Ordner, bleibt er außen vor.

## Abläufe (verkettete Umwandlungen)

Ein Ablauf führt mehrere eingebaute Ziele nacheinander aus und liefert nur das Endergebnis. Zwischendateien liegen in einem privaten Ordner (`.orbo-chain-…`) neben der Ausgabe und werden immer gelöscht.

### Einstellungen

`settings.chains`: `[{ id, name, steps: [{ target, preset: string | null }], in_ring: bool }]`, Standard leer. Kern und App kennen sie über `orbo_core::chains::configure(Vec<Chain>)`, aufgerufen beim Start und bei jedem `settings_set` (wie die KI-Einstellungen).

### Katalog

Jeder gültige Ablauf wird ein Ziel mit der ID `chain_<id>`: `title` = Name, `sub` = Kurzfassung der Schritte, `group` = „Abläufe“, `engine` = „chain“, `accepts` = Eingaben des ersten Schritts, `requires` = Vereinigung der Anforderungen aller Schritte, Ausgabe = Ergebnis des letzten Schritts. Ungültige Abläufe (unbekannter Schritt, unpassende Formatfolge) werden nicht angeboten. `Catalog::current()` liefert Katalog plus Abläufe, `Catalog::builtin()` bleibt unverändert.

Ein Ablauf wird für eine Datei nur angeboten, wenn Schritt 1 sie annimmt und jeder weitere Schritt das Ausgabeformat des vorigen annimmt (formatbasiert, ohne Dateizugriff). Schritte, die einen Ordner erzeugen, sind nur als letzter Schritt erlaubt. „Ordner wählen …“-Voreinstellungen sind nicht erlaubt. KI-Schritte sind erlaubt; der Ablauf trägt dann `ai: true` und ist als Ordnerziel ausgeschlossen.

### Ziel-Flag

In `suggest` (Ring und „Mehr“) und `watch_targets` tragen Ziele zusätzlich `chain: bool`. Die Oberfläche zeigt Abläufe mit dem Symbol `#i-chain` (zwei verkettete Ringe) in der Farbe der Endausgabe (`kind`). Mit `in_ring: true` belegt ein Ablauf nur freie Ringplätze (wie `fill` in den Regeln); sonst steht er unter „Mehr“.

### Befehle

| Befehl | Parameter | Ergebnis |
|---|---|---|
| `chain_validate` | `steps: [{ target, preset }]` | `{ ok, error, error_step, inputs }`. `error` ist ein deutscher Text, `error_step` der 0-basierte Index, `inputs` die Eingabeformate, für die der Ablauf funktioniert |
| `chain_candidates` | `steps` | Ziele, die als nächster Schritt passen: `[{ id, title, sub, group, kind, ai, presets }]`. Ohne Schritte alle Ziele, die einen Ablauf beginnen können |
| `chain_step_info` | – | Alle eingebauten Ziele im selben Format, zum Beschriften gespeicherter Schritte |

`convert` und die Überwachung nehmen `chain_<id>` wie jedes andere Ziel. Fortschritt ist (Schrittnummer + Schrittfortschritt) / Schrittzahl. Fehler nennen den Schritt: „Schritt 2 (WEBP): …“.

## Zusammenführen (mehrere Dateien zu einer)

Ziele mit `merge: true` (Gruppe „Zusammenführen“, `orbo-core/catalog/targets-merge.yaml`) machen aus mindestens zwei Dateien EINE:

| ID | Titel | Eingaben | Ergebnis |
|---|---|---|---|
| `pdf_merge` | Zusammenführen | PDFs, Bilder dazwischen werden Seiten (mindestens eine PDF nötig) | eine PDF |
| `images_pdf` | Als ein PDF | Bilder (alle lesbaren Formate, HEIC über `sips`) | eine PDF, ein Bild pro Seite; Voreinstellungen „Originalgröße“ (150 dpi), „A4 hoch“, „A4 quer“ (eingepasst mit 10 mm Rand) |
| `audio_concat` | Aneinanderhängen | Audiodateien (braucht ffmpeg) | eine Audiodatei im Format der ersten Datei (MP3, wenn dieses nicht kodiert wird); Voreinstellungen MP3, M4A, WAV |

- **Angebot**: `suggest` bekommt mehrere Pfade; Merge-Ziele erscheinen nur bei mindestens zwei Dateien, wenn das Ziel jede davon annimmt. Für eine einzelne Datei sind sie unsichtbar (`Target::accepts` ist `false`). Regeln mit `multi: true` setzen sie in die Mitte des Rings. Zielobjekte tragen `merge: bool`; `sub` lautet dann „N Dateien zu einer“ (Anzeige in Ring und Linse), `size_estimate` ist `null`.
- **Reihenfolge**: immer natürliche Sortierung nach Dateiname (Groß/Klein egal, Zahlen nach Wert: „Seite 2“ vor „Seite 10“). Die Reihenfolge der Ablage zählt nicht.
- **Ergebnis**: neben der ersten Datei (nach Sortierung) oder in `out_dir`, Name = Name der ersten Datei + „ (zusammengeführt)“ + Endung, nie überschreibend („ 2“, „ 3“ …), über eine temporäre Datei und atomares Umbenennen.
- **`convert`**: bei einem Merge-Ziel reicht Rust EINEN Auftrag mit allen Pfaden ein (`JobQueue::submit_merge`) und liefert eine Auftrags-ID. Weniger als zwei Pfade ergeben den Fehler „… braucht mindestens zwei Dateien“. Das Toast zeigt eine Karte „3 Dateien → Zusammenführen“, nach dem Ende den Namen der Ergebnisdatei.
- **Ausgeschlossen**: überwachte Ordner (`watch_targets` liefert sie nicht, `Target::accepts` ist für Einzeldateien `false`) und Abläufe (`chain_validate` lehnt sie ab, `chain_candidates` führt sie nicht auf).
- **Grenzen der PDF-Zusammenführung**: Lesezeichen, benannte Ziele und Formularfelder der Quellen entfallen; Seiten, Links und Anmerkungen bleiben. Passwortgeschützte PDFs werden abgelehnt (Fehler nennt die Datei). JPEGs (Grau oder RGB, nicht per EXIF gedreht) kommen ohne Neukodierung in die PDF, alles andere als JPEG mit Qualität 90 (Transparenz wird weiß).
