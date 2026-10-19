# UI/UX-Konzept

## Leitgedanken

1. **Nicht suchen lassen.** Der Ring zeigt höchstens 6 Ziele. Welche, entscheidet die Datei selbst (Inhalt, nicht nur Endung).
2. **Absicht vor Format.** Viele Menschen denken nicht „HEIC nach JPG", sondern „das soll per Mail raus". Deshalb mischt der Ring Formate mit Absichten: *Für E-Mail*, *Text herausholen*, *Ton extrahieren*.
3. **Muskelgedächtnis.** Gleiche Ziele liegen immer in derselben Richtung. Nach ein paar Tagen zieht man blind nach „oben" für JPG, ohne hinzuschauen (Prinzip Marking Menu).
4. **Nie im Weg.** Ohne Auslöser verhält sich der Drag ganz normal. Lässt man außerhalb des Rings los, passiert nichts Ungewolltes.
5. **Nie zerstören.** Original bleibt immer. Neue Datei landet daneben. Rückgängig mit ⌘Z / Strg+Z.

---

## 1. Auslöser: wann erscheint der Ring?

Wichtige Erkenntnis: die üblichen Sondertasten haben beim Klicken schon eine Bedeutung (⌘/Strg-Klick wählt ab, Shift erweitert Auswahl, Ctrl-Klick am Mac öffnet das Kontextmenü). Deshalb wird die Taste **erst während des Ziehens** ausgewertet. So bleibt die Auswahl unangetastet.

| Auslöser | Standard | Warum |
|---|---|---|
| **Taste während des Ziehens** | ⌥ Option (Mac), Alt (Windows), frei belegbar | Deine Grundidee. Bewusst, schnell, keine Fehlauslösung. |
| **Schütteln** | an | Datei 2 bis 3 Mal kurz hin und her bewegen. Ganz ohne Tastatur, ideal am Trackpad. |
| **Kurz verweilen** | aus (optional) | Drag 600 ms stillhalten über leerer Fläche. Für Leute, die nichts lernen wollen. |
| **Hotkey ohne Drag** | ⌥⇧Leertaste / Alt+Shift+Leertaste | Datei im Finder/Explorer markieren, Hotkey drücken, Ring erscheint am Mauszeiger, Bedienung per Pfeiltasten oder Zahlen. Wichtig für Barrierefreiheit. |

---

## 2. Der Ring („Orbit")

```
                 ┌─────────┐
           ┌─────┤   JPG   ├─────┐
           │     │ ~620 KB │     │
      ┌────┴──┐  └─────────┘  ┌──┴────┐
      │ WEBP  │   ┌───────┐   │ Mail  │
      │-82 %  │   │ ▣ IMG │   │ <1 MB │
      └────┬──┘   │ HEIC  │   └──┬────┘
      ┌────┴──┐   │ 3,1MB │   ┌──┴────┐
      │  PNG  │   └───────┘   │  OCR  │
      │       │               │ Text  │
      └────┬──┘  ┌─────────┐  └──┬────┘
           │     │ GPS weg │     │
           └─────┤ Privat  ├─────┘
                 └─────────┘
```

### Aufbau

