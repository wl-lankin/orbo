# Änderungen

## 1.1.3

- Windows: Das Ablegen auf einem Feld im Ring startet jetzt die Umwandlung. Vorher kam das Loslassen beim Ring nicht an.

## 1.1.2

- Windows: Der Ring erscheint ohne Fensterrahmen und Titelleiste und blitzt beim Öffnen nicht mehr weiß auf.

## 1.1.1

- Windows: Einrichtungsassistent und Einstellungen zeigen nur noch Windows-Tasten (Alt, Strg, Umschalt) und keine Mac-Texte mehr.
- Windows: In der Apps-Liste steht jetzt „Wolfgang Linz“ als Herausgeber.

## 1.1.0

- „Lautheit“ heißt jetzt „Lautstärke angleichen“.
- Markdown nach HTML funktioniert jetzt auch unter Windows.
- Einstellungen: Illustrationen für „Ordner“ und „Abläufe“.
- Mehrere Dateien zu einer: Ziehst du mehrere PDFs, Bilder oder Audiodateien auf Orbo, bietet der Ring „Zusammenführen“ (PDFs, auch mit Bildern), „Als ein PDF“ (ein Bild pro Seite, Originalgröße oder A4) und „Aneinanderhängen“ (Audio) an. Die Reihenfolge ergibt sich aus den Dateinamen („Seite 2“ vor „Seite 10“). Auch in `orbo convert` und im MCP-Server verfügbar.
- Das Kommandozeilen-Tool `orbo` gehört jetzt zur App und lässt sich in den Einstellungen einrichten.
- `orbo mcp`: MCP-Server für Claude Code, Claude Desktop und Codex (Dateien analysieren und umwandeln). Einrichtung per Klick in den Einstellungen.
- Das Kommandozeilen-Tool nutzt die Abläufe und KI-Ziele aus den Einstellungen der App.

## 1.0.1

- Windows: Der Installer fragt jetzt, ob Orbo nur für dich (ohne Admin-Rechte, Updates laufen still) oder für alle Benutzer unter „Programme“ installiert wird. Bei „für alle Benutzer“ fragt Windows bei Updates nach Admin-Rechten.
- Windows-Installer auf Deutsch.
- Windows: Der dritte Schritt der Einführung richtet jetzt das Rechtsklick-Menü im Explorer ein, statt die macOS-Ordnerfreigabe zu zeigen.

## 1.0.0

- Erste öffentliche Version von Orbo für macOS (Apple Silicon) und Windows.
- Automatische Updates: Orbo prüft regelmäßig auf neue Versionen und installiert sie, sobald gerade nichts umgewandelt wird.
