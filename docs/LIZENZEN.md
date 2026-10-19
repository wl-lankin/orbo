# Lizenzen und Verkauf (verworfen)

> **Entscheidung 18.10.2026:** Orbo ist komplett kostenlos, mit freiwilligem Link „Orbo unterstützen“ (`SUPPORT_URL` in `core/crates/orbo-app/src/commands.rs`). Grund: Filee (filee.sh) bietet dieselbe Grundidee kostenlos und als Open Source an; Einnahmen über Pro wären schwer zu rechtfertigen, Orbo dient als Aushängeschild.
>
> Der Baustein `core/crates/orbo-license` (Offline-Schlüssel mit Ed25519) bleibt im Projekt, ist aber nicht in die App eingebunden. Der private Signierschlüssel liegt weiter in `~/.orbo/license-signing-key`. Die folgende Beschreibung gilt nur, falls später doch ein Bezahlmodell kommt.


## Modell

- **Gratis:** alle Format-Umwandlungen, Ring, Notch, Toast, beide Stile.
- **Orbo Pro:** 24,99 € einmalig (Startpreis 19,99 €), 1 Jahr Updates inklusive, läuft danach für immer weiter. Verlängerung der Updates optional (ca. 12 €/Jahr).
- **Testphase:** 14 Tage alle Pro-Funktionen ab dem ersten Start.
- Pro-Funktionen sind im Katalog markiert (`pro: true` in `core/crates/orbo-core/catalog/targets.yaml`), dazu: Optionsfenster („…“) und mehrere Dateien auf einmal.

## Wie die Prüfung funktioniert

Ein Lizenzschlüssel ist `ORBO-<Nutzdaten>.<Signatur>`: JSON mit Name, E-Mail, Plan, Ausstellungsdatum und `updates_until`, signiert mit Ed25519. Die App kennt nur den öffentlichen Schlüssel (`core/crates/orbo-license/src/public_key.txt`) und prüft komplett offline. Pro gilt für alle Orbo-Builds, deren Erscheinungsdatum (`ORBO_RELEASE_DATE`, beim Bauen gesetzt) nicht nach `updates_until` liegt.

## Schlüssel ausstellen (von Hand)

```bash
cd core
cargo run -p orbo-license -- issue --name "Max Mustermann" --email max@example.com            # 1 Jahr Updates
cargo run -p orbo-license -- issue --name "…" --email "…" --years 2
cargo run -p orbo-license -- verify ORBO-…
```

Der **private Signierschlüssel** liegt in `~/.orbo/license-signing-key` (nur für dich lesbar, nicht im Projekt).
**Unbedingt sichern** (Passwortmanager oder verschlüsseltes Backup). Geht er verloren, kann man keine passenden Schlüssel mehr ausstellen; wird er bekannt, kann jeder Schlüssel fälschen.

## Verkauf automatisieren (nächster Schritt)

1. Konto bei **Paddle** oder **Lemon Squeezy** anlegen (beide sind „Merchant of Record“: sie verkaufen im eigenen Namen, kümmern sich um EU-Umsatzsteuer, Rechnungen, Erstattungen).
2. Produkt „Orbo Pro“ anlegen, Preis 24,99 €.
3. Nach dem Kauf schickt der Anbieter einen **Webhook** an eine kleine Funktion (z. B. Cloudflare Worker oder Vercel Function), die mit dem privaten Schlüssel (als Secret hinterlegt) einen Lizenzschlüssel signiert und per Mail verschickt. Die Signierlogik ist dieselbe wie in `orbo-license` (Ed25519 über das JSON).
4. Die Kaufseite (`BUY_URL` in `core/crates/orbo-app/src/license.rs`, derzeit `https://wolfgang-linz.de/orbo`) auf den Checkout-Link setzen.

Die App selbst braucht dafür keine Änderung und bleibt ohne Internetzugriff.