- **Zentrum:** Vorschau der Datei plus Steckbrief („HEIC · 4032×3024 · Live-Foto · 3,1 MB"). Loslassen im Zentrum = **Smart Default**, die wahrscheinlichste Aktion. Sie ist im Ring hervorgehoben.
- **Innerer Ring, 4 bis 6 Sektoren:** Formate und Absichten. Jedes Feld zeigt Name plus *einen* Nutzen in Klartext: „behält Transparenz", „halbe Größe", „⚡ ohne Neukodierung", „~620 KB".
- **Äußerer Bogen (Verweilen, 400 ms):** bleibt man auf einem Sektor stehen, fächern sich davor Voreinstellungen auf: *Web · Ausgewogen · Maximal* oder *1080p · 720p · Original*. Lässt man direkt los, gilt die zuletzt genutzte Einstellung.
- **Mausrad / Zwei-Finger-Scroll auf einem Sektor:** Qualität stufenlos ändern. Geschätzte Zielgröße zählt live mit („2,4 MB → 380 KB").
- **„Mehr …"-Sektor:** Datei dort ablegen öffnet eine kleine Befehlspalette mit Suchfeld (jetzt hat die App den Tastaturfokus). Dort ist der komplette Katalog, aber man muss ihn fast nie öffnen.

### Woher kommen die Vorschläge?

1. **Erkennung über Inhalt** (Magic Bytes plus Kurzanalyse), nicht über die Endung. Eine falsch benannte `.jpg`, die eigentlich PNG ist, wird richtig erkannt.
2. **Eigenschaften lesen:** Hat das Bild Transparenz? Ist das GIF animiert? Hat das PDF eine Textebene oder ist es ein Scan? Hat das Video Ton? Ist der Codec schon H.264 (dann reicht Umverpacken)?
3. **Regeln** daraus, Beispiele:
   - PNG **mit** Transparenz → WEBP/AVIF vorne, JPG mit Hinweis „weißer Hintergrund".
   - PDF **ohne** Textebene → „Durchsuchbar machen (OCR)" wird Smart Default.
   - Video > 25 MB → „Für E-Mail" erscheint.
   - Screenshot (erkannt an Metadaten/Name) → „Komprimieren" und „Text herausholen".
   - iPhone-Foto mit GPS → „Ortsdaten entfernen".
4. **Lernen, lokal:** Was man oft wählt, rückt in den Ring. Aber die **Positionen bleiben stabil**. Neue Favoriten ersetzen den schwächsten Platz, sie verschieben nicht alles.

### Mehrere Dateien

- Badge mit Anzahl am Zentrum.
- Gleicher Typ: normaler Ring, Aktion für alle.
- Gemischt: Ring zeigt Ziele, die für alle passen (z. B. *Als ZIP*, *Als PDF zusammenfügen*, *Metadaten entfernen*), plus „Nach Typ getrennt", das die Auswahl in kleine Unterringe aufteilt.
- Ordner ziehen = Stapelverarbeitung über den Inhalt.

### Gefühl und Feedback

- **Erscheinen:** Ring wächst in 120 ms aus dem Icon heraus (Federkurve), Sektoren erscheinen leicht versetzt im Uhrzeigersinn.
- **Hover:** Sektor hebt sich an, Glow in Kategoriefarbe, dezenter Klick-Sound optional, am Mac-Trackpad **haptisches Feedback** beim Wechsel zwischen Sektoren.
- **Ablegen:** Ring zieht sich zu einem Fortschrittskreis um das Original-Icon zusammen. Fertig: neue Datei „materialisiert" daneben, kleine Meldung „Urlaub.mp4 · 48 MB · −92 %" mit *Im Finder zeigen* und *Rückgängig*.
- **Längere Jobs:** laufen im Hintergrund weiter. Menüleisten-/Tray-Symbol zeigt einen Fortschrittsring. Benachrichtigung am Ende.
- **Abbrechen:** Esc oder weit vom Ring wegziehen. Ring fällt in sich zusammen, der Drag läuft normal weiter.

### Optionsfenster nach dem Ablegen

Manche Ziele brauchen eine Entscheidung, bevor es losgeht. Dann öffnet sich direkt an der Datei ein kleines Fenster mit Live-Vorschau (Vorbild: Crop- und „Add BG"-Dialoge moderner Screenshot-Tools):

| Ziel | Fenster zeigt |
|---|---|
| **Zuschneiden** | Bild mit Rahmen zum Ziehen, Seitenverhältnis Frei / 1:1 / 16:9 / 9:16 / 4:3 / 3:4, Breite und Höhe in px |
| **Schick** (Screenshot verschönern) | Vorschau, Hintergrund (Verläufe, Farben, eigenes Bild), Rand, Ecken, Schatten, Verhältnis Auto / 1:1 / 16:9 / 4:5 |
| **Kürzen** (Audio/Video) | Zeitleiste mit Wellenform bzw. Vorschaubildern, Start und Ende ziehen, Abspielen |
| **Größe** | Breite/Höhe mit Kette fürs Seitenverhältnis, Prozent, Vorlagen 1920/1280/800 |

- Welche Regler ein Ziel hat, steht im Katalog (`controls`), ob das Fenster sofort aufgeht, ebenfalls (`dialog: true`). Die Oberfläche baut das Fenster daraus, neue Ziele brauchen keinen neuen UI-Code.
- Normale Formate (JPG, MP3 …) laufen ohne Fenster sofort los. Wer doch Einstellungen will: auf dem Feld verweilen, im Vorlagen-Bogen erscheint zusätzlich „…“, das öffnet dasselbe Fenster mit Qualität, Auflösung, Bitrate usw.
- Unten im Fenster immer: *Zurücksetzen* links, *Speichern* rechts (Enter), Esc bricht ab.

### Ausgabe-Optionen (per Modifikator beim Loslassen)

| Beim Loslassen zusätzlich halten | Ergebnis |
|---|---|
| nichts | Neue Datei neben dem Original |
| ⇧ Shift | Ergebnis in die Zwischenablage (praktisch für Chat/Mail) |
| ⌘ / Strg | Original ersetzen (landet im Papierkorb, nicht gelöscht) |

---

## 3. Bessere oder ergänzende UI-Ideen

### A. „Notch-Portal" (Mac) / „Top-Portal" (Windows)
Datei Richtung Kamera-Notch ziehen, die Notch wächst wie Dynamic Island zu einer Leiste mit denselben Smart-Zielen. **Keine Taste nötig**, sehr auffindbar, sieht spektakulär aus. Unter Windows dasselbe als Lasche oben in der Bildschirmmitte. Gute Ergänzung zum Ring, weil man es zufällig entdeckt.

### B. Rezepte („Ketten")
Eigene Ziele aus mehreren Schritten: *Web-fertig* = auf 1920 px verkleinern + WEBP 80 % + Metadaten entfernen. Rezepte bekommen ein Symbol und einen festen Platz im Ring. Ideal für wiederkehrende Arbeit (Shop-Bilder, Social-Media-Videos).

### C. Magische Ordner
Jeden Ordner zur „Wandler-Station" machen: was hineinkommt, wird automatisch umgewandelt (z. B. `~/Desktop/→JPG`). Für Leute, die gar nicht ziehen wollen.

### D. Zwischenablage-Wandler
Kopiertes Bild/Text im Menüleisten-Menü direkt umwandeln: Screenshot in Zwischenablage → „als JPG < 500 KB in die Zwischenablage". Kein Datei-Umweg.

### E. Vorher/Nachher-Blick
Beim Verweilen auf einem Bild-Sektor zeigt das Zentrum eine Lupe mit Vorher/Nachher-Schieber in der gewählten Qualität. So sieht man Kompressionsartefakte, bevor man loslässt.

### F. Zielgröße statt Qualität
Statt „Qualität 78 %" fragt der Ring „Wie groß darf es sein?" mit Stufen 500 KB / 2 MB / 10 MB / 25 MB (Mail-Grenze). Die Engine findet die passende Einstellung selbst (bei Video per 2-Pass).

**Empfehlung:** Ring als Kern (Phase 1 bis 3), Rezepte und Zielgröße früh dazu, weil sie wenig kosten und viel bringen. Notch-Portal als Wow-Feature für den Launch. Magische Ordner und Zwischenablage später.

---

## 4. Erster Start und Einstellungen

- **Onboarding in 3 Schritten, interaktiv:** eine Beispieldatei liegt im Fenster, man zieht sie und probiert den Ring direkt aus. Danach die Rechteabfrage (Mac: Bedienungshilfen/Eingabeüberwachung) mit Erklärung, *warum*.
- **Einstellungen** in diesen Bereichen:
  - *Allgemein:* Darstellung **Klassisch** (dunkel, kontrastreich) oder **Glas** (Liquid Glass, auf dem Mac mit echtem `NSGlassEffectView`), Klick-Ton, Bewegung reduzieren.
  - *Auslöser:* Taste, Schütteln, Notch-Portal, Hotkey.
  - *Ring anpassen:* Sektoren pro Dateityp per Drag & Drop umsortieren, Rezepte.
  - *Ausgabe:* Ort, Namensschema `Name (JPG).jpg` oder `Name.jpg`.
  - *Module:* installierte Engines, Speicherbedarf.
  - *KI (lokal):* optionale Anbindung an ein **lokales Ollama**, siehe unten.

### KI über lokales Ollama (optional)

Wer Ollama installiert hat, kann es in den Einstellungen verbinden (Standard `http://localhost:11434`, Knopf „Verbindung testen", Modellauswahl aus `/api/tags`). Ohne Ollama funktioniert Orbit vollständig, nur die KI-Aktionen fehlen im Ring.

| KI-Aktion | Für | Beispiel |
|---|---|---|
| Sinnvoll umbenennen | alle | `IMG_4821.HEIC` → `2026-10-18 Sonnenuntergang Kochelsee.jpg` |
| Bildbeschreibung / Alt-Text | Bilder | Text in Zwischenablage oder als Metadaten |
| Zusammenfassen | PDF, Text, Transkript | `Vertrag (Zusammenfassung).txt` |
| Übersetzen | Text, Untertitel | SRT Deutsch → Englisch |
| Befehl in eigenen Worten | alle | Beim „Mehr …"-Feld tippen: „als 800px WEBP ohne Metadaten" → Orbit baut daraus die Kette |

Regeln:
- Nur Modelle auf dem eigenen Rechner. Zeigt die Server-Adresse auf einen anderen Rechner, warnt Orbit deutlich: „Dateiinhalte würden das Gerät verlassen."
- KI-Aktionen sind im Ring gekennzeichnet (Funken-Symbol), damit klar ist, dass das Ergebnis nicht exakt reproduzierbar ist.
- Die eingebauten Funktionen (OCR, Untertitel per whisper.cpp) brauchen kein Ollama.

## 5. Barrierefreiheit

- Komplette Bedienung per Tastatur über den Hotkey (Pfeiltasten wählen, Zahlen 1 bis 6, Enter, Esc).
- VoiceOver/Narrator-Beschriftung jedes Sektors inklusive Größenschätzung.
- „Bewegung reduzieren" respektieren: Ring blendet nur ein, keine Federn.
- Kontraste nach WCAG AA, Kategorie nicht nur über Farbe, sondern auch über Symbol.

## 6. Visuelle Sprache

- Glas-Optik passend zum System (macOS Vibrancy / Windows Mica-Acrylic), Ring in Dunkel und Hell.
- Kategoriefarben: Bild = Violett, Video = Koralle, Audio = Grün, Dokument = Blau, Daten = Gelb, Absicht/Rezept = Weiß mit Akzent.
- Typografie: Systemschrift (SF Pro / Segoe UI Variable), Formatnamen in fett und groß, Nutzen klein darunter.
- 120 fps auf ProMotion-Displays als Ziel, Ring erscheint in unter 80 ms nach Auslösung.
