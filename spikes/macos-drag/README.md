# Machbarkeitstest macOS (Phase 0)

Kleine Menüleisten-App (◎), die beweist, dass der Ring unter macOS funktioniert: Datei-Drag systemweit erkennen, Auslöser per Taste oder Schütteln, Ring-Fenster an der Maus, Drop empfangen, echte Umwandlung.

Bewusst in **Swift** statt Rust: Geprüft wird das Verhalten von macOS, nicht die Sprache. Der Mechanismus wird später 1:1 nach Rust (`objc2`) übertragen.

## Starten

```bash
./build.sh
open build/OrbitSpike.app
```

Menü ◎ in der Menüleiste: Darstellung (Klassisch / Glas), Auslöser-Taste, Schütteln an/aus, „Dateien früh lesen", Log öffnen.
Log: `~/Library/Logs/OrbitSpike.log`

## Wie es funktioniert

| Baustein | Umsetzung | Rechte nötig? |
|---|---|---|
| Maustaste gedrückt? | Abfrage `NSEvent.pressedMouseButtons`, 30 Hz im Leerlauf, 120 Hz bei gedrückter Taste | **keine** |
| Datei-Drag läuft? | `changeCount` der Drag-Zwischenablage (`NSPasteboard(name: .drag)`) ändert sich, Typen enthalten `fileURL` | **keine** |
| Taste gedrückt? | `NSEvent.modifierFlags` (Hardware-Zustand) | **keine** |
| Schütteln | 3 Richtungswechsel ≥ 18 pt in 0,7 s | **keine** |
| Ring | vorgewärmtes, nicht aktivierendes `NSPanel` über allen Fenstern und Spaces | - |
| Drop | Ring-View ist normales `NSDraggingDestination`, meldet `.copy` | - |
| Umwandlung | macOS-eigene Werkzeuge: `sips`, `avconvert`, `afconvert`, `textutil`, `ditto` | - |
| Glas-Stil | `NSGlassEffectView` (macOS 26+), sonst `NSVisualEffectView` | - |

**Wichtigste Erkenntnis:** Der Ansatz kommt ganz ohne Bedienungshilfen- oder Eingabeüberwachungs-Recht aus. Das macht das Onboarding viel einfacher als befürchtet.

## Automatisch getestet (18.10.2026, macOS 27.0.1, MacBook 1710×1107 @2x)

Mit einer Test-Quell-App, die per synthetischen Mausereignissen eine Datei zieht wie der Finder:

| Test | Ergebnis |
|---|---|
| Datei-Drag erkennen | ✅ 100 bis 135 ms nach Mausklick (= Startschwelle des Drags) |
| Dateien vor dem Drop lesen | ✅ keine Datenschutz-Abfrage, Pfade sofort bekannt |
| Auslöser ⌥ | ✅ 4 von 4 |
| Auslöser Schütteln | ✅ 3 von 3 |
| Ring sichtbar nach Auslöser | ✅ 2 bis 4 ms, beim ersten Mal nach Start 8 bis 37 ms (Ziel war < 100 ms) |
| Drop auf Feld, Umwandlung | ✅ PNG→JPG, PNG→PDF, AIFF→M4A, AIFF→ALAC, je 30 bis 80 ms |
| Stil Klassisch und Glas | ✅ beide gerendert |

**Gefundener und behobener Fehler:** Die Quell-App leert die Drag-Zwischenablage zuerst und schreibt danach. Wer genau dazwischen liest, sieht eine leere Typliste und hält den Drag fälschlich für „keine Dateien". Jetzt wird bei leerer Liste weiter gewartet. Das wäre im Alltag sporadisch aufgetreten.

## Noch von Hand zu testen

Synthetische Drags aus dem echten Finder habe ich bewusst nicht gemacht, damit keine echten Dateien verschoben werden. Bitte einmal durchgehen und Ergebnis eintragen:

| # | Test | Erwartung | Ergebnis |
|---|---|---|---|
| 1 | Datei aus Finder (Symbolansicht) ziehen, ⌥ drücken | Ring erscheint, Felder passen zum Typ | |
| 2 | dasselbe aus Listen- und Spaltenansicht | wie 1 | |
| 3 | Datei vom Schreibtisch | wie 1 | |
| 4 | mehrere Dateien gleichen Typs | Ring für alle, jede wird umgewandelt | |
| 5 | Ordner ziehen | nur ZIP | |
| 6 | Drop auf Feld | neue Datei neben Original, Ton „Glass", ◎ ✓ | |
| 7 | Ring offen, weit wegziehen (> 300 pt) | Ring verschwindet, Drag geht normal weiter | |
| 8 | Ring offen, in die **Ecke** des Ring-Fensters loslassen | Datei landet beim Fenster darunter (Transparenz lässt durch?) | |
| 9 | ohne ⌥ ziehen | nichts passiert, normales Verhalten | |
| 10 | ⌥ schon vor dem Klick halten | Ring kommt direkt beim Ziehen | |
| 11 | App im Vollbild (z. B. Safari), Datei aus Finder-Fenster daneben | Ring erscheint über dem Vollbild | |
| 12 | zweiter Bildschirm (falls vorhanden) | Ring auf dem richtigen Bildschirm | |
| 13 | Bild aus Safari, Anhang aus Mail ziehen | dokumentieren was passiert | |
| 14 | Text markieren und ziehen | **kein** Ring (Log: „Drag ohne Dateien") | |
| 15 | Aktivitätsanzeige: CPU im Leerlauf | < 0,5 % | |
| 16 | Datei auf Schreibtisch/Dokumente umwandeln | einmalige Ordner-Rechtefrage von macOS | |

## Offen für den Rust-Port

- Punkt 8 (Durchlässigkeit der Ecken) entscheidet, ob das Fenster eine runde Form bekommen muss.
- Esc zum Abbrechen braucht globale Tastenereignisse (= Eingabeüberwachung). Bisher reicht „weit wegziehen". Entscheidung, ob Esc das Recht wert ist.
- 30-Hz-Abfrage im Leerlauf: Stromverbrauch auf Akku messen. Alternative: globaler Monitor für `leftMouseDown` (braucht laut Doku kein Recht für Mausereignisse).
